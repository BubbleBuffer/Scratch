//! Search query parser: text with `key:value` operators becomes a
//! [`SearchSpec`] (what the search runs) and byte-spanned [`Token`]s (what the
//! UI marks). Syntax, date grammar and bad-input rules are settled in
//! `wiki/architecture/search-queries.md`.
//!
//! Parsing never fails. A known key with an unreadable value becomes an
//! `Error` token, is left out of the spec and adds a warning. An unknown
//! `word:` is plain text.
//!
//! Quoting: `"..."` groups a word. Inside quotes a backslash escapes the next
//! character; outside quotes a backslash is an ordinary character.

use crate::search_ops::{
    keyed_operator, ITEM_KEYS, ITEM_KINDS, MAX_CONTEXT, MAX_FULL_CHARS, MAX_LIMIT, SHAPE_KEYS,
};
use crate::types::{Condition, Filter, FilterOp, PropDef, PropKind, PropValue};
use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, TimeDelta, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// Ephemeral parser/completion metadata; never stored in a page's schema.
pub(crate) const TASK_STATUS_CATALOG_CONFIG: &str = r#"{"$task_status_projection":true}"#;

pub(crate) fn is_task_status_catalog(def: &PropDef) -> bool {
    def.config == TASK_STATUS_CATALOG_CONFIG
}

/// What a search runs: terms, operators, sort and the warnings for bad input.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchSpec {
    pub terms: Vec<SearchTerm>,
    pub operators: Vec<SearchOperator>,
    /// `updated`, `created`, `title` or a property's stable key.
    pub sort: Option<String>,
    pub warnings: Vec<String>,
}

impl SearchSpec {
    /// Whether the query selects items (to-dos, task notes, rows, mentions)
    /// rather than pages: it has `is:`, `done:`, `assignee:`, `calendar:` or
    /// `undated`.
    pub fn selects_items(&self) -> bool {
        self.operators
            .iter()
            .any(|op| !op.property && ITEM_KEYS.contains(&op.key.as_str()))
    }

    /// The value of a size control (`context:`, `full:`, `limit:`); the last
    /// one wins. Size controls never filter.
    pub fn shape(&self, key: &str) -> Option<u32> {
        self.operators
            .iter()
            .rev()
            .find(|op| !op.property && op.key == key)
            .and_then(|op| op.value.parse().ok())
    }
}

/// Whether an operator is executed by the item search or only shapes
/// output, so the page filters skip it.
pub(crate) fn is_item_or_shape(op: &SearchOperator) -> bool {
    !op.property && (ITEM_KEYS.contains(&op.key.as_str()) || SHAPE_KEYS.contains(&op.key.as_str()))
}

/// Vault facts the parser resolves `in:memory` and `calendar:` against.
#[derive(Debug, Clone, Default)]
pub struct SearchExtras {
    /// The top-level `Memory` page.
    pub memory_root: Option<String>,
    /// Calendar categories as (id, name).
    pub calendars: Vec<(String, String)>,
}

/// A word or phrase to match in the text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTerm {
    pub value: String,
    pub phrase: bool,
    pub exclude: bool,
}

/// A `key:value` operator.
///
/// `value` is the canonical text of the comma alternatives. Each alternative
/// is quoted only when it holds a comma, quote, backslash, bracket or
/// whitespace, so [`alternatives`] splits it back exactly. Page values print as
/// `[[id]],[[id]]`, so a printed query survives renames. `filter` is always in
/// its positive form: `negated` means the coordinator applies NOT.
///
/// Property operators and `created`, `updated`, `title`, `date` compile a
/// `filter`. `tag`, `in`, `links`, `from`, `has` have no filter; the
/// coordinator executes them from `value` and `resolved_id`. For `in`, `links`
/// and `from`, `resolved_id` holds the first page id (all ids are in `value`).
/// For `has:<prop>` with one property, it holds the property key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchOperator {
    /// A built-in name (lowercase) or a property's stable key.
    pub key: String,
    /// Canonical alternatives, see above.
    pub value: String,
    pub negated: bool,
    pub property: bool,
    pub resolved_id: Option<String>,
    pub filter: Option<Filter>,
    /// The date reaches the future: a `+N` form, or a range or comparison
    /// that does. Sorting uses it: future ranges soonest first.
    pub date_future: bool,
}

/// The parse of one query string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedQuery {
    pub spec: SearchSpec,
    pub tokens: Vec<Token>,
}

/// A word's byte span in the query, for chips and completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub start: u32,
    pub end: u32,
    pub kind: TokenKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenKind {
    Text {
        value: String,
    },
    Phrase {
        value: String,
    },
    Exclude {
        value: String,
        phrase: bool,
    },
    Operator {
        key: String,
        value: String,
        resolved_id: Option<String>,
    },
    Error {
        message: String,
    },
}

/// A page the parser can resolve `in:`, `links:`, `from:` and relation values to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchPage {
    pub id: String,
    pub title: String,
    pub updated_ms: i64,
}

/// What the parser knows: the schema, the pages, the clock and the time zone.
///
/// Day boundaries and relative dates use `tz`. Production uses
/// [`SearchContext::local`]. Tests and callers that need a fixed offset or a
/// named zone pass any `chrono::TimeZone` (for example `FixedOffset`, or a
/// DST-aware zone from a crate such as `chrono-tz`).
#[derive(Debug, Clone)]
pub struct SearchContext<Z: TimeZone = Local> {
    pub properties: Vec<PropDef>,
    pub pages: Vec<SearchPage>,
    pub now_ms: i64,
    pub tz: Z,
}

impl SearchContext<Local> {
    /// The context for production: the machine's local time zone.
    pub fn local(properties: Vec<PropDef>, pages: Vec<SearchPage>, now_ms: i64) -> Self {
        SearchContext {
            properties,
            pages,
            now_ms,
            tz: Local,
        }
    }
}

impl Default for SearchContext<Local> {
    /// No schema, no pages, the current time, the local time zone.
    fn default() -> Self {
        SearchContext::local(Vec::new(), Vec::new(), Utc::now().timestamp_millis())
    }
}

/// Parses with an empty schema, no pages, the current time and the local zone.
pub fn parse_search(query: &str) -> ParsedQuery {
    parse_search_with_context(query, &SearchContext::<Local>::default())
}

/// Parses against the schema, pages, clock and time zone in `ctx`.
pub fn parse_search_with_context<Z: TimeZone>(query: &str, ctx: &SearchContext<Z>) -> ParsedQuery {
    Parser::new(ctx).run(query)
}

/// Parse against the Vault's cached metadata without allocating a copy per
/// keystroke. The public owned context remains useful for deterministic tests.
pub(crate) fn parse_search_with_metadata(
    query: &str,
    properties: &[PropDef],
    pages: &[SearchPage],
    now_ms: i64,
    extras: &SearchExtras,
) -> ParsedQuery {
    let mut parser = Parser::with_metadata(properties, pages, now_ms, &Local);
    parser.extras = Some(extras);
    parser.run(query)
}

/// Parses against `ctx` plus the memory root and calendar categories.
pub fn parse_search_with_extras<Z: TimeZone>(
    query: &str,
    ctx: &SearchContext<Z>,
    extras: &SearchExtras,
) -> ParsedQuery {
    let mut parser = Parser::new(ctx);
    parser.extras = Some(extras);
    parser.run(query)
}

impl SearchSpec {
    /// Prints the spec back as query text. It parses back to the same spec when
    /// the schema and pages are unchanged; pages print by id, so renames do not
    /// change the text. Terms come first, then operators in their order, then
    /// the sort.
    pub fn to_query_string(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for term in &self.terms {
            let body = if term.phrase {
                format!("\"{}\"", escape(&term.value))
            } else if let Some((key, value)) = term.value.split_once(':') {
                format!("{key}:{}", encode_one(value))
            } else {
                term.value.clone()
            };
            parts.push(if term.exclude {
                format!("-{body}")
            } else {
                body
            });
        }
        for op in &self.operators {
            let prefix = if op.negated { "-" } else { "" };
            if !op.property && op.key == "undated" {
                parts.push(format!("{prefix}undated"));
                continue;
            }
            let status_alias =
                op.key == "status" && op.filter.as_ref().is_some_and(status_has_alias);
            let key = if op.property && !status_alias {
                format!("\"{}\"", escape(&op.key))
            } else {
                op.key.clone()
            };
            parts.push(format!("{prefix}{key}:{}", op.value));
        }
        if let Some(sort) = &self.sort {
            let (key, direction) = sort_direction(sort);
            let key = if needs_quotes(key) {
                format!("\"{}\"", escape(key))
            } else {
                key.to_string()
            };
            let direction = match direction {
                Some(true) => ",desc",
                Some(false) => ",asc",
                None => "",
            };
            parts.push(format!("sort:{key}{direction}"));
        }
        parts.join(" ")
    }
}

// ------------------------------------------------------------------ parser

/// Named periods that `bounds` knows. Each is a whole local period.
const NAMED_DATES: [&str; 8] = [
    "today",
    "yesterday",
    "tomorrow",
    "week",
    "lastweek",
    "month",
    "lastmonth",
    "year",
];

struct SearchInput<'a, Z: TimeZone> {
    properties: &'a [PropDef],
    pages: &'a [SearchPage],
    now_ms: i64,
    tz: &'a Z,
}

struct Parser<'a, Z: TimeZone> {
    ctx: SearchInput<'a, Z>,
    /// The clock as a wall-clock time in `ctx.tz`; `None` if the zone cannot show it.
    now_local: Option<NaiveDateTime>,
    spec: SearchSpec,
    tokens: Vec<Token>,
    extras: Option<&'a SearchExtras>,
    /// The query has a built-in `is:`: item keys win over same-named properties.
    has_is: bool,
    /// The query selects items, so `undated` and `date:` are item filters.
    item_mode: bool,
}

/// One operator before it is recorded in the spec and its token.
struct Built {
    key: String,
    value: String,
    resolved_id: Option<String>,
    filter: Option<Filter>,
    property: bool,
    future: bool,
    sort: Option<String>,
}

impl Built {
    /// An operator that compiles to a filter.
    fn filtered(key: &str, value: String, filter: Filter, property: bool, future: bool) -> Self {
        Built {
            key: key.to_string(),
            value,
            resolved_id: None,
            filter: Some(filter),
            property,
            future,
            sort: None,
        }
    }

    /// An operator the coordinator executes (tag, in, links, from, has, sort).
    fn special(key: &str, value: String, resolved_id: Option<String>) -> Self {
        Built {
            key: key.to_string(),
            value,
            resolved_id,
            filter: None,
            property: false,
            future: false,
            sort: None,
        }
    }
}

/// What a keyed word refers to.
enum Target<'c> {
    Builtin(&'static str),
    Property(&'c PropDef),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmp {
    Gt,
    Ge,
    Lt,
    Le,
}

/// A window of time in epoch milliseconds, `end` exclusive, and whether it
/// reaches the future.
#[derive(Debug, Clone, Copy)]
struct Span {
    start: i64,
    end: i64,
    future: bool,
}

impl<'a, Z: TimeZone> Parser<'a, Z> {
    fn new(ctx: &'a SearchContext<Z>) -> Self {
        Self::with_metadata(&ctx.properties, &ctx.pages, ctx.now_ms, &ctx.tz)
    }

    fn with_metadata(
        properties: &'a [PropDef],
        pages: &'a [SearchPage],
        now_ms: i64,
        tz: &'a Z,
    ) -> Self {
        let now_local = tz
            .timestamp_millis_opt(now_ms)
            .earliest()
            .map(|at| at.naive_local());
        Parser {
            ctx: SearchInput {
                properties,
                pages,
                now_ms,
                tz,
            },
            now_local,
            spec: SearchSpec::default(),
            tokens: Vec::new(),
            extras: None,
            has_is: false,
            item_mode: false,
        }
    }

    fn run(mut self, query: &str) -> ParsedQuery {
        let words = split_words(query);
        let keys: Vec<String> = words
            .iter()
            .filter_map(|&(start, end)| plain_key(&query[start..end]))
            .collect();
        let properties = self.ctx.properties;
        let unshadowed = |key: &str| find_prop(properties, key).is_none();
        self.has_is = keys.iter().any(|k| k == "is" && unshadowed(k));
        self.item_mode = self.has_is
            || keys
                .iter()
                .any(|k| matches!(k.as_str(), "done" | "assignee" | "calendar") && unshadowed(k));
        for (start, end) in words {
            self.word(query, start, end);
        }
        ParsedQuery {
            spec: self.spec,
            tokens: self.tokens,
        }
    }

    /// Classifies one whitespace-separated word.
    fn word(&mut self, src: &str, start: usize, end: usize) {
        let at = (start, end);
        let (neg, body_start) = match src[start..end].strip_prefix('-') {
            Some(_) => (true, start + 1),
            None => (false, start),
        };
        let body = &src[body_start..end];
        if body.is_empty() {
            return self.fail(at, "'-' needs a word or an operator".to_string());
        }
        if let Some(inner) = body.strip_prefix('"') {
            let (text, after) = unquote(inner);
            // `"due date":today` is a quoted key, not a phrase.
            if let Some(value) = after.strip_prefix(':') {
                return self.keyed(at, neg, &text, value, true);
            }
            if text.is_empty() {
                return self.fail(at, "empty phrase".to_string());
            }
            return self.push_term(at, text, true, neg);
        }
        if body.len() > 1 {
            if let Some(tag) = body.strip_prefix('#') {
                return self.keyed(at, neg, "tag", tag, false);
            }
        }
        if let Some(colon) = body.find(':') {
            let key = &body[..colon];
            if colon > 0 && !key.chars().any(|c| c == '[' || c == '"') {
                return self.keyed(at, neg, key, &body[colon + 1..], false);
            }
        }
        if self.item_mode && body.eq_ignore_ascii_case("undated") {
            return self.add(at, neg, Built::special("undated", "yes".into(), None));
        }
        self.push_term(at, body.replace('"', ""), false, neg)
    }

    /// A word with a `key:value` shape.
    fn keyed(&mut self, at: (usize, usize), neg: bool, key: &str, value: &str, quoted_key: bool) {
        let ctx = &self.ctx;
        let norm_key = normalize(key);
        // Built-ins win over property keys, unless the key is quoted.
        let builtin = if quoted_key {
            None
        } else {
            builtin_key(&norm_key)
        };
        // Operators added with item search yield to a same-named property,
        // so saved queries on such properties keep their meaning. In a query
        // with `is:` the item keys are always item filters.
        let has_is = self.has_is;
        let builtin = builtin.filter(|name| {
            let yields = keyed_operator(name).is_some_and(|op| op.yields_to_property);
            !yields
                || find_prop(ctx.properties, &norm_key).is_none()
                || (has_is && ITEM_KEYS.contains(name))
        });
        let target = match (builtin, find_prop(&ctx.properties, &norm_key)) {
            (Some(name), _) => Target::Builtin(name),
            (None, Some(def)) => Target::Property(def),
            (None, None) => {
                // An unknown word is text.
                let value = value.strip_prefix('"').map_or_else(
                    || value.replace('"', ""),
                    |inner| {
                        let (text, after) = unquote(inner);
                        format!("{text}{after}")
                    },
                );
                let text = format!("{key}:{value}");
                let phrase = quoted_key && key.chars().any(char::is_whitespace);
                return self.push_term(at, text, phrase, neg);
            }
        };
        if value.is_empty() {
            return self.fail(at, format!("{key}: needs a value"));
        }
        if let Some(problem) = quoting_problem(value, matches!(target, Target::Builtin("in"))) {
            return self.fail(at, format!("{key}: {problem}"));
        }
        let built = if !quoted_key
            && norm_key == "status"
            && find_prop(&ctx.properties, "status").is_some_and(is_task_status_catalog)
        {
            self.status_op(value)
        } else {
            match target {
                Target::Builtin(name) => self.builtin_op(name, value),
                Target::Property(def) => self.property_op(def, value),
            }
        };
        match built {
            Ok(built) => self.add(at, neg, built),
            Err(message) => self.fail(at, message),
        }
    }

    /// The task projection shares its search spelling with ordinary database
    /// properties named Status, whose stable keys are normally generated IDs.
    fn status_op(&self, value: &str) -> Result<Built, String> {
        let mut filters = Vec::new();
        let mut single_value = None;
        let mut future = false;
        let mut first_error = None;
        for def in self
            .ctx
            .properties
            .iter()
            .filter(|def| normalize(&def.key) == "status" || normalize(&def.name) == "status")
        {
            match self.property_op(def, value) {
                Ok(built) => {
                    if let Some(filter) = built.filter {
                        filters.push(filter);
                        single_value = Some(built.value);
                        future |= built.future;
                    }
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        if filters.is_empty() {
            return Err(first_error.unwrap_or_else(|| "status: no matching property".into()));
        }
        let canonical = if filters.len() == 1 {
            single_value.expect("a successful property supplied its canonical value")
        } else {
            encode_alternatives(&alternatives(value))
        };
        let empty =
            matches!(alternatives(value).as_slice(), [only] if only.eq_ignore_ascii_case("none"));
        let filter = if empty && filters.len() > 1 {
            // A shared field is empty only when every alias is absent.
            Filter::And(filters)
        } else {
            combine(filters)
        };
        Ok(Built::filtered("status", canonical, filter, true, future))
    }

    fn add(&mut self, at: (usize, usize), neg: bool, built: Built) {
        let Built {
            key,
            value,
            resolved_id,
            filter,
            property,
            future,
            sort,
        } = built;
        if key == "sort" {
            if neg {
                return self.fail(at, "sort can't be negated".to_string());
            }
            self.spec.sort = sort;
            return self.push_token(
                at,
                TokenKind::Operator {
                    key,
                    value,
                    resolved_id: None,
                },
            );
        }
        self.push_token(
            at,
            TokenKind::Operator {
                key: key.clone(),
                value: value.clone(),
                resolved_id: resolved_id.clone(),
            },
        );
        self.spec.operators.push(SearchOperator {
            key,
            value,
            negated: neg,
            property,
            resolved_id,
            filter,
            date_future: future,
        });
    }

    fn push_term(&mut self, at: (usize, usize), value: String, phrase: bool, exclude: bool) {
        let kind = if exclude {
            TokenKind::Exclude {
                value: value.clone(),
                phrase,
            }
        } else if phrase {
            TokenKind::Phrase {
                value: value.clone(),
            }
        } else {
            TokenKind::Text {
                value: value.clone(),
            }
        };
        self.push_token(at, kind);
        self.spec.terms.push(SearchTerm {
            value,
            phrase,
            exclude,
        });
    }

    fn push_token(&mut self, at: (usize, usize), kind: TokenKind) {
        self.tokens.push(Token {
            start: at.0 as u32,
            end: at.1 as u32,
            kind,
        });
    }

    fn fail(&mut self, at: (usize, usize), message: String) {
        self.spec.warnings.push(message.clone());
        self.push_token(at, TokenKind::Error { message });
    }

    /// Compiles a built-in operator. Errors are messages for the warning.
    fn builtin_op(&self, key: &'static str, value: &str) -> Result<Built, String> {
        let texts = alternatives(value);
        if texts.is_empty() {
            return Err(format!("{key}: needs a value"));
        }
        let canonical = encode_alternatives(&texts);
        match key {
            "tag" | "type" => Ok(Built::special(key, canonical, None)),
            "in" => self.in_op(value),
            "links" | "from" => {
                let mut ids = Vec::with_capacity(texts.len());
                for text in &texts {
                    let name = strip_brackets(text);
                    let memory = (key == "in"
                        && name == text.as_str()
                        && name.eq_ignore_ascii_case("memory"))
                    .then(|| self.extras.and_then(|x| x.memory_root.clone()))
                    .flatten();
                    let id = memory
                        .or_else(|| self.resolve_page(name))
                        .ok_or_else(|| format!("{key}: no page named '{name}'"))?;
                    ids.push(id);
                }
                Ok(Built::special(key, page_value(&ids), ids.first().cloned()))
            }
            "has" => self.has_op(&texts),
            "sort" => self.sort_op(&texts),
            "is" => {
                let mut kinds = Vec::with_capacity(texts.len());
                for text in &texts {
                    let lower = text.to_ascii_lowercase();
                    let kind = ITEM_KINDS
                        .iter()
                        .find(|(name, source)| *name == lower || *source == lower)
                        .ok_or_else(|| format!("is: '{text}' is not todo, task, row or mention"))?;
                    kinds.push(kind.0.to_string());
                }
                Ok(Built::special(key, encode_alternatives(&kinds), None))
            }
            "done" => match texts.as_slice() {
                [one] => match one.to_ascii_lowercase().as_str() {
                    "yes" | "true" => Ok(Built::special(key, "yes".into(), None)),
                    "no" | "false" => Ok(Built::special(key, "no".into(), None)),
                    _ => Err(format!("done: '{one}' is not yes or no")),
                },
                _ => Err("done: takes one value, yes or no".into()),
            },
            "assignee" => {
                let mut who = Vec::with_capacity(texts.len());
                for text in &texts {
                    let lower = text.to_ascii_lowercase();
                    if !matches!(lower.as_str(), "me" | "agent") {
                        return Err(format!("assignee: '{text}' is not me or agent"));
                    }
                    who.push(lower);
                }
                Ok(Built::special(key, encode_alternatives(&who), None))
            }
            "calendar" => {
                let calendars = self.extras.map(|x| x.calendars.as_slice()).unwrap_or(&[]);
                let mut ids = Vec::with_capacity(texts.len());
                for text in &texts {
                    let name = strip_brackets(text);
                    let found = calendars
                        .iter()
                        .find(|(id, _)| id == name)
                        .or_else(|| calendars.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)))
                        .ok_or_else(|| format!("calendar: no calendar named '{name}'"))?;
                    ids.push(found.0.clone());
                }
                Ok(Built::special(key, page_value(&ids), ids.first().cloned()))
            }
            "context" | "full" | "limit" => {
                let [one] = texts.as_slice() else {
                    return Err(format!("{key}: takes one number"));
                };
                let n: u32 = one
                    .parse()
                    .map_err(|_| format!("{key}: '{one}' is not a number"))?;
                let (low, high) = match key {
                    "context" => (0, MAX_CONTEXT),
                    "full" => (0, MAX_FULL_CHARS),
                    _ => (1, MAX_LIMIT),
                };
                if !(low..=high).contains(&n) {
                    return Err(format!("{key}: must be between {low} and {high}"));
                }
                Ok(Built::special(key, n.to_string(), None))
            }
            "title" => {
                let filters: Vec<Filter> = texts
                    .iter()
                    .map(|t| cond("$title", FilterOp::Contains, PropValue::Text(t.clone())))
                    .collect();
                Ok(Built::filtered(
                    key,
                    canonical,
                    combine(filters),
                    false,
                    false,
                ))
            }
            "created" | "updated" => {
                let prop = if key == "created" {
                    "$created"
                } else {
                    "$updated"
                };
                let keys = [prop.to_string()];
                let (filter, future) = self
                    .combine_alts(&texts, |p, t| p.date_alt(&keys, t))
                    .map_err(|e| format!("{key}: {e}"))?;
                Ok(Built::filtered(key, canonical, filter, false, future))
            }
            "date" if self.item_mode => {
                // Item searches filter the item's own date.
                let keys = [ITEM_DATE.to_string()];
                let (filter, future) = self
                    .combine_alts(&texts, |p, t| p.date_alt(&keys, t))
                    .map_err(|e| format!("date: {e}"))?;
                Ok(Built::filtered(key, canonical, filter, false, future))
            }
            "date" => {
                // The built-in dates, and every date property through `*`
                // (vault scope), so a stored spec matches new date properties.
                let keys = [
                    "$created".to_string(),
                    "$updated".to_string(),
                    "*".to_string(),
                ];
                let (filter, future) = self
                    .combine_alts(&texts, |p, t| p.date_alt(&keys, t))
                    .map_err(|e| format!("date: {e}"))?;
                Ok(Built::filtered(key, canonical, filter, false, future))
            }
            _ => Err(format!("unknown operator {key}")),
        }
    }

    /// `has:link`, `has:attachment` or `has:<property>`. Comma alternatives are OR.
    fn has_op(&self, texts: &[String]) -> Result<Built, String> {
        let mut values = Vec::with_capacity(texts.len());
        for text in texts {
            let value = if is_presence_word(text) {
                match text.to_ascii_lowercase().as_str() {
                    "attachment" | "attachments" => "attachment".to_string(),
                    _ => "link".to_string(),
                }
            } else {
                match find_prop(&self.ctx.properties, &normalize(text)) {
                    Some(def) => def.key.clone(),
                    None => {
                        return Err(format!(
                            "has: '{text}' is not link, attachment or a property"
                        ))
                    }
                }
            };
            values.push(value);
        }
        // One property alone is the single key the executor looks up.
        let resolved = match texts {
            [only] if !is_presence_word(only) => values.first().cloned(),
            _ => None,
        };
        Ok(Built::special(
            "has",
            encode_alternatives(&values),
            resolved,
        ))
    }

    /// `sort:updated`, `created`, `title` or a property.
    fn sort_op(&self, texts: &[String]) -> Result<Built, String> {
        let (text, direction) = match texts {
            [text] => (text, ""),
            [text, dir] if dir.eq_ignore_ascii_case("desc") => (text, "-"),
            [text, dir] if dir.eq_ignore_ascii_case("asc") => (text, "+"),
            _ => return Err("sort: takes one key, optionally with ,asc or ,desc".to_string()),
        };
        let lower = text.to_ascii_lowercase();
        let sort = if matches!(lower.as_str(), "updated" | "created" | "title") {
            lower
        } else if let Some(def) = find_prop(&self.ctx.properties, &normalize(text)) {
            match def.kind {
                PropKind::CreatedTime => "created".into(),
                PropKind::UpdatedTime => "updated".into(),
                _ => def.key.clone(),
            }
        } else {
            return Err(format!(
                "sort: '{text}' is not updated, created, title or a property"
            ));
        };
        Ok(Built {
            sort: Some(format!("{direction}{sort}")),
            ..Built::special("sort", encode_alternatives(texts), None)
        })
    }

    /// Compiles a property operator. The `none` value means empty.
    fn property_op(&self, def: &PropDef, value: &str) -> Result<Built, String> {
        let texts = alternatives(value);
        let label = def.name.as_str();
        if texts.is_empty() {
            return Err(format!("{label}: needs a value"));
        }
        let key = def.key.as_str();
        let mut canonical = encode_alternatives(&texts);
        if let [only] = texts.as_slice() {
            if only.eq_ignore_ascii_case("none") {
                let filter = cond(key, FilterOp::IsEmpty, PropValue::Empty);
                return Ok(Built::filtered(key, canonical, filter, true, false));
            }
        }
        // The shared indexed status key also holds task-note states. An
        // unrelated numeric/date/etc. schema must not turn those literals
        // into ignored error tokens; its other values keep their own parser.
        if key == "status"
            && !matches!(
                def.kind,
                PropKind::Text
                    | PropKind::Email
                    | PropKind::Phone
                    | PropKind::Url
                    | PropKind::Select
                    | PropKind::Status
            )
            && texts.iter().any(|text| {
                matches!(
                    text.to_ascii_lowercase().as_str(),
                    "todo" | "doing" | "done"
                )
            })
        {
            let (filter, future) = self.combine_alts(&texts, |parser, text| {
                let state = text.to_ascii_lowercase();
                if matches!(state.as_str(), "todo" | "doing" | "done") {
                    Ok((cond(key, FilterOp::Is, PropValue::Status(state)), false))
                } else {
                    let ordinary =
                        parser.property_op(def, &encode_alternatives(&[text.to_owned()]))?;
                    Ok((
                        ordinary.filter.expect("property operator has a filter"),
                        ordinary.future,
                    ))
                }
            })?;
            return Ok(Built::filtered(key, canonical, filter, true, future));
        }
        let result = match def.kind {
            PropKind::Text | PropKind::Email | PropKind::Phone | PropKind::Url => self
                .combine_alts(&texts, |_, t| {
                    Ok((
                        cond(key, FilterOp::Contains, PropValue::Text(t.to_string())),
                        false,
                    ))
                }),
            PropKind::Number => {
                self.combine_alts(&texts, |_, t| Ok((number_filter(key, t)?, false)))
            }
            PropKind::Formula | PropKind::Rollup => {
                self.combine_alts(&texts, |_, t| match number_filter(key, t) {
                    Ok(filter) => Ok((filter, false)),
                    Err(_) => Ok((
                        cond(key, FilterOp::Contains, PropValue::Text(t.to_string())),
                        false,
                    )),
                })
            }
            PropKind::Checkbox => {
                self.combine_alts(&texts, |_, t| Ok((checkbox_filter(key, t)?, false)))
            }
            PropKind::Select | PropKind::Status => self.combine_alts(&texts, |_, t| {
                let id = option_id(def, t)?;
                let value = if def.kind == PropKind::Status {
                    PropValue::Status(id)
                } else {
                    PropValue::Select(id)
                };
                Ok((cond(key, FilterOp::Is, value), false))
            }),
            PropKind::MultiSelect => self.combine_alts(&texts, |_, t| {
                let id = option_id(def, t)?;
                Ok((
                    cond(key, FilterOp::Contains, PropValue::MultiSelect(vec![id])),
                    false,
                ))
            }),
            PropKind::Relation => match self.relation_ids(&texts) {
                Ok(ids) => {
                    // Printed as ids, so a rename does not change the query.
                    canonical = page_value(&ids);
                    let filters = ids
                        .iter()
                        .map(|id| {
                            cond(
                                key,
                                FilterOp::Contains,
                                PropValue::Relation(vec![id.clone()]),
                            )
                        })
                        .collect();
                    Ok((combine(filters), false))
                }
                Err(message) => Err(message),
            },
            PropKind::Date => self.combine_alts(&texts, |p, t| p.date_alt(&[key.to_string()], t)),
            PropKind::CreatedTime => {
                self.combine_alts(&texts, |p, t| p.date_alt(&["$created".to_string()], t))
            }
            PropKind::UpdatedTime => {
                self.combine_alts(&texts, |p, t| p.date_alt(&["$updated".to_string()], t))
            }
            PropKind::Files => Err(format!("holds files; use has:{}", def.name)),
        };
        let (filter, future) = result.map_err(|e| format!("{label}: {e}"))?;
        Ok(Built::filtered(key, canonical, filter, true, future))
    }

    /// The page ids of relation alternatives, each by id or title.
    fn relation_ids(&self, texts: &[String]) -> Result<Vec<String>, String> {
        texts
            .iter()
            .map(|t| {
                self.resolve_page(t)
                    .ok_or_else(|| format!("no page named '{t}'"))
            })
            .collect()
    }

    /// Compiles each comma alternative and ORs them when there are several.
    /// The flag is true if any alternative reaches the future.
    fn combine_alts(
        &self,
        texts: &[String],
        mut each: impl FnMut(&Self, &str) -> Result<(Filter, bool), String>,
    ) -> Result<(Filter, bool), String> {
        let mut filters = Vec::with_capacity(texts.len());
        let mut future = false;
        for text in texts {
            let (filter, ahead) = each(self, text.as_str())?;
            filters.push(filter);
            future |= ahead;
        }
        Ok((combine(filters), future))
    }

    /// One date value (a date word, a range, or a comparison) on each key.
    /// `*` covers every date property, and `WithinRange` only matches date rows.
    fn date_alt(&self, keys: &[String], text: &str) -> Result<(Filter, bool), String> {
        let (cmp, rest) = split_cmp(text);
        let span = self.parse_span(rest)?;
        let mut conds = Vec::with_capacity(keys.len());
        let future = match cmp {
            None => {
                for key in keys {
                    conds.push(cond(
                        key,
                        FilterOp::WithinRange,
                        date_value(span.start, Some(span.end)),
                    ));
                }
                span.future
            }
            Some(cmp) => {
                // `>X` is after the end of X; `<X` is before its start.
                let (op, at) = match cmp {
                    Cmp::Gt => (FilterOp::OnOrAfter, span.end),
                    Cmp::Ge => (FilterOp::OnOrAfter, span.start),
                    Cmp::Lt => (FilterOp::Before, span.start),
                    Cmp::Le => (FilterOp::Before, span.end),
                };
                // A comparison with `now` is an instant: all-day items
                // compare by their whole day (see search_items).
                let instant = rest.eq_ignore_ascii_case("now");
                for key in keys {
                    conds.push(cond(
                        key,
                        op,
                        PropValue::Date {
                            start_ms: at,
                            end_ms: None,
                            has_time: instant,
                        },
                    ));
                }
                matches!(cmp, Cmp::Gt | Cmp::Ge) && span.end > self.ctx.now_ms
            }
        };
        Ok((combine(conds), future))
    }

    /// The window of a date value: `a..b` ranges, or one date word.
    fn parse_span(&self, text: &str) -> Result<Span, String> {
        if let Some((from, to)) = text.split_once("..") {
            let (a, b) = (self.parse_span(from)?, self.parse_span(to)?);
            if b.end <= a.start {
                return Err(format!("'{text}' is a reversed range"));
            }
            return Ok(Span {
                start: a.start,
                end: b.end,
                future: a.future || b.future || a.start > self.ctx.now_ms,
            });
        }
        self.bounds(text)
    }

    /// The window of one date word: a named period, a relative form or an
    /// absolute date. Named and absolute dates are whole local periods.
    /// Relative forms count from now, with local time of day.
    fn bounds(&self, text: &str) -> Result<Span, String> {
        let word = text.to_ascii_lowercase();
        // `now` is the current minute; `<now` is before this instant.
        if word == "now" {
            return Ok(Span {
                start: self.ctx.now_ms,
                end: self.ctx.now_ms + 60_000,
                future: false,
            });
        }
        if NAMED_DATES.contains(&word.as_str()) {
            let (first, end) = self.named_period(&word).ok_or_else(out_of_range)?;
            return self.whole_days(first, end);
        }
        // `7d` is the last 7 days, ending now. `+7d` is now through the next 7 days.
        let (plus, body) = match word.strip_prefix('+') {
            Some(rest) => (true, rest),
            None => (false, word.as_str()),
        };
        if let Some(unit) = body
            .chars()
            .last()
            .filter(|c| matches!(*c, 'd' | 'w' | 'm' | 'y'))
        {
            if let Ok(n) = body[..body.len() - 1].parse::<i64>() {
                if n > 0 {
                    let now = self.now_local.ok_or_else(out_of_range)?;
                    return if plus {
                        let end = self.shifted(now, unit, n)?;
                        Ok(Span {
                            start: self.ctx.now_ms,
                            end,
                            future: true,
                        })
                    } else {
                        let start = self.shifted(now, unit, -n)?;
                        Ok(Span {
                            start,
                            end: self.ctx.now_ms,
                            future: false,
                        })
                    };
                }
            }
        }
        let (first, end) = absolute(&word).ok_or_else(|| format!("'{text}' is not a date"))?;
        self.whole_days(first, end)
    }

    /// The named period `word`, from local calendar arithmetic on today.
    fn named_period(&self, word: &str) -> Option<(NaiveDate, NaiveDate)> {
        let today = self.now_local?.date();
        match word {
            "today" => Some((today, add_days(today, 1)?)),
            "yesterday" => Some((add_days(today, -1)?, today)),
            "tomorrow" => Some((add_days(today, 1)?, add_days(today, 2)?)),
            "week" => {
                let monday = monday(today)?;
                Some((monday, add_days(monday, 7)?))
            }
            "lastweek" => {
                let monday = add_days(monday(today)?, -7)?;
                Some((monday, add_days(monday, 7)?))
            }
            "month" => {
                let first = ymd(today.year(), today.month(), 1)?;
                Some((first, add_months(first, 1)?))
            }
            "lastmonth" => {
                let this_month = ymd(today.year(), today.month(), 1)?;
                Some((add_months(this_month, -1)?, this_month))
            }
            "year" => Some((
                ymd(today.year(), 1, 1)?,
                ymd(today.year().checked_add(1)?, 1, 1)?,
            )),
            _ => None,
        }
    }

    /// A whole local period from day `first` up to day `end` (exclusive).
    fn whole_days(&self, first: NaiveDate, end: NaiveDate) -> Result<Span, String> {
        let start = self.day_ms(first).ok_or_else(out_of_range)?;
        let end = self.day_ms(end).ok_or_else(out_of_range)?;
        Ok(Span {
            start,
            end,
            future: start > self.ctx.now_ms,
        })
    }

    /// The instant `n` units of `unit` from a wall-clock time, keeping its time of day.
    fn shifted(&self, from: NaiveDateTime, unit: char, n: i64) -> Result<i64, String> {
        let day = shift_by(from.date(), unit, n).ok_or_else(out_of_range)?;
        self.local_ms(NaiveDateTime::new(day, from.time()))
            .ok_or_else(out_of_range)
    }

    /// Local midnight of `day` in epoch milliseconds.
    fn day_ms(&self, day: NaiveDate) -> Option<i64> {
        self.local_ms(day.and_hms_opt(0, 0, 0)?)
    }

    /// A wall-clock time in epoch milliseconds. A time in a DST gap moves to
    /// the first valid time after it; an ambiguous time takes the earlier one.
    fn local_ms(&self, at: NaiveDateTime) -> Option<i64> {
        (0..3)
            .find_map(|hours| {
                let at = at.checked_add_signed(TimeDelta::try_hours(hours)?)?;
                self.ctx.tz.from_local_datetime(&at).earliest()
            })
            .map(|at| at.timestamp_millis())
    }

    /// `in:` values: page references (a page and its subtree) or path
    /// patterns (see `search_path`). An exact title or id wins over reading
    /// the value as a path, so a title with a `/` keeps resolving.
    fn in_op(&self, value: &str) -> Result<Built, String> {
        let memory_root = || self.extras.and_then(|x| x.memory_root.clone());
        let mut parts = Vec::new();
        let mut first_id = None;
        for (i, raw) in crate::search_path::split_raw(value, ',').iter().enumerate() {
            let text = alternatives(raw).pop().unwrap_or_default();
            let exact = self
                .ctx
                .pages
                .iter()
                .any(|p| p.id == text || p.title.eq_ignore_ascii_case(&text));
            if !exact && crate::search_path::is_pattern(raw) {
                let parsed =
                    crate::search_path::parse(raw, |t| self.resolve_page(t), memory_root());
                match parsed {
                    Ok(pattern) => {
                        parts.push(pattern.canonical());
                        continue;
                    }
                    // A value that names no anchor may still be a title.
                    Err(e) => {
                        if self.resolve_page(&text).is_none() {
                            return Err(e);
                        }
                    }
                }
            }
            let name = strip_brackets(&text);
            let memory = (name == text.as_str() && name.eq_ignore_ascii_case("memory"))
                .then(memory_root)
                .flatten();
            let id = memory
                .or_else(|| self.resolve_page(name))
                .ok_or_else(|| format!("in: no page named '{name}'"))?;
            if i == 0 {
                first_id = Some(id.clone());
            }
            parts.push(format!("[[{id}]]"));
        }
        if parts.is_empty() {
            return Err("in: needs a value".into());
        }
        Ok(Built::special("in", parts.join(","), first_id))
    }

    /// Resolves a page by id (`[[id]]` or a bare id), then by title: an exact
    /// title first, else a title that contains the text. Ties go to the page
    /// updated most recently.
    fn resolve_page(&self, text: &str) -> Option<String> {
        let text = strip_brackets(text);
        if let Some(page) = self.ctx.pages.iter().find(|p| p.id == text) {
            return Some(page.id.clone());
        }
        let lower = text.to_lowercase();
        let exact = self
            .ctx
            .pages
            .iter()
            .filter(|p| p.title.to_lowercase() == lower)
            .max_by_key(|p| p.updated_ms);
        let best = exact.or_else(|| {
            self.ctx
                .pages
                .iter()
                .filter(|p| p.title.to_lowercase().contains(lower.as_str()))
                .max_by_key(|p| p.updated_ms)
        });
        best.map(|p| p.id.clone())
    }
}

// ------------------------------------------------------------ text helpers

/// Splits a query into word spans. Whitespace inside quotes or `[[...]]` does
/// not split, and a backslash inside quotes keeps the next character from
/// closing the quote.
fn split_words(query: &str) -> Vec<(usize, usize)> {
    let chars: Vec<(usize, char)> = query.char_indices().collect();
    let mut words = Vec::new();
    let mut start: Option<usize> = None;
    let mut quoted = false;
    let mut bracket = false;
    let mut i = 0;
    while i < chars.len() {
        let (pos, c) = chars[i];
        if !quoted && !bracket && c.is_whitespace() {
            if let Some(from) = start.take() {
                words.push((from, pos));
            }
            i += 1;
            continue;
        }
        if start.is_none() {
            start = Some(pos);
        }
        let next = chars.get(i + 1).map(|&(_, n)| n);
        if quoted && c == '\\' {
            // The escaped character is part of the word, never a quote.
            i += 1;
        } else if c == '"' {
            quoted = !quoted;
        } else if !quoted && !bracket && c == '[' && next == Some('[') {
            bracket = true;
            i += 1;
        } else if bracket && c == ']' && next == Some(']') {
            bracket = false;
            i += 1;
        }
        i += 1;
    }
    if let Some(from) = start {
        words.push((from, query.len()));
    }
    words
}

/// Reads a quoted run whose opening quote is already consumed. Returns the
/// text with escapes resolved, and what follows the closing quote. An
/// unterminated run takes the rest of the input.
fn unquote(inner: &str) -> (String, &str) {
    let mut text = String::new();
    let mut chars = inner.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => {
                if let Some((_, next)) = chars.next() {
                    text.push(next);
                }
            }
            '"' => return (text, &inner[i + 1..]),
            _ => text.push(c),
        }
    }
    (text, "")
}

/// Splits a value into its comma alternatives. Commas and `[[...]]` outside
/// quotes do not split. Inside quotes a backslash escapes the next character.
/// Quotes are removed; each alternative is trimmed and empty ones are dropped.
pub(crate) fn alternatives(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut bracket = false;
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted && c == '\\' {
            if let Some(next) = chars.next() {
                current.push(next);
            }
        } else if c == '"' {
            quoted = !quoted;
        } else if c == '[' && !quoted && !bracket && chars.peek() == Some(&'[') {
            chars.next();
            bracket = true;
            current.push_str("[[");
        } else if c == ']' && bracket && chars.peek() == Some(&']') {
            chars.next();
            bracket = false;
            current.push_str("]]");
        } else if c == ',' && !quoted && !bracket {
            push_alternative(&mut out, &mut current);
        } else {
            current.push(c);
        }
    }
    push_alternative(&mut out, &mut current);
    out
}

fn push_alternative(out: &mut Vec<String>, current: &mut String) {
    let text = current.trim();
    if !text.is_empty() {
        out.push(text.to_string());
    }
    current.clear();
}

/// The canonical value of alternatives. Each is quoted only when it needs it,
/// so [`alternatives`] returns the same list.
fn encode_alternatives<S: AsRef<str>>(texts: &[S]) -> String {
    texts
        .iter()
        .map(|t| encode_one(t.as_ref()))
        .collect::<Vec<_>>()
        .join(",")
}

fn encode_one(text: &str) -> String {
    if text
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, ',' | '"' | '\\' | '[' | ']'))
    {
        format!("\"{}\"", escape(text))
    } else {
        text.to_string()
    }
}

/// Escapes what is special inside quotes: the backslash and the quote.
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The canonical value of page ids: `[[id]],[[id]]`.
fn page_value(ids: &[String]) -> String {
    ids.iter()
        .map(|id| format!("[[{id}]]"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Why a value's quotes do not read as one value: an unclosed quote, or text
/// after a closing quote.
/// With `segments` (`in:` paths), `/` starts a new segment that may be
/// quoted on its own.
fn quoting_problem(value: &str, segments: bool) -> Option<&'static str> {
    let mut quoted = false;
    // The last character closed a quote.
    let mut closed = false;
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '\\' => {
                    chars.next();
                }
                '"' => {
                    quoted = false;
                    closed = true;
                }
                _ => {}
            }
            continue;
        }
        match c {
            '"' if closed => return Some("text after a closing quote"),
            '"' => quoted = true,
            ',' => closed = false,
            '/' if segments => closed = false,
            _ if closed => return Some("text after a closing quote"),
            _ => {}
        }
    }
    quoted.then_some("an unclosed quote")
}

/// `[[X]]` gives `X`; any other text is returned as it is.
pub(crate) fn strip_brackets(text: &str) -> &str {
    match text
        .strip_prefix("[[")
        .and_then(|inner| inner.strip_suffix("]]"))
    {
        Some(inner) => inner,
        None => text,
    }
}

/// Key normalization: case, spaces, `_` and `-` do not count.
fn normalize(key: &str) -> String {
    key.chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

fn builtin_key(norm_key: &str) -> Option<&'static str> {
    keyed_operator(norm_key).map(|op| op.name)
}

/// The filter key `date:` compiles to in an item search: the item's date.
pub(crate) const ITEM_DATE: &str = "$item";

/// A sort key without its direction prefix, and the direction it asks for:
/// `-key` is descending, `+key` ascending, and a bare key keeps the key's
/// default order.
pub(crate) fn sort_direction(sort: &str) -> (&str, Option<bool>) {
    if let Some(key) = sort.strip_prefix('-') {
        (key, Some(true))
    } else if let Some(key) = sort.strip_prefix('+') {
        (key, Some(false))
    } else {
        (sort, None)
    }
}

/// The normalized key of an unquoted `key:value` word, for the pre-scan.
fn plain_key(word: &str) -> Option<String> {
    let body = word.strip_prefix('-').unwrap_or(word);
    if body.starts_with('"') {
        return None;
    }
    let colon = body.find(':')?;
    let key = &body[..colon];
    (colon > 0 && !key.chars().any(|c| c == '[' || c == '"')).then(|| normalize(key))
}

/// A property matched by its stable key first, then by its display name.
fn find_prop<'c>(props: &'c [PropDef], norm_key: &str) -> Option<&'c PropDef> {
    if norm_key.is_empty() {
        return None;
    }
    props
        .iter()
        .find(|d| normalize(&d.key) == norm_key)
        .or_else(|| props.iter().find(|d| normalize(&d.name) == norm_key))
}

fn status_has_alias(filter: &Filter) -> bool {
    match filter {
        Filter::Cond(condition) => condition.prop != "status",
        Filter::And(filters) | Filter::Or(filters) => filters.iter().any(status_has_alias),
    }
}

/// `link`, `attachment` and their plurals: the presence operators of `has:`.
fn is_presence_word(text: &str) -> bool {
    matches!(
        text.to_ascii_lowercase().as_str(),
        "link" | "links" | "attachment" | "attachments"
    )
}

/// An option id or its case-insensitive display name. Key normalization does
/// not apply to values: spaces, underscores and hyphens remain significant.
fn option_id(def: &PropDef, text: &str) -> Result<String, String> {
    let lower = text.to_lowercase();
    def.options
        .iter()
        .find(|o| o.id == text)
        .or_else(|| def.options.iter().find(|o| o.name.to_lowercase() == lower))
        .map(|o| o.id.clone())
        .ok_or_else(|| format!("'{text}' is not an option"))
}

fn checkbox_filter(key: &str, text: &str) -> Result<Filter, String> {
    match text.to_ascii_lowercase().as_str() {
        "yes" | "true" => Ok(cond(key, FilterOp::Is, PropValue::Checkbox(true))),
        "no" | "false" => Ok(cond(key, FilterOp::Is, PropValue::Checkbox(false))),
        _ => Err(format!("'{text}' is not yes or no")),
    }
}

/// A number `5`, a comparison `>5`, or a range `1..5` (both ends included).
fn number_filter(key: &str, text: &str) -> Result<Filter, String> {
    if let Some((low, high)) = text.split_once("..") {
        let low = parse_number(low)?;
        let high = parse_number(high)?;
        if high < low {
            return Err(format!("'{text}' is a reversed range"));
        }
        return Ok(Filter::And(vec![
            cond(key, FilterOp::GreaterOrEqual, PropValue::Number(low)),
            cond(key, FilterOp::LessOrEqual, PropValue::Number(high)),
        ]));
    }
    let (cmp, rest) = split_cmp(text);
    let op = match cmp {
        None => FilterOp::Is,
        Some(Cmp::Gt) => FilterOp::Greater,
        Some(Cmp::Ge) => FilterOp::GreaterOrEqual,
        Some(Cmp::Lt) => FilterOp::Less,
        Some(Cmp::Le) => FilterOp::LessOrEqual,
    };
    Ok(cond(key, op, PropValue::Number(parse_number(rest)?)))
}

fn parse_number(text: &str) -> Result<f64, String> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .ok_or_else(|| format!("'{text}' is not a number"))
}

/// A leading `>`, `>=`, `<` or `<=`, and the rest.
fn split_cmp(text: &str) -> (Option<Cmp>, &str) {
    if let Some(rest) = text.strip_prefix(">=") {
        (Some(Cmp::Ge), rest)
    } else if let Some(rest) = text.strip_prefix("<=") {
        (Some(Cmp::Le), rest)
    } else if let Some(rest) = text.strip_prefix('>') {
        (Some(Cmp::Gt), rest)
    } else if let Some(rest) = text.strip_prefix('<') {
        (Some(Cmp::Lt), rest)
    } else {
        (None, text)
    }
}

fn cond(prop: &str, op: FilterOp, value: PropValue) -> Filter {
    Filter::Cond(Condition {
        prop: prop.to_string(),
        op,
        value,
    })
}

/// One filter for the alternatives: the filter itself, or an OR of them.
fn combine(mut filters: Vec<Filter>) -> Filter {
    if filters.len() == 1 {
        if let Some(filter) = filters.pop() {
            return filter;
        }
    }
    Filter::Or(filters)
}

fn date_value(start_ms: i64, end_ms: Option<i64>) -> PropValue {
    PropValue::Date {
        start_ms,
        end_ms,
        has_time: false,
    }
}

fn out_of_range() -> String {
    "date is out of range".to_string()
}

/// Whether a printed value needs quotes to parse back as one word or value.
fn needs_quotes(text: &str) -> bool {
    text.is_empty()
        || text.starts_with(|c: char| matches!(c, '#' | '-' | '"'))
        || text
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, ':' | ',' | '[' | ']' | '"'))
}

// ------------------------------------------------------------ date helpers

/// A whole absolute date (`2026`, `2026-10`, `2026-10-10`) as its days
/// `[first, end)`. `None` if the text is not one, or the date is out of range.
fn absolute(word: &str) -> Option<(NaiveDate, NaiveDate)> {
    let parts: Vec<&str> = word.split('-').collect();
    match parts.as_slice() {
        [y] if y.len() == 4 => {
            let y: i32 = y.parse().ok()?;
            Some((ymd(y, 1, 1)?, ymd(y.checked_add(1)?, 1, 1)?))
        }
        [y, m] if y.len() == 4 => {
            let first = ymd(y.parse().ok()?, m.parse().ok()?, 1)?;
            Some((first, add_months(first, 1)?))
        }
        [y, m, d] if y.len() == 4 => {
            let first = ymd(y.parse().ok()?, m.parse().ok()?, d.parse().ok()?)?;
            Some((first, add_days(first, 1)?))
        }
        _ => None,
    }
}

fn add_days(day: NaiveDate, days: i64) -> Option<NaiveDate> {
    day.checked_add_signed(TimeDelta::try_days(days)?)
}

fn ymd(year: i32, month: u32, day: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(year, month, day)
}

/// The Monday of the week that holds `day`.
fn monday(day: NaiveDate) -> Option<NaiveDate> {
    add_days(day, -i64::from(day.weekday().num_days_from_monday()))
}

/// `n` days, weeks, months or years from `from` (negative `n` goes back).
/// Every step is checked, so a huge `n` is `None`, not a panic.
fn shift_by(from: NaiveDate, unit: char, n: i64) -> Option<NaiveDate> {
    match unit {
        'd' => add_days(from, n),
        'w' => add_days(from, n.checked_mul(7)?),
        'm' => add_months(from, n),
        _ => add_months(from, n.checked_mul(12)?),
    }
}

/// Adds months, clamping the day to the end of a shorter month.
fn add_months(from: NaiveDate, months: i64) -> Option<NaiveDate> {
    let index = i64::from(from.year())
        .checked_mul(12)?
        .checked_add(i64::from(from.month0()))?
        .checked_add(months)?;
    let year = i32::try_from(index.div_euclid(12)).ok()?;
    let month = u32::try_from(index.rem_euclid(12)).ok()? + 1;
    let last = days_in_month(year, month)?;
    ymd(year, month, from.day().min(last))
}

fn days_in_month(year: i32, month: u32) -> Option<u32> {
    let next = if month == 12 {
        ymd(year.checked_add(1)?, 1, 1)?
    } else {
        ymd(year, month + 1, 1)?
    };
    Some(next.checked_sub_signed(TimeDelta::try_days(1)?)?.day())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SelectOption;
    use chrono::FixedOffset;

    /// Noon UTC in milliseconds: the time of day of `now()`.
    const NOON: i64 = 12 * 3_600_000;

    fn utc(y: i32, m: u32, d: u32) -> i64 {
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0)
            .single()
            .expect("valid test date")
            .timestamp_millis()
    }

    /// 2026-10-10 12:00 UTC: a Saturday.
    fn now() -> i64 {
        utc(2026, 10, 10) + 12 * 3_600_000
    }

    fn ctx(properties: Vec<PropDef>, pages: Vec<SearchPage>) -> SearchContext<Utc> {
        SearchContext {
            properties,
            pages,
            now_ms: now(),
            tz: Utc,
        }
    }

    fn parse(query: &str) -> ParsedQuery {
        parse_search_with_context(query, &ctx(Vec::new(), Vec::new()))
    }

    fn parse_with(query: &str, properties: Vec<PropDef>, pages: Vec<SearchPage>) -> ParsedQuery {
        parse_search_with_context(query, &ctx(properties, pages))
    }

    fn prop(key: &str, name: &str, kind: PropKind) -> PropDef {
        PropDef {
            key: key.into(),
            name: name.into(),
            kind,
            options: Vec::new(),
            config: String::new(),
        }
    }

    fn option(id: &str, name: &str) -> SelectOption {
        SelectOption {
            id: id.into(),
            name: name.into(),
            color: String::new(),
        }
    }

    fn page(id: &str, title: &str, updated_ms: i64) -> SearchPage {
        SearchPage {
            id: id.into(),
            title: title.into(),
            updated_ms,
        }
    }

    fn window(prop: &str, start: i64, end: i64) -> Filter {
        cond(prop, FilterOp::WithinRange, date_value(start, Some(end)))
    }

    fn bound(prop: &str, op: FilterOp, at: i64) -> Filter {
        cond(prop, op, date_value(at, None))
    }

    fn only_operator(query: &ParsedQuery) -> &SearchOperator {
        assert_eq!(
            query.spec.operators.len(),
            1,
            "expected one operator: {:?}",
            query.spec
        );
        &query.spec.operators[0]
    }

    fn filter_of(query: &ParsedQuery) -> Filter {
        only_operator(query)
            .filter
            .clone()
            .expect("the operator compiles a filter")
    }

    fn warnings(query: &ParsedQuery) -> Vec<String> {
        query.spec.warnings.clone()
    }

    #[test]
    fn words_and_phrases_with_exclusions() {
        let q = parse(r#"meeting "exact phrase" -draft -"old note""#);
        assert_eq!(
            q.spec.terms,
            vec![
                SearchTerm {
                    value: "meeting".into(),
                    phrase: false,
                    exclude: false
                },
                SearchTerm {
                    value: "exact phrase".into(),
                    phrase: true,
                    exclude: false
                },
                SearchTerm {
                    value: "draft".into(),
                    phrase: false,
                    exclude: true
                },
                SearchTerm {
                    value: "old note".into(),
                    phrase: true,
                    exclude: true
                },
            ]
        );
        assert!(q.spec.operators.is_empty());
        assert!(q.spec.warnings.is_empty());
        assert_eq!(parse(&q.spec.to_query_string()).spec, q.spec);
        assert_eq!(q.tokens.len(), 4);
        assert_eq!(q.tokens[1].start, 8);
        assert_eq!(q.tokens[1].end, 22);
        assert_eq!(
            q.tokens[1].kind,
            TokenKind::Phrase {
                value: "exact phrase".into()
            }
        );
        assert_eq!(
            q.tokens[2].kind,
            TokenKind::Exclude {
                value: "draft".into(),
                phrase: false
            }
        );
    }

    #[test]
    fn unterminated_phrase_runs_to_the_end() {
        let q = parse(r#""open phrase"#);
        assert_eq!(
            q.spec.terms,
            vec![SearchTerm {
                value: "open phrase".into(),
                phrase: true,
                exclude: false
            }]
        );
    }

    #[test]
    fn lone_dash_and_empty_phrase_are_errors() {
        let q = parse("- \"\"");
        assert_eq!(
            warnings(&q),
            vec!["'-' needs a word or an operator", "empty phrase"]
        );
        assert!(matches!(q.tokens[0].kind, TokenKind::Error { .. }));
        assert!(q.spec.terms.is_empty());
    }

    #[test]
    fn unknown_keys_are_text() {
        let q = parse("http://x foo:bar 12:30");
        assert_eq!(
            q.spec.terms,
            vec![
                SearchTerm {
                    value: "http://x".into(),
                    phrase: false,
                    exclude: false
                },
                SearchTerm {
                    value: "foo:bar".into(),
                    phrase: false,
                    exclude: false
                },
                SearchTerm {
                    value: "12:30".into(),
                    phrase: false,
                    exclude: false
                },
            ]
        );
        assert!(q.spec.warnings.is_empty());
        assert_eq!(parse(&q.spec.to_query_string()).spec, q.spec);
        let quoted = parse(r#"unknown:"two words, with \"quotes\"""#);
        assert_eq!(parse(&quoted.spec.to_query_string()).spec, quoted.spec);
    }

    #[test]
    fn quoted_unknown_key_with_space_is_a_phrase() {
        let q = parse(r#""due date":today"#);
        assert_eq!(
            q.spec.terms,
            vec![SearchTerm {
                value: "due date:today".into(),
                phrase: true,
                exclude: false
            }]
        );
    }

    #[test]
    fn tags_keep_alternatives_and_hash_form_is_tag() {
        let q = parse("tag:work,home #urgent");
        assert_eq!(q.spec.operators.len(), 2);
        assert_eq!(q.spec.operators[0].key, "tag");
        assert_eq!(q.spec.operators[0].value, "work,home");
        assert!(q.spec.operators[0].filter.is_none());
        assert_eq!(q.spec.operators[1].key, "tag");
        assert_eq!(q.spec.operators[1].value, "urgent");
        assert_eq!(
            q.tokens[0].kind,
            TokenKind::Operator {
                key: "tag".into(),
                value: "work,home".into(),
                resolved_id: None
            }
        );
    }

    #[test]
    fn negated_operators_and_terms() {
        let q = parse("-tag:work -title:old -foo");
        assert_eq!(q.spec.operators.len(), 2);
        assert!(q.spec.operators.iter().all(|o| o.negated));
        assert_eq!(
            q.spec.terms,
            vec![SearchTerm {
                value: "foo".into(),
                phrase: false,
                exclude: true
            }]
        );
        assert_eq!(
            q.spec.operators[1].filter,
            Some(cond(
                "$title",
                FilterOp::Contains,
                PropValue::Text("old".into())
            ))
        );
    }

    #[test]
    fn created_day_is_a_utc_day_window() {
        let q = parse("created:2026-10-10");
        assert_eq!(
            filter_of(&q),
            window("$created", utc(2026, 10, 10), utc(2026, 10, 11))
        );
        assert!(!only_operator(&q).date_future);
        assert!(!only_operator(&q).property);
    }

    #[test]
    fn partial_dates_cover_the_period() {
        let q = parse("updated:2026-09..2026-10");
        assert_eq!(
            filter_of(&q),
            window("$updated", utc(2026, 9, 1), utc(2026, 11, 1))
        );
        let year = parse("created:2026");
        assert_eq!(
            filter_of(&year),
            window("$created", utc(2026, 1, 1), utc(2027, 1, 1))
        );
    }

    #[test]
    fn bare_relative_ends_now_and_plus_starts_now() {
        // `now()` is 2026-10-10 12:00 UTC.
        let past = parse("updated:7d");
        assert_eq!(
            filter_of(&past),
            window("$updated", utc(2026, 10, 3) + NOON, now())
        );
        assert!(!only_operator(&past).date_future);

        let future = parse("updated:+7d");
        assert_eq!(
            filter_of(&future),
            window("$updated", now(), utc(2026, 10, 17) + NOON)
        );
        assert!(only_operator(&future).date_future);
    }

    #[test]
    fn relative_units_and_months_clamp() {
        let weeks = parse("created:2w");
        assert_eq!(
            filter_of(&weeks),
            window("$created", utc(2026, 9, 26) + NOON, now())
        );
        let months = parse("created:3m");
        assert_eq!(
            filter_of(&months),
            window("$created", utc(2026, 7, 10) + NOON, now())
        );
        let years = parse("created:1y");
        assert_eq!(
            filter_of(&years),
            window("$created", utc(2025, 10, 10) + NOON, now())
        );
        // Plus forms end at now plus the same units.
        assert_eq!(
            filter_of(&parse("created:+2w")),
            window("$created", now(), utc(2026, 10, 24) + NOON)
        );
        assert_eq!(
            filter_of(&parse("created:+3m")),
            window("$created", now(), utc(2027, 1, 10) + NOON)
        );
        // Month shifts clamp to the end of a shorter month.
        let clamped = parse_search_with_context(
            "created:+1m",
            &SearchContext {
                properties: Vec::new(),
                pages: Vec::new(),
                now_ms: utc(2026, 1, 31) + NOON,
                tz: Utc,
            },
        );
        assert_eq!(
            filter_of(&clamped),
            window("$created", utc(2026, 1, 31) + NOON, utc(2026, 2, 28) + NOON)
        );
    }

    #[test]
    fn today_to_plus_range_ends_at_now_plus_units() {
        let q = parse("updated:today..+7d");
        assert_eq!(
            filter_of(&q),
            window("$updated", utc(2026, 10, 10), utc(2026, 10, 17) + NOON)
        );
        assert!(only_operator(&q).date_future);
    }

    #[test]
    fn reversed_ranges_are_errors() {
        let props = vec![prop("score", "Score", PropKind::Number)];
        let number = parse_with("score:5..1", props, Vec::new());
        assert!(number.spec.operators.is_empty());
        assert_eq!(warnings(&number), vec!["Score: '5..1' is a reversed range"]);
        assert!(matches!(number.tokens[0].kind, TokenKind::Error { .. }));

        let date = parse("created:2026-10-12..2026-10-10");
        assert!(date.spec.operators.is_empty());
        assert_eq!(
            warnings(&date),
            vec!["created: '2026-10-12..2026-10-10' is a reversed range"]
        );
        assert!(matches!(date.tokens[0].kind, TokenKind::Error { .. }));
    }

    #[test]
    fn huge_durations_are_errors_not_panics() {
        for query in [
            "created:99999999999d",
            "created:+9223372036854775807w",
            "created:9223372036854775807m",
            "created:+9223372036854775807y",
            "created:99999999999999999999999d",
        ] {
            let q = parse(query);
            assert!(q.spec.operators.is_empty(), "{query}");
            assert_eq!(q.spec.warnings.len(), 1, "{query}");
            assert!(
                matches!(q.tokens[0].kind, TokenKind::Error { .. }),
                "{query}"
            );
        }
    }

    #[test]
    fn malformed_quoted_values_are_warnings() {
        let open = parse(r#"tag:"open"#);
        assert!(open.spec.operators.is_empty());
        assert_eq!(warnings(&open), vec!["tag: an unclosed quote"]);

        let props = vec![prop("status", "Status", PropKind::Text)];
        let trailing = parse_with(r#"status:"Done"x"#, props, Vec::new());
        assert!(trailing.spec.operators.is_empty());
        assert_eq!(
            warnings(&trailing),
            vec!["status: text after a closing quote"]
        );
    }

    #[test]
    fn escaped_quotes_and_backslashes_round_trip() {
        let q = parse(r#"tag:"say \"hi\"\\there" "a \"b\" c\\d""#);
        let tag = only_operator(&q);
        assert_eq!(tag.value, r#""say \"hi\"\\there""#);
        assert_eq!(
            alternatives(&tag.value),
            vec![r#"say "hi"\there"#.to_string()]
        );
        assert_eq!(
            q.spec.terms,
            vec![SearchTerm {
                value: r#"a "b" c\d"#.into(),
                phrase: true,
                exclude: false
            }]
        );
        let printed = q.spec.to_query_string();
        assert_eq!(printed, r#""a \"b\" c\\d" tag:"say \"hi\"\\there""#);
        assert_eq!(parse(&printed).spec, q.spec);
    }

    #[test]
    fn quoted_commas_stay_one_value() {
        let tag = parse(r#"tag:"work,home""#);
        assert_eq!(only_operator(&tag).value, r#""work,home""#);
        assert_eq!(
            alternatives(&only_operator(&tag).value),
            vec!["work,home".to_string()]
        );

        let props = vec![prop("notes", "Notes", PropKind::Text)];
        let notes = parse_with(r#"notes:"a,b""#, props.clone(), Vec::new());
        assert_eq!(
            filter_of(&notes),
            cond("notes", FilterOp::Contains, PropValue::Text("a,b".into()))
        );
        assert_eq!(only_operator(&notes).value, r#""a,b""#);
        let printed = notes.spec.to_query_string();
        assert_eq!(printed, r#""notes":"a,b""#);
        assert_eq!(parse_with(&printed, props, Vec::new()).spec, notes.spec);

        let title = parse(r#"title:"foo,bar""#);
        assert_eq!(
            filter_of(&title),
            cond(
                "$title",
                FilterOp::Contains,
                PropValue::Text("foo,bar".into())
            )
        );
    }

    #[test]
    fn has_comma_is_or_and_repeated_has_is_and() {
        let q = parse("has:link,attachment");
        assert_eq!(q.spec.operators.len(), 1);
        assert_eq!(only_operator(&q).value, "link,attachment");
        assert_eq!(only_operator(&q).resolved_id, None);
        assert!(only_operator(&q).filter.is_none());

        let repeated = parse("has:link has:attachment");
        assert_eq!(repeated.spec.operators.len(), 2);

        let props = vec![prop("due", "Due", PropKind::Date)];
        let mixed = parse_with("has:due,link", props.clone(), Vec::new());
        assert_eq!(only_operator(&mixed).value, "due,link");
        assert_eq!(only_operator(&mixed).resolved_id, None);

        let single = parse_with("has:Due", props, Vec::new());
        assert_eq!(only_operator(&single).value, "due");
        assert_eq!(only_operator(&single).resolved_id.as_deref(), Some("due"));
    }

    #[test]
    fn page_comma_values_print_ids_and_survive_renames() {
        let before = vec![page("p1", "Alpha", 100), page("p2", "Beta", 200)];
        let original = parse_with("in:Alpha,Beta", Vec::new(), before);
        let op = only_operator(&original);
        assert_eq!(op.value, "[[p1]],[[p2]]");
        assert_eq!(op.resolved_id.as_deref(), Some("p1"));
        let printed = original.spec.to_query_string();
        assert_eq!(printed, "in:[[p1]],[[p2]]");

        // Both pages are renamed; the printed query still names the same pages.
        let renamed = vec![page("p1", "Gamma", 100), page("p2", "Delta", 200)];
        let reparsed = parse_with(&printed, Vec::new(), renamed);
        assert_eq!(reparsed.spec, original.spec);
    }

    #[test]
    fn date_compiles_to_vault_scope_and_survives_serde() {
        let q = parse("date:today");
        let today = |prop: &str| window(prop, utc(2026, 10, 10), utc(2026, 10, 11));
        assert_eq!(
            filter_of(&q),
            Filter::Or(vec![today("$created"), today("$updated"), today("*")])
        );

        // A new date property needs no reparse: `*` already covers it.
        let with_due = parse_with(
            "date:today",
            vec![prop("due", "Due", PropKind::Date)],
            Vec::new(),
        );
        assert_eq!(filter_of(&with_due), filter_of(&q));

        // A stored spec reads back equal, filter included.
        let json = serde_json::to_string(&q.spec).expect("the spec serializes");
        let stored: SearchSpec = serde_json::from_str(&json).expect("the spec deserializes");
        assert_eq!(stored, q.spec);
    }

    #[test]
    fn named_dates() {
        assert_eq!(
            filter_of(&parse("created:today")),
            window("$created", utc(2026, 10, 10), utc(2026, 10, 11))
        );
        assert_eq!(
            filter_of(&parse("created:yesterday")),
            window("$created", utc(2026, 10, 9), utc(2026, 10, 10))
        );
        assert_eq!(
            filter_of(&parse("created:tomorrow")),
            window("$created", utc(2026, 10, 11), utc(2026, 10, 12))
        );
        // 2026-10-10 is a Saturday, so the week starts on Monday 2026-10-05.
        assert_eq!(
            filter_of(&parse("created:week")),
            window("$created", utc(2026, 10, 5), utc(2026, 10, 12))
        );
        assert_eq!(
            filter_of(&parse("created:lastweek")),
            window("$created", utc(2026, 9, 28), utc(2026, 10, 5))
        );
        assert_eq!(
            filter_of(&parse("updated:lastmonth")),
            window("$updated", utc(2026, 9, 1), utc(2026, 10, 1))
        );
        assert_eq!(
            filter_of(&parse("updated:month")),
            window("$updated", utc(2026, 10, 1), utc(2026, 11, 1))
        );
        assert_eq!(
            filter_of(&parse("updated:year")),
            window("$updated", utc(2026, 1, 1), utc(2027, 1, 1))
        );
    }

    #[test]
    fn comparisons_use_the_boundary_of_the_date() {
        // `>2026-09` is after the end of September.
        assert_eq!(
            filter_of(&parse("updated:>2026-09")),
            bound("$updated", FilterOp::OnOrAfter, utc(2026, 10, 1))
        );
        // `<today` is before today's start (overdue for a due date).
        assert_eq!(
            filter_of(&parse("updated:<today")),
            bound("$updated", FilterOp::Before, utc(2026, 10, 10))
        );
        assert_eq!(
            filter_of(&parse("created:<=2026-09")),
            bound("$created", FilterOp::Before, utc(2026, 10, 1))
        );
        assert_eq!(
            filter_of(&parse("created:>=2026-09-15")),
            bound("$created", FilterOp::OnOrAfter, utc(2026, 9, 15))
        );
        assert!(!only_operator(&parse("updated:<today")).date_future);
    }

    #[test]
    fn bad_date_is_an_error_token_and_warning() {
        let q = parse("created:banana meeting");
        assert!(q.spec.operators.is_empty());
        assert_eq!(
            q.spec.terms,
            vec![SearchTerm {
                value: "meeting".into(),
                phrase: false,
                exclude: false
            }]
        );
        assert_eq!(warnings(&q), vec!["created: 'banana' is not a date"]);
        assert_eq!(
            q.tokens[0].kind,
            TokenKind::Error {
                message: "created: 'banana' is not a date".into()
            }
        );
    }

    #[test]
    fn date_range_is_or_over_alternatives() {
        let q = parse("created:2026-10-10,2026-10-12");
        assert_eq!(
            filter_of(&q),
            Filter::Or(vec![
                window("$created", utc(2026, 10, 10), utc(2026, 10, 11)),
                window("$created", utc(2026, 10, 12), utc(2026, 10, 13)),
            ])
        );
    }

    #[test]
    fn title_alternatives_are_or() {
        let q = parse("title:foo,bar");
        assert_eq!(
            filter_of(&q),
            Filter::Or(vec![
                cond("$title", FilterOp::Contains, PropValue::Text("foo".into())),
                cond("$title", FilterOp::Contains, PropValue::Text("bar".into())),
            ])
        );
    }

    #[test]
    fn property_keys_resolve_by_stable_key_and_display_name() {
        let props = vec![prop("due_date", "Due date", PropKind::Date)];
        let by_name = parse_with(r#""due date":today"#, props.clone(), Vec::new());
        let by_key = parse_with("duedate:today", props.clone(), Vec::new());
        let expected = window("due_date", utc(2026, 10, 10), utc(2026, 10, 11));
        assert_eq!(filter_of(&by_name), expected);
        assert_eq!(filter_of(&by_key), expected);
        assert_eq!(only_operator(&by_name).key, "due_date");
        assert!(only_operator(&by_name).property);
    }

    #[test]
    fn built_ins_win_unless_quoted() {
        let props = vec![prop("project", "Tag", PropKind::Text)];
        let builtin = parse_with("tag:x", props.clone(), Vec::new());
        assert_eq!(only_operator(&builtin).key, "tag");
        assert!(!only_operator(&builtin).property);

        let quoted = parse_with(r#""tag":x"#, props, Vec::new());
        assert_eq!(only_operator(&quoted).key, "project");
        assert!(only_operator(&quoted).property);
        assert_eq!(
            filter_of(&quoted),
            cond("project", FilterOp::Contains, PropValue::Text("x".into()))
        );
    }

    #[test]
    fn select_option_names_resolve_to_ids() {
        let mut status = prop("status", "Status", PropKind::Select);
        status.options = vec![option("o1", "In progress"), option("o2", "Done")];
        let q = parse_with(r#"status:"in progress""#, vec![status.clone()], Vec::new());
        assert_eq!(
            filter_of(&q),
            cond("status", FilterOp::Is, PropValue::Select("o1".into()))
        );

        let by_id = parse_with("status:o2", vec![status.clone()], Vec::new());
        assert_eq!(
            filter_of(&by_id),
            cond("status", FilterOp::Is, PropValue::Select("o2".into()))
        );

        for value in ["in_progress", "in-progress", "inprogress"] {
            let bad = parse_with(&format!("status:{value}"), vec![status.clone()], Vec::new());
            assert!(bad.spec.operators.is_empty());
            assert_eq!(bad.spec.warnings.len(), 1);
        }
        let bad = parse_with("status:banana", vec![status], Vec::new());
        assert!(bad.spec.operators.is_empty());
        assert_eq!(warnings(&bad), vec!["Status: 'banana' is not an option"]);
    }

    #[test]
    fn multi_select_comma_is_or_of_contains() {
        let mut labels = prop("labels", "Labels", PropKind::MultiSelect);
        labels.options = vec![option("o1", "Work"), option("o2", "Home")];
        let q = parse_with("labels:work,home", vec![labels], Vec::new());
        assert_eq!(
            filter_of(&q),
            Filter::Or(vec![
                cond(
                    "labels",
                    FilterOp::Contains,
                    PropValue::MultiSelect(vec!["o1".into()])
                ),
                cond(
                    "labels",
                    FilterOp::Contains,
                    PropValue::MultiSelect(vec!["o2".into()])
                ),
            ])
        );
    }

    #[test]
    fn numbers_comparisons_and_ranges() {
        let props = vec![prop("score", "Score", PropKind::Number)];
        assert_eq!(
            filter_of(&parse_with("score:5", props.clone(), Vec::new())),
            cond("score", FilterOp::Is, PropValue::Number(5.0))
        );
        assert_eq!(
            filter_of(&parse_with("score:>5", props.clone(), Vec::new())),
            cond("score", FilterOp::Greater, PropValue::Number(5.0))
        );
        assert_eq!(
            filter_of(&parse_with("score:1..5", props.clone(), Vec::new())),
            Filter::And(vec![
                cond("score", FilterOp::GreaterOrEqual, PropValue::Number(1.0)),
                cond("score", FilterOp::LessOrEqual, PropValue::Number(5.0)),
            ])
        );
        let bad = parse_with("score:banana", props, Vec::new());
        assert!(bad.spec.operators.is_empty());
        assert_eq!(warnings(&bad), vec!["Score: 'banana' is not a number"]);
    }

    #[test]
    fn checkbox_values() {
        let props = vec![prop("done", "Done", PropKind::Checkbox)];
        assert_eq!(
            filter_of(&parse_with("done:yes", props.clone(), Vec::new())),
            cond("done", FilterOp::Is, PropValue::Checkbox(true))
        );
        assert_eq!(
            filter_of(&parse_with("done:false", props.clone(), Vec::new())),
            cond("done", FilterOp::Is, PropValue::Checkbox(false))
        );
        assert!(parse_with("done:maybe", props, Vec::new())
            .spec
            .operators
            .is_empty());
    }

    #[test]
    fn none_means_empty() {
        let props = vec![prop("notes", "Notes", PropKind::Text)];
        assert_eq!(
            filter_of(&parse_with("notes:none", props, Vec::new())),
            cond("notes", FilterOp::IsEmpty, PropValue::Empty)
        );
    }

    #[test]
    fn text_property_contains() {
        let props = vec![prop("notes", "Notes", PropKind::Text)];
        assert_eq!(
            filter_of(&parse_with("notes:milk", props, Vec::new())),
            cond("notes", FilterOp::Contains, PropValue::Text("milk".into()))
        );
    }

    #[test]
    fn relation_resolves_a_page() {
        let props = vec![prop("project", "Project", PropKind::Relation)];
        let pages = vec![page("p3", "Projects", 200)];
        let q = parse_with("project:[[Projects]]", props, pages);
        assert_eq!(
            filter_of(&q),
            cond(
                "project",
                FilterOp::Contains,
                PropValue::Relation(vec!["p3".into()])
            )
        );
    }

    #[test]
    fn in_resolves_ids_titles_and_the_best_match() {
        let pages = vec![
            page("p1", "Projects", 100),
            page("p2", "Projects archive", 300),
            page("p3", "projects", 200),
        ];
        // Two exact matches: the more recently updated one wins.
        let exact = parse_with("in:[[Projects]]", Vec::new(), pages.clone());
        assert_eq!(only_operator(&exact).resolved_id.as_deref(), Some("p3"));
        assert_eq!(only_operator(&exact).value, "[[p3]]");
        assert!(only_operator(&exact).filter.is_none());

        let quoted = parse_with(r#"in:"Projects archive""#, Vec::new(), pages.clone());
        assert_eq!(only_operator(&quoted).resolved_id.as_deref(), Some("p2"));

        let by_id = parse_with("in:[[p1]]", Vec::new(), pages.clone());
        assert_eq!(only_operator(&by_id).resolved_id.as_deref(), Some("p1"));

        let links = parse_with("links:archive", Vec::new(), pages.clone());
        assert_eq!(only_operator(&links).key, "links");
        assert_eq!(only_operator(&links).resolved_id.as_deref(), Some("p2"));

        let from_two = parse_with("from:[[p1]],archive", Vec::new(), pages.clone());
        assert_eq!(only_operator(&from_two).resolved_id.as_deref(), Some("p1"));
        assert_eq!(only_operator(&from_two).value, "[[p1]],[[p2]]");
    }

    #[test]
    fn unresolved_page_is_an_error_token() {
        let q = parse_with("in:nope", Vec::new(), vec![page("p1", "Projects", 1)]);
        assert!(q.spec.operators.is_empty());
        assert_eq!(warnings(&q), vec!["in: no page named 'nope'"]);
    }

    #[test]
    fn has_forms() {
        let props = vec![prop("due", "Due", PropKind::Date)];
        let q = parse_with("has:link has:attachment has:due", props.clone(), Vec::new());
        assert_eq!(q.spec.operators.len(), 3);
        assert_eq!(q.spec.operators[0].value, "link");
        assert_eq!(q.spec.operators[1].value, "attachment");
        assert_eq!(q.spec.operators[2].resolved_id.as_deref(), Some("due"));
        assert!(q.spec.operators.iter().all(|o| o.filter.is_none()));

        let bad = parse_with("has:banana", props, Vec::new());
        assert!(bad.spec.operators.is_empty());
        assert_eq!(
            warnings(&bad),
            vec!["has: 'banana' is not link, attachment or a property"]
        );
    }

    #[test]
    fn sort_is_stored_on_the_spec() {
        let props = vec![prop("due_date", "Due date", PropKind::Date)];
        let updated = parse_with("sort:updated", props.clone(), Vec::new());
        assert_eq!(updated.spec.sort.as_deref(), Some("updated"));
        assert!(updated.spec.operators.is_empty());
        assert_eq!(
            updated.tokens[0].kind,
            TokenKind::Operator {
                key: "sort".into(),
                value: "updated".into(),
                resolved_id: None
            }
        );

        let by_prop = parse_with(r#"sort:"Due date""#, props.clone(), Vec::new());
        assert_eq!(by_prop.spec.sort.as_deref(), Some("due_date"));

        let bad = parse_with("sort:banana", props.clone(), Vec::new());
        assert_eq!(bad.spec.sort, None);
        assert_eq!(
            warnings(&bad),
            vec!["sort: 'banana' is not updated, created, title or a property"]
        );

        let negated = parse_with("-sort:title", props, Vec::new());
        assert_eq!(negated.spec.sort, None);
        assert_eq!(warnings(&negated), vec!["sort can't be negated"]);
    }

    #[test]
    fn missing_value_is_an_error() {
        let q = parse("in: meeting");
        assert!(q.spec.operators.is_empty());
        assert_eq!(warnings(&q), vec!["in: needs a value"]);
        assert_eq!(
            q.spec.terms,
            vec![SearchTerm {
                value: "meeting".into(),
                phrase: false,
                exclude: false
            }]
        );
    }

    #[test]
    fn token_spans_cover_each_word() {
        let q = parse("tag:work  foo");
        assert_eq!((q.tokens[0].start, q.tokens[0].end), (0, 8));
        assert_eq!((q.tokens[1].start, q.tokens[1].end), (10, 13));
    }

    #[test]
    fn brackets_keep_spaces_inside_one_word() {
        let pages = vec![page("p1", "Meeting notes", 1)];
        let q = parse_with("in:[[Meeting notes]] x", Vec::new(), pages);
        assert_eq!(only_operator(&q).resolved_id.as_deref(), Some("p1"));
        assert_eq!(q.tokens[0].end, 20);
        assert_eq!(q.spec.terms.len(), 1);
    }

    #[test]
    fn fixed_offset_zone_moves_day_boundaries() {
        let context = SearchContext {
            properties: Vec::new(),
            pages: Vec::new(),
            now_ms: now(),
            tz: FixedOffset::east_opt(2 * 3600).expect("valid offset"),
        };
        let q = parse_search_with_context("created:2026-10-10", &context);
        // Local midnight at +02:00 is 22:00 UTC the day before.
        assert_eq!(
            filter_of(&q),
            window(
                "$created",
                utc(2026, 10, 10) - 7_200_000,
                utc(2026, 10, 11) - 7_200_000
            )
        );
    }

    #[test]
    fn to_query_string_round_trips_resolved_pages() {
        let props = vec![prop("due_date", "Due date", PropKind::Date)];
        let pages = vec![page("p1", "Projects", 100), page("p3", "projects", 200)];
        let original = parse_with(
            r#"meeting -in:[[Projects]] tag:work "due date":today sort:updated"#,
            props.clone(),
            pages.clone(),
        );
        let printed = original.spec.to_query_string();
        assert_eq!(
            printed,
            r#"meeting -in:[[p3]] tag:work "due_date":today sort:updated"#
        );

        let reparsed = parse_with(&printed, props, pages);
        assert_eq!(reparsed.spec.terms, original.spec.terms);
        assert_eq!(reparsed.spec.sort, original.spec.sort);
        assert_eq!(reparsed.spec.operators.len(), original.spec.operators.len());
        assert_eq!(
            reparsed.spec.operators[0].resolved_id.as_deref(),
            Some("p3")
        );
        assert!(reparsed.spec.operators[0].negated);
        assert_eq!(
            reparsed.spec.operators[2].filter,
            original.spec.operators[2].filter
        );
    }

    #[test]
    fn to_query_string_quotes_text_that_would_be_an_operator() {
        let q = parse(r#""a:b" "c d""#);
        assert_eq!(q.spec.to_query_string(), r#""a:b" "c d""#);
    }

    #[test]
    fn negatives_only_still_parse() {
        let q = parse("-tag:work -old");
        assert!(q.spec.operators[0].negated);
        assert!(q.spec.terms[0].exclude);
        assert!(q.spec.warnings.is_empty());
    }

    #[test]
    fn default_context_parses_without_a_schema() {
        let q = parse_search("meeting tag:work");
        assert_eq!(q.spec.terms.len(), 1);
        assert_eq!(q.spec.operators[0].key, "tag");
        assert_eq!(q.tokens.len(), 2);
    }
}
