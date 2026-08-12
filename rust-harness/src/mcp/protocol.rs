use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: &str = "2025-06-18";

#[derive(Debug, Serialize)]
pub struct JsonRpcRequest<'a> {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: &'a str,
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcNotification<'a> {
    pub jsonrpc: &'static str,
    pub method: &'a str,
    pub params: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcResponse {
    pub id: Option<Value>,
    pub result: Option<Value>,
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    pub fn has_id(&self, expected: u64) -> bool {
        self.id.as_ref().is_some_and(|id| {
            id.as_u64() == Some(expected)
                || id.as_str().is_some_and(|id| id == expected.to_string())
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolCallEnvelope {
    #[serde(default, rename = "isError")]
    pub is_error: bool,
    #[serde(default)]
    pub content: Vec<ToolContent>,
    #[serde(default, rename = "structuredContent")]
    pub structured_content: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolContent {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    #[serde(default)]
    pub capabilities: Value,
    #[serde(default)]
    pub server_info: Option<ImplementationInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplementationInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolSummary {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolListResult {
    #[serde(default)]
    pub tools: Vec<ToolSummary>,
    pub next_cursor: Option<String>,
}
