//! Dev replay: a recorded session runs through the real `Agent::send` path.
//!
//! Every check sets `NOTES_DEV_REPLAY` under one lock, because the variable gates
//! both provider registration and `replay::load` for the whole process.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::json;
use vault_agent::bubblepaw_sdk::events::EventKind;
use vault_agent::bubblepaw_sdk::store::{Message, Role};
use vault_agent::bubblepaw_sdk::tools::ToolCall;
use vault_agent::replay::{self, ReplayStatus};
use vault_agent::Sent;
use vault_agent::{session_outline_at, Agent, AgentConfig};
use vault_core::Vault;

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Holds the env lock and sets `NOTES_DEV_REPLAY` (or unsets it) for the test.
fn env(value: Option<&str>) -> MutexGuard<'static, ()> {
    let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    match value {
        Some(value) => std::env::set_var("NOTES_DEV_REPLAY", value),
        None => std::env::remove_var("NOTES_DEV_REPLAY"),
    }
    guard
}

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn data_dir(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    fn agent(&self) -> Agent {
        let vault = Arc::new(
            Vault::open(self.dir.path().join("vault").to_string_lossy().into_owned()).unwrap(),
        );
        Agent::open(
            vault,
            AgentConfig {
                data_dir: self.data_dir(),
                // Sends to a loaded replay are routed to the replay model; this
                // default would fail rather than reach a real model.
                default_model: "replay/replay".into(),
                // A missing extension folder disables Claude Code deterministically.
                claude_code_dir: Some(self.data_dir().join("missing-test-claude-code")),
                ..AgentConfig::default()
            },
        )
        .unwrap()
    }

    /// A short recorded session with two turns, two chapters and three sections.
    fn replay_file(&self) -> PathBuf {
        let path = self.dir.path().join("replay.sqlite");
        let _ = std::fs::remove_file(&path);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions(id TEXT PRIMARY KEY, data TEXT NOT NULL);
             CREATE TABLE messages(seq INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, data TEXT NOT NULL);
             CREATE TABLE context_subject_chapters(id TEXT PRIMARY KEY, session_id TEXT NOT NULL, ordinal INTEGER NOT NULL, first_episode_ordinal INTEGER NOT NULL, last_episode_ordinal INTEGER NOT NULL, status TEXT NOT NULL, data TEXT NOT NULL);
             CREATE TABLE context_episodes(id TEXT PRIMARY KEY, session_id TEXT NOT NULL, ordinal INTEGER NOT NULL, first_message_seq INTEGER NOT NULL, last_message_seq INTEGER NOT NULL, chapter_id TEXT NOT NULL, data TEXT NOT NULL);
             CREATE TABLE context_decisions(id TEXT PRIMARY KEY, session_id TEXT NOT NULL, episode_ordinal INTEGER NOT NULL, kind TEXT NOT NULL, data TEXT NOT NULL);",
        )
        .unwrap();
        let call = |id: &str| ToolCall {
            id: id.into(),
            name: "exec".into(),
            arguments: json!({"command": "echo recorded"}),
        };
        let records: Vec<(Role, &str, Vec<ToolCall>, Option<&str>)> = vec![
            (Role::User, "first question", vec![], None),
            (
                Role::Assistant,
                "Looking into it.\n\nSecond paragraph.",
                vec![call("c1")],
                None,
            ),
            (Role::Tool, "recorded output", vec![], Some("c1")),
            (
                Role::Assistant,
                "Here is the answer.\n\nDone.",
                vec![],
                None,
            ),
            (Role::User, "second question", vec![], None),
            (Role::Assistant, "Checking more.", vec![], None),
            (Role::Assistant, "Found it.", vec![call("c2")], None),
            (Role::Tool, "recorded output two", vec![], Some("c2")),
            (Role::Assistant, "Final answer.", vec![], None),
        ];
        for (index, (role, content, tool_calls, tool_call_id)) in records.into_iter().enumerate() {
            let seq = index + 1;
            let message = Message {
                id: format!("r{seq}"),
                session_id: "source".into(),
                role,
                content: content.into(),
                tool_calls,
                tool_call_id: tool_call_id.map(Into::into),
                provider_state: None,
                origin: None,
                is_error: false,
                created_at: format!("2026-10-10T10:00:{seq:02}.000+00:00"),
            };
            conn.execute(
                "INSERT INTO messages(session_id,data) VALUES('source',?1)",
                [serde_json::to_string(&message).unwrap()],
            )
            .unwrap();
        }
        // Chapter A (closed) has episodes 1-2; chapter B (open) has episode 3.
        let chapter = |id: &str, ordinal: i64, status: &str, data: serde_json::Value| {
            conn.execute(
                "INSERT INTO context_subject_chapters VALUES(?1,'source',?2,1,1,?3,?4)",
                rusqlite::params![id, ordinal, status, data.to_string()],
            )
            .unwrap();
        };
        chapter(
            "chA",
            1,
            "closed",
            json!({"id":"chA","description":"Opening chapter","summary":"Chapter A summary","status":"closed"}),
        );
        chapter(
            "chB",
            2,
            "open",
            json!({"id":"chB","description":null,"summary":null,"status":"open"}),
        );
        let episode =
            |id: &str, ordinal: i64, first: i64, last: i64, chapter: &str, title: &str| {
                conn.execute(
                    "INSERT INTO context_episodes VALUES(?1,'source',?2,?3,?4,?5,?6)",
                    rusqlite::params![
                        id,
                        ordinal,
                        first,
                        last,
                        chapter,
                        json!({"title": title, "summary": format!("{title} summary")}).to_string()
                    ],
                )
                .unwrap();
            };
        episode("e1", 1, 1, 4, "chA", "Opening question");
        episode("e2", 2, 5, 6, "chA", "Follow-up");
        episode("e3", 3, 7, 9, "chB", "Final check");
        conn.execute(
            "INSERT INTO context_decisions VALUES('d1','source',1,'choice',?1)",
            [json!({"option_message_ids": ["r1", "r5"], "choice": "r1"}).to_string()],
        )
        .unwrap();
        path
    }
}

fn load(agent: &Agent, fixture: &Fixture, delay_ms: u64) -> (String, ReplayStatus) {
    let session = agent.create_session("replay").unwrap().id;
    let status = replay::load(
        &fixture.data_dir(),
        &session,
        &fixture.replay_file(),
        delay_ms,
    )
    .unwrap();
    (session, status)
}

/// Waits until every run has a saved end time and the session's last run lifecycle
/// event is `SessionSettled`. The end time is saved before the drive task clears the
/// busy state, so a send right after it is refused as busy (BubblePaw runtime.rs:235-236,
/// 568-571). See wiki/integrations/agent.md, "Store lock outlives Agent drop (BubblePaw)".
fn wait_idle(agent: &Agent, session: &str) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let state = agent.session_state(session).unwrap();
        if state.runs.iter().all(|run| run.ended_at.is_some()) && settled(agent, session) {
            return;
        }
        assert!(Instant::now() < deadline, "the run did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Whether the session's last `RunStarted` or `SessionSettled` event is a settle.
fn settled(agent: &Agent, session: &str) -> bool {
    let mut last = None;
    let mut after = 0;
    loop {
        let page = agent.events_after(after, Some(session)).unwrap().events;
        let Some(end) = page.last() else { break };
        after = end.sequence;
        for event in page {
            match event.kind {
                EventKind::RunStarted { .. } => last = Some(false),
                EventKind::SessionSettled {} => last = Some(true),
                _ => {}
            }
        }
    }
    last == Some(true)
}

fn send(agent: &Agent, session: &str, text: &str) {
    match agent.send(session, text, None).unwrap() {
        Sent::Started(_) => {}
        Sent::FollowUp(_) => panic!("the session was busy"),
    }
}

fn messages(agent: &Agent, session: &str) -> Vec<Message> {
    agent.session_state(session).unwrap().messages
}

#[test]
fn disabled_replay_cannot_load() {
    let _env = env(None);
    let fixture = Fixture::new();
    let agent = fixture.agent();
    let session = agent.create_session("off").unwrap().id;
    let error = replay::load(&fixture.data_dir(), &session, &fixture.replay_file(), 1).unwrap_err();
    assert!(error.to_string().contains("disabled"), "{error:#}");
    assert!(replay::status(&fixture.data_dir(), &session).is_err());
}

/// BubblePaw stores runtime notices as generated user messages. They are model
/// context: the app's message list and session DTO never contain them.
#[test]
fn runtime_notices_never_reach_the_app_message_list() {
    let _env = env(Some("1"));
    let fixture = Fixture::new();
    let agent = fixture.agent();
    let (session, _) = load(&agent, &fixture, 1);
    send(&agent, &session, "first question");
    wait_idle(&agent, &session);

    let stored = vault_agent::bubblepaw_sdk::store::Store::open(
        fixture.data_dir().join("agent").join("agent.sqlite"),
    )
    .unwrap()
    .messages(&session)
    .unwrap();
    let notice = |m: &Message| {
        m.role == Role::User
            && m.origin == Some(vault_agent::bubblepaw_sdk::store::MessageOrigin::Generated)
    };
    assert!(
        stored.iter().any(notice),
        "the run records a runtime notice"
    );

    let state = agent.session_state(&session).unwrap();
    assert!(!state.messages.iter().any(notice), "{:?}", state.messages);
    let dto = vault_agent::wire::session_state(state);
    let users: Vec<&str> = dto
        .messages
        .iter()
        .filter(|m| m.role == "user")
        .map(|m| m.text.as_str())
        .collect();
    assert_eq!(users, ["first question"]);
}

#[test]
fn replay_runs_through_the_normal_send_path() {
    let _env = env(Some("1"));
    let fixture = Fixture::new();
    let agent = fixture.agent();
    let (session, loaded) = load(&agent, &fixture, 1);
    assert_eq!(loaded.user_texts, ["first question", "second question"]);
    assert_eq!(
        (loaded.completed_users, loaded.state.as_str()),
        (0, "waiting")
    );

    // Loading again into the same session is refused.
    assert!(replay::load(&fixture.data_dir(), &session, &fixture.replay_file(), 1).is_err());

    send(&agent, &session, "first question");
    wait_idle(&agent, &session);
    let status = replay::status(&fixture.data_dir(), &session).unwrap();
    assert_eq!(
        (status.completed_users, status.state.as_str()),
        (1, "waiting"),
        "{status:?}"
    );

    let stored = messages(&agent, &session);
    let roles: Vec<Role> = stored.iter().map(|m| m.role.clone()).collect();
    assert_eq!(
        roles,
        [Role::User, Role::Assistant, Role::Tool, Role::Assistant]
    );
    assert_eq!(stored[1].content, "Looking into it.\n\nSecond paragraph.");
    assert_eq!(
        stored[2].tool_call_id.as_deref(),
        Some(stored[1].tool_calls[0].id.as_str())
    );
    assert_ne!(
        stored[1].tool_calls[0].id, "c1",
        "recorded IDs are not reused"
    );
    assert_eq!(stored[1].tool_calls[0].name, "js");

    send(&agent, &session, "second question");
    wait_idle(&agent, &session);
    let stored = messages(&agent, &session);
    assert_eq!(
        stored.len(),
        10,
        "every recorded record maps to one native message or a no-op step"
    );
    let status = replay::status(&fixture.data_dir(), &session).unwrap();
    assert_eq!(
        (status.completed_users, status.state.as_str()),
        (2, "complete"),
        "{status:?}"
    );
    let ids: std::collections::HashSet<_> = stored
        .iter()
        .flat_map(|m| m.tool_calls.iter().map(|c| c.id.clone()))
        .collect();
    assert_eq!(ids.len(), 3, "synthetic call IDs are unique");
}

#[test]
fn outline_grows_with_completed_sections() {
    let _env = env(Some("1"));
    let fixture = Fixture::new();
    let agent = fixture.agent();
    let (session, _) = load(&agent, &fixture, 1);

    // Nothing is published before a section's last source message exists.
    assert!(session_outline_at(&fixture.data_dir(), &session)
        .unwrap()
        .chapters
        .is_empty());

    send(&agent, &session, "first question");
    wait_idle(&agent, &session);
    replay::status(&fixture.data_dir(), &session).unwrap();
    let outline = session_outline_at(&fixture.data_dir(), &session).unwrap();
    assert_eq!(outline.chapters.len(), 1, "{outline:?}");
    let chapter = &outline.chapters[0];
    assert_eq!(chapter.title, "Opening question");
    assert!(chapter.ongoing && chapter.ended_at.is_none());
    assert_eq!(chapter.sections.len(), 1);
    assert_eq!(chapter.sections[0].title, "Opening question");
    let stored = messages(&agent, &session);
    assert_eq!(chapter.sections[0].first_message_id, stored[0].id);
    assert_eq!(chapter.sections[0].last_message_id, stored[3].id);
    assert_eq!(
        chapter.decisions,
        ["Context boundary selected from 2 candidates"]
    );

    send(&agent, &session, "second question");
    wait_idle(&agent, &session);
    replay::status(&fixture.data_dir(), &session).unwrap();
    let outline = session_outline_at(&fixture.data_dir(), &session).unwrap();
    assert_eq!(outline.chapters.len(), 2, "{outline:?}");
    let first = &outline.chapters[0];
    assert_eq!(first.title, "Opening chapter");
    assert_eq!(first.sections.len(), 2);
    assert!(
        !first.ongoing && first.ended_at.is_some(),
        "the first chapter closes at its boundary"
    );
    // Native order: the continuation step adds a tool message, so source seq 6 is
    // the sixth native message only up to there; index 5 is the assistant record.
    let stored = messages(&agent, &session);
    assert_eq!(first.last_message_id, stored[5].id);
    let second = &outline.chapters[1];
    assert!(second.ongoing && second.ended_at.is_none());
    assert_eq!(second.sections[0].title, "Final check");
    assert_eq!(second.first_message_id, stored[7].id);
    let title = agent
        .sessions()
        .unwrap()
        .into_iter()
        .find(|s| s.id == session)
        .unwrap()
        .title;
    assert_eq!(
        title, "Final check",
        "the session title follows the latest chapter"
    );

    // Polling again does not change anything.
    let before = vault_agent::session_outline_revision_at(&fixture.data_dir(), &session).unwrap();
    replay::status(&fixture.data_dir(), &session).unwrap();
    assert_eq!(
        before,
        vault_agent::session_outline_revision_at(&fixture.data_dir(), &session).unwrap()
    );
}

#[test]
fn pause_holds_the_stream_and_resume_continues() {
    let _env = env(Some("1"));
    let fixture = Fixture::new();
    let agent = fixture.agent();
    let (session, _) = load(&agent, &fixture, 1);

    replay::control(&session, "pause").unwrap();
    send(&agent, &session, "first question");
    std::thread::sleep(Duration::from_millis(200));
    let status = replay::status(&fixture.data_dir(), &session).unwrap();
    let runs = agent.session_state(&session).unwrap().runs;
    assert_eq!(
        status.state,
        "paused",
        "{status:?} {:?}",
        runs.iter().map(|r| r.error.clone()).collect::<Vec<_>>()
    );
    assert_eq!(
        messages(&agent, &session).len(),
        1,
        "only the user message while paused"
    );

    assert_eq!(
        replay::control(&session, "resume").unwrap().state,
        "running"
    );
    wait_idle(&agent, &session);
    let status = replay::status(&fixture.data_dir(), &session).unwrap();
    assert_eq!(
        (status.completed_users, status.state.as_str()),
        (1, "waiting")
    );
    assert_eq!(messages(&agent, &session).len(), 4);
}

#[test]
fn ongoing_chapter_includes_an_unfinished_section_before_it_closes() {
    let _env = env(Some("1"));
    let fixture = Fixture::new();
    let agent = fixture.agent();
    let (session, _) = load(&agent, &fixture, 1);
    send(&agent, &session, "first question");
    wait_idle(&agent, &session);
    replay::status(&fixture.data_dir(), &session).unwrap();
    replay::control(&session, "pause").unwrap();
    send(&agent, &session, "second question");
    std::thread::sleep(Duration::from_millis(50));
    replay::status(&fixture.data_dir(), &session).unwrap();
    let outline = session_outline_at(&fixture.data_dir(), &session).unwrap();
    let stored = messages(&agent, &session);
    assert!(outline.chapters[0].ongoing);
    assert_eq!(
        outline.chapters[0].sections.len(),
        1,
        "the second section has not landed"
    );
    assert_eq!(
        outline.chapters[0].last_message_id,
        stored.last().unwrap().id,
        "the bottom reader must already be inside the growing chapter"
    );
    replay::control(&session, "resume").unwrap();
    wait_idle(&agent, &session);
    replay::status(&fixture.data_dir(), &session).unwrap();
    assert!(
        !session_outline_at(&fixture.data_dir(), &session)
            .unwrap()
            .chapters[0]
            .ongoing
    );
}

#[test]
fn stop_and_cancel_end_the_turn_without_advancing() {
    let _env = env(Some("1"));
    let fixture = Fixture::new();
    let agent = fixture.agent();
    let (session, _) = load(&agent, &fixture, 1);

    replay::control(&session, "pause").unwrap();
    send(&agent, &session, "first question");
    std::thread::sleep(Duration::from_millis(50));
    assert!(agent.cancel(&session), "a run is in flight");
    wait_idle(&agent, &session);
    let status = replay::status(&fixture.data_dir(), &session).unwrap();
    assert_eq!(status.state, "error", "{status:?}");
    assert_eq!(status.completed_users, 0);
    // A recorded turn cannot resume mid-turn, so the next send is refused.
    send(&agent, &session, "first question");
    wait_idle(&agent, &session);
    assert_eq!(
        replay::status(&fixture.data_dir(), &session)
            .unwrap()
            .completed_users,
        0
    );

    let (stopped, _) = load(&agent, &fixture, 1);
    replay::control(&stopped, "pause").unwrap();
    send(&agent, &stopped, "first question");
    std::thread::sleep(Duration::from_millis(50));
    let status = replay::control(&stopped, "stop").unwrap();
    assert_eq!(status.state, "stopped");
    wait_idle(&agent, &stopped);
    assert_eq!(
        replay::status(&fixture.data_dir(), &stopped)
            .unwrap()
            .completed_users,
        0
    );
    assert!(replay::control(&stopped, "bogus").is_err());
}

#[test]
fn unexpected_user_text_is_refused_and_not_advanced() {
    let _env = env(Some("1"));
    let fixture = Fixture::new();
    let agent = fixture.agent();
    let (session, _) = load(&agent, &fixture, 1);

    send(&agent, &session, "something unrelated");
    wait_idle(&agent, &session);
    let status = replay::status(&fixture.data_dir(), &session).unwrap();
    assert_eq!(status.state, "error");
    assert!(status.error.unwrap().contains("does not match"));
    assert_eq!(status.completed_users, 0);

    // The recorded text still works, and maps anchors past the rejected message.
    send(&agent, &session, "first question");
    wait_idle(&agent, &session);
    let status = replay::status(&fixture.data_dir(), &session).unwrap();
    assert_eq!(
        (status.completed_users, status.state.as_str()),
        (1, "waiting"),
        "{status:?}"
    );
    let outline = session_outline_at(&fixture.data_dir(), &session).unwrap();
    let stored = messages(&agent, &session);
    let first = &outline.chapters[0].sections[0];
    assert_eq!(first.first_message_id, stored[1].id);
}

/// A recorded session with harness-written user text: leading instructions with an
/// orphan reply, an environment block in the middle of a turn, and a split decision.
fn filtered_replay_file(fixture: &Fixture) -> PathBuf {
    let path = fixture.dir.path().join("filtered.sqlite");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE sessions(id TEXT PRIMARY KEY, data TEXT NOT NULL);
         CREATE TABLE messages(seq INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, data TEXT NOT NULL);
         CREATE TABLE context_subject_chapters(id TEXT PRIMARY KEY, session_id TEXT NOT NULL, ordinal INTEGER NOT NULL, first_episode_ordinal INTEGER NOT NULL, last_episode_ordinal INTEGER NOT NULL, status TEXT NOT NULL, data TEXT NOT NULL);
         CREATE TABLE context_episodes(id TEXT PRIMARY KEY, session_id TEXT NOT NULL, ordinal INTEGER NOT NULL, first_message_seq INTEGER NOT NULL, last_message_seq INTEGER NOT NULL, chapter_id TEXT NOT NULL, data TEXT NOT NULL);
         CREATE TABLE context_decisions(id TEXT PRIMARY KEY, session_id TEXT NOT NULL, episode_ordinal INTEGER NOT NULL, kind TEXT NOT NULL, data TEXT NOT NULL);",
    )
    .unwrap();
    let call = |id: &str| ToolCall {
        id: id.into(),
        name: "exec".into(),
        arguments: json!({"command": "echo recorded"}),
    };
    let records: Vec<(Role, &str, Vec<ToolCall>, Option<&str>)> = vec![
        (
            Role::User,
            "# AGENTS.md instructions for /repo\n\n<INSTRUCTIONS>\nBe brief.",
            vec![],
            None,
        ),
        (Role::Assistant, "Orphan reply.", vec![], None),
        (Role::User, "first question", vec![], None),
        (Role::Assistant, "Looking into it.", vec![call("c1")], None),
        (Role::Tool, "recorded output", vec![], Some("c1")),
        (
            Role::User,
            "<environment_context>\n  <cwd>/repo</cwd>\n</environment_context>",
            vec![],
            None,
        ),
        (Role::Assistant, "Here is the answer.", vec![], None),
        (Role::User, "second question", vec![], None),
        (Role::Assistant, "Final answer.", vec![], None),
    ];
    for (index, (role, content, tool_calls, tool_call_id)) in records.into_iter().enumerate() {
        let seq = index + 1;
        let message = Message {
            id: format!("r{seq}"),
            session_id: "source".into(),
            role,
            content: content.into(),
            tool_calls,
            tool_call_id: tool_call_id.map(Into::into),
            provider_state: None,
            origin: None,
            is_error: false,
            created_at: format!("2026-10-10T10:00:{seq:02}.000+00:00"),
        };
        conn.execute(
            "INSERT INTO messages(session_id,data) VALUES('source',?1)",
            [serde_json::to_string(&message).unwrap()],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO context_subject_chapters VALUES('chA','source',1,1,1,'closed',?1)",
        [json!({"id":"chA","description":"Opening chapter","summary":"Chapter A summary","status":"closed"}).to_string()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO context_subject_chapters VALUES('chB','source',2,2,2,'open',?1)",
        [json!({"id":"chB","description":null,"summary":null,"status":"open"}).to_string()],
    )
    .unwrap();
    // Episode 1 starts on the leading harness message and ends on the harness
    // environment block; episode 2 is the second question.
    for (id, ordinal, first, last, chapter, title) in [
        ("e1", 1, 1, 7, "chA", "Opening question"),
        ("e2", 2, 8, 9, "chB", "Final check"),
    ] {
        conn.execute(
            "INSERT INTO context_episodes VALUES(?1,'source',?2,?3,?4,?5,?6)",
            rusqlite::params![
                id,
                ordinal,
                first,
                last,
                chapter,
                json!({"title": title, "summary": format!("{title} summary")}).to_string()
            ],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO context_decisions VALUES('d1','source',1,'split',?1)",
        [json!({"option_message_ids": ["r3", "r6"], "choice": "r3"}).to_string()],
    )
    .unwrap();
    path
}

#[test]
fn harness_user_text_and_split_decisions_are_not_shown() {
    let _env = env(Some("1"));
    let fixture = Fixture::new();
    let agent = fixture.agent();
    let session = agent.create_session("replay").unwrap().id;
    let status = replay::load(
        &fixture.data_dir(),
        &session,
        &filtered_replay_file(&fixture),
        1,
    )
    .unwrap();
    assert_eq!(
        status.user_texts,
        ["first question", "second question"],
        "harness users are not turns"
    );

    send(&agent, &session, "first question");
    wait_idle(&agent, &session);
    replay::status(&fixture.data_dir(), &session).unwrap();
    send(&agent, &session, "second question");
    wait_idle(&agent, &session);
    let status = replay::status(&fixture.data_dir(), &session).unwrap();
    assert_eq!(
        (status.completed_users, status.state.as_str()),
        (2, "complete"),
        "{status:?}"
    );

    // The tool result's content is the replay's no-op output, not the recorded payload,
    // so it is checked by role and by the call it answers.
    let stored = messages(&agent, &session);
    let roles: Vec<Role> = stored.iter().map(|m| m.role.clone()).collect();
    assert_eq!(
        roles,
        vec![
            Role::User,
            Role::Assistant,
            Role::Tool,
            Role::Assistant,
            Role::User,
            Role::Assistant,
        ],
        "no harness user or orphan reply is written"
    );
    let texts: Vec<&str> = stored
        .iter()
        .filter(|m| m.role != Role::Tool)
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(
        texts,
        [
            "first question",
            "Looking into it.",
            "Here is the answer.",
            "second question",
            "Final answer.",
        ]
    );
    assert_eq!(
        stored[2].tool_call_id.as_deref(),
        Some(stored[1].tool_calls[0].id.as_str())
    );

    let outline = session_outline_at(&fixture.data_dir(), &session).unwrap();
    assert_eq!(outline.chapters.len(), 2, "{outline:?}");
    let first = &outline.chapters[0];
    assert_eq!(first.title, "Opening chapter");
    assert!(
        first.decisions.is_empty(),
        "a split is not a decision: {first:?}"
    );
    // The leading harness seq aliases to the first kept message; the environment seq
    // aliases to the tool result before it, and the section still ends on the kept
    // reply at seq 7.
    assert_eq!(first.sections[0].first_message_id, stored[0].id);
    assert_eq!(first.sections[0].last_message_id, stored[3].id);
    assert!(first.sections[0].decisions.is_empty());
    let second = &outline.chapters[1];
    assert_eq!(second.first_message_id, stored[4].id);
    assert_eq!(second.sections[0].title, "Final check");
    assert!(second.decisions.is_empty());
}
