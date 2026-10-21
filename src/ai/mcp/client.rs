use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{
    protocol::{JsonRpcNotification, JsonRpcRequest, McpTool, McpToolCallResult},
    sse::SseTransport,
    stdio::StdioTransport,
};
use crate::config::McpServerConfig;

enum Transport {
    Stdio(StdioTransport),
    Sse(SseTransport),
}

pub struct McpClient {
    name: String,
    config: McpServerConfig,
    transport: Option<Transport>,
    tools: Vec<McpTool>,
    request_id_counter: AtomicU64,
}

impl McpClient {
    pub fn new(name: impl Into<String>, config: McpServerConfig) -> Self {
        Self {
            name: name.into(),
            config,
            transport: None,
            tools: Vec::new(),
            request_id_counter: AtomicU64::new(1),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn tools(&self) -> &[McpTool] {
        &self.tools
    }

    pub fn is_connected(&self) -> bool {
        self.transport.is_some()
    }

    fn next_id(&self) -> u64 {
        self.request_id_counter.fetch_add(1, Ordering::SeqCst)
    }

    pub async fn connect(&mut self) -> Result<()> {
        if self.transport.is_some() {
            return Ok(());
        }

        let transport = if self.config.transport == "sse" {
            let url = self
                .config
                .url
                .as_deref()
                .ok_or_else(|| anyhow!("Missing URL for SSE MCP server: {}", self.name))?;
            Transport::Sse(SseTransport::new(url))
        } else {
            let command = self
                .config
                .command
                .as_deref()
                .ok_or_else(|| anyhow!("Missing command for stdio MCP server: {}", self.name))?;
            let stdio = StdioTransport::spawn(command, &self.config.args, &self.config.env)?;
            Transport::Stdio(stdio)
        };

        self.transport = Some(transport);

        // Perform MCP initialize handshake
        self.initialize_handshake().await?;

        // Fetch tool list
        self.refresh_tools().await?;

        Ok(())
    }

    async fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let transport = self
            .transport
            .as_ref()
            .ok_or_else(|| anyhow!("MCP client '{}' is not connected", self.name))?;

        let id = self.next_id();
        let req = JsonRpcRequest::new(id, method, params);

        let resp = match transport {
            Transport::Stdio(s) => s.send_request(req).await?,
            Transport::Sse(s) => s.send_request(req).await?,
        };

        if let Some(err) = resp.error {
            return Err(anyhow!("MCP error {}: {}", err.code, err.message));
        }

        resp.result.ok_or_else(|| anyhow!("Empty result in MCP response"))
    }

    async fn send_notification(&self, method: &str, params: Option<Value>) -> Result<()> {
        let transport = self
            .transport
            .as_ref()
            .ok_or_else(|| anyhow!("MCP client '{}' is not connected", self.name))?;

        let notif = JsonRpcNotification::new(method, params);

        match transport {
            Transport::Stdio(s) => s.send_notification(notif).await,
            Transport::Sse(s) => s.send_notification(notif).await,
        }
    }

    async fn initialize_handshake(&self) -> Result<()> {
        let init_params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "murasame",
                "version": env!("CARGO_PKG_VERSION")
            }
        });

        let _ = self.send_request("initialize", Some(init_params)).await
            .with_context(|| format!("MCP initialize failed for '{}'", self.name))?;

        let _ = self.send_notification("notifications/initialized", None).await;
        Ok(())
    }

    pub async fn refresh_tools(&mut self) -> Result<Vec<McpTool>> {
        let result = self.send_request("tools/list", None).await?;

        let tools_val = result.get("tools").cloned().unwrap_or(Value::Array(Vec::new()));
        let tools: Vec<McpTool> = serde_json::from_value(tools_val)
            .context("Failed to parse tools array from tools/list")?;

        self.tools = tools.clone();
        Ok(tools)
    }

    pub async fn call_tool(&self, tool_name: &str, arguments: &Value) -> Result<McpToolCallResult> {
        let params = json!({
            "name": tool_name,
            "arguments": arguments
        });

        let result = self.send_request("tools/call", Some(params)).await
            .with_context(|| format!("Failed to execute tool '{}' on MCP server '{}'", tool_name, self.name))?;

        let tool_res: McpToolCallResult = serde_json::from_value(result)
            .context("Failed to deserialize tool execution result")?;
        Ok(tool_res)
    }
}
