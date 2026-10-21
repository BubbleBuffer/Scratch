//! Shared vault actions and metadata for agent hosts.
//!
//! Tool implementations operate directly on the vault core, with compact
//! markdown output and versioned edits.

mod agenda;
pub mod feedback;
pub mod harness;
pub mod help;
mod memory;
pub mod notes_cli;
pub mod notes_setup;
pub mod outline;
mod organize;
pub mod props;
mod query;
mod tool_sets;
mod tools;
pub mod wiki_import;

pub use agenda::{resolve_agenda_item, resolve_daily_note, AgendaTarget};
pub use tool_sets::ToolSet;
pub use tools::{
    alias, aliases, call, call_on, canonical, definitions, definitions_for, get, hidden,
    import_args, lookup, resolve_page, set_link_base, Alias, Approval, Surface, ToolDef,
    CREATE_PAGE_ROOTS_ERROR, IN_APP_TRANSPORT, JOBS_NEED_A_HUB,
};

/// A tool error, preserving the message reported by the vault action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolError(pub String);

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ToolError {}

impl From<vault_core::VaultError> for ToolError {
    fn from(e: vault_core::VaultError) -> Self {
        Self(e.to_string())
    }
}

pub(crate) type Result<T> = std::result::Result<T, ToolError>;

pub(crate) fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(ToolError(msg.into()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use serde_json::json;
    use vault_core::Vault;

    use super::*;

    struct Temp(PathBuf);

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn search_notes_keeps_body_recall_for_broad_queries() {
        let temp =
            Temp(std::env::temp_dir().join(format!("vault_tools_test_{}", vault_core::new_id())));
        let vault = Vault::open(temp.0.to_string_lossy().into_owned()).unwrap();
        for i in 0..201 {
            vault.create_page(None, format!("Growth {i}")).unwrap();
        }
        let body_page = vault.create_page(None, "Body only".into()).unwrap();
        vault
            .apply(
                body_page.clone(),
                vec![vault_core::BlockOp::Insert {
                    parent: None,
                    after: None,
                    block: vault_core::NewBlock {
                        id: "body".into(),
                        kind: vault_core::BlockKind::Paragraph,
                        text: "garden".into(),
                        marks: vec![],
                        checked: false,
                    },
                }],
            )
            .unwrap();
        vault.flush().unwrap();
        let adaptive = vault.search_adaptive("g".into(), 100).unwrap();
        assert!(adaptive.narrowed);
        assert!(adaptive.hits.iter().all(|hit| hit.id != body_page));
        let offset = vault
            .search_full("g".into(), 202)
            .unwrap()
            .iter()
            .position(|hit| hit.id == body_page)
            .unwrap();
        let output = call(
            &vault,
            "search_notes",
            &json!({"query": "g", "limit": 1, "offset": offset}),
        )
        .unwrap();
        assert!(
            output.contains(&body_page),
            "agent search must include body matches: {output}"
        );
    }

    const LISTED: [&str; 16] = [
        "search_notes",
        "read_page",
        "create_page",
        "edit_page",
        "trash_page",
        "move_page",
        "daily_note",
        "update_task",
        "upsert_row",
        "memory_read",
        "memory_write",
        "memory_update",
        "memory_delete",
        "feedback_submit",
        "notes_help",
        "jobs",
    ];

    #[test]
    fn listed_names_are_the_sixteen_in_order() {
        let names: Vec<&str> = definitions().iter().map(|t| t.name).collect();
        assert_eq!(names, LISTED);
        let mcp: Vec<&str> = definitions_for(Surface::Mcp).map(|t| t.name).collect();
        assert_eq!(mcp, LISTED);
        // Jobs are for outside agents: the in-app agent has no jobs tool.
        let app: Vec<&str> = definitions_for(Surface::InAppAgent)
            .map(|t| t.name)
            .collect();
        assert_eq!(app, LISTED[..15]);
    }

    #[test]
    fn no_alias_or_hidden_name_is_listed() {
        for tool in definitions() {
            assert!(alias(tool.name).is_none(), "{} is an alias", tool.name);
            assert!(
                hidden().iter().all(|h| h.name != tool.name),
                "{} is hidden",
                tool.name
            );
        }
        for old in aliases() {
            assert!(
                definitions().iter().all(|t| t.name != old.name),
                "{}",
                old.name
            );
            assert_eq!(lookup(old.name).unwrap().name, old.target, "{}", old.name);
        }
        let hidden_def = lookup("feedback_list").expect("feedback_list is callable");
        assert_eq!(hidden_def.name, "feedback_list");
        assert!(hidden_def.read_only);
        assert!(get("feedback_list").is_none());
        let import = lookup("import_markdown").expect("import_markdown is callable");
        assert_eq!(import.name, "import_markdown");
        assert!(!import.read_only);
        assert!(get("import_markdown").is_none());
        assert!(alias("import_markdown").is_none());
    }

    #[test]
    fn mcp_only_tools_are_not_served_in_app() {
        // Every listed tool but jobs, feedback_submit included, is on both surfaces.
        let mcp: Vec<_> = definitions_for(Surface::Mcp).map(|t| t.name).collect();
        let app: Vec<_> = definitions_for(Surface::InAppAgent)
            .map(|t| t.name)
            .collect();
        assert!(app.contains(&"feedback_submit"));
        // jobs is MCP only: jobs come from outside agents.
        assert!(mcp.contains(&"jobs") && !app.contains(&"jobs"));
        for tool in definitions().iter().filter(|t| t.name != "jobs") {
            assert!(
                mcp.contains(&tool.name) && app.contains(&tool.name),
                "{}",
                tool.name
            );
        }
        // The hidden tools are MCP only: the in-app agent never gets them.
        for tool in hidden() {
            assert!(tool.serves(Surface::Mcp), "{}", tool.name);
            assert!(!tool.serves(Surface::InAppAgent), "{}", tool.name);
        }
    }

    #[test]
    fn feedback_submit_is_served_in_app() {
        let temp =
            Temp(std::env::temp_dir().join(format!("vault_tools_test_{}", vault_core::new_id())));
        let vault = Vault::open(temp.0.to_string_lossy().into_owned()).unwrap();
        let out = call_on(
            Surface::InAppAgent,
            &vault,
            "feedback_submit",
            &json!({"summary": "In-app report"}),
        )
        .unwrap();
        assert!(!out.is_empty());
        let list = call(&vault, "feedback_list", &json!({})).unwrap();
        assert!(list.contains("In-app report"), "{list}");
        assert_eq!(
            call_on(Surface::InAppAgent, &vault, "feedback_list", &json!({})).unwrap_err(),
            ToolError("unknown tool: feedback_list".into())
        );
        assert_eq!(IN_APP_TRANSPORT, "in-app agent");
    }

    #[test]
    fn create_page_roots_errors_on_every_surface() {
        const IMPORT: [&str; 6] = [
            "roots",
            "repo_path",
            "git_remote",
            "last_activity",
            "exclude",
            "dry_run",
        ];
        let create = get("create_page").unwrap();
        for surface in [Surface::Mcp, Surface::InAppAgent] {
            let schema = create.input_schema_for(surface);
            for k in IMPORT {
                assert!(
                    schema["properties"].get(k).is_none(),
                    "{surface:?} lists {k}"
                );
            }
            assert!(schema["properties"].get("title").is_some());
            assert!(schema["properties"].get("markdown").is_some());
        }
        assert!(!create.description.contains("roots"));
        assert!(!create.description.to_lowercase().contains("import"));

        let temp =
            Temp(std::env::temp_dir().join(format!("vault_tools_test_{}", vault_core::new_id())));
        let vault = Vault::open(temp.0.to_string_lossy().into_owned()).unwrap();
        let with_roots = json!({
            "title": "Demo",
            "roots": [{"name": "docs", "archive_base64": ""}]
        });
        let expected = ToolError(CREATE_PAGE_ROOTS_ERROR.into());
        for surface in [Surface::Mcp, Surface::InAppAgent] {
            assert_eq!(
                call_on(surface, &vault, "create_page", &with_roots).unwrap_err(),
                expected,
                "{surface:?}"
            );
        }
        assert_eq!(
            call(&vault, "create_page", &with_roots).unwrap_err(),
            expected
        );
        assert!(CREATE_PAGE_ROOTS_ERROR.contains("import_markdown"));
        // The import tool is MCP only.
        assert_eq!(
            call_on(
                Surface::InAppAgent,
                &vault,
                "import_markdown",
                &json!({"name": "Demo"})
            )
            .unwrap_err(),
            ToolError("unknown tool: import_markdown".into())
        );
        assert!(call_on(
            Surface::Mcp,
            &vault,
            "create_page",
            &json!({"title": "Plain"})
        )
        .unwrap()
        .starts_with("Created **Plain**"));
        assert!(call_on(
            Surface::InAppAgent,
            &vault,
            "create_page",
            &json!({"title": "Plain 2"})
        )
        .unwrap()
        .starts_with("Created **Plain 2**"));
    }

    #[test]
    fn definitions_match_dispatch_and_metadata() {
        let temp =
            Temp(std::env::temp_dir().join(format!("vault_tools_test_{}", vault_core::new_id())));
        let vault = Vault::open(temp.0.to_string_lossy().into_owned()).unwrap();
        let mut names = BTreeSet::new();
        let mut read_only = BTreeSet::new();
        let mut destructive = BTreeSet::new();

        for tool in definitions() {
            assert!(names.insert(tool.name), "duplicate tool: {}", tool.name);
            assert!(std::ptr::eq(get(tool.name).unwrap(), tool));
            assert_eq!(tool.approval, Approval::None, "{}", tool.name);
            if tool.read_only {
                read_only.insert(tool.name);
            }
            if tool.destructive_hint {
                destructive.insert(tool.name);
            }
            if let Err(error) = call(&vault, tool.name, &json!({})) {
                assert!(
                    !error.0.starts_with("unknown tool:"),
                    "undispatched definition: {}",
                    tool.name
                );
            }
        }

        assert_eq!(
            read_only,
            BTreeSet::from([
                "search_notes",
                "read_page",
                "memory_read",
                "notes_help",
            ])
        );
        assert_eq!(
            destructive,
            BTreeSet::from(["edit_page", "trash_page", "memory_write", "memory_delete"])
        );
        assert!(get("unknown").is_none());
        assert!(get("search").is_none(), "the old notes name has no alias");
        assert!(alias("search").is_none(), "the old notes name has no alias");
        assert_eq!(
            call(&vault, "search", &json!({ "query": "x" })).unwrap_err(),
            ToolError("unknown tool: search".into())
        );
        assert_eq!(
            call(&vault, "unknown", &json!({})).unwrap_err(),
            ToolError("unknown tool: unknown".into())
        );
    }
}
