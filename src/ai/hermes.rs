use anyhow::{Context, Result, anyhow};
use futures_util::StreamExt;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

use crate::config::AiConfig;

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
}

#[derive(Debug, Deserialize)]
struct SseResponseObj {
    id: String,
}

pub struct HermesClient {
    client: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
}

fn normalize_hermes_url(base_url: &str) -> String {
    let clean = base_url.trim().trim_end_matches('/');
    if clean.ends_with("/responses") {
        clean.to_string()
    } else if clean.ends_with("/v1") {
        format!("{clean}/responses")
    } else {
        format!("{clean}/v1/responses")
    }
}

impl HermesClient {
    pub fn new(config: &AiConfig) -> Self {
        let raw_url = if config.hermes_base_url.is_empty() {
            config.base_url.as_deref().unwrap_or("")
        } else {
            &config.hermes_base_url
        };
        let endpoint = normalize_hermes_url(raw_url);
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

    pub async fn ask_stream(
        &self,
        prompt: &str,
        instructions: Option<&str>,
        previous_response_id: Option<&str>,
        token_sender: UnboundedSender<String>,
    ) -> Result<(String, Option<String>)> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if !self.api_key.is_empty() {
            let auth_str = format!("Bearer {}", self.api_key.trim());
            if let Ok(val) = HeaderValue::from_str(&auth_str) {
                headers.insert(AUTHORIZATION, val);
            }
        }

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

        if !response.status().is_success() {
            let status = response.status();
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
                    if evt.event_type == "response.created" {
                        if let Some(resp_obj) = evt.response {
                            new_response_id = Some(resp_obj.id);
                        }
                    } else if evt.event_type == "response.output_text.delta" {
                        if let Some(delta) = evt.delta {
                            full_response.push_str(&delta);
                            let _ = token_sender.send(delta);
                        }
                    }
                } else if let Ok(val) = serde_json::from_str::<serde_json::Value>(data_str) {
                    // Fallback JSON navigation if schema varies
                    if let Some(t) = val.get("type").and_then(|t| t.as_str()) {
                        if t == "response.created" {
                            if let Some(id) = val.pointer("/response/id").and_then(|i| i.as_str()) {
                                new_response_id = Some(id.to_string());
                            }
                        } else if t == "response.output_text.delta" {
                            if let Some(delta) = val.get("delta").and_then(|d| d.as_str()) {
                                full_response.push_str(delta);
                                let _ = token_sender.send(delta.to_string());
                            }
                        }
                    }
                }
            }
        }

        Ok((full_response, new_response_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hermes_payload_serialization() {
        let payload = HermesRequestBody {
            model: "murasame",
            input: "你好呀",
            instructions: Some("You are Murasame"),
            stream: true,
            previous_response_id: Some("resp_12345"),
        };

        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"model\":\"murasame\""));
        assert!(json.contains("\"stream\":true"));
        assert!(json.contains("\"previous_response_id\":\"resp_12345\""));
        assert!(json.contains("\"instructions\":\"You are Murasame\""));
        assert!(json.contains("\"input\":\"你好呀\""));
    }

    #[test]
    fn test_hermes_sse_event_deserialization() {
        let created_raw = r#"{"type":"response.created","response":{"id":"resp_abc123"}}"#;
        let evt: SseGenericEvent = serde_json::from_str(created_raw).unwrap();
        assert_eq!(evt.event_type, "response.created");
        assert_eq!(evt.response.unwrap().id, "resp_abc123");

        let delta_raw = r#"{"type":"response.output_text.delta","delta":"汝好！"}"#;
        let evt_delta: SseGenericEvent = serde_json::from_str(delta_raw).unwrap();
        assert_eq!(evt_delta.event_type, "response.output_text.delta");
        assert_eq!(evt_delta.delta.unwrap(), "汝好！");
    }
}
