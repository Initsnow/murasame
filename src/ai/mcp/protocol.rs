use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(id: u64, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcNotification {
    pub fn new(method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "inputSchema", default)]
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolCallResult {
    #[serde(default)]
    pub content: Vec<McpContentItem>,
    #[serde(rename = "isError", default)]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpContentItem {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub data: Option<String>,
    #[serde(rename = "mimeType", default)]
    pub mime_type: Option<String>,
}

impl McpToolCallResult {
    pub fn to_display_string(&self) -> String {
        let mut out = String::new();
        for item in &self.content {
            if let Some(ref text) = item.text {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
        }
        if out.is_empty() {
            if self.is_error {
                "Tool execution failed without output".to_string()
            } else {
                "Tool execution completed (empty output)".to_string()
            }
        } else {
            out
        }
    }

    pub fn extract_first_image(&self) -> Option<crate::ai::agent::builtins::ToolImageResult> {
        for item in &self.content {
            if item.content_type == "image" {
                if let Some(ref data) = item.data {
                    return Some(crate::ai::agent::builtins::ToolImageResult {
                        base64: data.clone(),
                        mime_type: item
                            .mime_type
                            .clone()
                            .unwrap_or_else(|| "image/jpeg".to_string()),
                    });
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jsonrpc_request_serialization() {
        let req = JsonRpcRequest::new(1, "tools/list", None);
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"id\":1"));
        assert!(json.contains("\"method\":\"tools/list\""));
    }

    #[test]
    fn test_mcp_tool_deserialization() {
        let raw = r#"{
            "name": "search_db",
            "description": "Search database",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                },
                "required": ["query"]
            }
        }"#;

        let tool: McpTool = serde_json::from_str(raw).unwrap();
        assert_eq!(tool.name, "search_db");
        assert_eq!(tool.description.as_deref(), Some("Search database"));
        assert!(tool.input_schema.get("properties").is_some());
    }

    #[test]
    fn test_mcp_tool_call_result_display() {
        let res = McpToolCallResult {
            content: vec![
                McpContentItem {
                    content_type: "text".to_string(),
                    text: Some("First line".to_string()),
                    data: None,
                    mime_type: None,
                },
                McpContentItem {
                    content_type: "text".to_string(),
                    text: Some("Second line".to_string()),
                    data: None,
                    mime_type: None,
                },
                McpContentItem {
                    content_type: "image".to_string(),
                    text: None,
                    data: Some("base64_img_data".to_string()),
                    mime_type: Some("image/png".to_string()),
                },
            ],
            is_error: false,
        };
        assert_eq!(res.to_display_string(), "First line\nSecond line");

        let img = res.extract_first_image().expect("should extract image");
        assert_eq!(img.base64, "base64_img_data");
        assert_eq!(img.mime_type, "image/png");

        let empty_err = McpToolCallResult {
            content: vec![],
            is_error: true,
        };
        assert_eq!(empty_err.to_display_string(), "Tool execution failed without output");
    }
}
