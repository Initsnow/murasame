use anyhow::{Result, anyhow};
use async_openai::{
    Client,
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
        ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
        ChatCompletionStreamOptions, CreateChatCompletionRequestArgs,
    },
};
use futures_util::StreamExt;
use std::sync::OnceLock;
use tokenizers::Tokenizer;
use tokio::sync::mpsc::UnboundedSender;

use crate::{config::AiConfig, db::ChatMessage};

static TOKENIZER: OnceLock<Tokenizer> = OnceLock::new();

fn get_tokenizer() -> &'static Tokenizer {
    TOKENIZER.get_or_init(|| {
        for path in [
            "assets/tokenizer.json",
            "tokenizer.json",
            "config/tokenizer.json",
        ] {
            if std::path::Path::new(path).exists() {
                if let Ok(tok) = Tokenizer::from_file(path) {
                    return tok;
                }
            }
        }
        let alphabet = tokenizers::pre_tokenizers::byte_level::ByteLevel::alphabet();
        let mut vocab = tokenizers::models::bpe::Vocab::default();
        for (i, c) in alphabet.into_iter().enumerate() {
            vocab.insert(c.to_string(), i as u32);
        }
        let bpe = tokenizers::models::bpe::BPE::builder()
            .vocab_and_merges(vocab, vec![])
            .build()
            .unwrap_or_default();
        let mut tokenizer = Tokenizer::new(bpe);
        tokenizer.with_pre_tokenizer(Some(
            tokenizers::pre_tokenizers::byte_level::ByteLevel::default(),
        ));
        tokenizer
    })
}

pub fn count_tokens(text: &str, _model: &str) -> i64 {
    if text.is_empty() {
        return 0;
    }
    let tok = get_tokenizer();
    match tok.encode(text, false) {
        Ok(encoding) => encoding.get_ids().len() as i64,
        Err(_) => (text.chars().count() / 2).max(1) as i64,
    }
}

pub struct LlmClient {
    client: Client<OpenAIConfig>,
    model: String,
    system_prompt: String,
    max_context_tokens: usize,
    compression_strategy: String,
}

impl LlmClient {
    pub fn new(config: &AiConfig, full_system_prompt: String) -> Self {
        let raw_url = if config.openai_base_url.is_empty() {
            &config.hermes_base_url
        } else {
            &config.openai_base_url
        };
        let api_base = crate::config::normalize_api_base_url(raw_url);
        let openai_config = OpenAIConfig::default()
            .with_api_base(&api_base)
            .with_api_key(&config.openai_api_key);

        let client = Client::with_config(openai_config);

        Self {
            client,
            model: if config.openai_model.is_empty() {
                "deepseek-flash".to_string()
            } else {
                config.openai_model.clone()
            },
            system_prompt: full_system_prompt,
            max_context_tokens: config.max_context_tokens.max(1000),
            compression_strategy: config.compression_strategy.clone(),
        }
    }

    /// Build context messages from history applying token-based limits and compression
    async fn prepare_messages(
        &self,
        history: &[ChatMessage],
        current_prompt: &str,
    ) -> Vec<ChatCompletionRequestMessage> {
        let mut messages = Vec::new();
        let mut effective_system_prompt = self.system_prompt.clone();

        let prompt_tokens = count_tokens(current_prompt, &self.model) as usize;
        let sys_tokens = count_tokens(&self.system_prompt, &self.model) as usize;
        let budget = self
            .max_context_tokens
            .saturating_sub(prompt_tokens + sys_tokens + 50);

        let mut accumulated_tokens = 0usize;
        let mut split_index = 0usize;

        // Traverse history backwards to fit within token budget
        for (i, msg) in history.iter().enumerate().rev() {
            let msg_tok = if msg.token_count > 0 {
                msg.token_count as usize
            } else {
                count_tokens(&msg.content, &self.model) as usize
            };

            if accumulated_tokens + msg_tok > budget {
                split_index = i + 1;
                break;
            }
            accumulated_tokens += msg_tok;
        }

        if split_index > 0 && split_index <= history.len() {
            let older_slice = &history[..split_index];
            let recent_slice = &history[split_index..];

            if self.compression_strategy == "summarize" {
                let summary_res = self.summarize_older_history(older_slice).await;
                match summary_res {
                    Ok(summary) if !summary.trim().is_empty() => {
                        effective_system_prompt.push_str(&format!(
                            "\n\n[前文历史摘要]\n{}\n[以上为更早前对话概要，请在此基础上继续交互]",
                            summary.trim()
                        ));
                    }
                    _ => {
                        effective_system_prompt.push_str("\n\n[注：部分早期历史对话已自动折叠]");
                    }
                }
            }

            if let Ok(sys_msg) = ChatCompletionRequestSystemMessageArgs::default()
                .content(effective_system_prompt)
                .build()
            {
                messages.push(ChatCompletionRequestMessage::System(sys_msg));
            }

            for msg in recent_slice {
                if msg.role == "user" {
                    if let Ok(u) = ChatCompletionRequestUserMessageArgs::default()
                        .content(msg.content.clone())
                        .build()
                    {
                        messages.push(ChatCompletionRequestMessage::User(u));
                    }
                } else if msg.role == "assistant" {
                    if let Ok(a) = ChatCompletionRequestAssistantMessageArgs::default()
                        .content(msg.content.clone())
                        .build()
                    {
                        messages.push(ChatCompletionRequestMessage::Assistant(a));
                    }
                }
            }
        } else {
            if let Ok(sys_msg) = ChatCompletionRequestSystemMessageArgs::default()
                .content(effective_system_prompt)
                .build()
            {
                messages.push(ChatCompletionRequestMessage::System(sys_msg));
            }

            for msg in history {
                if msg.role == "user" {
                    if let Ok(u) = ChatCompletionRequestUserMessageArgs::default()
                        .content(msg.content.clone())
                        .build()
                    {
                        messages.push(ChatCompletionRequestMessage::User(u));
                    }
                } else if msg.role == "assistant" {
                    if let Ok(a) = ChatCompletionRequestAssistantMessageArgs::default()
                        .content(msg.content.clone())
                        .build()
                    {
                        messages.push(ChatCompletionRequestMessage::Assistant(a));
                    }
                }
            }
        }

        if let Ok(u) = ChatCompletionRequestUserMessageArgs::default()
            .content(current_prompt)
            .build()
        {
            messages.push(ChatCompletionRequestMessage::User(u));
        }

        messages
    }

    /// Fast one-shot summary of older dialog turns
    async fn summarize_older_history(&self, older_msgs: &[ChatMessage]) -> Result<String> {
        let mut conversation_text = String::new();
        for msg in older_msgs {
            let role_label = if msg.role == "user" {
                "用户"
            } else {
                "少女"
            };
            conversation_text.push_str(&format!("{}: {}\n", role_label, msg.content));
        }

        let summary_instruction = "请用极其简明扼要的中文总结以下对话中用户的核心诉求、设定及重要上下文信息（100字以内）：\n";
        let req = CreateChatCompletionRequestArgs::default()
            .model(&self.model)
            .messages(vec![ChatCompletionRequestMessage::User(
                ChatCompletionRequestUserMessageArgs::default()
                    .content(format!("{}{}", summary_instruction, conversation_text))
                    .build()?,
            )])
            .build()?;

        let res = self.client.chat().create(req).await?;
        if let Some(choice) = res.choices.first() {
            if let Some(content) = &choice.message.content {
                return Ok(content.clone());
            }
        }
        Ok(String::new())
    }

    pub async fn ask_stream_with_history(
        &self,
        history: &[ChatMessage],
        prompt: &str,
        token_sender: UnboundedSender<String>,
    ) -> Result<(String, Option<u32>)> {
        let messages = self.prepare_messages(history, prompt).await;

        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.model)
            .messages(messages)
            .stream(true)
            .stream_options(ChatCompletionStreamOptions {
                include_usage: true,
            })
            .build()
            .map_err(|e| anyhow!("Failed to build chat completion request: {e}"))?;

        let mut stream = self
            .client
            .chat()
            .create_stream(request)
            .await
            .map_err(|e| anyhow!("Failed to create chat completion stream: {e}"))?;

        let mut full_response = String::new();
        let mut completion_tokens: Option<u32> = None;

        while let Some(chunk_res) = stream.next().await {
            match chunk_res {
                Ok(response) => {
                    if let Some(usage) = response.usage {
                        completion_tokens = Some(usage.completion_tokens);
                    }
                    for choice in response.choices {
                        if let Some(content) = choice.delta.content {
                            full_response.push_str(&content);
                            let _ = token_sender.send(content);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Error in chat stream: {e}");
                    return Err(anyhow!("Stream error: {e}"));
                }
            }
        }

        Ok((full_response, completion_tokens))
    }

    pub async fn ask_stream(
        &self,
        prompt: &str,
        token_sender: UnboundedSender<String>,
    ) -> Result<String> {
        let (res, _) = self
            .ask_stream_with_history(&[], prompt, token_sender)
            .await?;
        Ok(res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_tokens() {
        let count_en = count_tokens("Hello, world!", "deepseek-flash");
        println!("count_en: {count_en}");
        assert!(count_en > 0);
        let count_cn = count_tokens("你好，世界！这是一段中文测试。", "deepseek-flash");
        println!("count_cn: {count_cn}");
        assert!(count_cn > 0);
        assert_eq!(count_tokens("", "deepseek-flash"), 0);
    }
}
