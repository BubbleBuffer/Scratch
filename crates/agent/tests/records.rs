use notes_agent_wire::{ContextPage, MessageContext};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use vault_agent::{Agent, AgentConfig, Forwarded, Sent, TestHooks};
use vault_core::Vault;

struct Fixture {
    _dir: tempfile::TempDir,
    vault: Arc<Vault>,
    agent: Agent,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let vault =
            Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into_owned()).unwrap());
        let agent = Agent::open(
            vault.clone(),
            AgentConfig {
                data_dir: dir.path().join("data"),
                default_model: "scripted/scripted".into(),
                test: TestHooks {
                    scripted: Some((vec![], "done".into())),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        Self {
            _dir: dir,
            vault,
            agent,
        }
    }
    fn page(&self, title: &str, body: &str) -> String {
        let id = self.vault.create_page(None, title.into()).unwrap();
        self.vault
            .replace_markdown(id.clone(), vec![], body.into())
            .unwrap();
        id
    }
    fn script(&self, session: &str, code: &str) {
        let run = self.agent.run_script(session, "test", code).unwrap();
        assert_eq!(run.status, "completed", "{run:?}");
    }
}

#[test]
fn nested_activity_diff_same_run_merge_and_guarded_undo() {
    let f = Fixture::new();
    let id = f.page("Garden plan", "one\n\ntwo\n\nstay");
    let session = f.agent.create_session("Edits").unwrap();
    let page = serde_json::to_string(&id).unwrap();
    f.script(&session.id,&format!("await tools.search_notes({{query:'garden'}}); await tools.read_page({{page:{page}}}); await tools.edit_page({{page:{page},op:'set_body',markdown:'first\\n\\ntwo\\n\\nstay'}}); await tools.edit_page({{page:{page},op:'set_body',markdown:'first\\n\\nsecond\\n\\nstay'}});"));
    let state = f.agent.session_state(&session.id).unwrap();
    assert_eq!(state.edits.len(), 1);
    let edit = &state.edits[0];
    assert_eq!((edit.added, edit.removed), (2, 2));
    assert!(!edit.changed_blocks.is_empty());
    assert!(edit
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .any(|line| line.tag == "+" && line.text == "second"));
    assert_eq!(
        state
            .activity
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "search_notes",
            "read_page",
            "edit_page",
            "edit_page"
        ]
    );
    assert_eq!(state.activity[0].summary, "Searched notes “garden”");
    assert_eq!(state.activity[1].summary, "Read Garden plan");
    assert_eq!(state.activity[2].edit_id.as_deref(), Some(edit.id.as_str()));
    assert_eq!(state.activity[3].edit_id.as_deref(), Some(edit.id.as_str()));
    let dto = vault_agent::wire::session_state(state.clone());
    let json = serde_json::to_string(&dto).unwrap();
    assert!(!json.contains("before_json"));
    let undone = f.agent.edit_undo(&edit.id).unwrap();
    assert!(undone.undone);
    assert_eq!(
        f.vault.page_markdown(id.clone()).unwrap(),
        "one\n\ntwo\n\nstay"
    );
    assert!(f.agent.edit_undo(&edit.id).is_err());
    f.script(
        &session.id,
        &format!("await tools.edit_page({{page:{page},op:'set_body',markdown:'agent edit'}});"),
    );
    let edit = f
        .agent
        .session_state(&session.id)
        .unwrap()
        .edits
        .last()
        .unwrap()
        .clone();
    f.vault
        .replace_markdown(id.clone(), vec![], "user edit".into())
        .unwrap();
    assert_eq!(
        f.agent.edit_undo(&edit.id).unwrap_err().to_string(),
        "page changed since the agent's edit; use page history"
    );
    assert_eq!(f.vault.page_markdown(id).unwrap(), "user edit");
}

#[test]
fn created_page_undo_trashes_and_metadata_edits_guard_later_changes() {
    let f = Fixture::new();
    let session = f.agent.create_session("Create").unwrap();
    f.script(
        &session.id,
        "await tools.create_page({title:'New page',markdown:'hello'});",
    );
    let edit = f.agent.session_state(&session.id).unwrap().edits[0].clone();
    assert_eq!(edit.kind, "created");
    f.agent.edit_undo(&edit.id).unwrap();
    assert!(f.vault.load_page(edit.page_id).unwrap().in_trash);
    let id = f.page("Original", "body");
    let page = serde_json::to_string(&id).unwrap();
    f.script(
        &session.id,
        &format!("await tools.edit_page({{page:{page},op:'set_props',title:'Renamed',icon:'🌱'}});"),
    );
    let edit = f
        .agent
        .session_state(&session.id)
        .unwrap()
        .edits
        .last()
        .unwrap()
        .clone();
    f.agent.edit_undo(&edit.id).unwrap();
    let restored = f.vault.load_page(id.clone()).unwrap();
    assert_eq!(restored.title, "Original");
    assert_eq!(restored.icon, None);
    f.script(
        &session.id,
        &format!("await tools.edit_page({{page:{page},op:'set_props',title:'Agent title'}});"),
    );
    let edit = f
        .agent
        .session_state(&session.id)
        .unwrap()
        .edits
        .last()
        .unwrap()
        .clone();
    f.vault.rename_page(id, "User title".into()).unwrap();
    assert!(f
        .agent
        .edit_undo(&edit.id)
        .unwrap_err()
        .to_string()
        .contains("page changed since"));
}

#[test]
fn scoped_ui_actions_notify_and_cancelled_viewers_do_not_open_pages() {
    let f = Fixture::new();
    let id = f.page("Garden", "body");
    let block = f.vault.load_page(id.clone()).unwrap().blocks[0].id.clone();
    let session = f.agent.create_session("Showing").unwrap();
    let (tx, rx) = mpsc::channel();
    let token = f
        .agent
        .forward_events_for_session(session.id.clone(), move |event| tx.send(event).is_ok())
        .unwrap();
    f.script(&session.id,&format!("const opened=await tools.ui_open_page({{page:'Garden',block:{}}}); const highlighted=await tools.ui_highlight({{page:'Garden',blocks:[{}]}});text(opened);text(highlighted);",serde_json::to_string(&block).unwrap(),serde_json::to_string(&block).unwrap()));
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut actions = vec![];
    while actions.len() < 2 {
        if let Forwarded::UiAction(action) = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap()
        {
            actions.push(action);
        }
    }
    assert_eq!(actions[0].action, "open_page");
    assert_eq!(actions[0].page_id, id);
    assert_eq!(actions[1].action, "highlight");
    assert_eq!(actions[1].block_ids, vec![block]);
    token.cancel();
    let run = f
        .agent
        .run_script(
            &session.id,
            "unviewed",
            "return await tools.ui_open_page({page:'Garden'});",
        )
        .unwrap();
    assert!(run
        .text
        .unwrap()
        .contains("no app is showing this chat; nothing opened"));
    let other = f.agent.create_session("Other").unwrap();
    let (tx, _rx) = mpsc::channel();
    let _global = f.agent.forward_events(move |event| tx.send(event).is_ok());
    let run = f
        .agent
        .run_script(
            &other.id,
            "global only",
            "return await tools.ui_open_page({page:'Garden'});",
        )
        .unwrap();
    assert!(run.text.unwrap().contains("no app is showing this chat"));
}

#[test]
fn send_context_is_model_visible_and_transcript_text_is_clean() {
    let f = Fixture::new();
    let session = f.agent.create_session("Context").unwrap();
    let (tx, rx) = mpsc::channel();
    f.agent.forward_events(move |event| tx.send(event).is_ok());
    let context = MessageContext {
        pages: vec![ContextPage {
            id: "p".into(),
            title: "Garden </notes-context>".into(),
        }],
        chats: vec![],
        selection: None,
    };
    assert!(matches!(
        f.agent
            .send_with_context(&session.id, "What next?", None, Some(context.clone()))
            .unwrap(),
        Sent::Started(_)
    ));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Forwarded::Event(event) = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap()
        {
            if matches!(
                event.kind,
                vault_agent::bubblepaw_sdk::events::EventKind::RunFinished { .. }
            ) {
                break;
            }
        }
    }
    let state = vault_agent::wire::session_state(f.agent.session_state(&session.id).unwrap());
    let user = state
        .messages
        .iter()
        .find(|message| message.role == "user")
        .unwrap();
    assert!(user.content.starts_with("<notes-context>"));
    assert_eq!(user.text, "What next?");
    assert_eq!(user.context, Some(context));
    let info = f.agent.host_info();
    for tool in [
        "search_notes",
        "update_task",
        "edit_page",
        "move_page",
        "notes_help",
        "daily_note",
        "ui_open_page",
        "ui_highlight",
    ] {
        assert!(info.tools.iter().any(|name| name == tool), "missing {tool}");
    }
    // Old names (hidden aliases included) are not registered in the in-app agent.
    for old in ["tasks", "overdue", "set_done", "reschedule", "replace_blocks", "update_page", "append_blocks", "list_pages", "agenda", "query_database", "memory_search"] {
        assert!(!info.tools.iter().any(|name| name == old), "old name {old} registered");
    }
    assert_eq!(
        info.permission_mode,
        notes_agent_wire::PermissionMode::Bypass
    );
}

#[test]
fn partial_metadata_failure_is_recorded_and_restored() {
    let f = Fixture::new();
    let id = f.page("Before", "body");
    let session = f.agent.create_session("Partial").unwrap();
    let page = serde_json::to_string(&id).unwrap();
    f.script(&session.id,&format!("try{{return await tools.edit_page({{page:{page},op:'set_props',title:'Partial rename',properties:{{bad:{{unsupported:true}}}}}});}}catch(error){{return String(error);}}"));
    let state = f.agent.session_state(&session.id).unwrap();
    assert_eq!(state.edits.len(), 1);
    assert!(state.activity[0].is_error);
    assert_eq!(
        state.activity[0].edit_id.as_deref(),
        Some(state.edits[0].id.as_str())
    );
    assert_eq!(
        f.vault.load_page(id.clone()).unwrap().title,
        "Partial rename"
    );
    f.agent.edit_undo(&state.edits[0].id).unwrap();
    assert_eq!(f.vault.load_page(id).unwrap().title, "Before");
}

#[test]
fn presentation_records_survive_agent_restart_and_deleted_chats_forget_them() {
    let f = Fixture::new();
    let id = f.page("Durable", "old");
    let session = f.agent.create_session("Persistent").unwrap();
    let page = serde_json::to_string(&id).unwrap();
    f.script(
        &session.id,
        &format!("await tools.edit_page({{page:{page},op:'set_body',markdown:'new'}});"),
    );
    let before = f.agent.session_state(&session.id).unwrap();
    let data = f._dir.path().join("data");
    drop(f.agent);
    let reopened = Agent::open(
        f.vault.clone(),
        AgentConfig {
            data_dir: data,
            ..Default::default()
        },
    )
    .unwrap();
    let state = reopened.session_state(&session.id).unwrap();
    assert_eq!(state.activity, before.activity);
    assert_eq!(state.edits, before.edits);
    reopened.edit_undo(&state.edits[0].id).unwrap();
    assert_eq!(f.vault.page_markdown(id).unwrap(), "old");
    reopened.delete_session(&session.id).unwrap();
    assert!(reopened.session_state(&session.id).is_err());
}

#[test]
fn trashed_and_created_then_edited_same_run_undo_restore_expected_state() {
    let f = Fixture::new();
    let id = f.page("Trashable", "old");
    let session = f.agent.create_session("Trash").unwrap();
    let page = serde_json::to_string(&id).unwrap();
    f.script(&session.id,&format!("await tools.edit_page({{page:{page},op:'set_body',markdown:'changed'}});await tools.trash_page({{page:{page}}});"));
    let edit = f.agent.session_state(&session.id).unwrap().edits[0].clone();
    assert_eq!(edit.kind, "trashed");
    f.agent.edit_undo(&edit.id).unwrap();
    assert!(!f.vault.load_page(id.clone()).unwrap().in_trash);
    assert_eq!(f.vault.page_markdown(id).unwrap(), "old");
    f.script(&session.id,"await tools.create_page({title:'Created and edited',markdown:'first'});await tools.edit_page({page:'Created and edited',op:'set_body',markdown:'second'});");
    let state = f.agent.session_state(&session.id).unwrap();
    let edit = state.edits.last().unwrap();
    assert_eq!(edit.kind, "created");
    f.agent.edit_undo(&edit.id).unwrap();
    assert!(f.vault.load_page(edit.page_id.clone()).unwrap().in_trash);
}

#[test]
fn titles_with_backticks_keep_created_page_and_row_records_undoable() {
    let f = Fixture::new();
    let session = f.agent.create_session("Backticks").unwrap();
    f.script(
        &session.id,
        "await tools.create_page({title:'Use `git`',markdown:'created'});",
    );
    let edit = f.agent.session_state(&session.id).unwrap().edits[0].clone();
    assert_eq!(edit.page_title, "Use `git`");
    f.agent.edit_undo(&edit.id).unwrap();
    assert!(f.vault.load_page(edit.page_id).unwrap().in_trash);
    let database = f.vault.create_page(None, "Rows".into()).unwrap();
    f.vault.make_collection(database.clone()).unwrap();
    let database = serde_json::to_string(&database).unwrap();
    f.script(&session.id,&format!("await tools.upsert_row({{database:{database},title:'Run `cargo test`',markdown:'row'}});"));
    let edit = f
        .agent
        .session_state(&session.id)
        .unwrap()
        .edits
        .last()
        .unwrap()
        .clone();
    assert_eq!(edit.page_title, "Run `cargo test`");
    f.agent.edit_undo(&edit.id).unwrap();
    assert!(f.vault.load_page(edit.page_id).unwrap().in_trash);
}

#[test]
fn daily_reuse_adds_no_edit_and_row_mutations_merge_and_undo_metadata() {
    let f = Fixture::new();
    let daily = f.agent.create_session("Daily").unwrap();
    f.script(
        &daily.id,
        "await tools.daily_note({date:'2026-10-09'});await tools.daily_note({date:'2026-10-09'});",
    );
    let state = f.agent.session_state(&daily.id).unwrap();
    assert_eq!(state.edits.len(), 1);
    assert_eq!(state.edits[0].kind, "created");
    f.script(&daily.id, "await tools.daily_note({date:'2026-10-09'});");
    assert_eq!(f.agent.session_state(&daily.id).unwrap().edits.len(), 1);
    f.agent.edit_undo(&state.edits[0].id).unwrap();
    assert!(
        f.vault
            .load_page(state.edits[0].page_id.clone())
            .unwrap()
            .in_trash
    );

    let database = f.vault.create_page(None, "Tasks".into()).unwrap();
    let def = |key: &str, kind| vault_core::PropDef {
        key: key.into(),
        name: key.into(),
        kind,
        options: vec![],
        config: String::new(),
    };
    f.vault
        .set_schema(
            database.clone(),
            vec![
                def("date", vault_core::PropKind::Date),
                def("done", vault_core::PropKind::Checkbox),
            ],
        )
        .unwrap();
    let result=vault_tools::call(&f.vault,"upsert_row",&serde_json::json!({"database":database,"title":"Original row","properties":{"date":"2026-10-09","done":false},"markdown":"original"})).unwrap();
    let row = result.rsplit('`').nth(1).unwrap().to_owned();
    let before = f.vault.load_page(row.clone()).unwrap();
    let session = f.agent.create_session("Rows").unwrap();
    let item = serde_json::to_string(&format!("row:{database}/{row}")).unwrap();
    let db_arg = serde_json::to_string(&database).unwrap();
    let row_arg = serde_json::to_string(&row).unwrap();
    f.script(&session.id,&format!("await tools.upsert_row({{database:{db_arg},row:{row_arg},title:'Changed row',markdown:'changed'}});await tools.update_task({{item:{item},done:true}});await tools.update_task({{item:{item},due:'2026-10-12 10:30'}});"));
    let state = f.agent.session_state(&session.id).unwrap();
    assert_eq!(state.edits.len(), 1);
    assert_eq!(state.edits[0].kind, "row");
    assert_eq!(state.activity.len(), 3);
    assert!(state
        .activity
        .iter()
        .all(
            |activity| activity.edit_id.as_deref() == Some(state.edits[0].id.as_str())
                && !activity.is_error
        ));
    f.agent.edit_undo(&state.edits[0].id).unwrap();
    let restored = f.vault.load_page(row.clone()).unwrap();
    assert_eq!(restored.title, before.title);
    assert_eq!(restored.properties, before.properties);
    assert_eq!(f.vault.page_markdown(row).unwrap(), "original");
}

/// Titles of the live children of `parent` (None: top level), in sibling order.
fn children(f: &Fixture, parent: Option<&str>) -> Vec<String> {
    f.vault
        .list_children(parent.map(str::to_owned), 0, u32::MAX)
        .unwrap()
        .into_iter()
        .map(|page| page.title)
        .collect()
}

#[test]
fn move_page_is_recorded_and_undo_puts_the_page_back() {
    let f = Fixture::new();
    let alpha = f.page("Alpha", "a");
    let parent = f.page("Parent", "p");
    let kid1 = f.vault.create_page(Some(parent.clone()), "Kid one".into()).unwrap();
    let kid2 = f.vault.create_page(Some(parent.clone()), "Kid two".into()).unwrap();
    let session = f.agent.create_session("Move one").unwrap();
    let (kid2_json, alpha_json) = (
        serde_json::to_string(&kid2).unwrap(),
        serde_json::to_string(&alpha).unwrap(),
    );
    f.script(
        &session.id,
        &format!("await tools.move_page({{pages:[{kid2_json}],after:{alpha_json}}});"),
    );
    let state = f.agent.session_state(&session.id).unwrap();
    assert_eq!(state.edits.len(), 1);
    let edit: notes_agent_wire::AgentEdit = state.edits[0].clone();
    assert_eq!(edit.page_id, kid2);
    assert_eq!(edit.kind, "edited");
    assert_eq!(state.activity[0].summary, "Moved Kid two");
    assert_eq!(children(&f, None), ["Alpha", "Kid two", "Parent"]);
    assert_eq!(children(&f, Some(parent.as_str())), ["Kid one"]);
    f.agent.edit_undo(&edit.id).unwrap();
    assert_eq!(children(&f, Some(parent.as_str())), ["Kid one", "Kid two"]);
    assert_eq!(children(&f, None), ["Alpha", "Parent"]);
    assert!(f.agent.edit_undo(&edit.id).is_err());
}

#[test]
fn moved_pages_undo_back_to_original_parent_and_order() {
    let f = Fixture::new();
    let parent = f.page("Parent", "p");
    for title in ["Kid one", "Kid two", "Kid three"] {
        f.vault.create_page(Some(parent.clone()), title.into()).unwrap();
    }
    let other = f.page("Other", "o");
    f.vault.create_page(Some(other.clone()), "Other kid".into()).unwrap();
    let session = f.agent.create_session("Move two").unwrap();
    // Moved in this order by title, so the call order differs from the original order.
    f.script(
        &session.id,
        &format!(
            "await tools.move_page({{pages:['Kid three','Kid two'],parent:{}}});",
            serde_json::to_string(&other).unwrap()
        ),
    );
    let state = f.agent.session_state(&session.id).unwrap();
    assert_eq!(state.edits.len(), 1);
    let edit = state.edits[0].clone();
    assert_eq!(children(&f, Some(parent.as_str())), ["Kid one"]);
    assert_eq!(
        children(&f, Some(other.as_str())),
        ["Kid three", "Kid two", "Other kid"]
    );
    f.agent.edit_undo(&edit.id).unwrap();
    assert_eq!(
        children(&f, Some(parent.as_str())),
        ["Kid one", "Kid two", "Kid three"]
    );
    assert_eq!(children(&f, Some(other.as_str())), ["Other kid"]);
}

#[test]
fn move_to_top_level_undo_places_page_first_when_old_sibling_moved_away() {
    let f = Fixture::new();
    let alpha = f.page("Alpha", "a");
    let parent = f.page("Parent", "p");
    let kid1 = f.vault.create_page(Some(parent.clone()), "Kid one".into()).unwrap();
    let kid2 = f.vault.create_page(Some(parent.clone()), "Kid two".into()).unwrap();
    let session = f.agent.create_session("Move top").unwrap();
    let (kid1_json, kid2_json, alpha_json) = (
        serde_json::to_string(&kid1).unwrap(),
        serde_json::to_string(&kid2).unwrap(),
        serde_json::to_string(&alpha).unwrap(),
    );
    // Kid two goes to the top level after Alpha; then Kid one goes to the top level first.
    f.script(
        &session.id,
        &format!("await tools.move_page({{pages:[{kid2_json}],after:{alpha_json}}});"),
    );
    f.script(
        &session.id,
        &format!("await tools.move_page({{pages:[{kid1_json}]}});"),
    );
    let state = f.agent.session_state(&session.id).unwrap();
    assert_eq!(state.edits.len(), 2);
    let (kid2_edit, kid1_edit) = (state.edits[0].clone(), state.edits[1].clone());
    assert_eq!(children(&f, Some(parent.as_str())), Vec::<String>::new());
    // Kid one, Kid two's old previous sibling, is no longer under Parent, so Kid two returns first.
    f.agent.edit_undo(&kid2_edit.id).unwrap();
    assert_eq!(children(&f, Some(parent.as_str())), ["Kid two"]);
    f.agent.edit_undo(&kid1_edit.id).unwrap();
    assert_eq!(children(&f, Some(parent.as_str())), ["Kid one", "Kid two"]);
    assert_eq!(children(&f, None), ["Alpha", "Parent"]);
}

#[test]
fn move_undo_reports_a_trashed_former_parent_and_moves_nothing() {
    let f = Fixture::new();
    let parent = f.page("Parent", "p");
    let kid1 = f.vault.create_page(Some(parent.clone()), "Kid one".into()).unwrap();
    let kid2 = f.vault.create_page(Some(parent.clone()), "Kid two".into()).unwrap();
    let session = f.agent.create_session("Move trashed").unwrap();
    f.script(
        &session.id,
        &format!(
            "await tools.move_page({{pages:[{}]}}); await tools.trash_page({{page:{}}});",
            serde_json::to_string(&kid2).unwrap(),
            serde_json::to_string(&parent).unwrap()
        ),
    );
    let state = f.agent.session_state(&session.id).unwrap();
    let edit = state
        .edits
        .iter()
        .find(|edit| edit.page_id == kid2)
        .unwrap()
        .clone();
    assert_eq!(edit.kind, "edited");
    assert_eq!(
        f.agent.edit_undo(&edit.id).unwrap_err().to_string(),
        "the page's former parent is gone or in the trash; use page history"
    );
    assert!(f.vault.load_page(parent).unwrap().in_trash);
    assert_eq!(children(&f, None), ["Kid two"]);
    assert!(f.vault.load_page(kid1).is_ok());
}
