use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum RuntimeEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolCallPending {
        id: String,
        name: String,
        args: Value,
    },
    ToolCallCompleted {
        id: String,
        result: Value,
    },
    ApprovalRequired {
        id: String,
        tool_name: String,
        details: Value,
    },
    TurnComplete {
        tokens: usize,
        duration_ms: u64,
    },
    Error {
        code: String,
        message: String,
    },
}
