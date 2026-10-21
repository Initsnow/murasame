use anyhow::{Context, Result, anyhow};
use futures_util::StreamExt;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::{ai::event::AgentStreamEvent, config::AiConfig};

#[derive(Debug, Serialize)]
struct HermesRequestBody<'a> {
    model: &'a str,
    input: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_response_id: Option<&'a str>,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct SseGenericEvent {
    #[serde(rename = "type", default)]
    event_type: String,
    #[serde(default)]
    response: Option<SseResponseObj>,
    #[serde(default)]
    delta: Option<String>,
    #[serde(default)]
    item: Option<SseItemObj>,
    #[serde(default)]
    error: Option<SseErrorObj>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct SseResponseObj {
    id: String,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct SseItemObj {
    #[serde(rename = "type", default)]
    item_type: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
    #[serde(default)]
    output: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct SseErrorObj {
    #[serde(default)]
    message: String,
    #[serde(default)]
    code: Option<String>,
}

#[derive(Clone)]
pub struct HermesClient {
    client: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
}

pub fn normalize_hermes_url(base_url: &str) -> String {
    let clean = base_url.trim().trim_end_matches('/');
    if clean.ends_with("/responses") {
        clean.to_string()
    } else if clean.ends_with("/v1") {
        format!("{clean}/responses")
    } else {
        format!("{clean}/v1/responses")
    }
}

pub fn normalize_hermes_chat_url(base_url: &str) -> String {
    let clean = base_url.trim().trim_end_matches('/');
    if clean.ends_with("/chat/completions") {
        clean.to_string()
    } else if clean.ends_with("/v1") {
        format!("{clean}/chat/completions")
    } else {
        format!("{clean}/v1/chat/completions")
    }
}

impl HermesClient {
    pub fn new(config: &AiConfig) -> Self {
        let raw_url = if config.hermes_base_url.is_empty() {
            config.base_url.as_deref().unwrap_or("")
        } else {
            &config.hermes_base_url
        };
        let endpoint = if config.hermes_endpoint == "chat_completions" {
            normalize_hermes_chat_url(raw_url)
        } else {
            normalize_hermes_url(raw_url)
        };
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let api_key = if config.hermes_api_key.is_empty() {
            config.api_key.clone().unwrap_or_default()
        } else {
            config.hermes_api_key.clone()
        };

        let model = if config.hermes_model.is_empty() {
            config
                .model
                .clone()
                .unwrap_or_else(|| "murasame".to_string())
        } else {
            config.hermes_model.clone()
        };

        Self {
            client,
            endpoint,
            api_key,
            model,
        }
    }

    pub fn build_headers(
        &self,
        session_key: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        if !self.api_key.is_empty() {
            let auth_str = format!("Bearer {}", self.api_key.trim());
            if let Ok(val) = HeaderValue::from_str(&auth_str) {
                headers.insert(AUTHORIZATION, val);
            }
        }

        if let Some(s_key) = session_key {
            if !s_key.trim().is_empty() {
                if let Ok(val) = HeaderValue::from_str(s_key.trim()) {
                    if let Ok(name) = HeaderName::from_bytes(b"x-hermes-session-key") {
                        headers.insert(name, val);
                    }
                }
            }
        }

        let idem_key = idempotency_key
            .map(|k| k.to_string())
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        if let Ok(val) = HeaderValue::from_str(&idem_key) {
            if let Ok(name) = HeaderName::from_bytes(b"idempotency-key") {
                headers.insert(name, val);
            }
        }

        headers
    }

    /// Stream request to Hermes Agent API Server with session key, idempotency key and rich event stream
    pub async fn ask_stream_events(
        &self,
        prompt: &str,
        instructions: Option<&str>,
        session_key: Option<&str>,
        previous_response_id: Option<&str>,
        event_sender: UnboundedSender<AgentStreamEvent>,
    ) -> Result<(String, Option<String>)> {
        let headers = self.build_headers(session_key, None);

        let body = HermesRequestBody {
            model: &self.model,
            input: prompt,
            instructions,
            previous_response_id,
            stream: true,
        };

        let response = self
            .client
            .post(&self.endpoint)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .with_context(|| {
                format!(
                    "Failed to send request to Hermes endpoint: {}",
                    self.endpoint
                )
            })?;

        let status = response.status();
        if status.as_u16() == 409 {
            let err_text = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Hermes admission conflict (HTTP 409): {err_text}"
            ));
        }

        if !status.is_success() {
            let err_text = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Hermes server returned HTTP {}: {}",
                status,
                err_text
            ));
        }

        let mut byte_stream = response.bytes_stream();
        let mut full_response = String::new();
        let mut new_response_id: Option<String> = None;
        let mut buffer = String::new();

        while let Some(chunk_res) = byte_stream.next().await {
            let chunk = chunk_res.with_context(|| "Error reading response chunk from Hermes")?;
            let text = String::from_utf8_lossy(&chunk);
            buffer.push_str(&text);

            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim().to_string();
                buffer.drain(..=pos);

                if line.is_empty() {
                    continue;
                }

                let data_str = if let Some(stripped) = line.strip_prefix("data:") {
                    stripped.trim()
                } else {
                    continue;
                };

                if data_str == "[DONE]" {
                    break;
                }

                if let Ok(evt) = serde_json::from_str::<SseGenericEvent>(data_str) {
                    match evt.event_type.as_str() {
                        "response.created" => {
                            if let Some(resp_obj) = evt.response {
                                new_response_id = Some(resp_obj.id.clone());
                                let _ = event_sender.send(AgentStreamEvent::ResponseId(resp_obj.id));
                            }
                        }
                        "response.output_text.delta" => {
                            if let Some(delta) = evt.delta {
                                full_response.push_str(&delta);
                                let _ = event_sender.send(AgentStreamEvent::Token(delta));
                            }
                        }
                        "response.output_item.added" | "response.tool_call" => {
                            if let Some(item) = evt.item {
                                let name = item.name.unwrap_or_else(|| "tool".to_string());
                                let id = item.call_id.or(item.id).unwrap_or_else(|| Uuid::new_v4().to_string());
                                let input = item.arguments.unwrap_or_default();
                                let _ = event_sender.send(AgentStreamEvent::Status(format!("正在执行: {name}...")));
                                let _ = event_sender.send(AgentStreamEvent::ToolCallStart {
                                    id,
                                    name,
                                    input,
                                });
                            }
                        }
                        "response.output_item.done" | "response.tool_call.done" => {
                            if let Some(item) = evt.item {
                                let name = item.name.unwrap_or_else(|| "tool".to_string());
                                let id = item.call_id.or(item.id).unwrap_or_default();
                                let output = item.output.unwrap_or_default();
                                let _ = event_sender.send(AgentStreamEvent::ToolCallDone {
                                    id,
                                    name,
                                    output,
                                    is_error: false,
                                });
                                let _ = event_sender.send(AgentStreamEvent::Status(String::new()));
                            }
                        }
                        "response.cancelled" => {
                            let _ = event_sender.send(AgentStreamEvent::Cancelled);
                            return Err(anyhow!("Hermes turn was cancelled by server"));
                        }
                        "response.failed" => {
                            let err_msg = evt.error.map(|e| e.message).unwrap_or_else(|| "Unknown failure".to_string());
                            let _ = event_sender.send(AgentStreamEvent::Failed(err_msg.clone()));
                            return Err(anyhow!("Hermes response failed: {err_msg}"));
                        }
                        _ => {}
                    }
                } else if let Ok(val) = serde_json::from_str::<serde_json::Value>(data_str) {
                    // Fallback JSON navigation if schema varies
                    if let Some(t) = val.get("type").and_then(|t| t.as_str()) {
                        if t == "response.created" {
                            if let Some(id) = val.pointer("/response/id").and_then(|i| i.as_str()) {
                                new_response_id = Some(id.to_string());
                                let _ = event_sender.send(AgentStreamEvent::ResponseId(id.to_string()));
                            }
                        } else if t == "response.output_text.delta" {
                            if let Some(delta) = val.get("delta").and_then(|d| d.as_str()) {
                                full_response.push_str(delta);
                                let _ = event_sender.send(AgentStreamEvent::Token(delta.to_string()));
                            }
                        }
                    }
                }
            }
        }

        Ok((full_response, new_response_id))
    }

    /// Backward-compatible streaming call forwarding tokens directly to token_sender
    pub async fn ask_stream(
        &self,
        prompt: &str,
        instructions: Option<&str>,
        previous_response_id: Option<&str>,
        token_sender: UnboundedSender<String>,
    ) -> Result<(String, Option<String>)> {
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<AgentStreamEvent>();

        let forward_task = tokio::spawn(async move {
            while let Some(evt) = event_rx.recv().await {
                if let AgentStreamEvent::Token(tok) = evt {
                    let _ = token_sender.send(tok);
                }
            }
        });

        let res = self
            .ask_stream_events(prompt, instructions, None, previous_response_id, event_tx)
            .await;
        let _ = forward_task.await;
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_hermes_url() {
        assert_eq!(
            normalize_hermes_url("http://127.0.0.1:8642"),
            "http://127.0.0.1:8642/v1/responses"
        );
        assert_eq!(
            normalize_hermes_url("http://127.0.0.1:8642/v1"),
            "http://127.0.0.1:8642/v1/responses"
        );
        assert_eq!(
            normalize_hermes_url("http://127.0.0.1:8642/v1/responses/"),
            "http://127.0.0.1:8642/v1/responses"
        );
        assert_eq!(
            normalize_hermes_chat_url("http://127.0.0.1:8642/v1"),
            "http://127.0.0.1:8642/v1/chat/completions"
        );
    }

    #[test]
    fn test_hermes_headers_builder() {
        let mut cfg = AiConfig::default();
        cfg.hermes_api_key = "test_key_123".to_string();
        let client = HermesClient::new(&cfg);

        let headers = client.build_headers(Some("session_abc"), Some("idem_fixed"));
        assert_eq!(
            headers.get("authorization").unwrap().to_str().unwrap(),
            "Bearer test_key_123"
        );
        assert_eq!(
            headers.get("x-hermes-session-key").unwrap().to_str().unwrap(),
            "session_abc"
        );
        assert_eq!(
            headers.get("idempotency-key").unwrap().to_str().unwrap(),
            "idem_fixed"
        );
    }

    #[test]
    fn test_hermes_tool_call_sse_deserialization() {
        let tool_added_raw = r#"{"type":"response.output_item.added","item":{"type":"function_call","name":"web_search","call_id":"call_123","arguments":"{\"query\":\"rust\"}"}}"#;
        let evt: SseGenericEvent = serde_json::from_str(tool_added_raw).unwrap();
        assert_eq!(evt.event_type, "response.output_item.added");
        let item = evt.item.unwrap();
        assert_eq!(item.name.as_deref(), Some("web_search"));
        assert_eq!(item.call_id.as_deref(), Some("call_123"));

        let cancelled_raw = r#"{"type":"response.cancelled"}"#;
        let evt_cancel: SseGenericEvent = serde_json::from_str(cancelled_raw).unwrap();
        assert_eq!(evt_cancel.event_type, "response.cancelled");

        let failed_raw = r#"{"type":"response.failed","error":{"message":"Model error"}}"#;
        let evt_failed: SseGenericEvent = serde_json::from_str(failed_raw).unwrap();
        assert_eq!(evt_failed.event_type, "response.failed");
        assert_eq!(evt_failed.error.unwrap().message, "Model error");
    }
}
