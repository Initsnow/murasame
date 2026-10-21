use anyhow::{Result, anyhow};
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::{
    ai::{
        agent::builtins::BuiltinTools,
        event::AgentStreamEvent,
        hermes::{HermesClient, HermesContentPart, HermesInput, HermesInputMessage},
        mcp::McpManager,
        skills::SkillManager,
    },
    config::AgentConfig,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedToolCall {
    pub name: String,
    pub arguments: Value,
    pub raw_arguments: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FilterEvent {
    Token(String),
    Thought(String),
}

pub struct ToolCallStreamFilter {
    buffer: String,
    emitted_thought_len: usize,
    emitted_token_idx: usize,
    is_tool_turn: bool,
}

impl ToolCallStreamFilter {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            emitted_thought_len: 0,
            emitted_token_idx: 0,
            is_tool_turn: false,
        }
    }

    pub fn process_chunk(&mut self, chunk: &str) -> Vec<FilterEvent> {
        self.buffer.push_str(chunk);
        let mut events = Vec::new();

        // 1. Tool call detection
        if self.buffer.contains("<client_tool_call") {
            self.is_tool_turn = true;
        }

        // 2. Thought extraction: check for <thought> or <think>
        let thought_open = if let Some(idx) = self.buffer.find("<thought>") {
            Some((idx, idx + "<thought>".len(), "</thought>"))
        } else if let Some(idx) = self.buffer.find("<think>") {
            Some((idx, idx + "<think>".len(), "</think>"))
        } else {
            None
        };

        let mut thought_closed_idx = None;

        if let Some((_tag_start, content_start, close_tag)) = thought_open {
            if let Some(close_pos) = self.buffer[content_start..].find(close_tag) {
                let content_end = content_start + close_pos;
                thought_closed_idx = Some(content_end + close_tag.len());
                let thought_str = &self.buffer[content_start..content_end];
                if thought_str.len() > self.emitted_thought_len {
                    let new_part = &thought_str[self.emitted_thought_len..];
                    events.push(FilterEvent::Thought(new_part.to_string()));
                    self.emitted_thought_len = thought_str.len();
                }
            } else {
                let content = &self.buffer[content_start..];
                let safe_len = if let Some(last_lt) = content.rfind('<') {
                    if last_lt + close_tag.len() >= content.len() {
                        last_lt
                    } else {
                        content.len()
                    }
                } else {
                    content.len()
                };

                if safe_len > self.emitted_thought_len {
                    let new_part = &content[self.emitted_thought_len..safe_len];
                    events.push(FilterEvent::Thought(new_part.to_string()));
                    self.emitted_thought_len = safe_len;
                }
            }
        }

        // 3. Token extraction: conversational answer (only if not a tool turn)
        if !self.is_tool_turn {
            let natural_start = if thought_open.is_some() {
                thought_closed_idx
            } else {
                Some(0)
            };

            if let Some(start_idx) = natural_start {
                if start_idx >= self.emitted_token_idx {
                    self.emitted_token_idx = start_idx;
                }
                let available = &self.buffer[self.emitted_token_idx..];
                let safe_token_len = if let Some(last_lt) = available.rfind('<') {
                    let tail = &available[last_lt..];
                    if "<client_tool_call".starts_with(tail)
                        || "<thought".starts_with(tail)
                        || "<think".starts_with(tail)
                    {
                        last_lt
                    } else {
                        available.len()
                    }
                } else {
                    available.len()
                };

                if safe_token_len > 0 {
                    let new_tok = &available[..safe_token_len];
                    events.push(FilterEvent::Token(new_tok.to_string()));
                    self.emitted_token_idx += safe_token_len;
                }
            }
        }

        events
    }

    pub fn finish(self) -> (Vec<FilterEvent>, Vec<ParsedToolCall>, String) {
        let mut tool_calls = Vec::new();
        let re = regex::Regex::new(
            r#"(?s)<client_tool_call\s+name=["']?([^"'>\s]+)["']?\s*>(.*?)</client_tool_call>"#,
        )
        .unwrap();

        for cap in re.captures_iter(&self.buffer) {
            let name = cap[1].to_string();
            let raw_args = cap[2].trim().to_string();
            let args_val = serde_json::from_str::<Value>(&raw_args)
                .unwrap_or_else(|_| serde_json::json!({}));
            tool_calls.push(ParsedToolCall {
                name,
                arguments: args_val,
                raw_arguments: raw_args,
            });
        }

        let is_tool = !tool_calls.is_empty() || self.is_tool_turn;
        let mut remaining_events = Vec::new();

        if !is_tool {
            let natural_start = if let Some(idx) = self.buffer.find("</thought>") {
                Some(idx + "</thought>".len())
            } else if let Some(idx) = self.buffer.find("</think>") {
                Some(idx + "</think>".len())
            } else if !self.buffer.contains("<thought") && !self.buffer.contains("<think") {
                Some(0)
            } else {
                None
            };

            if let Some(start_idx) = natural_start {
                let effective_start = self.emitted_token_idx.max(start_idx);
                if effective_start < self.buffer.len() {
                    let tail = &self.buffer[effective_start..];
                    if !tail.is_empty() {
                        remaining_events.push(FilterEvent::Token(tail.to_string()));
                    }
                }
            }
        }

        (remaining_events, tool_calls, self.buffer)
    }
}

pub struct HermesRunner {
    hermes_client: HermesClient,
    agent_config: AgentConfig,
    instructions: String,
    session_id: String,
    max_steps: usize,
    is_vision: bool,
}

impl HermesRunner {
    pub fn new(
        hermes_client: HermesClient,
        agent_config: &AgentConfig,
        system_prompt: String,
        session_id: String,
        client_context: String,
        is_vision: bool,
    ) -> Self {
        let mut instructions = system_prompt;
        if !client_context.is_empty() {
            if !instructions.is_empty() {
                instructions.push_str("\n\n");
            }
            instructions.push_str(&client_context);
        }
        let max_steps = agent_config.max_steps.max(1);

        Self {
            hermes_client,
            agent_config: agent_config.clone(),
            instructions,
            session_id,
            max_steps,
            is_vision,
        }
    }

    pub async fn run_turn(
        &self,
        initial_prompt: &str,
        last_response_id: Option<String>,
        skills: &SkillManager,
        mcp_manager: &mut McpManager,
        event_sender: UnboundedSender<AgentStreamEvent>,
    ) -> Result<(String, String, Option<String>)> {
        let mut step = 0;
        let mut current_input = HermesInput::Text(initial_prompt.to_string());
        let mut prev_resp_id = last_response_id;
        let mut composite_accumulator = String::new();
        let mut final_response_text = String::new();

        while step < self.max_steps {
            step += 1;

            let (turn_tx, mut turn_rx) =
                tokio::sync::mpsc::unbounded_channel::<AgentStreamEvent>();
            let client = self.hermes_client.clone();
            let instructions = if self.instructions.trim().is_empty() {
                None
            } else {
                Some(self.instructions.clone())
            };

            let input_ref = current_input.clone();
            let session_key = self.session_id.clone();
            let prev_id = prev_resp_id.clone();

            let req_handle = tokio::spawn(async move {
                client
                    .ask_stream_events_input(
                        &input_ref,
                        instructions.as_deref(),
                        Some(&session_key),
                        prev_id.as_deref(),
                        turn_tx,
                    )
                    .await
            });

            let mut filter = ToolCallStreamFilter::new();
            let mut turn_new_resp_id: Option<String> = None;

            while let Some(evt) = turn_rx.recv().await {
                match evt {
                    AgentStreamEvent::ResponseId(id) => {
                        turn_new_resp_id = Some(id.clone());
                        let _ = event_sender.send(AgentStreamEvent::ResponseId(id));
                    }
                    AgentStreamEvent::Token(delta) => {
                        let filter_evts = filter.process_chunk(&delta);
                        for fe in filter_evts {
                            match fe {
                                FilterEvent::Token(tok) => {
                                    let _ = event_sender.send(AgentStreamEvent::Token(tok));
                                }
                                FilterEvent::Thought(th) => {
                                    let _ = event_sender.send(AgentStreamEvent::Thought(th));
                                }
                            }
                        }
                    }
                    AgentStreamEvent::Thought(th) => {
                        let _ = event_sender.send(AgentStreamEvent::Thought(th));
                    }
                    AgentStreamEvent::Status(st) => {
                        let _ = event_sender.send(AgentStreamEvent::Status(st));
                    }
                    AgentStreamEvent::Cancelled => {
                        let _ = event_sender.send(AgentStreamEvent::Cancelled);
                        return Err(anyhow!("Hermes turn was cancelled"));
                    }
                    AgentStreamEvent::Failed(err) => {
                        let _ = event_sender.send(AgentStreamEvent::Failed(err.clone()));
                        return Err(anyhow!("Hermes response failed: {err}"));
                    }
                    _ => {}
                }
            }

            let ask_result = req_handle
                .await
                .map_err(|e| anyhow!("Hermes request task join error: {e}"))??;

            if let Some(id) = ask_result.1 {
                turn_new_resp_id = Some(id);
            }
            if let Some(ref nid) = turn_new_resp_id {
                prev_resp_id = Some(nid.clone());
            }

            let (remaining_events, tool_calls, raw_turn_text) = filter.finish();
            for fe in remaining_events {
                match fe {
                    FilterEvent::Token(tok) => {
                        let _ = event_sender.send(AgentStreamEvent::Token(tok));
                    }
                    FilterEvent::Thought(th) => {
                        let _ = event_sender.send(AgentStreamEvent::Thought(th));
                    }
                }
            }

            if !composite_accumulator.is_empty() {
                composite_accumulator.push('\n');
            }
            composite_accumulator.push_str(&raw_turn_text);

            if tool_calls.is_empty() {
                // Converged on final conversational answer
                final_response_text = crate::ai::cue_parser::strip_tool_xml(&raw_turn_text);
                break;
            }

            // Execute intercepted tool calls
            let mut tool_results_xml = String::new();
            let mut collected_images = Vec::new();

            for call in tool_calls {
                let call_id = format!("call_{}", Uuid::new_v4().simple());
                let _ = event_sender.send(AgentStreamEvent::Status(format!(
                    "正在执行: {}...",
                    call.name
                )));
                let _ = event_sender.send(AgentStreamEvent::ToolCallStart {
                    id: call_id.clone(),
                    name: call.name.clone(),
                    input: call.raw_arguments.clone(),
                });

                let (output_str, is_err) = if BuiltinTools::is_builtin(&call.name) {
                    match BuiltinTools::execute(
                        &call.name,
                        &call.arguments,
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
                    match mcp_manager.execute_tool(&call.name, &call.arguments).await {
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
                    id: call_id,
                    name: call.name.clone(),
                    output: output_str.clone(),
                    is_error: is_err,
                });
                let _ = event_sender.send(AgentStreamEvent::Status(String::new()));

                let status_header = if is_err {
                    "[执行失败]"
                } else {
                    "[执行成功]"
                };
                let result_block = format!(
                    "<client_tool_result name=\"{}\">\n{}\n{}\n</client_tool_result>\n",
                    call.name, status_header, output_str
                );
                tool_results_xml.push_str(&result_block);
            }

            composite_accumulator.push('\n');
            composite_accumulator.push_str(&tool_results_xml);

            // Next turn feedback
            if collected_images.is_empty() {
                current_input = HermesInput::Text(tool_results_xml);
            } else {
                let mut content = vec![HermesContentPart::Text {
                    text: "Done.".to_string(),
                }];
                for img in collected_images {
                    content.push(HermesContentPart::Image {
                        image_url: format!("data:{};base64,{}", img.mime_type, img.base64),
                    });
                }
                current_input = HermesInput::Structured(vec![HermesInputMessage {
                    role: "user".to_string(),
                    content,
                }]);
            }
        }

        if final_response_text.is_empty() && step >= self.max_steps {
            final_response_text =
                format!("已达到最大推理步数限制 {} 轮，执行终止。", self.max_steps);
            composite_accumulator.push_str(&format!("\n{}", final_response_text));
        }

        Ok((composite_accumulator, final_response_text, prev_resp_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_call_stream_filter_natural_text() {
        let mut filter = ToolCallStreamFilter::new();
        let evts1 = filter.process_chunk("你好呀！");
        let evts2 = filter.process_chunk("[smile] 今天想做些什么呢？");
        let (remaining, tool_calls, _) = filter.finish();

        let mut all_tokens = Vec::new();
        for fe in evts1.into_iter().chain(evts2).chain(remaining) {
            if let FilterEvent::Token(t) = fe {
                all_tokens.push(t);
            }
        }
        assert_eq!(all_tokens.join(""), "你好呀！[smile] 今天想做些什么呢？");
        assert!(tool_calls.is_empty());
    }

    #[test]
    fn test_tool_call_stream_filter_thought_and_text() {
        let mut filter = ToolCallStreamFilter::new();
        let evts1 = filter.process_chunk("<thought>先思考一下</thought>");
        let evts2 = filter.process_chunk("你好呀！");
        let (remaining, tool_calls, _) = filter.finish();

        let mut thoughts = Vec::new();
        let mut tokens = Vec::new();

        for fe in evts1.into_iter().chain(evts2).chain(remaining) {
            match fe {
                FilterEvent::Thought(th) => thoughts.push(th),
                FilterEvent::Token(tok) => tokens.push(tok),
            }
        }

        assert_eq!(thoughts.join(""), "先思考一下");
        assert_eq!(tokens.join(""), "你好呀！");
        assert!(tool_calls.is_empty());
    }

    #[test]
    fn test_tool_call_stream_filter_tool_call() {
        let mut filter = ToolCallStreamFilter::new();
        let evts1 = filter.process_chunk("<client_tool_call name=\"read_file\">\n");
        let evts2 = filter.process_chunk("{\"path\": \"config.toml\"}\n");
        let evts3 = filter.process_chunk("</client_tool_call>");
        let (remaining, tool_calls, _) = filter.finish();

        let mut all_tokens = Vec::new();
        for fe in evts1.into_iter().chain(evts2).chain(evts3).chain(remaining) {
            if let FilterEvent::Token(t) = fe {
                all_tokens.push(t);
            }
        }

        // Must suppress tool call XML from being emitted as natural tokens
        assert!(all_tokens.is_empty());
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].name, "read_file");
        assert_eq!(tool_calls[0].arguments["path"], "config.toml");
    }

    #[test]
    fn test_tool_call_stream_filter_chunked_tag() {
        let mut filter = ToolCallStreamFilter::new();
        let evts1 = filter.process_chunk("<cl");
        let evts2 = filter.process_chunk("ient_tool_call name=\"get_current_time\">");
        let evts3 = filter.process_chunk("{}\n</client_tool_call>");
        let (remaining, tool_calls, _) = filter.finish();

        let mut all_tokens = Vec::new();
        for fe in evts1.into_iter().chain(evts2).chain(evts3).chain(remaining) {
            if let FilterEvent::Token(t) = fe {
                all_tokens.push(t);
            }
        }

        assert!(all_tokens.is_empty());
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].name, "get_current_time");
    }

    #[test]
    fn test_tool_call_stream_filter_multiple_calls() {
        let mut filter = ToolCallStreamFilter::new();
        let _ = filter.process_chunk("<client_tool_call name=\"get_current_time\">{}</client_tool_call>\n<client_tool_call name=\"read_file\">{\"path\":\"a.txt\"}</client_tool_call>");
        let (remaining, tool_calls, _) = filter.finish();

        assert!(remaining.is_empty());
        assert_eq!(tool_calls.len(), 2);
        assert_eq!(tool_calls[0].name, "get_current_time");
        assert_eq!(tool_calls[1].name, "read_file");
        assert_eq!(tool_calls[1].arguments["path"], "a.txt");
    }

    #[test]
    fn test_tool_call_stream_filter_normal_angle_bracket() {
        let mut filter = ToolCallStreamFilter::new();
        let evts1 = filter.process_chunk("10 < 20 并且 30 > 15");
        let (remaining, tool_calls, _) = filter.finish();

        let mut all_tokens = Vec::new();
        for fe in evts1.into_iter().chain(remaining) {
            if let FilterEvent::Token(t) = fe {
                all_tokens.push(t);
            }
        }
        assert_eq!(all_tokens.join(""), "10 < 20 并且 30 > 15");
        assert!(tool_calls.is_empty());
    }
}
