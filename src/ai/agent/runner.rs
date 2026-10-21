use anyhow::{Result, anyhow};
use async_openai::{
    Client,
    config::OpenAIConfig,
    types::chat::{
        ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls,
        ChatCompletionRequestAssistantMessageArgs,
        ChatCompletionRequestMessage,
        ChatCompletionRequestMessageContentPartImageArgs,
        ChatCompletionRequestMessageContentPartTextArgs,
        ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestToolMessageArgs,
        ChatCompletionRequestUserMessageArgs,
        ChatCompletionRequestUserMessageContent,
        ChatCompletionRequestUserMessageContentPart,
        ChatCompletionTool, ChatCompletionTools,
        FunctionCall, FunctionObject, ImageUrl,
    },
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use tokio::sync::mpsc::UnboundedSender;

use super::{
    builtins::BuiltinTools,
    hermes_runner::{FilterEvent, ToolCallStreamFilter},
};
use crate::{
    ai::{
        event::AgentStreamEvent,
        mcp::McpManager,
        skills::{SkillManager, render_skills_prompt},
    },
    config::{AgentConfig, AiConfig},
    db::ChatMessage,
};

#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)]
struct OpenAiFunctionCallChunk {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)]
struct OpenAiToolCallChunk {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    function: Option<OpenAiFunctionCallChunk>,
}

#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)]
struct OpenAiStreamDelta {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(alias = "reasoning", default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAiToolCallChunk>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OpenAiStreamChoice {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    delta: OpenAiStreamDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OpenAiStreamUsage {
    #[serde(default)]
    prompt_tokens: Option<u32>,
    #[serde(default)]
    completion_tokens: Option<u32>,
    #[serde(default)]
    total_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OpenAiStreamChunk {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    choices: Vec<OpenAiStreamChoice>,
    #[serde(default)]
    usage: Option<OpenAiStreamUsage>,
}

pub struct AgentRunner {
    client: Client<OpenAIConfig>,
    model: String,
    system_prompt: String,
    max_steps: usize,
    agent_config: AgentConfig,
    thinking_enabled: bool,
    thinking_intensity: String,
    is_vision: bool,
}

impl AgentRunner {
    pub fn new(
        ai_config: &AiConfig,
        agent_config: &AgentConfig,
        base_system_prompt: String,
        skills: &SkillManager,
    ) -> Self {
        let raw_url = if ai_config.openai_base_url.is_empty() {
            &ai_config.hermes_base_url
        } else {
            &ai_config.openai_base_url
        };
        let api_base = crate::config::normalize_api_base_url(raw_url);
        let openai_config = OpenAIConfig::default()
            .with_api_base(&api_base)
            .with_api_key(&ai_config.openai_api_key);

        let client = Client::with_config(openai_config);

        // Append progressive disclosure skills catalog to system prompt
        let mut full_system_prompt = base_system_prompt;
        if agent_config.skills_enabled && !skills.is_empty() {
            let skills_list = skills.list_skills();
            let skills_xml = render_skills_prompt(&skills_list);
            full_system_prompt.push_str(&skills_xml);
        }

        let is_vision = ai_config.is_vision_supported();

        Self {
            client,
            model: if ai_config.openai_model.is_empty() {
                "deepseek-flash".to_string()
            } else {
                ai_config.openai_model.clone()
            },
            system_prompt: full_system_prompt,
            max_steps: agent_config.max_steps.max(1),
            agent_config: agent_config.clone(),
            thinking_enabled: ai_config.thinking_enabled,
            thinking_intensity: ai_config.thinking_intensity.clone(),
            is_vision,
        }
    }

    fn build_tools(&self, mcp_manager: &McpManager) -> Vec<ChatCompletionTools> {
        let mut tools = BuiltinTools::get_definitions(&self.agent_config.builtin_tools, self.is_vision);

        // Convert MCP tools to OpenAI ChatCompletionTools
        for (_server_name, mcp_tool) in mcp_manager.list_all_tools() {
            tools.push(ChatCompletionTools::Function(ChatCompletionTool {
                function: FunctionObject {
                    name: mcp_tool.name.clone(),
                    description: mcp_tool.description.clone(),
                    parameters: Some(mcp_tool.input_schema.clone()),
                    strict: None,
                },
            }));
        }

        tools
    }

    pub async fn run_turn(
        &self,
        history: &[ChatMessage],
        user_prompt: &str,
        skills: &SkillManager,
        mcp_manager: &mut McpManager,
        event_sender: UnboundedSender<AgentStreamEvent>,
    ) -> Result<(String, Option<u32>)> {
        let mut messages: Vec<ChatCompletionRequestMessage> = Vec::new();

        // 1. Initial system prompt
        if let Ok(sys_msg) = ChatCompletionRequestSystemMessageArgs::default()
            .content(self.system_prompt.clone())
            .build()
        {
            messages.push(ChatCompletionRequestMessage::System(sys_msg));
        }

        // 2. Add history (strip internal XML tags so history sent to model is pristine natural language)
        for msg in history {
            if msg.role == "user" {
                if let Ok(u) = ChatCompletionRequestUserMessageArgs::default()
                    .content(msg.content.clone())
                    .build()
                {
                    messages.push(ChatCompletionRequestMessage::User(u));
                }
            } else if msg.role == "assistant" {
                let clean = crate::ai::cue_parser::strip_tool_xml(&msg.content);
                if !clean.is_empty() {
                    if let Ok(a) = ChatCompletionRequestAssistantMessageArgs::default()
                        .content(clean)
                        .build()
                    {
                        messages.push(ChatCompletionRequestMessage::Assistant(a));
                    }
                }
            }
        }

        // 3. Current user prompt
        if let Ok(u) = ChatCompletionRequestUserMessageArgs::default()
            .content(user_prompt.to_string())
            .build()
        {
            messages.push(ChatCompletionRequestMessage::User(u));
        }

        let mut step = 0;
        let mut final_response_text = String::new();
        let mut total_completion_tokens: Option<u32> = None;
        let mut executed_history: Vec<(String, String, String)> = Vec::new();
        let mut accumulated_all_thoughts = String::new();

        while step < self.max_steps {
            step += 1;
            let available_tools = self.build_tools(mcp_manager);

            let mut req_body = serde_json::json!({
                "model": self.model,
                "messages": messages,
                "stream": true,
                "stream_options": { "include_usage": true },
            });

            if !available_tools.is_empty() {
                req_body["tools"] = serde_json::to_value(&available_tools)
                    .map_err(|e| anyhow!("Failed to serialize tools: {e}"))?;
            }

            if self.thinking_enabled {
                let effort = match self.thinking_intensity.as_str() {
                    "low" => "low",
                    "high" => "high",
                    _ => "medium",
                };
                req_body["reasoning_effort"] = serde_json::json!(effort);
                req_body["enable_thinking"] = serde_json::json!(true);
            } else {
                req_body["enable_thinking"] = serde_json::json!(false);
            }

            // Using async-openai's Bring Your Own Types (byot) feature to capture reasoning_content
            let mut stream = self
                .client
                .chat()
                .create_stream_byot::<Value, OpenAiStreamChunk>(req_body)
                .await
                .map_err(|e| anyhow!("Failed to connect to LLM stream: {e}"))?;

            let mut filter = ToolCallStreamFilter::new();
            let mut current_turn_text = String::new();
            let mut step_thought_text = String::new();
            let mut pending_tool_calls: HashMap<usize, (String, String, String)> = HashMap::new();

            while let Some(chunk_res) = stream.next().await {
                match chunk_res {
                    Ok(stream_chunk) => {
                        if let Some(usage) = stream_chunk.usage {
                            if let Some(toks) = usage.completion_tokens {
                                total_completion_tokens = Some(toks);
                            }
                        }

                        for choice in stream_chunk.choices {
                            let delta = choice.delta;

                            // 1. Thinking / reasoning content (DashScope, DeepSeek, SiliconFlow, OpenRouter)
                            let reasoning = delta.reasoning_content;
                            if let Some(th) = reasoning {
                                if !th.is_empty() {
                                    step_thought_text.push_str(&th);
                                    let _ = event_sender.send(AgentStreamEvent::Thought(th));
                                }
                            }

                            // 2. Normal text content (may contain <thought> or <think> tags)
                            if let Some(content) = delta.content {
                                current_turn_text.push_str(&content);
                                let filter_evts = filter.process_chunk(&content);
                                for fe in filter_evts {
                                    match fe {
                                        FilterEvent::Token(tok) => {
                                            let _ = event_sender.send(AgentStreamEvent::Token(tok));
                                        }
                                        FilterEvent::Thought(th) => {
                                            step_thought_text.push_str(&th);
                                            let _ = event_sender.send(AgentStreamEvent::Thought(th));
                                        }
                                    }
                                }
                            }

                            // 3. Tool calls
                            if let Some(tool_chunks) = delta.tool_calls {
                                for tc in tool_chunks {
                                    let idx = tc.index;
                                    let entry = pending_tool_calls.entry(idx).or_insert_with(|| {
                                        (String::new(), String::new(), String::new())
                                    });
                                    if let Some(id) = tc.id {
                                        entry.0.push_str(&id);
                                    }
                                    if let Some(func) = tc.function {
                                        if let Some(name) = func.name {
                                            entry.1.push_str(&name);
                                        }
                                        if let Some(args) = func.arguments {
                                            entry.2.push_str(&args);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        return Err(anyhow!("Stream error during agent loop: {e}"));
                    }
                }
            }

            let (remaining_events, _, _) = filter.finish();
            for fe in remaining_events {
                match fe {
                    FilterEvent::Token(tok) => {
                        let _ = event_sender.send(AgentStreamEvent::Token(tok));
                    }
                    FilterEvent::Thought(th) => {
                        step_thought_text.push_str(&th);
                        let _ = event_sender.send(AgentStreamEvent::Thought(th));
                    }
                }
            }

            if !step_thought_text.is_empty() {
                if !accumulated_all_thoughts.is_empty() {
                    accumulated_all_thoughts.push_str("\n\n");
                }
                accumulated_all_thoughts.push_str(&step_thought_text);
            }

            // If no tool calls were requested, this is the final conversational turn
            if pending_tool_calls.is_empty() {
                final_response_text = current_turn_text;
                break;
            }

            // Convert accumulated tool chunks to complete tool calls
            let mut executed_tool_calls = Vec::new();
            let mut tool_results = Vec::new();
            let mut collected_images = Vec::new();

            let mut sorted_indices: Vec<usize> = pending_tool_calls.keys().cloned().collect();
            sorted_indices.sort();

            for idx in sorted_indices {
                if let Some((id, name, args_str)) = pending_tool_calls.remove(&idx) {
                    let call_id = if id.is_empty() {
                        format!("call_{}", uuid::Uuid::new_v4().simple())
                    } else {
                        id
                    };

                    let args_val: Value = serde_json::from_str(&args_str)
                        .unwrap_or_else(|_| serde_json::json!({}));

                    let _ =
                        event_sender.send(AgentStreamEvent::Status(format!("正在执行: {name}...")));
                    let _ = event_sender.send(AgentStreamEvent::ToolCallStart {
                        id: call_id.clone(),
                        name: name.clone(),
                        input: args_str.clone(),
                    });

                    // Execute tool (either built-in or via MCP manager)
                    let (output_str, is_err) = if BuiltinTools::is_builtin(&name) {
                        match BuiltinTools::execute(
                            &name,
                            &args_val,
                            skills,
                            &self.agent_config.security_level,
                            &self.agent_config.builtin_tools,
                            self.is_vision,
                        )
                        .await
                        {
                            Ok(res) => {
                                if let Some(img) = res.image {
                                    collected_images.push(img);
                                }
                                (res.display_text, false)
                            }
                            Err(e) => (format!("执行错误: {e}"), true),
                        }
                    } else {
                        match mcp_manager.execute_tool(&name, &args_val).await {
                            Ok(res) => {
                                if let Some(img) = res.extract_first_image() {
                                    collected_images.push(img);
                                }
                                (res.to_display_string(), res.is_error)
                            }
                            Err(e) => (format!("MCP 执行错误: {e}"), true),
                        }
                    };

                    let _ = event_sender.send(AgentStreamEvent::ToolCallDone {
                        id: call_id.clone(),
                        name: name.clone(),
                        output: output_str.clone(),
                        is_error: is_err,
                    });
                    let _ = event_sender.send(AgentStreamEvent::Status(String::new()));

                    executed_tool_calls.push(ChatCompletionMessageToolCalls::Function(
                        ChatCompletionMessageToolCall {
                            id: call_id.clone(),
                            function: FunctionCall {
                                name: name.clone(),
                                arguments: args_str.clone(),
                            },
                        },
                    ));

                    executed_history.push((name.clone(), args_str.clone(), output_str.clone()));
                    tool_results.push((call_id, output_str));
                }
            }

            // Append assistant tool-calls message to history
            let mut assistant_builder = ChatCompletionRequestAssistantMessageArgs::default();
            if !current_turn_text.is_empty() {
                assistant_builder.content(current_turn_text);
            }
            assistant_builder.tool_calls(executed_tool_calls);

            if let Ok(assistant_msg) = assistant_builder.build() {
                messages.push(ChatCompletionRequestMessage::Assistant(assistant_msg));
            }

            // Append each tool response
            for (call_id, output) in tool_results {
                if let Ok(tool_msg) = ChatCompletionRequestToolMessageArgs::default()
                    .tool_call_id(call_id)
                    .content(output)
                    .build()
                {
                    messages.push(ChatCompletionRequestMessage::Tool(tool_msg));
                }
            }

            // If any tool produced images, append a multimodal user message so the vision model actually receives it
            if !collected_images.is_empty() {
                let mut parts: Vec<ChatCompletionRequestUserMessageContentPart> = Vec::new();
                if let Ok(text_part) = ChatCompletionRequestMessageContentPartTextArgs::default()
                    .text("Done.")
                    .build()
                {
                    parts.push(ChatCompletionRequestUserMessageContentPart::Text(text_part));
                }
                for img in collected_images {
                    let data_url = format!("data:{};base64,{}", img.mime_type, img.base64);
                    if let Ok(img_part) = ChatCompletionRequestMessageContentPartImageArgs::default()
                        .image_url(ImageUrl {
                            url: data_url,
                            detail: None,
                        })
                        .build()
                    {
                        parts.push(ChatCompletionRequestUserMessageContentPart::ImageUrl(img_part));
                    }
                }
                if let Ok(user_msg) = ChatCompletionRequestUserMessageArgs::default()
                    .content(ChatCompletionRequestUserMessageContent::Array(parts))
                    .build()
                {
                    messages.push(ChatCompletionRequestMessage::User(user_msg));
                }
            }
        }

        let mut composite_text = String::new();
        if !accumulated_all_thoughts.is_empty() {
            composite_text.push_str(&format!(
                "<thought>\n{}\n</thought>\n",
                accumulated_all_thoughts.trim()
            ));
        }
        for (name, args, output) in executed_history {
            composite_text.push_str(&format!(
                "<client_tool_call name=\"{}\">\n{}\n</client_tool_call>\n<client_tool_result name=\"{}\">\n{}\n</client_tool_result>\n",
                name, args, name, output
            ));
        }
        composite_text.push_str(&final_response_text);

        Ok((composite_text, total_completion_tokens))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openai_stream_reasoning_deserialization() {
        // 1. DashScope / DeepSeek format: delta.reasoning_content
        let json_dashscope = r#"{
            "id": "chatcmpl-test-1",
            "choices": [{
                "index": 0,
                "delta": {
                    "content": "",
                    "reasoning_content": "正在深入分析问题..."
                },
                "finish_reason": null
            }]
        }"#;

        let chunk1: OpenAiStreamChunk = serde_json::from_str(json_dashscope).unwrap();
        assert_eq!(chunk1.choices.len(), 1);
        assert_eq!(
            chunk1.choices[0].delta.reasoning_content.as_deref(),
            Some("正在深入分析问题...")
        );
        assert_eq!(chunk1.choices[0].delta.content.as_deref(), Some(""));

        // 2. OpenRouter format: delta.reasoning
        let json_openrouter = r#"{
            "id": "chatcmpl-test-2",
            "choices": [{
                "index": 0,
                "delta": {
                    "reasoning": "OpenRouter reasoning tokens"
                }
            }]
        }"#;

        let chunk2: OpenAiStreamChunk = serde_json::from_str(json_openrouter).unwrap();
        assert_eq!(
            chunk2.choices[0].delta.reasoning_content.as_deref(),
            Some("OpenRouter reasoning tokens")
        );

        // 3. Tool call chunk
        let json_tool = r#"{
            "id": "chatcmpl-test-3",
            "choices": [{
                "index": 0,
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_123",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\": \"config.toml\"}"
                        }
                    }]
                }
            }]
        }"#;

        let chunk3: OpenAiStreamChunk = serde_json::from_str(json_tool).unwrap();
        let tools = chunk3.choices[0].delta.tool_calls.as_ref().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].id.as_deref(), Some("call_123"));
        assert_eq!(tools[0].function.as_ref().unwrap().name.as_deref(), Some("read_file"));
    }
}
