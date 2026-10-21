//! `move_page`: moves pages (with their subtrees) to a new parent and place,
//! as the app's sidebar drag and drop does.

use std::collections::HashMap;

use serde_json::Value;
use vault_core::{PageId, Vault};

use crate::tools::resolve_page;
use crate::{err, Result, ToolError};

/// The pages a `pages` argument names: one id or title, or a list.
fn pages_arg(v: &Vault, a: &Value) -> Result<Vec<PageId>> {
    let names: Vec<&str> = match a.get("pages") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(list)) if !list.is_empty() => list
            .iter()
            .map(|p| {
                p.as_str()
                    .ok_or_else(|| ToolError("pages must be page ids or exact titles".into()))
            })
            .collect::<Result<_>>()?,
        _ => return err("missing argument: pages (one or more page ids or exact titles)"),
    };
    let mut ids: Vec<PageId> = Vec::new();
    for name in names {
        let id = resolve_page(v, name)?;
        if ids.contains(&id) {
            return err(format!("page {id} is named twice in pages"));
        }
        ids.push(id);
    }
    Ok(ids)
}

fn optional_page(v: &Vault, a: &Value, key: &str) -> Result<Option<PageId>> {
    match a.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) => resolve_page(v, s).map(Some),
        Some(_) => err(format!("{key} must be a page id or exact title")),
    }
}

/// The page and its ancestors, innermost first.
fn chain(v: &Vault, page: &PageId) -> Result<Vec<PageId>> {
    let mut out = vec![page.clone()];
    let mut next = v.load_page(page.clone())?.parent;
    while let Some(id) = next {
        if out.contains(&id) || out.len() > 256 {
            break;
        }
        next = v.load_page(id.clone())?.parent;
        out.push(id);
    }
    Ok(out)
}

pub(crate) fn move_page(v: &Vault, a: &Value) -> Result<String> {
    let pages = pages_arg(v, a)?;
    let parent = optional_page(v, a, "parent")?;
    let after = optional_page(v, a, "after")?;
    if let Some(after) = &after {
        if pages.contains(after) {
            return err("after can't be one of the pages being moved");
        }
        let sibling_parent = v.load_page(after.clone())?.parent;
        if sibling_parent != parent {
            return err(format!("page {after} is not a child of the new parent"));
        }
    }
    // Refuse before moving anything, with the core's own error.
    if let Some(parent) = &parent {
        let above = chain(v, parent)?;
        if pages.iter().any(|p| above.contains(p)) {
            return err("cannot move a page into itself");
        }
    }
    let mut previous = after;
    for page in &pages {
        v.move_page(page.clone(), parent.clone(), previous.clone())?;
        previous = Some(page.clone());
    }
    let mut cache = HashMap::new();
    let mut out = String::from("Moved:\n");
    for page in &pages {
        let p = v.load_page(page.clone())?;
        let path = crate::query::path_of(v, &p, &mut cache);
        let title = if p.title.trim().is_empty() {
            "Untitled"
        } else {
            p.title.as_str()
        };
        let place = if path.is_empty() {
            "top level".to_string()
        } else {
            path
        };
        out.push_str(&format!("- **{title}** `{}` → {place}\n", p.id));
    }
    Ok(out.trim_end().to_string())
}
