//! Property values between JSON (what assistants send) and the core's
//! `PropValue`, by property name; dates as ISO strings.
//!
//! Property kinds this module does not know (added to the core later) are
//! shown as JSON and cannot be set until they are added here.

use serde_json::{json, Value};
use vault_core::{PageId, PropDef, PropKind, PropValue, SelectOption, Vault};

use crate::{err, Result};

/// The built-in columns every database row has.
pub const TITLE: &str = "$title";

/// The schema entry named `name` (case-insensitive), or the built-ins
/// `Title`, `Created`, `Updated` as pseudo-keys.
pub fn key_of(schema: &[PropDef], name: &str) -> Option<String> {
    let n = name.trim();
    if let Some(d) = schema
        .iter()
        .find(|d| d.name.eq_ignore_ascii_case(n) || d.key == n)
    {
        return Some(d.key.clone());
    }
    match n.to_ascii_lowercase().as_str() {
        "title" | "name" | "$title" => Some(TITLE.into()),
        "created" | "$created" => Some("$created".into()),
        "updated" | "edited" | "$updated" => Some("$updated".into()),
        _ => None,
    }
}

pub fn kind_name(k: PropKind) -> &'static str {
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
        #[allow(unreachable_patterns)]
        _ => "other",
    }
}

/// One line per property: `- Name (kind): option, option`.
pub fn schema_lines(schema: &[PropDef]) -> String {
    let mut out = String::new();
    for d in schema {
        out.push_str(&format!("- {} ({})", d.name, kind_name(d.kind)));
        if !d.options.is_empty() {
            let names: Vec<&str> = d.options.iter().map(|o| o.name.as_str()).collect();
            out.push_str(&format!(": {}", names.join(", ")));
        }
        out.push('\n');
    }
    out
}

/// A value for display (options by name, dates as ISO, relations by title).
pub fn display(v: &Vault, value: &PropValue, def: Option<&PropDef>) -> String {
    let opt = |id: &str| -> String {
        def.and_then(|d| d.options.iter().find(|o| o.id == id))
            .map(|o| o.name.clone())
            .unwrap_or_else(|| id.to_string())
    };
    match value {
        PropValue::Empty => String::new(),
        PropValue::Text(s) | PropValue::Url(s) => s.clone(),
        PropValue::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{n:.0}")
            } else {
                n.to_string()
            }
        }
        PropValue::Checkbox(b) => if *b { "yes" } else { "no" }.into(),
        PropValue::Select(id) | PropValue::Status(id) => opt(id),
        PropValue::MultiSelect(ids) => ids.iter().map(|i| opt(i)).collect::<Vec<_>>().join(", "),
        PropValue::Date {
            start_ms,
            end_ms,
            has_time,
        } => {
            let s = iso(*start_ms, *has_time);
            match end_ms {
                Some(e) => format!("{s} → {}", iso(*e, *has_time)),
                None => s,
            }
        }
        PropValue::Relation(ids) => ids
            .iter()
            .map(|id| {
                v.load_page(id.clone())
                    .map(|p| p.title)
                    .unwrap_or_else(|_| id.clone())
            })
            .collect::<Vec<_>>()
            .join(", "),
        #[allow(unreachable_patterns)]
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// Converts a JSON value for property `def`. `null` (or "") clears it.
/// Unknown select and multi-select option names are added to `def` (the
/// caller saves the schema when `added` is set).
pub fn to_value(v: &Vault, def: &mut PropDef, j: &Value, added: &mut bool) -> Result<PropValue> {
    if j.is_null() || j.as_str().is_some_and(|s| s.trim().is_empty()) {
        return Ok(PropValue::Empty);
    }
    let text = || -> Result<String> {
        match j {
            Value::String(s) => Ok(s.clone()),
            Value::Number(n) => Ok(n.to_string()),
            Value::Bool(b) => Ok(b.to_string()),
            _ => err(format!("{}: expected text", def.name)),
        }
    };
    Ok(match def.kind {
        PropKind::Text | PropKind::Email | PropKind::Phone => PropValue::Text(text()?),
        PropKind::Url => PropValue::Url(text()?),
        PropKind::Number => match j {
            Value::Number(n) => PropValue::Number(n.as_f64().unwrap_or(0.0)),
            Value::String(s) => PropValue::Number(
                s.trim()
                    .parse()
                    .map_err(|_| crate::ToolError(format!("{}: not a number: {s}", def.name)))?,
            ),
            _ => return err(format!("{}: expected a number", def.name)),
        },
        PropKind::Checkbox => PropValue::Checkbox(match j {
            Value::Bool(b) => *b,
            Value::String(s) => matches!(
                s.trim().to_ascii_lowercase().as_str(),
                "true" | "yes" | "1" | "x"
            ),
            Value::Number(n) => n.as_f64() != Some(0.0),
            _ => return err(format!("{}: expected true or false", def.name)),
        }),
        PropKind::Select => PropValue::Select(option(def, &text()?, true, added)?),
        PropKind::Status => PropValue::Status(option(def, &text()?, false, added)?),
        PropKind::MultiSelect => {
            let names: Vec<String> = match j {
                Value::Array(a) => a
                    .iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect(),
                Value::String(s) => s
                    .split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect(),
                _ => return err(format!("{}: expected a list of names", def.name)),
            };
            let mut ids = Vec::new();
            for n in names {
                ids.push(option(def, &n, true, added)?);
            }
            PropValue::MultiSelect(ids)
        }
        PropKind::Date => {
            date_value(j).map_err(|e| crate::ToolError(format!("{}: {e}", def.name)))?
        }
        PropKind::Relation => {
            let items: Vec<String> = match j {
                Value::Array(a) => a
                    .iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect(),
                Value::String(s) => vec![s.clone()],
                _ => return err(format!("{}: expected page ids or titles", def.name)),
            };
            let mut ids = Vec::new();
            for it in items {
                ids.push(crate::tools::resolve_page(v, &it)?);
            }
            PropValue::Relation(ids)
        }
        PropKind::CreatedTime | PropKind::UpdatedTime => {
            return err(format!("{} is computed and cannot be set", def.name))
        }
        #[allow(unreachable_patterns)]
        _ => {
            return err(format!(
                "{}: this property type cannot be set over MCP yet",
                def.name
            ))
        }
    })
}

/// A plain page's property (no schema): text, number or checkbox.
pub fn loose_value(j: &Value) -> Result<PropValue> {
    Ok(match j {
        Value::Null => PropValue::Empty,
        Value::Bool(b) => PropValue::Checkbox(*b),
        Value::Number(n) => PropValue::Number(n.as_f64().unwrap_or(0.0)),
        Value::String(s) => PropValue::Text(s.clone()),
        _ => return err("properties of plain pages are text, numbers or true/false"),
    })
}

fn option(def: &mut PropDef, name: &str, create: bool, added: &mut bool) -> Result<String> {
    let n = name.trim();
    if let Some(o) = def
        .options
        .iter()
        .find(|o| o.name.eq_ignore_ascii_case(n) || o.id == n)
    {
        return Ok(o.id.clone());
    }
    if !create {
        let names: Vec<&str> = def.options.iter().map(|o| o.name.as_str()).collect();
        return err(format!(
            "{}: no option {n:?} (options: {})",
            def.name,
            names.join(", ")
        ));
    }
    // The core assigns the id when the schema is saved; use a fresh one
    // now so the value can refer to it.
    let id = vault_core::new_id();
    def.options.push(SelectOption {
        id: id.clone(),
        name: n.to_string(),
        color: "default".into(),
    });
    *added = true;
    Ok(id)
}

/// `"2026-10-01"`, `"2026-10-01T14:30"` (UTC), or
/// `{"start": ..., "end": ...}`.
pub fn date_value(j: &Value) -> std::result::Result<PropValue, String> {
    let (s, e) = match j {
        Value::String(s) => (s.clone(), None),
        Value::Object(o) => (
            o.get("start")
                .and_then(Value::as_str)
                .ok_or("date needs start")?
                .to_string(),
            o.get("end").and_then(Value::as_str).map(String::from),
        ),
        _ => return Err("expected an ISO date like 2026-10-01".into()),
    };
    let (start_ms, t1) = parse_iso(&s).ok_or(format!("not an ISO date: {s}"))?;
    let end = match e.filter(|x| !x.trim().is_empty()) {
        Some(e) => Some(parse_iso(&e).ok_or(format!("not an ISO date: {e}"))?),
        None => None,
    };
    Ok(PropValue::Date {
        start_ms,
        end_ms: end.map(|x| x.0),
        has_time: t1 || end.is_some_and(|x| x.1),
    })
}

/// `YYYY-MM-DD[THH:MM[:SS]][Z]` -> (ms, has_time), UTC.
pub fn parse_iso(s: &str) -> Option<(i64, bool)> {
    let s = s.trim().trim_end_matches('Z');
    let (d, t) = match s.split_once(['T', ' ']) {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let mut it = d.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let day: i64 = it.next()?.parse().ok()?;
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }
    let mut ms = days_from_civil(y, m, day) * 86_400_000;
    if let Some(t) = t {
        let mut p = t.split(':');
        let h: i64 = p.next()?.parse().ok()?;
        let mi: i64 = p.next().unwrap_or("0").parse().ok()?;
        let sec: f64 = p.next().unwrap_or("0").parse().ok()?;
        ms += h * 3_600_000 + mi * 60_000 + (sec * 1000.0) as i64;
    }
    Some((ms, t.is_some()))
}

/// ISO date (and time, UTC) of `ms`.
pub fn iso(ms: i64, time: bool) -> String {
    let days = ms.div_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    if !time {
        return format!("{y:04}-{m:02}-{d:02}");
    }
    let r = ms.rem_euclid(86_400_000) / 1000;
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}Z", r / 3600, (r / 60) % 60)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Sets `props` (name -> JSON) on `page`. Rows of a database are checked
/// against its schema (new select options are added to it); other pages
/// take text, numbers and booleans. Returns the names set.
pub fn set_props(
    v: &Vault,
    page: &PageId,
    db: Option<&PageId>,
    props: &Value,
) -> Result<Vec<String>> {
    let Some(map) = props.as_object() else {
        return err("properties must be an object of name: value");
    };
    let mut done = Vec::new();
    match db {
        Some(db) => {
            let mut schema = v.get_schema(db.clone())?;
            let mut values = Vec::new();
            let mut added = false;
            for (name, j) in map {
                let key = key_of(&schema, name).ok_or_else(|| {
                    let names: Vec<&str> = schema.iter().map(|d| d.name.as_str()).collect();
                    crate::ToolError(format!(
                        "no property {name:?} (properties: Title, {})",
                        names.join(", ")
                    ))
                })?;
                if key == TITLE {
                    let t = j
                        .as_str()
                        .ok_or_else(|| crate::ToolError("Title must be text".into()))?;
                    values.push((key, None, Some(t.to_string())));
                    continue;
                }
                let def = schema
                    .iter_mut()
                    .find(|d| d.key == key)
                    .ok_or_else(|| crate::ToolError(format!("{name} cannot be set")))?;
                let value = to_value(v, def, j, &mut added)?;
                values.push((key, Some(value), None));
            }
            if added {
                v.set_schema(db.clone(), schema)?;
            }
            for (key, value, title) in values {
                match (value, title) {
                    (_, Some(t)) => v.rename_page(page.clone(), t)?,
                    (Some(PropValue::Empty), _) => v.remove_property(page.clone(), key.clone())?,
                    (Some(val), _) => v.set_property(page.clone(), key.clone(), val)?,
                    (None, None) => {}
                }
                done.push(key);
            }
        }
        None => {
            for (name, j) in map {
                match loose_value(j)? {
                    PropValue::Empty => v.remove_property(page.clone(), name.clone())?,
                    val => v.set_property(page.clone(), name.clone(), val)?,
                }
                done.push(name.clone());
            }
        }
    }
    Ok(done)
}

/// A JSON value for a filter condition on `def` (no option creation).
pub fn filter_value(v: &Vault, def: Option<&PropDef>, j: &Value) -> Result<PropValue> {
    match def {
        None => Ok(match j {
            Value::String(s) => match parse_iso(s) {
                Some((ms, t)) if s.len() >= 10 && s.as_bytes()[4] == b'-' => PropValue::Date {
                    start_ms: ms,
                    end_ms: None,
                    has_time: t,
                },
                _ => PropValue::Text(s.clone()),
            },
            other => loose_value(other)?,
        }),
        Some(d) => {
            let mut d = d.clone();
            let mut added = false;
            let val = to_value(v, &mut d, j, &mut added)?;
            if added {
                return err(format!("{}: no such option: {j}", d.name));
            }
            Ok(val)
        }
    }
}

/// Short JSON shape hint per kind, for tool errors.
pub fn example(k: PropKind) -> Value {
    match k {
        PropKind::Number => json!(3),
        PropKind::Checkbox => json!(true),
        PropKind::MultiSelect => json!(["a", "b"]),
        PropKind::Date => json!("2026-10-01"),
        PropKind::Relation => json!(["<page id or title>"]),
        _ => json!("text"),
    }
}
