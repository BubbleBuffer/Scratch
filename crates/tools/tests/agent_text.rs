//! Drift tests for agent-facing text: the MCP server instructions, the
//! session-start memory rules, the Claude Code plugin skills, the
//! notes_help pages and the tool descriptions may only name tools and
//! arguments that exist in the registry, and never an old tool name.

use std::collections::BTreeSet;

use serde_json::Value;

const SKILLS: &[(&str, &str)] = &[
    (
        "skills/memory/SKILL.md",
        include_str!("../../../integrations/claude-plugin/skills/memory/SKILL.md"),
    ),
    (
        "skills/vault/SKILL.md",
        include_str!("../../../integrations/claude-plugin/skills/vault/SKILL.md"),
    ),
];

fn texts() -> Vec<(String, String)> {
    let mut out = vec![
        (
            "SERVER_INSTRUCTIONS".to_string(),
            vault_tools::help::SERVER_INSTRUCTIONS.to_string(),
        ),
        (
            "MEMORY_RULES".to_string(),
            vault_tools::help::MEMORY_RULES.to_string(),
        ),
    ];
    out.extend(SKILLS.iter().map(|(n, t)| (n.to_string(), t.to_string())));
    for topic in vault_tools::help::topics() {
        out.push((format!("notes_help {}", topic.id), topic.text()));
    }
    for t in vault_tools::definitions() {
        out.push((format!("{} description", t.name), t.description.to_string()));
    }
    out
}

fn listed() -> BTreeSet<&'static str> {
    vault_tools::definitions().iter().map(|t| t.name).collect()
}

/// Identifier-like words (letters, digits, `_`) with their byte offsets.
fn words(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        let part = c.is_ascii_alphanumeric() || c == '_';
        match (part, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s, &text[s..i]));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, &text[s..]));
    }
    out
}

/// The argument names a tool's schema accepts, including edit_page's
/// per-op fields.
fn args_of(name: &str) -> BTreeSet<String> {
    fn keys(schema: &Value, out: &mut BTreeSet<String>) {
        if let Some(Value::Object(props)) = schema.get("properties") {
            out.extend(props.keys().cloned());
        }
        if let Some(items) = schema.pointer("/properties/ops/items") {
            keys(items, out);
        }
    }
    let mut out = BTreeSet::new();
    keys(&vault_tools::get(name).unwrap().input_schema(), &mut out);
    out
}

#[test]
fn agent_text_names_only_listed_tools() {
    let listed = listed();
    let old: BTreeSet<&str> = vault_tools::aliases()
        .iter()
        .map(|a| a.name)
        .chain(vault_tools::hidden().iter().map(|t| t.name))
        .collect();
    fn enums(v: &Value, out: &mut BTreeSet<String>) {
        match v {
            Value::Object(m) => {
                if let Some(Value::Array(e)) = m.get("enum") {
                    out.extend(e.iter().filter_map(Value::as_str).map(String::from));
                }
                m.values().for_each(|v| enums(v, out));
            }
            Value::Array(a) => a.iter().for_each(|v| enums(v, out)),
            _ => {}
        }
    }
    let mut enum_values = BTreeSet::new();
    for t in vault_tools::definitions() {
        enums(&t.input_schema(), &mut enum_values);
    }
    let mut problems = Vec::new();
    for (source, text) in texts() {
        for (_, w) in words(&text) {
            let w = w.strip_prefix("mcp__plugin_notes_notes__").unwrap_or(w);
            // Old snake_case tool names; one-word ones (agenda, overdue,
            // backlinks, reschedule) are ordinary words unless backticked.
            if w.contains('_') && old.contains(w) {
                problems.push(format!("{source}: old tool name {w}"));
            }
            // `memory_*` stands for the family.
            if w.starts_with("memory_") && w != "memory_" && !listed.contains(w) {
                problems.push(format!("{source}: no tool {w}"));
            }
        }
        for one_word in old.iter().filter(|o| !o.contains('_')) {
            // A backticked value a listed tool accepts (agenda's `overdue`
            // view) is not a tool name; a call `overdue(` always is.
            let value = enum_values.contains(*one_word);
            if (!value && text.contains(&format!("`{one_word}`")))
                || text.contains(&format!("`{one_word}("))
            {
                problems.push(format!("{source}: old tool name {one_word}"));
            }
        }
        for (_, w) in words(&text) {
            if let Some(tool) = w.strip_prefix("mcp__plugin_notes_notes__") {
                if !listed.contains(tool) {
                    problems.push(format!("{source}: no tool {tool}"));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn agent_text_names_only_real_arguments() {
    let listed = listed();
    let mut problems = Vec::new();
    for (source, text) in texts() {
        for (at, w) in words(&text) {
            if !listed.contains(w) {
                continue;
            }
            // `tool(arg, arg?: ..., arg=...)`: each arg must be in the schema.
            let rest = &text[at + w.len()..];
            let Some(inner) = rest.strip_prefix('(') else {
                continue;
            };
            let Some(end) = inner.find(')') else {
                continue;
            };
            let known = args_of(w);
            for part in inner[..end].split(',') {
                let arg: String = part
                    .trim()
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                // Literal values (`notes_help("markdown")`) are not names.
                if arg.is_empty() || part.trim().starts_with(['"', '\'', '`']) {
                    continue;
                }
                if !known.contains(&arg) {
                    problems.push(format!("{source}: {w}({arg}) has no argument {arg}"));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

/// The user's term rule: a memory is addressed by its `path`; agent text
/// never calls the identifier a name.
#[test]
fn memory_text_calls_the_identifier_path() {
    const NAME_AS_ID: &[&str] = &[
        "memory's name",
        "memory name",
        "by name",
        "<name>",
        "name: global/",
        "name=global/",
    ];
    let mut problems = Vec::new();
    let memory_text = |source: &str| {
        source == "MEMORY_RULES" || source == "SERVER_INSTRUCTIONS" || source.contains("memory")
    };
    for (source, text) in texts().into_iter().filter(|(s, _)| memory_text(s)) {
        let lower = text.to_ascii_lowercase();
        for phrase in NAME_AS_ID {
            if lower.contains(phrase) {
                problems.push(format!("{source}: {phrase:?}"));
            }
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
    for tool in [
        "memory_read",
        "memory_update",
        "memory_delete",
        "memory_write",
    ] {
        assert!(args_of(tool).contains("path"), "{tool}");
    }
}

#[test]
fn planning_text_and_generated_tools_use_search_notes() {
    let listed = listed();
    // agenda is a hidden alias of search_notes item queries; tasks is gone.
    assert!(!listed.contains("agenda"));
    assert!(listed.contains("search_notes"));
    assert!(listed.contains("move_page"));
    assert!(!listed.contains("tasks"));
    assert!(vault_tools::alias("tasks").is_none());
    assert!(vault_tools::lookup("tasks").is_none());
    assert_eq!(vault_tools::alias("agenda").unwrap().target, "search_notes");
    let generated = vault_tools::help::topics()
        .iter()
        .find(|t| t.id == "tools")
        .unwrap()
        .text();
    assert!(generated.contains("## search_notes("), "{generated}");
    assert!(generated.contains("## move_page("), "{generated}");
    assert!(!generated.contains("## agenda("), "{generated}");
    assert!(!generated.contains("## tasks("), "{generated}");
    for arg in [
        "query",
        "parent",
        "depth",
        "backlinks_of",
        "limit",
        "offset",
    ] {
        assert!(
            args_of("search_notes").contains(arg),
            "search_notes missing {arg}"
        );
    }
    assert!(vault_tools::help::SERVER_INSTRUCTIONS.contains("search_notes"));
    assert!(vault_tools::help::SERVER_INSTRUCTIONS.contains("move_page"));
    assert!(!vault_tools::help::SERVER_INSTRUCTIONS.contains("agenda"));
    let manifest: Value = serde_json::from_str(include_str!(
        "../../../integrations/claude-plugin/.claude-plugin/plugin.json"
    ))
    .unwrap();
    assert_eq!(manifest["version"], "0.6.5");
}
