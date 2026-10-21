//! `notes_help`: how Scratch notes are written, for every client. The pages
//! are markdown files in `docs/notes/`, compiled in. The tool list is built
//! from the registry at runtime; the block type, property type and system
//! role lists are doc sections that the drift tests below check against
//! the code.

use serde_json::Value;
use vault_core::PropKind;

use crate::Result;

/// The MCP server instructions (`initialize`), for every MCP client.
pub const SERVER_INSTRUCTIONS: &str = "Tools for the user's private notes vault (Scratch, a notes app). Pages hold markdown blocks; databases are pages whose child pages (rows) have properties. Use search_notes to find or list pages, read_page before editing, and edit_page with block ids (append, replace, delete) over rewriting whole pages. search_notes also lists planning data: is:todo,task,row,mention with date:, done:, assignee:, calendar:, in:[[Page]] or tag: returns dated and undated items, each with the ref update_task takes (notes_help topic query has the whole language). move_page moves pages and their sub-pages. read_page with block_ids includes nested editable ids. To link a page for the user, write [Title](<link>) with the link shown in tool results (the `link:` line); to link a block, use the `block link:` line. Call notes_help for how notes are written (markdown, links, properties, day notes, tasks, templates, system and memory pages). Memories are addressed by path (project/memory) with the memory_* tools. Every edit is saved to the page history and syncs to the user's devices.";

/// The memory rules agents get at session start (`notes memory rules`,
/// printed by the Claude Code plugin's session-start hook).
pub const MEMORY_RULES: &str = include_str!("memory_rules.md");

/// A help topic: an id for `notes_help(topic)`, a one-line summary for the
/// table of contents, and the page.
pub struct Topic {
    pub id: &'static str,
    pub summary: &'static str,
    body: Body,
}

enum Body {
    Doc(&'static str),
    /// Built from the code when asked for.
    Generated(fn() -> String),
}

macro_rules! doc {
    ($file:literal) => {
        Body::Doc(include_str!(concat!("../../../docs/notes/", $file)))
    };
}

static TOPICS: &[Topic] = &[
    Topic {
        id: "markdown",
        summary: "Markdown and blocks: how each block type is written and read back, and inline marks.",
        body: doc!("markdown.md"),
    },
    Topic {
        id: "links",
        summary: "Page links, embeds, linked pages and databases, images and files.",
        body: doc!("links.md"),
    },
    Topic {
        id: "properties",
        summary: "Page properties, databases and their schemas, property types and value formats.",
        body: doc!("properties.md"),
    },
    Topic {
        id: "day-notes",
        summary: "Daily notes: one page per date, their sections, and how to write into them.",
        body: doc!("day-notes.md"),
    },
    Topic {
        id: "tasks",
        summary: "To-dos, task notes, date mentions, dated database rows, item queries and update_task.",
        body: doc!("tasks.md"),
    },
    Topic {
        id: "query",
        summary: "The search language: words, operators, dates, item and database queries, result size (operator table generated from the code).",
        body: doc!("search.md"),
    },
    Topic {
        id: "templates",
        summary: "Template pages: how they are marked, used and kept out of search.",
        body: doc!("templates.md"),
    },
    Topic {
        id: "system-pages",
        summary: "Hidden system pages (Feedback and others), their roles, and what not to edit.",
        body: doc!("system-pages.md"),
    },
    Topic {
        id: "memory",
        summary: "Agent memory pages: projects, paths, fields and the memory_* tools.",
        body: doc!("memory.md"),
    },
    Topic {
        id: "tools",
        summary: "Every vault tool with its arguments (generated from the tool registry).",
        body: Body::Generated(tool_list),
    },
];

/// The help topics, in table-of-contents order.
pub fn topics() -> &'static [Topic] {
    TOPICS
}

impl Topic {
    /// The page, without the drift-check markers.
    pub fn text(&self) -> String {
        match &self.body {
            Body::Doc(text) => text
                .lines()
                .filter(|l| !is_marker(l))
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string(),
            Body::Generated(build) => build(),
        }
    }
}

fn is_marker(line: &str) -> bool {
    let l = line.trim();
    l.starts_with("<!-- check:")
        || l == "<!-- /check -->"
        || l.starts_with("<!-- generated:")
        || l == "<!-- /generated -->"
}

pub(crate) fn notes_help(a: &Value) -> Result<String> {
    let topic = a
        .get("topic")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty());
    Ok(match topic {
        None => contents(),
        Some(id) => match TOPICS.iter().find(|t| t.id.eq_ignore_ascii_case(id)) {
            Some(t) => t.text(),
            None => format!("No help topic {id:?}.\n\n{}", contents()),
        },
    })
}

fn contents() -> String {
    let mut out = String::from(
        "# Scratch notes help\n\nCall notes_help with one of these topics:\n\n",
    );
    for t in TOPICS {
        out.push_str(&format!("- `{}`: {}\n", t.id, t.summary));
    }
    out.trim_end().to_string()
}

/// `name(required, optional?)` and the description of every listed tool.
fn tool_list() -> String {
    let mut out = String::from("# Tools\n\nGenerated from the tool registry. Arguments marked ? are optional.\n");
    for t in crate::definitions() {
        let schema = t.input_schema();
        let required: Vec<&str> = schema["required"]
            .as_array()
            .map(|r| r.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        // Required arguments first, in the order the schema requires them,
        // then the optional ones.
        let mut args: Vec<String> = required.iter().map(|k| k.to_string()).collect();
        if let Some(p) = schema["properties"].as_object() {
            args.extend(
                p.keys()
                    .filter(|k| !required.contains(&k.as_str()))
                    .map(|k| format!("{k}?")),
            );
        }
        let only = if t.serves(crate::Surface::InAppAgent) {
            ""
        } else {
            " (MCP only)"
        };
        out.push_str(&format!(
            "\n## {}({}){only}\n\n{}\n",
            t.name,
            args.join(", "),
            t.description
        ));
    }
    out.trim_end().to_string()
}

/// The id the docs use for a property type. Exhaustive, so a new
/// [`PropKind`] fails to compile until it is named here and documented.
pub fn prop_type_id(k: PropKind) -> &'static str {
    match k {
        PropKind::Text => "text",
        PropKind::Number => "number",
        PropKind::Date => "date",
        PropKind::Checkbox => "checkbox",
        PropKind::Select => "select",
        PropKind::MultiSelect => "multi_select",
        PropKind::Status => "status",
        PropKind::Relation => "relation",
        PropKind::Url => "url",
        PropKind::Email => "email",
        PropKind::Phone => "phone",
        PropKind::CreatedTime => "created_time",
        PropKind::UpdatedTime => "updated_time",
        PropKind::Files => "files",
        PropKind::Formula => "formula",
        PropKind::Rollup => "rollup",
    }
}

/// Every property type; [`prop_type_id`] keeps it complete.
pub const PROP_KINDS: &[PropKind] = &[
    PropKind::Text,
    PropKind::Number,
    PropKind::Date,
    PropKind::Checkbox,
    PropKind::Select,
    PropKind::MultiSelect,
    PropKind::Status,
    PropKind::Relation,
    PropKind::Url,
    PropKind::Email,
    PropKind::Phone,
    PropKind::CreatedTime,
    PropKind::UpdatedTime,
    PropKind::Files,
    PropKind::Formula,
    PropKind::Rollup,
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;
    use vault_core::BlockKind;

    use super::*;

    fn docs() -> Vec<(&'static str, &'static str)> {
        TOPICS
            .iter()
            .filter_map(|t| match t.body {
                Body::Doc(text) => Some((t.id, text)),
                Body::Generated(_) => None,
            })
            .collect()
    }

    /// The first backticked word of each `- ` line in a checked section.
    fn section(name: &str) -> BTreeSet<String> {
        let open = format!("<!-- check: {name} -->");
        let mut found = None;
        for (id, text) in docs() {
            if let Some(start) = text.find(&open) {
                assert!(found.is_none(), "section {name} appears twice ({id})");
                let rest = &text[start + open.len()..];
                let end = rest.find("<!-- /check -->").unwrap_or_else(|| panic!("section {name} in {id} is not closed"));
                let items: BTreeSet<String> = rest[..end]
                    .lines()
                    .filter_map(|l| l.trim().strip_prefix("- `"))
                    .filter_map(|l| l.split('`').next())
                    .map(String::from)
                    .collect();
                found = Some(items);
            }
        }
        found.unwrap_or_else(|| panic!("no section {name} in docs/notes"))
    }

    fn assert_same(doc: BTreeSet<String>, code: BTreeSet<String>, what: &str) {
        let missing: Vec<_> = code.difference(&doc).collect();
        let unknown: Vec<_> = doc.difference(&code).collect();
        assert!(missing.is_empty(), "{what} missing from docs/notes: {missing:?}");
        assert!(unknown.is_empty(), "docs/notes name {what} that do not exist: {unknown:?}");
    }

    #[test]
    fn block_types_match_the_core() {
        let code = BlockKind::BUILTINS.iter().map(|k| k.as_str().to_string()).collect();
        assert_same(section("block-types"), code, "block types");
    }

    #[test]
    fn property_types_match_the_core() {
        let code: BTreeSet<String> = PROP_KINDS.iter().map(|k| prop_type_id(*k).to_string()).collect();
        assert_eq!(code.len(), PROP_KINDS.len(), "duplicate property type id");
        assert_same(section("property-types"), code, "property types");
    }

    #[test]
    fn system_roles_match_the_core() {
        let code = vault_core::PAGE_ROLES.iter().map(|r| r.to_string()).collect();
        assert_same(section("system-roles"), code, "system roles");
    }

    /// Backticked snake_case words: tool names, arguments, type ids, roles.
    fn snake_words(text: &str) -> BTreeSet<String> {
        text.split('`')
            .skip(1)
            .step_by(2)
            .map(|w| w.trim_end_matches("()").trim_end_matches('?'))
            .filter(|w| {
                let parts: Vec<&str> = w.split('_').collect();
                parts.len() > 1
                    && w.starts_with(|c: char| c.is_ascii_lowercase())
                    && parts.iter().all(|p| {
                        !p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                    })
            })
            .map(String::from)
            .collect()
    }

    fn schema_keys(schema: &Value, out: &mut BTreeSet<String>) {
        match schema {
            Value::Object(m) => {
                if let Some(Value::Object(props)) = m.get("properties") {
                    out.extend(props.keys().cloned());
                }
                for v in m.values() {
                    schema_keys(v, out);
                }
            }
            Value::Array(items) => items.iter().for_each(|v| schema_keys(v, out)),
            _ => {}
        }
    }

    /// Snake_case words the docs may use that are not tools or arguments.
    const OTHER_WORDS: &[&str] = &[
        "chat_ref", "multi_select", "created_time", "updated_time", "is_template",
        "is_not", "not_contains", "starts_with", "ends_with", "is_empty", "is_not_empty",
        "on_or_before", "on_or_after", "link_to_page", "page_id", "set_body", "set_props",
        // Agenda output/source names and the app's completion-view API.
        "page_title", "task_note", "done_since",
        // The hub setting that sets the base of page links (docs/notes/links.md).
        "public_url",
        // Hub settings for Android App Links (docs/notes/links.md).
        "android_package", "android_sha256_certs",
    ];

    #[test]
    fn every_tool_is_documented_and_docs_name_only_real_tools() {
        let listed: BTreeSet<String> = crate::definitions().iter().map(|t| t.name.to_string()).collect();
        let all_docs: String = docs().iter().map(|(_, t)| *t).collect::<Vec<_>>().join("\n");
        let words = snake_words(&all_docs);
        let ticked: BTreeSet<&str> = all_docs
            .split('`')
            .skip(1)
            .step_by(2)
            .map(|w| w.trim_end_matches("()").trim_end_matches('?'))
            .collect();
        let missing: Vec<_> = listed.iter().filter(|n| !ticked.contains(n.as_str())).collect();
        assert!(missing.is_empty(), "tools missing from docs/notes: {missing:?}");

        let old: Vec<_> = crate::aliases().iter().map(|a| a.name).chain(crate::hidden().iter().map(|t| t.name)).filter(|n| words.contains(*n)).collect();
        assert!(old.is_empty(), "docs/notes name old or unlisted tools: {old:?}");

        let mut known = listed.clone();
        for t in crate::definitions() {
            schema_keys(&t.input_schema(), &mut known);
        }
        known.extend(BlockKind::BUILTINS.iter().map(|k| k.as_str().to_string()));
        known.extend(PROP_KINDS.iter().map(|k| prop_type_id(*k).to_string()));
        known.extend(vault_core::PAGE_ROLES.iter().map(|r| r.to_string()));
        known.extend(OTHER_WORDS.iter().map(|w| w.to_string()));
        let unknown: Vec<_> = words.difference(&known).collect();
        assert!(
            unknown.is_empty(),
            "docs/notes name tools or arguments that do not exist (or add them to OTHER_WORDS): {unknown:?}"
        );
    }

    #[test]
    fn the_tool_topic_lists_every_tool_with_its_arguments() {
        let text = notes_help(&json!({ "topic": "tools" })).unwrap();
        for t in crate::definitions() {
            assert!(text.contains(&format!("## {}(", t.name)), "{}", t.name);
        }
        assert!(text.contains("## edit_page(page, ") && text.contains("op?"), "{text}");
        for a in crate::aliases() {
            assert!(!text.contains(&format!("## {}(", a.name)), "{}", a.name);
        }
    }

    #[test]
    fn contents_page_and_unknown_topic() {
        let toc = notes_help(&json!({})).unwrap();
        for t in TOPICS {
            assert!(toc.contains(&format!("- `{}`: ", t.id)), "{toc}");
        }
        assert_eq!(notes_help(&json!({ "topic": "" })).unwrap(), toc);
        let page = notes_help(&json!({ "topic": "markdown" })).unwrap();
        assert!(page.starts_with("# "), "{page}");
        assert!(!page.contains("<!-- check:"), "{page}");
        assert!(page.contains("`paragraph`"), "{page}");
        let unknown = notes_help(&json!({ "topic": "nope" })).unwrap();
        assert!(unknown.starts_with("No help topic \"nope\"."), "{unknown}");
        assert!(unknown.ends_with(&toc), "{unknown}");
    }

    /// Repo files that carry the generated operator table.
    const OPERATOR_DOCS: &[&str] = &["docs/notes/search.md", "wiki/architecture/search-queries.md"];
    const OPEN: &str = "<!-- generated: operators -->";
    const CLOSE: &str = "<!-- /generated -->";

    fn repo_file(rel: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(rel)
    }

    /// The text between the generated-section markers, and its byte range.
    fn generated(text: &str, rel: &str) -> (usize, usize) {
        let start = text.find(OPEN).unwrap_or_else(|| panic!("{rel} has no {OPEN} marker")) + OPEN.len();
        let end = start + text[start..].find(CLOSE).unwrap_or_else(|| panic!("{rel}: {OPEN} is not closed"));
        (start, end)
    }

    /// Fails when a repo doc's operator table is stale. Regenerate with
    /// `UPDATE_GENERATED_DOCS=1 cargo test -p vault_tools generated_operator`.
    #[test]
    fn generated_operator_tables_are_current() {
        let want = format!("\n{}", vault_core::search_ops::operator_reference());
        let update = std::env::var_os("UPDATE_GENERATED_DOCS").is_some();
        for rel in OPERATOR_DOCS {
            let path = repo_file(rel);
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{rel}: {e}"));
            let (start, end) = generated(&text, rel);
            if text[start..end] == want {
                continue;
            }
            if update {
                let fresh = format!("{}{want}{}", &text[..start], &text[end..]);
                std::fs::write(&path, fresh).unwrap();
                continue;
            }
            panic!("{rel}: the generated operator table is stale; rerun with UPDATE_GENERATED_DOCS=1");
        }
    }

    /// Fails when the query docs name an operator the parser doesn't know.
    #[test]
    fn query_docs_name_only_real_operators() {
        // Property names and reference prefixes the examples use.
        const NOT_OPERATORS: &[&str] = &["key", "status", "block", "note", "row", "mention"];
        let text = std::fs::read_to_string(repo_file("docs/notes/search.md")).unwrap();
        let mut unknown = Vec::new();
        for word in text.split('`').skip(1).step_by(2) {
            for part in word.split_whitespace() {
                let part = part.trim_start_matches('-');
                let Some((key, _)) = part.split_once(':') else { continue };
                if key.is_empty() || !key.chars().all(|c| c.is_ascii_lowercase()) {
                    continue;
                }
                if vault_core::search_ops::keyed_operator(key).is_none() && !NOT_OPERATORS.contains(&key) {
                    unknown.push(part.to_string());
                }
            }
        }
        assert!(unknown.is_empty(), "docs/notes/search.md names unknown operators: {unknown:?}");
        for op in vault_core::search_ops::OPERATORS {
            assert!(text.contains(&format!("`{}", op.name)), "operator {} is not in docs/notes/search.md", op.name);
        }
    }

    #[test]
    fn search_description_is_short_and_points_to_the_reference() {
        let d = crate::get("search_notes").unwrap().description;
        println!("search_notes description: {} bytes", d.len());
        assert!(d.len() < 2500, "search_notes description is {} bytes", d.len());
        assert!(d.contains("notes_help topic query"), "{d}");
        for op in vault_core::search_ops::OPERATORS {
            assert!(d.contains(op.name), "description lacks {}", op.name);
        }
        let page = notes_help(&json!({ "topic": "query" })).unwrap();
        assert!(page.contains("| `is:` |") && !page.contains(OPEN), "{page}");
    }

    #[test]
    fn every_doc_page_has_a_title_and_text() {
        for (id, text) in docs() {
            assert!(text.starts_with("# "), "{id} must start with a # title");
            assert!(text.len() > 200, "{id} is too short");
        }
    }
}
