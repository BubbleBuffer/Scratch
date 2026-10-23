//! Agent-owned durable presentation records. Undo remains a UI API, never a tool.
use anyhow::{ensure, Context, Result};
use bubblepaw_core::{Store, ToolContext};
use notes_agent_wire::{AgentEdit, DiffHunk, DiffLine, ToolActivity};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use similar::{ChangeTag, TextDiff};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use vault_core::{Block, Page, Vault};

#[derive(serde::Serialize, serde::Deserialize)]
struct Snapshot {
    page: Page,
    markdown: String,
    blocks: BTreeMap<String, String>,
}
impl Snapshot {
    fn capture(vault: &Vault, id: &str) -> Result<Self> {
        let page = vault.load_page(id.into())?;
        let markdown = vault.page_markdown(id.into())?;
        fn collect(blocks: &[Block], out: &mut BTreeMap<String, String>) -> Result<()> {
            for block in blocks {
                let mut own = block.clone();
                own.children.clear();
                out.insert(block.id.clone(), serde_json::to_string(&own)?);
                collect(&block.children, out)?;
            }
            Ok(())
        }
        let mut blocks = BTreeMap::new();
        collect(&page.blocks, &mut blocks)?;
        Ok(Self {
            page,
            markdown,
            blocks,
        })
    }
    fn guarded_state(&self) -> Result<Value> {
        let mut page = self.page.clone();
        page.created_ms = 0;
        page.updated_ms = 0;
        Ok(serde_json::to_value(page)?)
    }
}

/// Where each page of a `move_page` call sat, before the call or after it.
/// Stored in `before_json` and `after_json` in place of a `Snapshot`; a
/// snapshot has no `moves`, so the two shapes tell records apart.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct MoveSnapshot {
    moves: Vec<Placement>,
}
/// A page's parent, the sibling just before it (`None`: first) and its
/// position among its siblings, as `Vault::list_children` lists them.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Placement {
    page: String,
    parent: Option<String>,
    previous: Option<String>,
    index: Option<u32>,
}

pub struct Records {
    db: Mutex<Connection>,
    store: Store,
    // Gateway sessions bind their authenticated author before executing cells.
    // Copy it into each synchronous mutation worker; never hold thread-local
    // attribution across an async suspension.
    actors: Mutex<BTreeMap<String, SessionActor>>,
    // Hold across each agent vault action, including its before/after capture.
    pub(crate) mutation: Mutex<()>,
}

struct SessionActor {
    caller: String,
    label: String,
}

impl Records {
    pub fn open(path: &Path, store: Store) -> Result<Self> {
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(10))?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS agent_edits (id TEXT PRIMARY KEY,session_id TEXT NOT NULL,run_id TEXT NOT NULL,page_id TEXT NOT NULL,record TEXT NOT NULL,before_json TEXT,after_json TEXT NOT NULL); CREATE INDEX IF NOT EXISTS agent_edits_session ON agent_edits(session_id); CREATE TABLE IF NOT EXISTS agent_activity (seq INTEGER PRIMARY KEY AUTOINCREMENT,session_id TEXT NOT NULL,record TEXT NOT NULL);")?;
        Ok(Self {
            db: Mutex::new(db),
            store,
            actors: Mutex::new(BTreeMap::new()),
            mutation: Mutex::new(()),
        })
    }
    pub(crate) fn bind_session_actor(
        &self,
        session: &str,
        caller: &str,
        actor: String,
    ) -> Result<()> {
        let mut actors = self.actors.lock().unwrap();
        if let Some(existing) = actors.get_mut(session) {
            ensure!(existing.caller == caller, "gateway session author changed");
            existing.label = actor;
        } else {
            actors.insert(
                session.to_owned(),
                SessionActor {
                    caller: caller.to_owned(),
                    label: actor,
                },
            );
        }
        Ok(())
    }
    pub(crate) fn session_actor(&self, session: &str) -> Option<String> {
        self.actors
            .lock()
            .unwrap()
            .get(session)
            .map(|actor| actor.label.clone())
    }
    pub fn activity_start(
        &self,
        context: &ToolContext,
        name: &str,
        input: &Value,
        vault: &Vault,
    ) -> Result<(i64, String)> {
        // BubblePaw now places effects in each canonical chat database. The
        // notes-owned presentation tables remain in their existing database.
        let original: Option<String> = if context.invocation_id.is_empty() {
            None
        } else {
            let path = self
                .store
                .chat_path(&context.session_id)?
                .context("agent activity needs persistent chat storage")?;
            let chat = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            chat.busy_timeout(std::time::Duration::from_secs(10))?;
            chat.query_row(
                "SELECT call_id FROM effects WHERE id=?1 AND session_id=?2 AND run_id=?3",
                params![context.invocation_id, context.session_id, context.run_id],
                |row| row.get(0),
            )
            .optional()?
        };
        let call_id = original.unwrap_or_else(|| {
            if context.invocation_id.is_empty() {
                uuid::Uuid::new_v4().to_string()
            } else {
                context.invocation_id.clone()
            }
        });
        let (tool, args) = as_new_tool(name, input);
        let record = ToolActivity {
            run_id: context.run_id.clone(),
            call_id: call_id.clone(),
            order: 0,
            name: name.into(),
            summary: summary(vault, &tool, &args),
            is_error: false,
            edit_id: None,
        };
        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT INTO agent_activity(session_id,record) VALUES (?1,?2)",
            params![context.session_id, serde_json::to_string(&record)?],
        )?;
        Ok((db.last_insert_rowid(), call_id))
    }
    pub fn activity_finish(&self, seq: i64, is_error: bool, edit_id: Option<String>) -> Result<()> {
        let db = self.db.lock().unwrap();
        let json: String = db.query_row(
            "SELECT record FROM agent_activity WHERE seq=?1",
            [seq],
            |row| row.get(0),
        )?;
        let mut record: ToolActivity = serde_json::from_str(&json)?;
        record.is_error = is_error;
        record.edit_id = edit_id;
        record.order = seq;
        db.execute(
            "UPDATE agent_activity SET record=?2 WHERE seq=?1",
            params![seq, serde_json::to_string(&record)?],
        )?;
        Ok(())
    }
    pub fn activity(&self, session: &str) -> Result<Vec<ToolActivity>> {
        let db = self.db.lock().unwrap();
        let mut query =
            db.prepare("SELECT seq,record FROM agent_activity WHERE session_id=?1 ORDER BY seq")?;
        let rows = query.query_map([session], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (seq, json) = row?;
            let mut record: ToolActivity = serde_json::from_str(&json)?;
            record.order = seq;
            Ok(record)
        })
        .collect()
    }
    pub fn edits(&self, session: &str) -> Result<Vec<AgentEdit>> {
        let db = self.db.lock().unwrap();
        let mut query =
            db.prepare("SELECT record FROM agent_edits WHERE session_id=?1 ORDER BY rowid")?;
        let rows = query.query_map([session], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }
    pub fn forget_session(&self, session: &str) -> Result<()> {
        self.actors.lock().unwrap().remove(session);
        let db = self.db.lock().unwrap();
        db.execute("DELETE FROM agent_edits WHERE session_id=?1", [session])?;
        db.execute("DELETE FROM agent_activity WHERE session_id=?1", [session])?;
        Ok(())
    }
    pub(crate) fn before(
        &self,
        vault: &Vault,
        name: &str,
        input: &Value,
    ) -> Result<Option<String>> {
        let (name, input) = as_new_tool(name, input);
        let (name, input) = (name.as_str(), &input);
        if name == "move_page" {
            let moves = move_names(input)
                .into_iter()
                .map(|page| -> Result<Placement> {
                    let id = vault_tools::resolve_page(vault, page)?;
                    placement(vault, &id)
                })
                .collect::<Result<_>>()?;
            return Ok(Some(serde_json::to_string(&MoveSnapshot { moves })?));
        }
        let target = if name == "daily_note" {
            vault_tools::resolve_daily_note(vault, input)?
        } else if name == "update_task" {
            input
                .get("item")
                .and_then(Value::as_str)
                .map(|item| vault_tools::resolve_agenda_item(vault, item).map(|target| target.page))
                .transpose()?
        } else {
            input
                .get(if name == "upsert_row" { "row" } else { "page" })
                .and_then(Value::as_str)
                .map(|page| vault_tools::resolve_page(vault, page))
                .transpose()?
        };
        target
            .map(|id| Ok(serde_json::to_string(&Snapshot::capture(vault, &id)?)?))
            .transpose()
    }
    pub(crate) fn record(
        &self,
        vault: &Vault,
        context: &ToolContext,
        call_id: &str,
        name: &str,
        before: Option<String>,
        result: &str,
    ) -> Result<Option<String>> {
        let name = vault_tools::canonical(name);
        if name == "move_page" {
            return self.record_move(vault, context, call_id, before);
        }
        let old: Option<Snapshot> = before
            .as_ref()
            .map(|json| serde_json::from_str(json))
            .transpose()?;
        let id = old
            .as_ref()
            .map(|snapshot| snapshot.page.id.clone())
            .or_else(|| {
                serde_json::from_str::<Value>(result)
                    .ok()
                    .and_then(|value| value["id"].as_str().map(str::to_owned))
            })
            // Page titles may themselves contain backticks. The result's
            // final code span is the newly created page's id.
            .or_else(|| result.rsplit('`').nth(1).map(str::to_owned));
        let Some(id) = id else { return Ok(None) };
        let after = Snapshot::capture(vault, &id)?;
        if let Some(old) = &old {
            if old.guarded_state()? == after.guarded_state()? {
                return Ok(None);
            }
        }
        // daily_note returning an existing page is not an edit.
        if name == "daily_note"
            && serde_json::from_str::<Value>(result)
                .ok()
                .and_then(|value| value["created"].as_bool())
                == Some(false)
        {
            return Ok(None);
        }
        let db = self.db.lock().unwrap();
        let prior:Option<(String,Option<String>)>=db.query_row("SELECT record,before_json FROM agent_edits WHERE session_id=?1 AND run_id=?2 AND page_id=?3 ORDER BY rowid DESC LIMIT 1",params![context.session_id,context.run_id,id],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
        let (mut edit, first_before) = match prior {
            // A move is its own edit: later changes to the page do not merge into it.
            Some((json, first)) if !is_move(first.as_deref()) => {
                let record: AgentEdit = serde_json::from_str(&json)?;
                if record.undone {
                    (
                        new_edit(context, call_id, &after, name, old.is_none()),
                        before,
                    )
                } else {
                    (record, first)
                }
            }
            _ => (
                new_edit(context, call_id, &after, name, old.is_none()),
                before,
            ),
        };
        if name == "trash_page" && edit.kind != "created" {
            edit.kind = "trashed".into()
        }
        let first: Option<Snapshot> = first_before
            .as_ref()
            .map(|json| serde_json::from_str(json))
            .transpose()?;
        let before_md = first.as_ref().map(|s| s.markdown.as_str()).unwrap_or("");
        let (added, removed, hunks) = diff(before_md, &after.markdown);
        edit.added = added;
        edit.removed = removed;
        edit.hunks = hunks;
        edit.page_title = after.page.title.clone();
        edit.changed_blocks = after
            .blocks
            .iter()
            .filter(|(id, text)| first.as_ref().and_then(|s| s.blocks.get(*id)) != Some(*text))
            .map(|(id, _)| id.clone())
            .collect();
        db.execute("INSERT INTO agent_edits(id,session_id,run_id,page_id,record,before_json,after_json) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET record=excluded.record,after_json=excluded.after_json",params![edit.id,context.session_id,context.run_id,id,serde_json::to_string(&edit)?,first_before,serde_json::to_string(&after)?])?;
        Ok(Some(edit.id))
    }
    /// A move_page call is one edit for every page it moved. Its `before` is
    /// where each page sat before the call. Nothing is recorded when no page's
    /// place changed (for example a call that failed before moving).
    fn record_move(
        &self,
        vault: &Vault,
        context: &ToolContext,
        call_id: &str,
        before: Option<String>,
    ) -> Result<Option<String>> {
        let Some(before) = before else {
            return Ok(None);
        };
        let old: MoveSnapshot = serde_json::from_str(&before)?;
        let after = MoveSnapshot {
            moves: old
                .moves
                .iter()
                .map(|placed| placement(vault, &placed.page))
                .collect::<Result<_>>()?,
        };
        let Some(first) = after.moves.first() else {
            return Ok(None);
        };
        if after == old {
            return Ok(None);
        }
        // Kind stays "edited": the app labels unknown kinds as edits of the
        // page, and a move has no text to diff.
        let edit = AgentEdit {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: context.session_id.clone(),
            run_id: context.run_id.clone(),
            call_id: call_id.into(),
            at: chrono::Utc::now().to_rfc3339(),
            page_id: first.page.clone(),
            page_title: vault.load_page(first.page.clone())?.title,
            kind: "edited".into(),
            added: 0,
            removed: 0,
            hunks: vec![],
            changed_blocks: vec![],
            undone: false,
            undoable: true,
        };
        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT INTO agent_edits(id,session_id,run_id,page_id,record,before_json,after_json) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                edit.id,
                context.session_id,
                context.run_id,
                edit.page_id,
                serde_json::to_string(&edit)?,
                before,
                serde_json::to_string(&after)?
            ],
        )?;
        Ok(Some(edit.id))
    }
    pub fn undo(&self, vault: &Vault, id: &str) -> Result<AgentEdit> {
        let _mutation = self.mutation.lock().unwrap();
        let db = self.db.lock().unwrap();
        let (json, before, after): (String, Option<String>, String) = db.query_row(
            "SELECT record,before_json,after_json FROM agent_edits WHERE id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let mut edit: AgentEdit = serde_json::from_str(&json)?;
        ensure!(!edit.undone, "edit already undone");
        ensure!(edit.undoable, "edit cannot be undone; use page history");
        match serde_json::from_str::<MoveSnapshot>(&after) {
            Ok(after) => undo_moves(vault, before.as_deref(), &after)?,
            Err(_) => undo_snapshot(vault, &edit, before.as_deref(), &after)?,
        }
        edit.undone = true;
        db.execute(
            "UPDATE agent_edits SET record=?2 WHERE id=?1",
            params![id, serde_json::to_string(&edit)?],
        )?;
        Ok(edit)
    }
}
fn is_move(json: Option<&str>) -> bool {
    json.is_some_and(|json| serde_json::from_str::<MoveSnapshot>(json).is_ok())
}
fn undo_snapshot(vault: &Vault, edit: &AgentEdit, before: Option<&str>, after: &str) -> Result<()> {
    let after: Snapshot = serde_json::from_str(after)?;
    let current = Snapshot::capture(vault, &edit.page_id)?;
    ensure!(
        current.markdown == after.markdown && current.guarded_state()? == after.guarded_state()?,
        "page changed since the agent's edit; use page history"
    );
    match edit.kind.as_str() {
        "created" => vault.delete_page(edit.page_id.clone())?,
        _ => {
            if edit.kind == "trashed" {
                vault.restore_page(edit.page_id.clone())?;
            }
            let before: Snapshot = serde_json::from_str(
                before.ok_or_else(|| anyhow::anyhow!("missing undo snapshot"))?,
            )?;
            vault.replace_markdown(edit.page_id.clone(), vec![], before.markdown)?;
            if current.page.title != before.page.title {
                vault.rename_page(edit.page_id.clone(), before.page.title)?;
            }
            if current.page.icon != before.page.icon {
                vault.set_icon(edit.page_id.clone(), before.page.icon)?;
            }
            for property in &current.page.properties {
                if !before.page.properties.iter().any(|p| p.key == property.key) {
                    vault.remove_property(edit.page_id.clone(), property.key.clone())?;
                }
            }
            for property in before.page.properties {
                vault.set_property(edit.page_id.clone(), property.key, property.value)?;
            }
        }
    }
    Ok(())
}
/// Puts each page a move_page call moved back under its old parent, after its
/// old sibling. Every conflict is checked before anything moves. Pages return
/// in each old parent's original order, so a sibling that was moved too is
/// back before it is needed.
fn undo_moves(vault: &Vault, before: Option<&str>, after: &MoveSnapshot) -> Result<()> {
    let before: MoveSnapshot =
        serde_json::from_str(before.ok_or_else(|| anyhow::anyhow!("missing undo snapshot"))?)?;
    for placed in &after.moves {
        let unchanged = vault
            .load_page(placed.page.clone())
            .is_ok_and(|page| !page.in_trash && page.parent == placed.parent);
        ensure!(
            unchanged,
            "page changed since the agent's edit; use page history"
        );
    }
    let mut plan: Vec<&Placement> = before.moves.iter().collect();
    plan.sort_by_key(|placed| (placed.parent.clone(), placed.index));
    for placed in &plan {
        if let Some(parent) = &placed.parent {
            let usable = vault
                .load_page(parent.clone())
                .is_ok_and(|page| !page.in_trash);
            ensure!(
                usable,
                "the page's former parent is gone or in the trash; use page history"
            );
        }
    }
    for placed in plan {
        // An old sibling that is gone or has moved away is not used: the page goes first.
        let previous = placed.previous.clone().filter(|sibling| {
            vault
                .load_page(sibling.clone())
                .is_ok_and(|page| !page.in_trash && page.parent == placed.parent)
        });
        vault.move_page(placed.page.clone(), placed.parent.clone(), previous)?;
    }
    Ok(())
}
fn new_edit(
    context: &ToolContext,
    call_id: &str,
    after: &Snapshot,
    name: &str,
    created: bool,
) -> AgentEdit {
    AgentEdit {
        id: uuid::Uuid::new_v4().to_string(),
        session_id: context.session_id.clone(),
        run_id: context.run_id.clone(),
        call_id: call_id.into(),
        at: chrono::Utc::now().to_rfc3339(),
        page_id: after.page.id.clone(),
        page_title: after.page.title.clone(),
        kind: if created {
            "created"
        } else if name == "upsert_row" {
            "row"
        } else if name == "trash_page" {
            "trashed"
        } else {
            "edited"
        }
        .into(),
        added: 0,
        removed: 0,
        hunks: vec![],
        changed_blocks: vec![],
        undone: false,
        undoable: true,
    }
}
fn diff(before: &str, after: &str) -> (i64, i64, Vec<DiffHunk>) {
    let diff = TextDiff::from_lines(before, after);
    let mut added = 0;
    let mut removed = 0;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => added += 1,
            ChangeTag::Delete => removed += 1,
            _ => {}
        }
    }
    let hunks = diff
        .grouped_ops(3)
        .into_iter()
        .map(|ops| {
            let old_start = ops
                .first()
                .map(|op| op.old_range().start as i64 + 1)
                .unwrap_or(0);
            let new_start = ops
                .first()
                .map(|op| op.new_range().start as i64 + 1)
                .unwrap_or(0);
            let lines = ops
                .iter()
                .flat_map(|op| diff.iter_changes(op))
                .map(|change| DiffLine {
                    tag: match change.tag() {
                        ChangeTag::Insert => "+",
                        ChangeTag::Delete => "-",
                        ChangeTag::Equal => "=",
                    }
                    .into(),
                    text: change.value().trim_end_matches('\n').into(),
                })
                .collect();
            DiffHunk {
                old_start,
                new_start,
                lines,
            }
        })
        .collect();
    (added, removed, hunks)
}
fn summary(vault: &Vault, name: &str, input: &Value) -> String {
    let value = |key: &str| input[key].as_str().unwrap_or("");
    let page = || {
        let raw = if name == "upsert_row" {
            value("row")
        } else {
            value("page")
        };
        vault_tools::resolve_page(vault, raw)
            .ok()
            .and_then(|id| vault.load_page(id).ok())
            .map(|page| page.title)
            .unwrap_or_else(|| raw.into())
    };
    match name {
        "search_notes" => format!("Searched notes “{}”", value("query")),
        "search" => format!("Searched files “{}”", value("query")),
        "read" => format!("Read {}", value("path")),
        "edit" | "write" => format!("Edited {}", value("path")),
        "list" => format!("Listed {}", value("path")),
        "read_page" => format!("Read {}", page()),
        "create_page" => format!("Created {}", value("title")),
        "edit_page" if value("op") == "append" => format!("Added blocks to {}", page()),
        "edit_page" => format!("Edited {}", page()),
        "trash_page" => format!("Trashed {}", page()),
        "move_page" => match move_names(input).as_slice() {
            [only] => format!(
                "Moved {}",
                vault_tools::resolve_page(vault, only)
                    .ok()
                    .and_then(|id| vault.load_page(id).ok())
                    .map(|page| page.title)
                    .unwrap_or_else(|| (*only).into())
            ),
            names => format!("Moved {} pages", names.len()),
        },
        "upsert_row" => format!(
            "Updated {}",
            if value("row").is_empty() {
                value("title").into()
            } else {
                page()
            }
        ),
        "exec" => format!(
            "Ran `{}`",
            if value("command").is_empty() {
                value("cmd")
            } else {
                value("command")
            }
        ),
        "ui_open_page" => format!("Opened {}", page()),
        "ui_highlight" => format!("Highlighted {}", page()),
        "update_task" => {
            let set = |key: &str| input.get(key).is_some_and(|v| !v.is_null());
            match (set("due") || set("date"), set("done")) {
                (true, false) => "Rescheduled a task".into(),
                (false, true) => "Changed task completion".into(),
                _ => "Updated a task".into(),
            }
        }
        "daily_note" => "Opened the daily note".into(),
        _ => name.into(),
    }
}

/// The page names a move_page call's `pages` argument lists: one name or a list.
fn move_names(input: &Value) -> Vec<&str> {
    match input.get("pages") {
        Some(Value::String(name)) => vec![name.as_str()],
        Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    }
}

/// A page's place as `Vault::move_page` sees it.
fn placement(vault: &Vault, page: &str) -> Result<Placement> {
    let parent = vault.load_page(page.into())?.parent;
    let siblings = vault.list_children(parent.clone(), 0, u32::MAX)?;
    let index = siblings.iter().position(|sibling| sibling.id == page);
    let previous = index
        .and_then(|i| i.checked_sub(1))
        .map(|i| siblings[i].id.clone());
    Ok(Placement {
        page: page.into(),
        parent,
        previous,
        index: index.map(|i| i as u32),
    })
}

/// The tool a call runs and its arguments as that tool reads them: an old
/// name becomes its new tool with the rewritten arguments (see
/// `vault_tools::alias`), any other name is returned as it is.
fn as_new_tool(name: &str, input: &Value) -> (String, Value) {
    match vault_tools::alias(name) {
        Some(alias) => (
            alias.target.into(),
            alias.rewrite(input).unwrap_or_else(|_| input.clone()),
        ),
        None => (name.into(), input.clone()),
    }
}
