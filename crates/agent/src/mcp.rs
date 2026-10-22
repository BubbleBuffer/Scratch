//! MCP servers for the desktop agent, through BubblePaw's MCP client
//! (`bubblepaw-mcp`).
//!
//! - The app owns the server list and applies it with
//!   [`crate::Agent::set_mcp`]; [`crate::AgentConfig::mcp`] is the list at
//!   open. Only changed servers restart.
//! - An older `<data dir>/mcp.toml` ([`FILE`], BubblePaw's
//!   `[mcp.servers.<name>]` format) is read once by [`import_file`].
//! - A server that fails to start is left out and reported by
//!   [`crate::Agent::mcp_status`]; the others run.
//! - Only desktop engines with the Everything tool set have servers. In-app
//!   chats and the hub relay ([`crate::Agent::relay_call`]) see their tools;
//!   both run under the app's Bypass policy, so a server's `approval =
//!   "confirm"` does not prompt.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use bubblepaw_core::ToolRegistry;
pub use bubblepaw_mcp::{McpHost, McpServerSettings, McpSettings};
use serde::Deserialize;
use vault_core::Vault;

use crate::records::Records;
use crate::tools::RecordedTool;

/// The file name, in the app's data directory.
pub const FILE: &str = "mcp.toml";
/// What [`import_file`] renames [`FILE`] to.
pub const IMPORTED: &str = "mcp.toml.imported";

#[derive(Deserialize, Default)]
struct McpFile {
    #[serde(default)]
    mcp: McpSettings,
}

/// Imports `<dir>/mcp.toml` once: `None` when it is missing; its `[mcp]`
/// table after renaming it to [`IMPORTED`]. A file that cannot be read or
/// parsed is an error and stays in place.
pub fn import_file(dir: &Path) -> Result<Option<McpSettings>> {
    let path = dir.join(FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    let file: McpFile =
        toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    let imported = dir.join(IMPORTED);
    std::fs::rename(&path, &imported)
        .with_context(|| format!("rename {} to {}", path.display(), imported.display()))?;
    Ok(Some(file.mcp))
}

/// A configured server's state, for [`ServerStatus`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerState {
    Running,
    Failed,
    /// Disabled in the settings.
    Off,
}

impl ServerState {
    /// `running`, `failed` or `off`.
    pub fn as_str(self) -> &'static str {
        match self {
            ServerState::Running => "running",
            ServerState::Failed => "failed",
            ServerState::Off => "off",
        }
    }
}

/// One configured server, from [`crate::Agent::mcp_status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerStatus {
    pub name: String,
    pub state: ServerState,
    /// Tools it offers now (0 unless running).
    pub tools: usize,
    /// Why it failed: its host warnings without the `MCP server <name>: ` prefix.
    pub error: Option<String>,
}

/// The status of each server in `settings`, in name order, from the host's
/// warnings for the last applied settings and its current tool names
/// (`mcp__<server>__<tool>`, see [`bubblepaw_mcp::tool_name`]). A server
/// without tools and with a warning failed.
pub fn status(settings: &McpSettings, warnings: &[String], tools: &[String]) -> Vec<ServerStatus> {
    // Each tool counts for the configured server with the longest matching
    // prefix, so `a` does not take the tools of `a__b`.
    let prefixes: Vec<(&str, String)> = settings
        .servers
        .iter()
        .filter(|(_, config)| config.enabled)
        .map(|(name, _)| (name.as_str(), bubblepaw_mcp::tool_name(name, "")))
        .collect();
    let owner = |tool: &str| {
        prefixes
            .iter()
            .filter(|(_, prefix)| tool.starts_with(prefix.as_str()))
            .max_by_key(|(_, prefix)| prefix.len())
            .map(|(name, _)| *name)
    };
    let owners: Vec<&str> = tools.iter().filter_map(|tool| owner(tool)).collect();
    settings
        .servers
        .iter()
        .map(|(name, config)| {
            if !config.enabled {
                return ServerStatus {
                    name: name.clone(),
                    state: ServerState::Off,
                    tools: 0,
                    error: None,
                };
            }
            let count = owners
                .iter()
                .filter(|owner| **owner == name.as_str())
                .count();
            let lead = format!("MCP server {name}: ");
            let errors: Vec<&str> = warnings
                .iter()
                .filter_map(|warning| warning.strip_prefix(&lead))
                .collect();
            let failed = count == 0 && !errors.is_empty();
            ServerStatus {
                name: name.clone(),
                state: if failed {
                    ServerState::Failed
                } else {
                    ServerState::Running
                },
                tools: count,
                error: failed.then(|| errors.join("; ")),
            }
        })
        .collect()
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
