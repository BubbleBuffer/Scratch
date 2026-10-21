//! Queries over the index: filter trees and sorts compiled to SQL over the
//! `pages` and `props` tables.
//!
//! Scope is a collection (its live rows, without its default template) or the
//! whole vault (every live page that is not a collection's template).

use std::collections::{BTreeMap, HashMap};

use rusqlite::types::Value;

use crate::error::{invalid, not_found, Result};
use crate::index::{fold, join_ids, Index, SEP};
use crate::types::{
    Condition, Filter, FilterOp, Group, PropDef, PropKind, PropValue, QueryResult, RangeEntry, Sort,
};

struct Scope {
    search: bool,
    collection: Option<String>,
    schema: Vec<PropDef>,
    template: Option<String>,
}

fn scope(db: &Index, collection: Option<&str>) -> Result<Scope> {
    match collection {
        None => Ok(Scope {
            search: false,
            collection: None,
            schema: Vec::new(),
            template: None,
        }),
        Some(c) => {
            if !db.exists(c)? {
                return Err(not_found("page", c));
            }
            Ok(Scope {
                search: false,
                collection: Some(c.to_string()),
                schema: db.schema_of(c)?.unwrap_or_default(),
                template: db.template_of(c)?,
            })
        }
    }
}

impl Scope {
    fn kind(&self, key: &str) -> Option<PropKind> {
        self.schema.iter().find(|d| d.key == key).map(|d| d.kind)
    }
    fn def(&self, key: &str) -> Option<&PropDef> {
        self.schema.iter().find(|d| d.key == key)
    }
    fn where_sql(&self, params: &mut Vec<Value>) -> String {
        match &self.collection {
            Some(c) => {
                params.push(Value::Text(c.clone()));
                params.push(match &self.template {
                    Some(t) => Value::Text(t.clone()),
                    None => Value::Null,
                });
                "p.parent = ? AND p.in_trash = 0 AND p.id IS NOT ? AND p.is_template = 0"
                    .to_string()
            }
            None => format!(
                "p.in_trash = 0 AND {} AND {}",
                crate::index::NOT_TEMPLATE,
                crate::index::NOT_FEEDBACK
            ),
        }
    }
}

/// Where a condition reads its value from.
enum Field {
    /// A property (`None` = any property).
    Prop(Option<String>),
    /// A text column of `pages` (compared folded).
    TextCol(&'static str),
    /// A time column of `pages`.
    DateCol(&'static str),
}

fn field(prop: &str, sc: &Scope) -> Field {
    match prop {
        "$title" => Field::TextCol("p.title_fold"),
        "$created" => Field::DateCol("p.created_ms"),
        "$updated" => Field::DateCol("p.updated_ms"),
        "*" => Field::Prop(None),
        k => match sc.kind(k) {
            Some(PropKind::CreatedTime) => Field::DateCol("p.created_ms"),
            Some(PropKind::UpdatedTime) => Field::DateCol("p.updated_ms"),
            _ => Field::Prop(Some(k.to_string())),
        },
    }
}

/// Column expressions for a predicate.
struct Cols {
    t: String,
    tf: String,
    n: String,
    d0: String,
    d1: String,
}

fn text_param(p: &mut Vec<Value>, s: &str) {
    p.push(Value::Text(s.to_string()));
}

fn contains_id(t: &str, id: &str, p: &mut Vec<Value>) -> String {
    p.push(Value::Text(format!("{SEP}{id}{SEP}")));
    format!("instr(char(31) || {t} || char(31), ?) > 0")
}

/// A positive predicate (true when the value matches).
fn pred(op: FilterOp, v: &PropValue, c: &Cols, p: &mut Vec<Value>) -> Result<String> {
    use FilterOp::*;
    use PropValue as V;
    let bad = || {
        invalid(format!(
            "filter op {op:?} does not apply to a value of kind {:?}",
            v.kind()
        ))
    };
    let date_start = || match v {
        V::Date { start_ms, .. } => Ok(*start_ms),
        _ => Err(bad()),
    };
    Ok(match op {
        Is => match v {
            V::Text(s) | V::Url(s) => {
                text_param(p, &fold(s));
                format!("{} = ?", c.tf)
            }
            V::Number(x) => {
                p.push(Value::Real(*x));
                format!("{} = ?", c.n)
            }
            V::Checkbox(true) => format!("{} = 1", c.n),
            V::Select(id) | V::Status(id) => {
                text_param(p, id);
                format!("{} = ?", c.t)
            }
            V::MultiSelect(ids) | V::Relation(ids) | V::Files(ids) => {
                text_param(p, &join_ids(ids));
                format!("{} = ?", c.t)
            }
            V::Error(s) => {
                text_param(p, &fold(s));
                format!("{} = ?", c.tf)
            }
            V::Date { start_ms, .. } => {
                p.push(Value::Integer(*start_ms));
                format!("{} = ?", c.d0)
            }
            V::Checkbox(false) | V::Empty => return Err(bad()),
        },
        Contains => match v {
            V::Text(s) | V::Url(s) => {
                text_param(p, &fold(s));
                format!("instr({}, ?) > 0", c.tf)
            }
            V::Select(id) | V::Status(id) => contains_id(&c.t, id, p),
            V::MultiSelect(ids) | V::Relation(ids) | V::Files(ids) => {
                if ids.is_empty() {
                    "1".into()
                } else {
                    ids.iter()
                        .map(|id| contains_id(&c.t, id, p))
                        .collect::<Vec<_>>()
                        .join(" AND ")
                }
            }
            _ => return Err(bad()),
        },
        StartsWith | EndsWith => match v {
            V::Text(s) | V::Url(s) => {
                let f = fold(s);
                let n = f.chars().count() as i64;
                if n == 0 {
                    return Ok("1".into());
                }
                p.push(Value::Integer(if op == StartsWith { 1 } else { -n }));
                p.push(Value::Integer(n));
                text_param(p, &f);
                format!("substr({}, ?, ?) = ?", c.tf)
            }
            _ => return Err(bad()),
        },
        Greater | GreaterOrEqual | Less | LessOrEqual => {
            let cmp = match op {
                Greater => ">",
                GreaterOrEqual => ">=",
                Less => "<",
                _ => "<=",
            };
            match v {
                V::Number(x) => {
                    p.push(Value::Real(*x));
                    format!("{} {cmp} ?", c.n)
                }
                V::Date { start_ms, .. } => {
                    p.push(Value::Integer(*start_ms));
                    format!("{} {cmp} ?", c.d0)
                }
                _ => return Err(bad()),
            }
        }
        Before | After | OnOrBefore | OnOrAfter => {
            let cmp = match op {
                Before => "<",
                After => ">",
                OnOrBefore => "<=",
                _ => ">=",
            };
            p.push(Value::Integer(date_start()?));
            format!("{} {cmp} ?", c.d0)
        }
        WithinRange => match v {
            V::Date {
                start_ms, end_ms, ..
            } => match end_ms {
                Some(e) => {
                    p.push(Value::Integer(*e));
                    p.push(Value::Integer(*start_ms));
                    format!("{} < ? AND {} >= ?", c.d0, c.d1)
                }
                None => {
                    p.push(Value::Integer(*start_ms));
                    format!("{} >= ?", c.d1)
                }
            },
            _ => return Err(bad()),
        },
        Checked => format!("{} = 1", c.n),
        IsNot | NotContains | IsEmpty | IsNotEmpty | Unchecked => {
            unreachable!("negative ops are rewritten")
        }
    })
}

/// Normalizes an op to (positive op, negated).
fn positive(op: FilterOp, v: &PropValue) -> (FilterOp, bool) {
    use FilterOp::*;
    match (op, v) {
        (Is, PropValue::Empty) => (IsNotEmpty, true),
        (IsNot, PropValue::Empty) => (IsNotEmpty, false),
        (Is, PropValue::Checkbox(false)) | (IsNot, PropValue::Checkbox(true)) | (Unchecked, _) => {
            (Checked, true)
        }
        (IsNot, PropValue::Checkbox(false)) => (Checked, false),
        (IsNot, _) => (Is, true),
        (NotContains, _) => (Contains, true),
        (IsEmpty, _) => (IsNotEmpty, true),
        (op, _) => (op, false),
    }
}

/// The value of a rule on a formula or rollup, whose result kind varies:
/// numbers typed as text compare as numbers, numbers match as text. `None`:
/// the rule matches nothing (a comparison with text that is no number).
fn coerce_computed(op: FilterOp, v: &PropValue) -> Option<PropValue> {
    use FilterOp::*;
    match (op, v) {
        (Greater | GreaterOrEqual | Less | LessOrEqual, PropValue::Text(s)) => {
            s.trim().parse::<f64>().ok().map(PropValue::Number)
        }
        (Contains | NotContains | StartsWith | EndsWith, PropValue::Number(x)) => {
            Some(PropValue::Text(crate::formula::format_num(*x)))
        }
        _ => Some(v.clone()),
    }
}

fn cond(c: &Condition, sc: &Scope, p: &mut Vec<Value>) -> Result<String> {
    if matches!(sc.kind(&c.prop), Some(PropKind::Formula | PropKind::Rollup)) {
        let Some(value) = coerce_computed(c.op, &c.value) else {
            return Ok("0".into());
        };
        let c = Condition {
            prop: c.prop.clone(),
            op: c.op,
            value,
        };
        return cond_of(&c, sc, p);
    }
    cond_of(c, sc, p)
}

fn cond_of(c: &Condition, sc: &Scope, p: &mut Vec<Value>) -> Result<String> {
    let (op, neg) = positive(c.op, &c.value);
    let not = if neg { "NOT " } else { "" };
    if sc.search && op == FilterOp::Contains {
        if let PropValue::MultiSelect(ids) = &c.value {
            if !ids.is_empty() {
                let mut predicates = Vec::new();
                for value in ids {
                    let key = match &c.prop[..] {
                        "*" => String::new(),
                        key => {
                            p.push(Value::Text(key.to_string()));
                            "x.key = ? AND ".to_string()
                        }
                    };
                    p.push(Value::Text(fold(value)));
                    predicates.push(format!(
                        "p.rowid IN (SELECT x.page FROM search_multi x WHERE {key}x.value = ?)"
                    ));
                }
                return Ok(format!("{not}({})", predicates.join(" AND ")));
            }
        }
    }
    match field(&c.prop, sc) {
        Field::Prop(key) => {
            let mut sub = Vec::new();
            let key_sql = match &key {
                Some(k) => {
                    sub.push(Value::Text(k.clone()));
                    " AND x.key = ?"
                }
                None => "",
            };
            let body = if op == FilterOp::IsNotEmpty {
                String::new()
            } else {
                let cols = Cols {
                    t: if sc.search { "search_fold(x.t)" } else { "x.t" }.into(),
                    tf: "x.tf".into(),
                    n: "x.n".into(),
                    d0: if sc.search { "search_date_start(x.d0, coalesce(json_extract(x.json, '$.Date.has_time'), 1))" } else { "x.d0" }.into(),
                    d1: if sc.search { "search_date_end(coalesce(x.d1, x.d0), coalesce(json_extract(x.json, '$.Date.has_time'), 1))" } else { "coalesce(x.d1, x.d0)" }.into(),
                };
                format!(" AND ({})", pred(op, &c.value, &cols, &mut sub)?)
            };
            p.extend(sub);
            Ok(format!(
                "{not}EXISTS(SELECT 1 FROM props x WHERE x.page = p.rowid{key_sql}{body})"
            ))
        }
        Field::TextCol(col) => {
            if op == FilterOp::IsNotEmpty {
                return Ok(format!("{not}({col} != '')"));
            }
            let cols = Cols {
                t: col.into(),
                tf: col.into(),
                n: "NULL".into(),
                d0: "NULL".into(),
                d1: "NULL".into(),
            };
            Ok(format!(
                "{not}coalesce({}, 0)",
                pred(op, &c.value, &cols, p)?
            ))
        }
        Field::DateCol(col) => {
            if op == FilterOp::IsNotEmpty {
                return Ok(format!("{not}1"));
            }
            let cols = Cols {
                t: "NULL".into(),
                tf: "NULL".into(),
                n: col.into(),
                d0: col.into(),
                d1: col.into(),
            };
            Ok(format!(
                "{not}coalesce({}, 0)",
                pred(op, &c.value, &cols, p)?
            ))
        }
    }
}

fn filter_sql_inner(f: &Filter, sc: &Scope, p: &mut Vec<Value>, depth: u32) -> Result<String> {
    if depth > 64 {
        return Err(invalid("filter nested too deeply"));
    }
    Ok(match f {
        Filter::Cond(c) => cond(c, sc, p)?,
        Filter::And(v) if v.is_empty() => "1".into(),
        Filter::Or(v) if v.is_empty() => "0".into(),
        Filter::And(v) | Filter::Or(v) => {
            let sep = if matches!(f, Filter::And(_)) {
                " AND "
            } else {
                " OR "
            };
            let parts = v
                .iter()
                .map(|x| filter_sql_inner(x, sc, p, depth + 1))
                .collect::<Result<Vec<_>>>()?;
            format!("({})", parts.join(sep))
        }
    })
}

pub(crate) fn normalize_search_filter(f: &Filter) -> Filter {
    match f {
        Filter::And(v) | Filter::Or(v) => {
            let mut v: Vec<_> = v.iter().map(normalize_search_filter).collect();
            v.sort_by_cached_key(|f| serde_json::to_string(f).unwrap_or_default());
            if matches!(f, Filter::And(_)) {
                Filter::And(v)
            } else {
                Filter::Or(v)
            }
        }
        Filter::Cond(c) => {
            let mut c = c.clone();
            c.value = match &c.value {
                PropValue::Select(v) => PropValue::Select(fold(v)),
                PropValue::Status(v) => PropValue::Status(fold(v)),
                PropValue::MultiSelect(v) => {
                    PropValue::MultiSelect(v.iter().map(|s| fold(s)).collect())
                }
                PropValue::Relation(v) => PropValue::Relation(v.iter().map(|s| fold(s)).collect()),
                PropValue::Files(v) => PropValue::Files(v.iter().map(|s| fold(s)).collect()),
                PropValue::Text(v) => PropValue::Text(fold(v)),
                PropValue::Url(v) => PropValue::Url(fold(v)),
                PropValue::Error(v) => PropValue::Error(fold(v)),
                v => v.clone(),
            };
            Filter::Cond(c)
        }
    }
}

/// Compile a vault-wide filter with numbered bindings. Existing structured
/// queries retain their collection scope and anonymous binding order.
pub(crate) fn filter_sql(f: &Filter, p: &mut Vec<Value>) -> Result<String> {
    let start = p.len();
    let sc = Scope {
        search: true,
        collection: None,
        schema: Vec::new(),
        template: None,
    };
    let sql = filter_sql_inner(&normalize_search_filter(f), &sc, p, 0)?;
    let mut n = start;
    Ok(sql
        .chars()
        .map(|c| {
            if c == '?' {
                n += 1;
                format!("?{n}")
            } else {
                c.to_string()
            }
        })
        .collect())
}

/// One sort key: an SQL expression and whether it sorts descending.
type SortKey = (String, bool);

/// (joins, sort keys, tie-break order by) for the sorts. The tie-break is
/// the scope's order, which the indexes give without sorting.
fn sort_sql(
    sorts: &[Sort],
    sc: &Scope,
    p: &mut Vec<Value>,
) -> (String, Vec<SortKey>, &'static str) {
    let mut joins = String::new();
    let mut order: Vec<SortKey> = Vec::new();
    for (i, s) in sorts.iter().enumerate() {
        let desc = s.descending;
        match field(&s.prop, sc) {
            Field::TextCol(c) | Field::DateCol(c) => order.push((c.to_string(), desc)),
            Field::Prop(None) => {}
            Field::Prop(Some(k)) => {
                let a = format!("s{i}");
                joins.push_str(&format!(
                    " LEFT JOIN props {a} ON {a}.page = p.rowid AND {a}.key = ?"
                ));
                p.push(Value::Text(k.clone()));
                // Empty values last, whatever the direction.
                order.push((format!("({a}.page IS NULL)"), false));
                match sc.kind(&k) {
                    Some(PropKind::Number | PropKind::Checkbox) => {
                        order.push((format!("{a}.n"), desc))
                    }
                    Some(PropKind::Date) => order.push((format!("{a}.d0"), desc)),
                    Some(PropKind::Text | PropKind::Url | PropKind::Email | PropKind::Phone) => {
                        order.push((format!("{a}.tf"), desc))
                    }
                    Some(PropKind::Select | PropKind::Status) => {
                        let opts = &sc.def(&k).expect("def").options;
                        let mut case = format!("CASE {a}.t");
                        for (j, o) in opts.iter().enumerate() {
                            // Option ids are inlined as quoted literals.
                            case.push_str(&format!(
                                " WHEN '{}' THEN {j}",
                                o.id.replace('\'', "''")
                            ));
                        }
                        case.push_str(&format!(" ELSE {} END", opts.len()));
                        order.push((case, desc));
                    }
                    Some(PropKind::Formula | PropKind::Rollup) | None => {
                        for c in ["n", "d0", "tf", "t"] {
                            order.push((format!("{a}.{c}"), desc));
                        }
                    }
                    Some(_) => order.push((format!("{a}.t"), desc)),
                }
            }
        }
    }
    let tie = match sc.collection {
        Some(_) => "p.sort_key, p.created_ms, p.id",
        None => "p.updated_ms DESC, p.id DESC",
    };
    (joins, order, tie)
}

/// SQLite's ordering of values: NULL, then numbers, then text, then blobs.
fn cmp_value(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    fn rank(v: &Value) -> u8 {
        match v {
            Value::Null => 0,
            Value::Integer(_) | Value::Real(_) => 1,
            Value::Text(_) => 2,
            Value::Blob(_) => 3,
        }
    }
    match (a, b) {
        (Value::Integer(x), Value::Integer(y)) => x.cmp(y),
        (Value::Integer(x), Value::Real(y)) => {
            (*x as f64).partial_cmp(y).unwrap_or(Ordering::Equal)
        }
        (Value::Real(x), Value::Integer(y)) => {
            x.partial_cmp(&(*y as f64)).unwrap_or(Ordering::Equal)
        }
        (Value::Real(x), Value::Real(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Value::Text(x), Value::Text(y)) => x.as_bytes().cmp(y.as_bytes()),
        (Value::Blob(x), Value::Blob(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

pub(crate) fn query(
    db: &Index,
    collection: Option<&str>,
    filter: Option<&Filter>,
    sorts: &[Sort],
    offset: u32,
    limit: u32,
) -> Result<QueryResult> {
    let sc = scope(db, collection)?;
    let mut p = Vec::new();
    let (joins, order, tie) = sort_sql(sorts, &sc, &mut p);
    let mut w = sc.where_sql(&mut p);
    if let Some(f) = filter {
        w = format!("{w} AND {}", filter_sql_inner(f, &sc, &mut p, 0)?);
    }
    // Every matching row in the scope's order with its sort keys, then a
    // stable sort here: cheaper than an SQL sort plus a window count, and
    // the total comes for free.
    let cols: String = order.iter().map(|(e, _)| format!(", {e}")).collect();
    let sql = format!("SELECT p.rowid{cols} FROM pages p{joins} WHERE {w} ORDER BY {tie}");
    let n = order.len();
    let mut rows: Vec<(usize, i64, Vec<Value>)> = Vec::new();
    {
        let mut st = db.conn.prepare_cached(&sql)?;
        let mut q = st.query(rusqlite::params_from_iter(p.iter()))?;
        while let Some(r) = q.next()? {
            let mut keys = Vec::with_capacity(n);
            for i in 0..n {
                keys.push(r.get::<_, Value>(i + 1)?);
            }
            rows.push((rows.len(), r.get(0)?, keys));
        }
    }
    let total = rows.len();
    let start = (offset as usize).min(total);
    let end = start.saturating_add(limit as usize).min(total);
    if n > 0 {
        let by = |a: &(usize, i64, Vec<Value>), b: &(usize, i64, Vec<Value>)| {
            for (i, (_, desc)) in order.iter().enumerate() {
                let o = cmp_value(&a.2[i], &b.2[i]);
                if o.is_ne() {
                    return if *desc { o.reverse() } else { o };
                }
            }
            a.0.cmp(&b.0)
        };
        if end < total {
            // Only the first `end` rows need to be in order.
            rows.select_nth_unstable_by(end, by);
            rows.truncate(end);
        }
        rows.sort_unstable_by(by);
    }
    let rowids: Vec<i64> = rows[start..end].iter().map(|r| r.1).collect();
    Ok(QueryResult {
        rows: db.summaries(&rowids)?,
        total: total as u32,
    })
}

fn option_label(def: Option<&PropDef>, id: &str) -> (String, Option<String>) {
    match def.and_then(|d| d.options.iter().find(|o| o.id == id)) {
        Some(o) => (o.name.clone(), Some(o.color.clone())),
        None => (id.to_string(), None),
    }
}

fn display(v: &PropValue) -> String {
    match v {
        PropValue::Empty => String::new(),
        PropValue::Text(s)
        | PropValue::Url(s)
        | PropValue::Select(s)
        | PropValue::Status(s)
        | PropValue::Error(s) => s.clone(),
        PropValue::Number(x) => {
            if x.fract() == 0.0 && x.abs() < 1e15 {
                format!("{x:.0}")
            } else {
                x.to_string()
            }
        }
        PropValue::Checkbox(b) => b.to_string(),
        PropValue::Date {
            start_ms,
            end_ms,
            has_time,
        } => crate::markdown::date_text(*start_ms, *end_ms, *has_time),
        PropValue::MultiSelect(v) | PropValue::Relation(v) | PropValue::Files(v) => v.join(", "),
    }
}

pub(crate) fn groups(
    db: &Index,
    collection: Option<&str>,
    filter: Option<&Filter>,
    group_by: &str,
) -> Result<Vec<Group>> {
    let sc = scope(db, collection)?;
    if group_by.starts_with('$') || group_by == "*" {
        return Err(invalid(format!("cannot group by {group_by}")));
    }
    let mut p = vec![Value::Text(group_by.to_string())];
    let mut w = sc.where_sql(&mut p);
    if let Some(f) = filter {
        w = format!("{w} AND {}", filter_sql_inner(f, &sc, &mut p, 0)?);
    }
    let sql = format!(
        "SELECT g.json, count(*) FROM pages p
         LEFT JOIN props g ON g.page = p.rowid AND g.key = ?
         WHERE {w} GROUP BY g.json"
    );
    let mut counts: Vec<(Option<PropValue>, u32)> = Vec::new();
    {
        let mut st = db.conn.prepare_cached(&sql)?;
        let mut rows = st.query(rusqlite::params_from_iter(p.iter()))?;
        while let Some(r) = rows.next()? {
            let json: Option<String> = r.get(0)?;
            let n: i64 = r.get(1)?;
            counts.push((json.and_then(|j| serde_json::from_str(&j).ok()), n as u32));
        }
    }
    let def = sc.def(group_by);
    let kind = def.map(|d| d.kind).or_else(|| {
        counts
            .iter()
            .find_map(|(v, _)| v.as_ref().and_then(|v| v.kind()))
    });
    let mut empty = 0u32;
    let mut out: Vec<Group> = Vec::new();
    match kind {
        Some(PropKind::Checkbox) => {
            let mut yes = 0;
            for (v, n) in &counts {
                if v == &Some(PropValue::Checkbox(true)) {
                    yes += n;
                } else {
                    empty += n;
                }
            }
            out.push(Group {
                value: PropValue::Checkbox(false),
                label: "false".into(),
                color: None,
                count: empty,
            });
            out.push(Group {
                value: PropValue::Checkbox(true),
                label: "true".into(),
                color: None,
                count: yes,
            });
            return Ok(out);
        }
        Some(k @ (PropKind::Select | PropKind::Status | PropKind::MultiSelect)) => {
            let mut per: HashMap<String, u32> = HashMap::new();
            let mut unknown: Vec<String> = Vec::new();
            for (v, n) in &counts {
                let ids: Vec<String> = match v {
                    Some(PropValue::Select(id)) | Some(PropValue::Status(id)) => vec![id.clone()],
                    Some(PropValue::MultiSelect(ids)) if !ids.is_empty() => ids.clone(),
                    _ => {
                        empty += n;
                        continue;
                    }
                };
                for id in ids {
                    *per.entry(id.clone()).or_default() += n;
                    let known = def.is_some_and(|d| d.options.iter().any(|o| o.id == id));
                    if !known && !unknown.contains(&id) {
                        unknown.push(id);
                    }
                }
            }
            let make = |id: &str| -> PropValue {
                match k {
                    PropKind::Status => PropValue::Status(id.to_string()),
                    PropKind::MultiSelect => PropValue::MultiSelect(vec![id.to_string()]),
                    _ => PropValue::Select(id.to_string()),
                }
            };
            out.push(Group {
                value: PropValue::Empty,
                label: String::new(),
                color: None,
                count: empty,
            });
            if let Some(d) = def {
                for o in &d.options {
                    out.push(Group {
                        value: make(&o.id),
                        label: o.name.clone(),
                        color: Some(o.color.clone()),
                        count: per.get(&o.id).copied().unwrap_or(0),
                    });
                }
            }
            unknown.sort();
            for id in unknown {
                let (label, color) = option_label(def, &id);
                out.push(Group {
                    value: make(&id),
                    label,
                    color,
                    count: per.get(&id).copied().unwrap_or(0),
                });
            }
            return Ok(out);
        }
        _ => {}
    }
    // Other kinds: one group per distinct value, sorted.
    let mut distinct: BTreeMap<(i64, String), (PropValue, u32)> = BTreeMap::new();
    for (v, n) in counts {
        match v {
            None | Some(PropValue::Empty) => empty += n,
            Some(v) => {
                let sort = match &v {
                    PropValue::Number(x) => (*x * 1000.0) as i64,
                    PropValue::Date { start_ms, .. } => *start_ms,
                    _ => 0,
                };
                let e = distinct
                    .entry((sort, display(&v).to_lowercase() + &format!("{v:?}")))
                    .or_insert((v, 0));
                e.1 += n;
            }
        }
    }
    out.push(Group {
        value: PropValue::Empty,
        label: String::new(),
        color: None,
        count: empty,
    });
    for (_, (v, n)) in distinct {
        out.push(Group {
            label: display(&v),
            value: v,
            color: None,
            count: n,
        });
    }
    Ok(out)
}

pub(crate) fn range(
    db: &Index,
    collection: Option<&str>,
    date_prop: &str,
    from_ms: i64,
    to_ms: i64,
    filter: Option<&Filter>,
) -> Result<Vec<RangeEntry>> {
    let sc = scope(db, collection)?;
    let mut p = Vec::new();
    let sql = match field(date_prop, &sc) {
        Field::DateCol(col) => {
            p.push(Value::Text(date_prop.to_string()));
            let mut w = sc.where_sql(&mut p);
            if let Some(f) = filter {
                w = format!("{w} AND {}", filter_sql_inner(f, &sc, &mut p, 0)?);
            }
            p.push(Value::Integer(from_ms));
            p.push(Value::Integer(to_ms));
            format!(
                "SELECT p.rowid, ?, {col}, NULL, NULL FROM pages p
                 WHERE {w} AND {col} >= ? AND {col} < ? ORDER BY {col}, p.id"
            )
        }
        Field::Prop(key) => {
            let key_sql = match &key {
                Some(k) => {
                    p.push(Value::Text(k.clone()));
                    " AND r.key = ?"
                }
                None => "",
            };
            let mut w = sc.where_sql(&mut p);
            if let Some(f) = filter {
                w = format!("{w} AND {}", filter_sql_inner(f, &sc, &mut p, 0)?);
            }
            p.push(Value::Integer(to_ms));
            p.push(Value::Integer(from_ms));
            format!(
                "SELECT p.rowid, r.key, r.d0, r.d1, r.json FROM pages p
                 JOIN props r ON r.page = p.rowid{key_sql}
                 WHERE {w} AND r.d0 IS NOT NULL AND r.d0 < ? AND coalesce(r.d1, r.d0) >= ?
                 ORDER BY r.d0, p.id, r.key"
            )
        }
        Field::TextCol(_) => return Err(invalid("$title is not a date")),
    };
    let mut hits: Vec<(i64, String, i64, Option<i64>, bool)> = Vec::new();
    {
        let mut st = db.conn.prepare_cached(&sql)?;
        let mut rows = st.query(rusqlite::params_from_iter(p.iter()))?;
        while let Some(r) = rows.next()? {
            let json: Option<String> = r.get(4)?;
            let has_time = match json.and_then(|j| serde_json::from_str::<PropValue>(&j).ok()) {
                Some(PropValue::Date { has_time, .. }) => has_time,
                _ => true,
            };
            hits.push((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, has_time));
        }
    }
    let mut ids: Vec<i64> = hits.iter().map(|h| h.0).collect();
    ids.sort_unstable();
    ids.dedup();
    let sums = db.summaries(&ids)?;
    let by: HashMap<i64, usize> = ids.iter().enumerate().map(|(i, r)| (*r, i)).collect();
    Ok(hits
        .into_iter()
        .map(|(row, prop, start, end, has_time)| RangeEntry {
            page: sums[by[&row]].clone(),
            prop,
            start_ms: start,
            end_ms: end,
            has_time,
        })
        .collect())
}
