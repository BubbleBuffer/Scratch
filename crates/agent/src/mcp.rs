//! MCP servers for the desktop agent, through BubblePaw's MCP client
//! (`bubblepaw-mcp`).
//!
//! - The servers are listed in `<data dir>/mcp.toml` ([`FILE`]) in the same
//!   `[mcp.servers.<name>]` format as BubblePaw's runner config, so entries
//!   can be copied over. There is no settings UI.
//! - A missing or empty file means no servers. An unreadable file is
//!   reported and means no servers. A server that fails to start is logged
//!   and left out; the agent opens without it.
//! - Only desktop engines with the Everything tool set start them. In-app
//!   chats and the hub relay ([`crate::Agent::relay_call`]) see their tools;
//!   both run under the app's Bypass policy, so a server's `approval =
//!   "confirm"` does not prompt.

use std::path::Path;
use std::sync::Arc;

use bubblepaw_core::ToolRegistry;
pub use bubblepaw_mcp::{McpHost, McpServerSettings, McpSettings};
use serde::Deserialize;
use vault_core::Vault;

use crate::records::Records;
use crate::tools::RecordedTool;

/// The file name, in the app's data directory.
pub const FILE: &str = "mcp.toml";

#[derive(Deserialize, Default)]
struct McpFile {
    #[serde(default)]
    mcp: McpSettings,
}

/// Reads the `[mcp]` table of the file at `path`. A missing file gives no
/// servers and no error; an unreadable or invalid one gives no servers and
/// the reason.
pub fn load_settings(path: &Path) -> (McpSettings, Option<String>) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (McpSettings::default(), None);
        }
        Err(error) => {
            return (
                McpSettings::default(),
                Some(format!("{}: {error}; no MCP servers", path.display())),
            );
        }
    };
    match toml::from_str::<McpFile>(&text) {
        Ok(file) => (file.mcp, None),
        Err(error) => (
            McpSettings::default(),
            Some(format!("{}: {error}; no MCP servers", path.display())),
        ),
    }
}

/// Registers the host's current tools in `tools`, recorded like the other
/// host tools. A name already taken is skipped and returned.
pub(crate) fn register(
    tools: &mut ToolRegistry,
    host: &McpHost,
    records: &Arc<Records>,
    vault: &Arc<Vault>,
) -> Vec<String> {
    let mut skipped = Vec::new();
    for tool in host.tools() {
        let name = tool.definition().name;
        let recorded = RecordedTool::new(tool, records.clone(), vault.clone(), false);
        if let Err(error) = tools.register_host(Arc::new(recorded)) {
            skipped.push(format!("MCP tool {name}: {error}"));
        }
    }
    skipped
}
