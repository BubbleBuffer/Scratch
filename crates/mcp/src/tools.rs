//! MCP adapter for the shared vault tools.

use serde_json::{json, Value};
use std::cell::Cell;
use vault_core::{PageId, Vault};

use crate::Result;

/// The `tools/list` entries.
pub fn definitions() -> Vec<Value> {
    vault_tools::definitions_for(vault_tools::Surface::Mcp)
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.input_schema_for(vault_tools::Surface::Mcp),
                "annotations": {
                    "readOnlyHint": t.read_only,
                    "destructiveHint": t.destructive_hint,
                    "openWorldHint": false,
                },
            })
        })
        .collect()
}

/// The `tools/list` entries for a read-only scope (only the registry's
/// read-only tools) or for the full scope (the same as [`definitions`]).
pub fn definitions_for(readonly: bool) -> Vec<Value> {
    let mut all = definitions();
    if readonly {
        all.retain(|t| t["annotations"]["readOnlyHint"] == true);
    }
    all
}

// HTTP requests and stdio calls share synchronous dispatch, including the
// hub's actor-attributing callback. Keep transport metadata scoped to that
// dispatch so no authentication or session plumbing is needed.
thread_local! {
    static TRANSPORT: Cell<&'static str> = const { Cell::new("stdio") };
}

pub(crate) fn with_transport<T>(transport: &'static str, call: impl FnOnce() -> T) -> T {
    struct Restore(&'static str);
    impl Drop for Restore {
        fn drop(&mut self) {
            TRANSPORT.with(|slot| slot.set(self.0));
        }
    }
    let _restore = Restore(TRANSPORT.with(|slot| slot.replace(transport)));
    call()
}

/// Whether a request in the given scope may use tool `name` (a listed tool,
/// a hidden one, or an old name's alias, by the tool it runs).
pub fn allowed(name: &str, readonly: bool) -> bool {
    !readonly || vault_tools::lookup(name).is_some_and(|t| t.read_only)
}

/// Whether `name` can be called over MCP, including hidden tools and the
/// aliases of old names (not listed).
pub fn exists(name: &str) -> bool {
    vault_tools::lookup(name).is_some_and(|t| t.serves(vault_tools::Surface::Mcp))
}

/// Runs tool `name`; the text result, or an error for the assistant.
pub fn call(v: &Vault, name: &str, a: &Value) -> Result<String> {
    if name == "feedback_submit" {
        return TRANSPORT
            .with(|slot| vault_tools::feedback::submit(v, a, slot.get()).map_err(Into::into));
    }
    vault_tools::call_on(vault_tools::Surface::Mcp, v, name, a).map_err(Into::into)
}

/// A page id from an id or an exact (case-insensitive) title.
pub fn resolve_page(v: &Vault, p: &str) -> Result<PageId> {
    vault_tools::resolve_page(v, p).map_err(Into::into)
}
