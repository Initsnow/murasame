use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AgentStreamEvent {
    /// LLM text token chunk
    Token(String),
    /// LLM reasoning / thinking chunk
    Thought(String),
    /// Tool call initiated / running
    ToolCallStart {
        id: String,
        name: String,
        input: String,
    },
    /// Tool call progress update
    ToolCallProgress {
        id: String,
        message: String,
    },
    /// Tool call completed
    ToolCallDone {
        id: String,
        name: String,
        output: String,
        is_error: bool,
    },
    /// Status message for subtitle / UI indicator (e.g. "[正在搜索网页...]")
    Status(String),
    /// Upstream response id (for Hermes session continuation)
    ResponseId(String),
    /// Terminal cancellation
    Cancelled,
    /// Terminal failure with error message
    Failed(String),
}
