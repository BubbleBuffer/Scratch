//! Moves a task's card between the columns of the kanban boards that hold it.
//!
//! A board keeps its columns and cards in its own `kanban` type data
//! (`columns: [{id, title}]`, `cards: {<column id>: [<card page id>]}`). A
//! finished run moves its card to the column titled Review (any case) on each
//! board that has one; Accept moves it to the last column of each board. A
//! board without the target column, or with the card already there, is left
//! alone, so every move can run again. The boards' readable Markdown body
//! follows at the next edit made in the app.

use serde_json::Value;
use vault_core::{ListItem, Vault};

use crate::Result;

const ADDON: &str = "kanban";
const PAGE_TYPE: &str = "kanban";

/// Where a card goes on a board.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// The column titled Review.
    Review,
    /// The board's last column.
    Last,
}

/// Moves `card` on every kanban board that lists it. Returns how many boards
/// changed. One board failing does not stop the others.
pub fn move_card(v: &Vault, card: &str, target: Target) -> Result<usize> {
    let mut moved = 0;
    let mut offset = 0u32;
    loop {
        let page = v.list_pages(None, offset, 200)?;
        if page.is_empty() {
            break;
        }
        offset += page.len() as u32;
        for p in page.iter().filter(|p| p.page_type == PAGE_TYPE) {
            if let Ok(true) = move_on_board(v, &p.id, card, target) {
                moved += 1;
            }
        }
    }
    Ok(moved)
}

fn move_on_board(v: &Vault, board: &str, card: &str, target: Target) -> Result<bool> {
    let raw = v
        .type_data(board.to_string(), ADDON.into())
        ?;
    let data: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    let columns: Vec<(String, String)> = data["columns"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| Some((c["id"].as_str()?.to_string(), c["title"].as_str()?.to_string())))
        .collect();
    let holds = |id: &str| {
        data["cards"][id]
            .as_array()
            .is_some_and(|l| l.iter().any(|x| x.as_str() == Some(card)))
    };
    let Some(from) = columns.iter().find(|(id, _)| holds(id)) else {
        return Ok(false);
    };
    let to = match target {
        Target::Review => columns
            .iter()
            .find(|(_, title)| title.trim().eq_ignore_ascii_case("review")),
        Target::Last => columns.last(),
    };
    let Some(to) = to else {
        return Ok(false);
    };
    if to.0 == from.0 {
        return Ok(false);
    }
    if !data["cards"][&to.0].is_array() {
        v.type_data_set(
            board.to_string(),
            ADDON.into(),
            vec!["cards".into(), to.0.clone()],
            "[]".into(),
        )
        ?;
    }
    v.type_data_list_delete(
        board.to_string(),
        ADDON.into(),
        vec!["cards".into(), from.0.clone()],
        ListItem::Id(card.to_string()),
    )
    ?;
    v.type_data_list_insert(
        board.to_string(),
        ADDON.into(),
        vec!["cards".into(), to.0.clone()],
        None,
        Value::String(card.to_string()).to_string(),
    )
    ?;
    Ok(true)
}
