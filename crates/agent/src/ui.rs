//! Ephemeral actions are only sent when an app explicitly displays this chat.
use anyhow::{ensure, Result};
use async_trait::async_trait;
use bubblepaw_sdk::approvals::Approval;
use bubblepaw_sdk::harness::CancellationToken;
use bubblepaw_sdk::tools::{Tool, ToolContext, ToolDefinition, ToolRegistry, ToolResult};
use notes_agent_wire::UiAction;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use vault_core::Vault;

pub struct Ui {
    viewers: Mutex<Vec<(String, CancellationToken)>>,
    pub(crate) actions: tokio::sync::broadcast::Sender<UiAction>,
}
impl Ui {
    pub fn new() -> Self {
        let (actions, _) = tokio::sync::broadcast::channel(256);
        Self {
            viewers: Mutex::new(vec![]),
            actions,
        }
    }
    pub fn viewing(&self, session: String, stop: CancellationToken) {
        let mut viewers = self.viewers.lock().unwrap();
        viewers.retain(|(_, token)| !token.is_cancelled());
        viewers.push((session, stop));
    }
    fn show(&self, action: UiAction) -> bool {
        let mut viewers = self.viewers.lock().unwrap();
        viewers.retain(|(_, token)| !token.is_cancelled());
        if !viewers
            .iter()
            .any(|(session, _)| session == &action.session_id)
        {
            return false;
        }
        self.actions.send(action).is_ok()
    }
}
struct UiTool {
    name: &'static str,
    ui: Arc<Ui>,
    vault: Arc<Vault>,
}
#[async_trait]
impl Tool for UiTool {
    fn definition(&self) -> ToolDefinition {
        let open = self.name == "ui_open_page";
        ToolDefinition{name:self.name.into(),description:if open{"Show a page to the user when they are viewing this chat; optionally scroll to a block."}else{"Highlight existing blocks of a page for the user viewing this chat."}.into(),input_schema:if open{json!({"type":"object","properties":{"page":{"type":"string"},"block":{"type":"string"}},"required":["page"],"additionalProperties":false})}else{json!({"type":"object","properties":{"page":{"type":"string"},"blocks":{"type":"array","items":{"type":"string"}}},"required":["page","blocks"],"additionalProperties":false})},read_only:true,approval:Approval::None}
    }
    async fn invoke(&self, context: ToolContext, input: Value) -> Result<ToolResult> {
        let page = input["page"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("page must be a string"))?;
        let id = vault_tools::resolve_page(&self.vault, page)?;
        let page = self.vault.load_page(id.clone())?;
        ensure!(!page.in_trash, "page is in the trash");
        let blocks = if self.name == "ui_open_page" {
            input
                .get("block")
                .map(|value| {
                    value
                        .as_str()
                        .map(|id| vec![id.into()])
                        .ok_or_else(|| anyhow::anyhow!("block must be a string"))
                })
                .transpose()?
                .unwrap_or_default()
        } else {
            input["blocks"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("blocks must be an array"))?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| anyhow::anyhow!("block ids must be strings"))
                })
                .collect::<Result<Vec<_>>>()?
        };
        fn contains(blocks: &[vault_core::Block], id: &str) -> bool {
            blocks
                .iter()
                .any(|block| block.id == id || contains(&block.children, id))
        }
        for block in &blocks {
            ensure!(
                contains(&page.blocks, block),
                "no block {block} in the page"
            );
        }
        let shown = self.ui.show(UiAction {
            session_id: context.session_id,
            run_id: context.run_id,
            action: if self.name == "ui_open_page" {
                "open_page"
            } else {
                "highlight"
            }
            .into(),
            page_id: id,
            block_ids: blocks,
        });
        Ok(ToolResult::text(if shown {
            "shown to the user"
        } else {
            "no app is showing this chat; nothing opened"
        }))
    }
}
pub fn register(
    registry: &mut ToolRegistry,
    ui: Arc<Ui>,
    vault: Arc<Vault>,
    records: Arc<crate::records::Records>,
) -> Result<()> {
    for name in ["ui_open_page", "ui_highlight"] {
        registry.show_always([name]);
        registry.register_host(Arc::new(crate::tools::RecordedTool::new(
            Arc::new(UiTool {
                name,
                ui: ui.clone(),
                vault: vault.clone(),
            }),
            records.clone(),
            vault.clone(),
            false,
        )))?;
    }
    Ok(())
}
