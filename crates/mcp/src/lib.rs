//! `vault_mcp`: gives AI assistants (Claude, ChatGPT, Codex) access to a
//! notes vault over the Model Context Protocol, and stores the AI tasks the
//! in-app agent answers.
//!
//! - [`McpServer`]: JSON-RPC handling and the vault tools ([`tools`]).
//! - [`stdio`]: newline-delimited JSON-RPC over stdin/stdout.
//! - [`http`]: streamable HTTP (`POST /mcp`, JSON responses) with a bearer
//!   token, bound to 127.0.0.1 by default.
//! - [`endpoint`]: the vault's MCP token and the running endpoint's address
//!   (`.vault/mcp/`), so `notes-mcp` can reach the app that holds the vault.
//! - [`tasks`]: AI tasks stored in the vault (answered by the BubblePaw
//!   agent on the desktop or the hub; the app starts no other AI program).
//! - [`merge`]: the prompt and answer format for an agent merging a sync
//!   conflict.
//!
//! Design notes: `wiki/integrations/ai-assistants.md`.

pub mod endpoint;
pub mod http;
pub mod jobs;
pub mod merge;
pub use vault_tools::props;
pub mod proxy;
pub mod runners;
mod server;
pub mod stdio;
pub mod tasks;
pub mod tools;
pub mod triage;

pub use server::{BodyReply, McpServer, PROTOCOL_VERSIONS};

/// One error type for this crate: a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpError(pub String);

impl std::fmt::Display for McpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for McpError {}

impl From<vault_tools::ToolError> for McpError {
    fn from(e: vault_tools::ToolError) -> Self {
        Self(e.0)
    }
}

impl From<vault_core::VaultError> for McpError {
    fn from(e: vault_core::VaultError) -> McpError {
        McpError(e.to_string())
    }
}

impl From<std::io::Error> for McpError {
    fn from(e: std::io::Error) -> McpError {
        McpError(e.to_string())
    }
}

impl From<serde_json::Error> for McpError {
    fn from(e: serde_json::Error) -> McpError {
        McpError(e.to_string())
    }
}

impl From<McpError> for String {
    fn from(e: McpError) -> String {
        e.0
    }
}

pub type Result<T> = std::result::Result<T, McpError>;

pub(crate) fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(McpError(msg.into()))
}

/// 32 random bytes as hex.
pub(crate) fn random_hex() -> Result<String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| McpError(format!("random: {e}")))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}
