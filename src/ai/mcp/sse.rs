use anyhow::{Context, Result, anyhow};
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use std::sync::atomic::{AtomicU64, Ordering};

use super::protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};

pub struct SseTransport {
    client: reqwest::Client,
    endpoint_url: String,
    request_counter: AtomicU64,
}

impl SseTransport {
    pub fn new(url: &str) -> Self {
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            client,
            endpoint_url: url.trim().to_string(),
            request_counter: AtomicU64::new(1),
        }
    }

    pub fn next_id(&self) -> u64 {
        self.request_counter.fetch_add(1, Ordering::SeqCst)
    }

    pub async fn send_request(&self, req: JsonRpcRequest) -> Result<JsonRpcResponse> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let resp = self
            .client
            .post(&self.endpoint_url)
            .headers(headers)
            .json(&req)
            .send()
            .await
            .with_context(|| format!("Failed to send request to MCP SSE endpoint: {}", self.endpoint_url))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("MCP SSE server returned HTTP {status}: {text}"));
        }

        let json_resp: JsonRpcResponse = resp
            .json()
            .await
            .context("Failed to deserialize JSON-RPC response from MCP SSE server")?;
        Ok(json_resp)
    }

    pub async fn send_notification(&self, notif: JsonRpcNotification) -> Result<()> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let _ = self
            .client
            .post(&self.endpoint_url)
            .headers(headers)
            .json(&notif)
            .send()
            .await;
        Ok(())
    }
}
