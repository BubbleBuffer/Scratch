//! Test-only host tools, registered only when [`crate::TestHooks`] asks for
//! them (the app's tests reach this through the bridge's `test_tools` flag).

use anyhow::Result;
use async_trait::async_trait;
use bubblepaw_core::{Approval, Tool, ToolContext, ToolDefinition, ToolResult};
use serde_json::{json, Value};

/// The test tool's name: `tools.test_confirm({text})` inside `js`.
pub const CONFIRM_TOOL: &str = "test_confirm";

/// Declares `Confirm`, so each call waits for a decision; once approved it
/// returns `confirmed: <text>`.
pub struct ConfirmTool;

#[async_trait]
impl Tool for ConfirmTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: CONFIRM_TOOL.into(),
            description: "Test tool: asks for approval, then echoes `text`.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {"text": {"type": "string"}},
            }),
            read_only: false,
            approval: Approval::Confirm,
        }
    }

    async fn invoke(&self, _context: ToolContext, input: Value) -> Result<ToolResult> {
        let text = input["text"].as_str().unwrap_or_default();
        Ok(ToolResult::text(format!("confirmed: {text}")))
    }
}
