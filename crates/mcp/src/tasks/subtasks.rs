//! A run's todo list, mirrored into the task's page as nested todos (see
//! [`super`]). A run sends its whole list ([`mirror_subtasks`]); each item
//! becomes a todo under the task's source todo, or under its placeholder when
//! it has none.
//!
//! The attempt's tracking is the row's `subtask_blocks` property, as JSON
//! (see [`State`]): the session that began the attempt, the block the
//! subtasks nest under, and the items in list order. An item owns a block
//! until that block is gone or has moved away from the parent; then it has
//! none, and the item stays suppressed while the list keeps it, within this
//! attempt only. A todo the attempt owns is deleted only when it has no
//! children. A todo with the user's children is left exactly as it is and is
//! no longer the attempt's. The parent is never changed.

use serde::{Deserialize, Serialize};
use vault_core::{Block, BlockKind, BlockOp, NewBlock, PageId, PropValue, Vault};

use super::{collection, find, get, migrate_schema, parent_of, text, Task, DONE, RUNNING};
use crate::{err, Result};

/// The row property that holds the attempt's tracking.
const PROPERTY: &str = "subtask_blocks";
/// The row property that names the session that claimed the task.
const SESSION: &str = "session";

/// One item of a run's todo list. Its todo is checked only when `status` is
/// `done`. The text is kept exactly as given.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SubtaskItem {
    pub text: String,
    pub status: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    /// The session whose attempt this is: the only one that may write.
    session: String,
    /// The block the subtasks nest under. Kept with the attempt so that a
    /// later check can tell which blocks are still under it.
    parent: String,
    /// The items in list order.
    items: Vec<Item>,
}

/// One item of the list. `block` is the todo the attempt owns for it, or
/// None once that todo is gone or has left the parent: the item is then
/// suppressed while the list keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Item {
    block: Option<String>,
    text: String,
}

/// The tracking text. Unset (or blank) is the initial state. Malformed text
/// is an error: reading it as the initial state would lose the attempt's
/// ownership.
fn parse(s: &str) -> Result<State> {
    if s.trim().is_empty() {
        return Ok(State::default());
    }
    match serde_json::from_str(s) {
        Ok(state) => Ok(state),
        Err(_) => err("the subtask tracking is damaged"),
    }
}

/// The blocks the attempt owns now (its mirrored todos), from the row's
/// tracking text. A damaged or empty tracking owns none.
pub(super) fn owned_blocks(tracking: &str) -> Vec<String> {
    parse(tracking)
        .map(|s| s.items.into_iter().filter_map(|i| i.block).collect())
        .unwrap_or_default()
}

/// A to-do the user added under the running task: steering for the worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub block: String,
    pub text: String,
}

/// The block steps nest under: the attempt's parent once it began, else the
/// task's own.
fn steps_parent(t: &Task, state: &State) -> Option<String> {
    Some(state.parent.clone())
        .filter(|p| !p.is_empty())
        .or_else(|| nest_under(t))
}

/// The to-dos under a running task that the user added and the worker has not
/// been told about: unchecked, with text, not owned by the run, not there
/// when the run began and not sent already. Empty unless the task runs and is
/// not being cancelled.
pub fn pending_steps(v: &Vault, task_id: &str) -> Result<Vec<Step>> {
    let (t, state, _) = running(v, task_id)?;
    if t.cancel_requested_ms.is_some() {
        return Ok(Vec::new());
    }
    let (Some(page), Some(parent)) = (t.page.clone(), steps_parent(&t, &state)) else {
        return Ok(Vec::new());
    };
    let p = v.load_page(page)?;
    let Some(pb) = find(&p.blocks, &parent) else {
        return Ok(Vec::new());
    };
    let attempt = super::attempts::list(v, task_id)?
        .into_iter()
        .find(|a| a.id == t.attempt)
        .unwrap_or_default();
    let owned: Vec<&String> = state.items.iter().filter_map(|i| i.block.as_ref()).collect();
    Ok(pb
        .children
        .iter()
        .filter(|c| {
            c.kind == BlockKind::Todo
                && !c.checked
                && !c.text.trim().is_empty()
                && !owned.contains(&&c.id)
                && !attempt.steered.contains(&c.id)
                && !attempt.baseline.contains(&c.id)
        })
        .map(|c| Step {
            block: c.id.clone(),
            text: c.text.trim().to_string(),
        })
        .collect())
}

/// Notes the to-dos under the task now as the user's own, so they are never
/// sent as steering. Call once the attempt's subtasks have begun.
pub fn record_baseline(v: &Vault, task_id: &str) -> Result<()> {
    let (t, state, _) = running(v, task_id)?;
    let (Some(page), Some(parent)) = (t.page.clone(), steps_parent(&t, &state)) else {
        return Ok(());
    };
    let p = v.load_page(page)?;
    let ids: Vec<String> = find(&p.blocks, &parent)
        .map(|pb| pb.children.iter().map(|c| c.id.clone()).collect())
        .unwrap_or_default();
    super::attempts::update(v, task_id, None, |a| a.baseline = ids)
}

/// Adds a to-do with `text` under the running task, for the worker: the
/// runner sends it as steering. The task must be running and not being
/// cancelled.
pub fn add_step(v: &Vault, task_id: &str, text: &str) -> Result<()> {
    let text = text.trim();
    if text.is_empty() {
        return err("the step needs text");
    }
    let (t, state, _) = running(v, task_id)?;
    if t.cancel_requested_ms.is_some() {
        return err("the task is being cancelled");
    }
    let Some(page) = t.page.clone() else {
        return err("the task has no page");
    };
    let p = v.load_page(page.clone())?;
    let Some(parent) = steps_parent(&t, &state).filter(|b| find(&p.blocks, b).is_some()) else {
        return err("no block to add the step under");
    };
    let after = find(&p.blocks, &parent).and_then(|b| b.children.last().map(|c| c.id.clone()));
    v.apply(
        page,
        vec![BlockOp::Insert {
            parent: Some(parent),
            after,
            block: NewBlock {
                id: vault_core::new_id(),
                kind: BlockKind::Todo,
                text: text.to_string(),
                marks: vec![],
                checked: false,
            },
        }],
    )?;
    Ok(())
}

/// The task, which must be running, its tracking, and the session its row
/// names (empty when none).
fn running(v: &Vault, task_id: &str) -> Result<(Task, State, String)> {
    let t = get(v, task_id)?;
    if t.status != RUNNING {
        return err("subtasks are only written while the task runs");
    }
    let row = v.load_page(t.id.clone())?;
    let state = parse(&text(&row.properties, PROPERTY))?;
    let row_session = text(&row.properties, SESSION);
    Ok((t, state, row_session))
}

/// A session other than the one the row names may not write. This stops an
/// old attempt from overwriting a newer one.
fn check_row_session(row_session: &str, session_id: &str) -> Result<()> {
    if !row_session.is_empty() && row_session != session_id {
        return err("the task belongs to another session");
    }
    Ok(())
}

/// Stores the tracking. The property is added to an older database first.
fn save(v: &Vault, task_id: &str, state: &State) -> Result<()> {
    if let Some(c) = collection(v)? {
        migrate_schema(v, &c)?;
    }
    v.set_property(
        task_id.into(),
        PROPERTY.into(),
        PropValue::Text(serde_json::to_string(state)?),
    )?;
    Ok(())
}

/// Applies `ops`, planned from the block tree `expected`, only if the tree is
/// still that. A user edit that raced the plan is an error, with no change.
fn apply_plan(v: &Vault, page: PageId, expected: &[Block], ops: Vec<BlockOp>) -> Result<()> {
    if ops.is_empty() {
        return Ok(());
    }
    v.apply_if_blocks_match(page, expected, ops)?;
    Ok(())
}

/// The block the subtasks of `t` nest under: its source todo, else its
/// placeholder.
fn nest_under(t: &Task) -> Option<String> {
    t.source_block.clone().or_else(|| t.block.clone())
}

/// Whether block `id` is a child of `parent`.
fn under(blocks: &[Block], parent: &str, id: &str) -> bool {
    parent_of(blocks, None, id) == Some(Some(parent.to_string()))
}

/// The state's items, each with its block only while the block is still a
/// child of the attempt's parent.
fn refresh(blocks: &[Block], state: &State) -> Vec<Item> {
    state
        .items
        .iter()
        .map(|i| Item {
            block: i.block.clone().filter(|b| under(blocks, &state.parent, b)),
            text: i.text.clone(),
        })
        .collect()
}

/// The op that deletes the owned todo `block`. A todo with children is not
/// deleted: its subtree is the user's, so it stays as it is (None).
fn removal(block: &Block) -> Option<BlockOp> {
    block.children.is_empty().then(|| BlockOp::Delete {
        id: block.id.clone(),
    })
}

/// Starts `session_id`'s attempt at the task's subtasks; the task must be
/// running, and the session must be the row's when the row names one. A
/// session that already began the attempt changes nothing. Otherwise the
/// earlier attempt's todos are deleted when all of them are unchecked and
/// they have no children; todos with children, and all of them when any is
/// checked, stay as they are and are no longer the attempt's. The new attempt
/// starts with an empty list: nothing from the earlier attempt, suppression
/// included, carries over. [`mirror_subtasks`] adds the todos.
pub fn begin_subtasks(v: &Vault, task_id: &str, session_id: &str) -> Result<()> {
    if session_id.trim().is_empty() {
        return err("the subtasks need a session");
    }
    let (t, state, row_session) = running(v, task_id)?;
    check_row_session(&row_session, session_id)?;
    if state.session == session_id {
        return Ok(());
    }
    let Some(page) = t.page.clone() else {
        return err("the task has no page");
    };
    let p = v.load_page(page.clone())?;
    let Some(parent) = nest_under(&t).filter(|b| find(&p.blocks, b).is_some()) else {
        return err("no block to nest the subtasks under");
    };
    let old = refresh(&p.blocks, &state);
    let mirrors: Vec<&Block> = old
        .iter()
        .filter_map(|i| i.block.as_deref().and_then(|b| find(&p.blocks, b)))
        .collect();
    let mut ops = Vec::new();
    if mirrors.iter().all(|b| !b.checked) {
        ops.extend(mirrors.iter().filter_map(|b| removal(b)));
    }
    apply_plan(v, page, &p.blocks, ops)?;
    save(
        v,
        &t.id,
        &State {
            session: session_id.to_string(),
            parent,
            items: Vec::new(),
        },
    )
}

/// Makes the task's subtasks match `items`, the run's whole todo list. Items
/// keep their todos: first by equal text, in order, then by position among
/// the items left over. New items are added at the end, and items that left
/// the list lose their todos (see [`removal`]). The owned todos are put in
/// list order, and the others stay where they are. Each todo has the item's
/// text exactly, and is checked only when the item's status is `done`. Blank
/// items are skipped.
///
/// Only the session that began the attempt may call this, and only while the
/// task runs. Anything else is an error, and nothing is written.
pub fn mirror_subtasks(
    v: &Vault,
    task_id: &str,
    session_id: &str,
    items: &[SubtaskItem],
) -> Result<()> {
    if session_id.trim().is_empty() {
        return err("the subtasks need a session");
    }
    let (t, state, row_session) = running(v, task_id)?;
    check_row_session(&row_session, session_id)?;
    if state.session != session_id {
        return err("the subtasks were begun by another session, or not begun");
    }
    let Some(page) = t.page.clone() else {
        return err("the task has no page");
    };
    let p = v.load_page(page.clone())?;
    let blocks = &p.blocks;
    let parent = state.parent.clone();
    let Some(pb) = find(blocks, &parent) else {
        return err("the block the subtasks nest under is gone");
    };

    // To-dos the user added that the worker was told about: the worker's list
    // now holds them too, so a new item with the same text takes the user's
    // to-do instead of adding a second one.
    let steered: Vec<String> = super::attempts::list(v, task_id)?
        .into_iter()
        .find(|a| a.id == t.attempt)
        .map(|a| a.steered)
        .unwrap_or_default();

    let wanted: Vec<(String, bool)> = items
        .iter()
        .filter(|i| !i.text.trim().is_empty())
        .map(|i| (i.text.clone(), i.status == DONE))
        .collect();

    // Matching: equal text first, in order, then the leftovers by position.
    let have = refresh(blocks, &state);
    let mut have_of: Vec<Option<usize>> = vec![None; have.len()];
    let mut want_of: Vec<Option<usize>> = vec![None; wanted.len()];
    for (w, (line, _)) in wanted.iter().enumerate() {
        let found = (0..have.len()).find(|&e| have_of[e].is_none() && have[e].text == *line);
        if let Some(e) = found {
            have_of[e] = Some(w);
            want_of[w] = Some(e);
        }
    }
    let free_w: Vec<usize> = (0..wanted.len())
        .filter(|&w| want_of[w].is_none())
        .collect();
    let free_e: Vec<usize> = (0..have.len()).filter(|&e| have_of[e].is_none()).collect();
    for (w, e) in free_w.into_iter().zip(free_e) {
        have_of[e] = Some(w);
        want_of[w] = Some(e);
    }

    let mut ops = Vec::new();

    // Items that left the list: their todos go (or, with children, stay).
    let mut gone: Vec<&Block> = Vec::new();
    for (e, item) in have.iter().enumerate() {
        if have_of[e].is_none() {
            if let Some(b) = item.block.as_deref().and_then(|b| find(blocks, b)) {
                gone.push(b);
            }
        }
    }
    for b in &gone {
        ops.extend(removal(b));
    }

    let mut adoptable: Vec<&Block> = pb
        .children
        .iter()
        .filter(|c| {
            steered.contains(&c.id) && !have.iter().any(|h| h.block.as_deref() == Some(&c.id))
        })
        .collect();
    let mut next = Vec::new();
    let mut desired: Vec<String> = Vec::new();
    let mut inserts: Vec<(String, String, bool)> = Vec::new();
    for (w, (line, done)) in wanted.iter().enumerate() {
        match want_of[w] {
            Some(e) => {
                let block = have[e].block.clone();
                if let Some(b) = block.as_deref().and_then(|b| find(blocks, b)) {
                    if b.text != *line {
                        ops.push(BlockOp::SetText {
                            id: b.id.clone(),
                            text: line.clone(),
                            marks: vec![],
                        });
                    }
                    if b.checked != *done {
                        ops.push(BlockOp::SetChecked {
                            id: b.id.clone(),
                            checked: *done,
                        });
                    }
                    desired.push(b.id.clone());
                }
                next.push(Item {
                    block,
                    text: line.clone(),
                });
            }
            None if adoptable.iter().any(|b| b.text.trim() == line.trim()) => {
                let at = adoptable
                    .iter()
                    .position(|b| b.text.trim() == line.trim())
                    .unwrap_or_default();
                let b = adoptable.remove(at);
                if b.checked != *done {
                    ops.push(BlockOp::SetChecked {
                        id: b.id.clone(),
                        checked: *done,
                    });
                }
                desired.push(b.id.clone());
                next.push(Item {
                    block: Some(b.id.clone()),
                    text: line.clone(),
                });
            }
            None => {
                let id = vault_core::new_id();
                desired.push(id.clone());
                inserts.push((id.clone(), line.clone(), *done));
                next.push(Item {
                    block: Some(id),
                    text: line.clone(),
                });
            }
        }
    }

    // The parent's children once the deleted todos are gone, then the new
    // todos at the end.
    let removed: Vec<&str> = gone
        .iter()
        .filter(|g| g.children.is_empty())
        .map(|g| g.id.as_str())
        .collect();
    let mut cur: Vec<String> = pb
        .children
        .iter()
        .filter(|c| !removed.contains(&c.id.as_str()))
        .map(|c| c.id.clone())
        .collect();
    for (id, line, done) in inserts {
        ops.push(BlockOp::Insert {
            parent: Some(parent.clone()),
            after: cur.last().cloned(),
            block: NewBlock {
                id: id.clone(),
                kind: BlockKind::Todo,
                text: line,
                marks: vec![],
                checked: done,
            },
        });
        cur.push(id);
    }

    ops.extend(reorder_ops(&parent, &cur, &desired)?);

    apply_plan(v, page, &p.blocks, ops)?;
    save(
        v,
        &t.id,
        &State {
            session: session_id.to_string(),
            parent,
            items: next,
        },
    )
}

/// The moves that put the owned todos `desired`, in list order, into the
/// owned slots of `cur`, the parent's children. An owned slot is a position
/// that holds an owned todo; the k-th one takes the k-th desired id, and every
/// other block keeps its position. Only owned ids are moved, each to just
/// after its predecessor in that target order, so no user block is operated
/// on. The moves are simulated, so an id already in place gets no move.
fn reorder_ops(parent: &str, cur: &[String], desired: &[String]) -> Result<Vec<BlockOp>> {
    let slots: Vec<usize> = cur
        .iter()
        .enumerate()
        .filter(|(_, id)| desired.contains(id))
        .map(|(i, _)| i)
        .collect();
    if slots.len() != desired.len() {
        return err("the subtasks changed while they were placed");
    }
    let mut target = cur.to_vec();
    for (slot, id) in slots.into_iter().zip(desired) {
        target[slot] = id.clone();
    }

    let mut sim = cur.to_vec();
    let mut ops = Vec::new();
    for j in 0..target.len() {
        if !desired.contains(&target[j]) {
            continue;
        }
        let Some(from) = sim.iter().position(|id| *id == target[j]) else {
            return err("the subtasks changed while they were placed");
        };
        let placed = if j == 0 {
            from == 0
        } else {
            from > 0 && sim[from - 1] == target[j - 1]
        };
        if placed {
            continue;
        }
        let id = sim.remove(from);
        let at = if j == 0 {
            0
        } else {
            match sim.iter().position(|x| *x == target[j - 1]) {
                Some(p) => p + 1,
                None => return err("the subtasks changed while they were placed"),
            }
        };
        sim.insert(at, id.clone());
        ops.push(BlockOp::Move {
            id,
            parent: Some(parent.to_string()),
            after: (j > 0).then(|| target[j - 1].clone()),
        });
    }
    if sim != target {
        return err("the subtasks changed while they were placed");
    }
    Ok(ops)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Applies the moves to `cur` the way the core does, to check the result.
    fn apply_moves(cur: &[String], ops: &[BlockOp]) -> Vec<String> {
        let mut out = cur.to_vec();
        for op in ops {
            let BlockOp::Move { id, after, .. } = op else {
                panic!("only moves: {op:?}");
            };
            out.retain(|x| x != id);
            let at = match after {
                None => 0,
                Some(a) => out.iter().position(|x| x == a).unwrap() + 1,
            };
            out.insert(at, id.clone());
        }
        out
    }

    #[test]
    fn owned_moves_leave_the_users_block_in_place() {
        // A, user, B becomes B, user, A: only A and B move.
        let cur: Vec<String> = ["a", "u", "b"].map(String::from).to_vec();
        let desired: Vec<String> = ["b", "a"].map(String::from).to_vec();
        let ops = reorder_ops("p", &cur, &desired).unwrap();
        assert!(ops.iter().all(|op| match op {
            BlockOp::Move { id, .. } => id != "u",
            _ => false,
        }));
        assert_eq!(apply_moves(&cur, &ops), ["b", "u", "a"]);
    }

    #[test]
    fn an_order_already_in_place_gets_no_moves() {
        let cur: Vec<String> = ["a", "u", "b"].map(String::from).to_vec();
        let desired: Vec<String> = ["a", "b"].map(String::from).to_vec();
        assert!(reorder_ops("p", &cur, &desired).unwrap().is_empty());
    }
}
