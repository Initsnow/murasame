use anyhow::{Result, anyhow};
use serde_json::Value;
use std::collections::HashMap;

use super::{
    client::McpClient,
    protocol::{McpTool, McpToolCallResult},
};
use crate::config::McpServerConfig;

pub struct McpManager {
    clients: HashMap<String, McpClient>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
        }
    }

    pub fn from_config(configs: &HashMap<String, McpServerConfig>) -> Self {
        let mut clients = HashMap::new();
        for (name, cfg) in configs {
            if cfg.enabled {
                clients.insert(name.clone(), McpClient::new(name, cfg.clone()));
            }
        }
        Self { clients }
    }

    pub fn client_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.clients.keys().cloned().collect();
        names.sort();
        names
    }

    /// Connect to all configured enabled MCP servers
    pub async fn ensure_all_connected(&mut self) {
        for (name, client) in &mut self.clients {
            if !client.is_connected() {
                if let Err(e) = client.connect().await {
                    tracing::warn!("Failed to connect to MCP server '{name}': {e}");
                }
            }
        }
    }

    /// List all discovered tools across connected servers
    pub fn list_all_tools(&self) -> Vec<(&str, &McpTool)> {
        let mut out = Vec::new();
        for (server_name, client) in &self.clients {
            for tool in client.tools() {
                out.push((server_name.as_str(), tool));
            }
        }
        out.sort_by(|a, b| a.1.name.cmp(&b.1.name));
        out
    }

    /// Execute a tool by name, routing to the appropriate MCP client
    pub async fn execute_tool(&mut self, tool_name: &str, arguments: &Value) -> Result<McpToolCallResult> {
        // First ensure servers are connected
        self.ensure_all_connected().await;

        // Find which server has this tool
        for (_server_name, client) in &self.clients {
            if client.tools().iter().any(|t| t.name == tool_name) {
                return client.call_tool(tool_name, arguments).await;
            }
        }

        Err(anyhow!("No active MCP server found providing tool '{tool_name}'"))
    }
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}
