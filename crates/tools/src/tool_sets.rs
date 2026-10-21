//! Tool sets: the one permission model for every principal that calls tools
//! (agent tokens, schedules, the in-app agent's runners). A surface only
//! checks membership.
//!
//! The built-in sets are fixed (no user-defined sets yet):
//! - [`ToolSet::Read`]: the registry's read-only vault tools.
//! - [`ToolSet::Write`]: every vault tool (listed, hidden and old names).
//! - [`ToolSet::Everything`]: every tool a surface serves, including non-vault
//!   tools (host tools, `js`) and tools added later. It is always offered.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::tools::lookup;

/// A named set of tools a principal may list and call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolSet {
    Read,
    Write,
    Everything,
}

impl ToolSet {
    /// Every built-in set, narrowest first. Everything is always offered.
    pub const ALL: [ToolSet; 3] = [ToolSet::Read, ToolSet::Write, ToolSet::Everything];

    /// The set's stored name: `read`, `write` or `everything`.
    pub fn as_str(self) -> &'static str {
        match self {
            ToolSet::Read => "read",
            ToolSet::Write => "write",
            ToolSet::Everything => "everything",
        }
    }

    /// A short label for pickers.
    pub fn label(self) -> &'static str {
        match self {
            ToolSet::Read => "Read only",
            ToolSet::Write => "Read and write",
            ToolSet::Everything => "Everything",
        }
    }

    /// Whether a call to `name` is in this set. Vault tools (listed, hidden,
    /// or an old name, by the tool it runs) follow the set; any other tool a
    /// surface serves is only in Everything.
    pub fn allows(self, name: &str) -> bool {
        match lookup(name) {
            Some(def) => match self {
                ToolSet::Read => def.read_only,
                ToolSet::Write | ToolSet::Everything => true,
            },
            None => self == ToolSet::Everything,
        }
    }

    /// The refusal text for a call to `name` outside this set.
    pub fn refusal(self, name: &str) -> String {
        match self {
            // The text read tokens have always got.
            ToolSet::Read => format!("tool not available in read-only scope: {name}"),
            _ => format!("tool not in the {} tool set: {name}", self.as_str()),
        }
    }
}

impl fmt::Display for ToolSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ToolSet {
    type Err = String;

    fn from_str(s: &str) -> Result<ToolSet, String> {
        match s {
            "read" => Ok(ToolSet::Read),
            "write" => Ok(ToolSet::Write),
            "everything" => Ok(ToolSet::Everything),
            _ => Err(format!(
                "tool set must be read, write or everything, not {s:?}"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_serde_matches() {
        for set in ToolSet::ALL {
            assert_eq!(set.as_str().parse::<ToolSet>().unwrap(), set);
            assert_eq!(
                serde_json::to_value(set).unwrap(),
                serde_json::Value::String(set.as_str().into())
            );
        }
        assert!("admin".parse::<ToolSet>().is_err());
    }

    #[test]
    fn membership() {
        // Listed read-only, listed write, hidden write, an old name, a host tool.
        assert!(ToolSet::Read.allows("search_notes"));
        assert!(!ToolSet::Read.allows("create_page"));
        assert!(!ToolSet::Read.allows("import_markdown"));
        assert!(ToolSet::Read.allows("memory_list"));
        assert!(ToolSet::Write.allows("create_page"));
        assert!(ToolSet::Write.allows("import_markdown"));
        assert!(!ToolSet::Write.allows("js"));
        assert!(ToolSet::Everything.allows("js"));
        assert!(ToolSet::Everything.allows("create_page"));
        for alias in crate::aliases() {
            let target = lookup(alias.target).unwrap();
            assert_eq!(ToolSet::Read.allows(alias.name), target.read_only);
            assert!(ToolSet::Write.allows(alias.name));
        }
    }
}
