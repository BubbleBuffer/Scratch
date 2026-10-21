//! The search operator table: the single source for the query language's
//! built-in operators. The parser's key recognition, the completion list,
//! the `search_notes` tool description, the `notes_help` topic `query` and
//! the generated section of `wiki/architecture/search-queries.md` all read
//! it. Drift tests check that every row is documented and handled.

use serde::Serialize;

/// Whether an operator selects results or only shapes how they are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum OperatorRole {
    /// Narrows the results.
    Filter,
    /// Controls output size; never filters. The app keeps these in a saved
    /// query's text and otherwise ignores them.
    Shape,
}

/// One row of the operator table.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct OperatorDoc {
    /// The key as typed before `:`, or a bare word (`undated`) or a sigil
    /// form (`#tag`, `<property>`).
    pub name: &'static str,
    /// The value grammar; empty for a bare word.
    pub values: &'static str,
    pub meaning: &'static str,
    pub example: &'static str,
    pub role: OperatorRole,
    /// A keyed built-in the parser recognizes as `name:value`.
    pub keyed: bool,
    /// Added with item search: a property with the same name keeps its
    /// filter (so saved queries on such properties do not change).
    pub yields_to_property: bool,
    /// Fixed values offered by completion.
    pub completions: &'static [&'static str],
}

const fn keyed(
    name: &'static str,
    values: &'static str,
    meaning: &'static str,
    example: &'static str,
) -> OperatorDoc {
    OperatorDoc {
        name,
        values,
        meaning,
        example,
        role: OperatorRole::Filter,
        keyed: true,
        yields_to_property: false,
        completions: &[],
    }
}

const fn new_keyed(
    name: &'static str,
    values: &'static str,
    meaning: &'static str,
    example: &'static str,
    role: OperatorRole,
    completions: &'static [&'static str],
) -> OperatorDoc {
    OperatorDoc {
        name,
        values,
        meaning,
        example,
        role,
        keyed: true,
        yields_to_property: true,
        completions,
    }
}

/// The built-in operators, in documentation order.
pub const OPERATORS: &[OperatorDoc] = &[
    keyed("created", "DATE", "page created in that period", "created:week"),
    keyed("updated", "DATE", "page edited in that period", "updated:7d"),
    keyed(
        "date",
        "DATE",
        "created, updated or any date property; with item operators, the item's date",
        "date:today..+7d",
    ),
    keyed(
        "tag",
        "name",
        "tagged (tag:work also matches work/meetings); #name is the same",
        "tag:work,home",
    ),
    keyed("type", "document|database|chat", "page type", "type:database"),
    keyed(
        "in",
        "[[Page]]|\"Title\"|memory|path",
        "that page and its subtree; a path glob (* one level, ** any depth) matches only the pages it names; memory is the agent memory pages",
        "in:Work/*",
    ),
    keyed("title", "word", "the title contains it", "title:plan"),
    keyed("links", "[[Page]]", "links to that page", "links:[[Roadmap]]"),
    keyed("from", "[[Page]]", "linked from that page", "from:[[Inbox]]"),
    keyed(
        "has",
        "link|attachment|<property>",
        "has a link, an attachment or a non-empty property",
        "has:attachment",
    ),
    keyed(
        "sort",
        "updated|created|title|<property>",
        "result order",
        "sort:title",
    ),
    new_keyed(
        "is",
        "todo|task|row|mention",
        "item results: to-do blocks, task notes, dated database rows, date mentions",
        "is:todo",
        OperatorRole::Filter,
        &["todo", "task", "row", "mention"],
    ),
    new_keyed(
        "done",
        "yes|no",
        "items that are (not) done; implies item results",
        "done:no",
        OperatorRole::Filter,
        &["yes", "no"],
    ),
    new_keyed(
        "assignee",
        "me|agent",
        "task notes assigned to you or the agent; implies item results",
        "assignee:agent",
        OperatorRole::Filter,
        &["me", "agent"],
    ),
    new_keyed(
        "calendar",
        "name",
        "task notes in that calendar category; implies item results",
        "calendar:Work",
        OperatorRole::Filter,
        &[],
    ),
    OperatorDoc {
        name: "undated",
        values: "",
        meaning: "with item operators: only items without a date (otherwise a plain word)",
        example: "is:todo undated",
        role: OperatorRole::Filter,
        keyed: false,
        yields_to_property: false,
        completions: &[],
    },
    new_keyed(
        "context",
        "N",
        "blocks shown around each match (default 2, max 20)",
        "context:0",
        OperatorRole::Shape,
        &[],
    ),
    new_keyed(
        "full",
        "CHARS",
        "show a page whole when it is at most this long (default 1500, 0 never)",
        "full:4000",
        OperatorRole::Shape,
        &[],
    ),
    new_keyed(
        "limit",
        "N",
        "hits per call (default 20, max 200)",
        "limit:50",
        OperatorRole::Shape,
        &[],
    ),
    OperatorDoc {
        name: "<property>",
        values: "value",
        meaning: "text contains; select option; number 5, >5 or 1..5; checkbox yes/no; dates use DATE; none means empty",
        example: "status:doing",
        role: OperatorRole::Filter,
        keyed: false,
        yields_to_property: false,
        completions: &[],
    },
];

/// Item kinds for `is:`, as the agenda names its sources.
pub const ITEM_KINDS: &[(&str, &str)] = &[
    ("todo", "todo"),
    ("task", "task_note"),
    ("row", "row"),
    ("mention", "mention"),
];

/// Keys that select item results (to-dos, task notes, rows, mentions).
pub const ITEM_KEYS: &[&str] = &["is", "done", "assignee", "calendar", "undated"];

/// Keys that shape output and never filter.
pub const SHAPE_KEYS: &[&str] = &["context", "full", "limit"];

/// Output defaults the tools use when the query has no shape operators.
pub const DEFAULT_CONTEXT: u32 = 2;
pub const MAX_CONTEXT: u32 = 20;
pub const DEFAULT_FULL_CHARS: u32 = 1500;
pub const MAX_FULL_CHARS: u32 = 20_000;
pub const DEFAULT_LIMIT: u32 = 20;
pub const MAX_LIMIT: u32 = 200;
/// The total size of one search result. Previews shrink before hits drop.
pub const OUTPUT_BUDGET_CHARS: usize = 12_000;

/// The row for a keyed built-in, by normalized key.
pub fn keyed_operator(name: &str) -> Option<&'static OperatorDoc> {
    OPERATORS.iter().find(|op| op.keyed && op.name == name)
}

/// The keyed built-in names, in table order.
pub fn keyed_names() -> impl Iterator<Item = &'static str> {
    OPERATORS.iter().filter(|op| op.keyed).map(|op| op.name)
}

fn usage(op: &OperatorDoc) -> String {
    if op.values.is_empty() {
        op.name.to_string()
    } else {
        format!("{}:{}", op.name, op.values)
    }
}

/// One line per operator: the short grammar for tool descriptions.
pub fn operator_grammar() -> String {
    OPERATORS
        .iter()
        .map(|op| format!("{} {}", usage(op), op.meaning))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The full reference as a markdown table, for `notes_help("query")` and
/// the repo doc.
pub fn operator_reference() -> String {
    let mut out =
        String::from("| Operator | Values | Meaning | Example | Kind |\n|---|---|---|---|---|\n");
    for op in OPERATORS {
        let name = if op.keyed {
            format!("`{}:`", op.name)
        } else {
            format!("`{}`", op.name)
        };
        let values = if op.values.is_empty() {
            "(none)".to_string()
        } else {
            format!("`{}`", op.values.replace('|', "\\|"))
        };
        out.push_str(&format!(
            "| {name} | {values} | {} | `{}` | {} |\n",
            op.meaning.replace('|', "\\|"),
            op.example,
            match op.role {
                OperatorRole::Filter => "filter",
                OperatorRole::Shape => "output size",
            }
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_documented() {
        for op in OPERATORS {
            assert!(!op.name.is_empty());
            assert!(!op.meaning.is_empty(), "{} has no meaning", op.name);
            assert!(!op.example.is_empty(), "{} has no example", op.name);
            if op.keyed {
                assert!(!op.values.is_empty(), "{} has no value grammar", op.name);
            }
        }
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<_> = OPERATORS.iter().map(|op| op.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), OPERATORS.len());
    }
}
