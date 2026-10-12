use anyhow::{Result, anyhow};
use async_openai::{
    Client,
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
        ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
        CreateChatCompletionRequestArgs,
    },
};
use futures_util::StreamExt;
use tokio::sync::mpsc::UnboundedSender;

use crate::config::AiConfig;

pub struct LlmClient {
    client: Client<OpenAIConfig>,
    model: String,
    system_prompt: String,
    history: Vec<ChatCompletionRequestMessage>,
}

impl LlmClient {
    pub fn new(config: &AiConfig, full_system_prompt: String) -> Self {
        let openai_config = OpenAIConfig::default()
            .with_api_base(&config.base_url)
            .with_api_key(&config.api_key);

        let client = Client::with_config(openai_config);

        let system_message = ChatCompletionRequestSystemMessageArgs::default()
            .content(full_system_prompt.clone())
            .build()
            .expect("Failed to build system message");

        Self {
            client,
            model: config.model.clone(),
            system_prompt: full_system_prompt,
            history: vec![ChatCompletionRequestMessage::System(system_message)],
        }
    }

    pub async fn ask_stream(
        &mut self,
        prompt: &str,
        token_sender: UnboundedSender<String>,
    ) -> Result<String> {
        let user_msg = ChatCompletionRequestUserMessageArgs::default()
            .content(prompt)
            .build()
            .map_err(|e| anyhow!("Failed to build user message: {e}"))?;

        self.history
            .push(ChatCompletionRequestMessage::User(user_msg));

        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.model)
            .messages(self.history.clone())
            .stream(true)
            .build()
            .map_err(|e| anyhow!("Failed to build chat completion request: {e}"))?;

        let mut stream = self
            .client
            .chat()
            .create_stream(request)
            .await
            .map_err(|e| anyhow!("Failed to create chat completion stream: {e}"))?;

        let mut full_response = String::new();

        while let Some(chunk_res) = stream.next().await {
            match chunk_res {
                Ok(response) => {
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

        if !full_response.is_empty() {
            if let Ok(assistant_msg) = ChatCompletionRequestAssistantMessageArgs::default()
                .content(full_response.clone())
                .build()
            {
                self.history
                    .push(ChatCompletionRequestMessage::Assistant(assistant_msg));
            }
        }

        Ok(full_response)
    }

    pub fn reset_history(&mut self) {
        let system_message = ChatCompletionRequestSystemMessageArgs::default()
            .content(self.system_prompt.clone())
            .build()
            .expect("Failed to build system message");
        self.history = vec![ChatCompletionRequestMessage::System(system_message)];
    }
}
