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

/// The sleeping test tool's name.
pub const SLEEP_TOOL: &str = "test_sleep";

/// Waits `ms` milliseconds (default 60 s), then returns `slept`. Lets tests
/// hold a run open, to cancel it or let it hit a time limit.
pub struct SleepTool;

#[async_trait]
impl Tool for SleepTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: SLEEP_TOOL.into(),
            description: "Test tool: waits `ms` milliseconds.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {"ms": {"type": "integer"}},
            }),
            read_only: true,
            approval: Approval::None,
        }
    }

    async fn invoke(&self, context: ToolContext, input: Value) -> Result<ToolResult> {
        let ms = input["ms"].as_u64().unwrap_or(60_000);
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(ms)) => {}
            _ = context.cancellation.cancelled() => anyhow::bail!("cancelled"),
        }
        Ok(ToolResult::text("slept"))
    }
}
