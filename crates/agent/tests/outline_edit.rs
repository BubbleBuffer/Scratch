//! Outline edits through the agent data directory: anchors must be messages of the
//! session, in order; closing and no-op edits; and the revision that tracks them.

use std::path::PathBuf;
use std::sync::Arc;

use vault_agent::bubblepaw_core::{Message, Role, Store};
use vault_agent::{
    close_outline_chapter_at, session_outline_at, session_outline_revision_at,
    upsert_outline_chapter_at, upsert_outline_section_at, Agent, AgentConfig, TestHooks,
};
use vault_core::Vault;
use vault_tools::outline::{OutlineChapter, OutlineSection, FILE_NAME};

struct Fixture {
    dir: tempfile::TempDir,
    _agent: Agent,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let vault =
            Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into_owned()).unwrap());
        let agent = Agent::open(
            vault,
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
        Self { dir, _agent: agent }
    }

    fn data_dir(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    fn agent_dir(&self) -> PathBuf {
        self.data_dir().join("agent")
    }

    /// The agent's own store, the same one the open agent holds.
    fn store(&self) -> Store {
        Store::open(self.agent_dir().join("agent.sqlite")).unwrap()
    }

    /// Seeds `count` messages into `session` through BubblePaw's batch append and
    /// returns their ids in order.
    fn messages(&self, session: &str, count: usize) -> Vec<String> {
        let messages: Vec<Message> = (0..count)
            .map(|i| Message {
                id: vault_core::new_id(),
                session_id: session.into(),
                role: if i % 2 == 0 {
                    Role::User
                } else {
                    Role::Assistant
                },
                content: format!("message {i}"),
                tool_calls: vec![],
                tool_call_id: None,
                provider_state: None,
                origin: None,
                is_error: false,
                created_at: format!("2026-10-10T09:{i:02}:00Z"),
            })
            .collect();
        self.store().append_batch(&messages).unwrap();
        messages.into_iter().map(|message| message.id).collect()
    }
}

fn chapter(id: &str, first: &str, last: &str) -> OutlineChapter {
    OutlineChapter {
        id: id.into(),
        title: format!("Chapter {id}"),
        summary: "Summary".into(),
        decisions: vec![],
        ongoing: true,
        first_message_id: first.into(),
        last_message_id: last.into(),
        started_at: "2026-10-10T09:00:00Z".into(),
        ended_at: None,
        sections: vec![],
    }
}

fn section(id: &str, first: &str, last: &str) -> OutlineSection {
    OutlineSection {
        id: id.into(),
        title: format!("Section {id}"),
        summary: "Part".into(),
        decisions: vec![],
        first_message_id: first.into(),
        last_message_id: last.into(),
    }
}

#[test]
fn upserts_and_closing_persist_and_count_only_real_changes() {
    let fx = Fixture::new();
    let data = fx.data_dir();
    let session = fx.store().create_session("chat").unwrap().id;
    let m = fx.messages(&session, 4);
    assert_eq!(session_outline_revision_at(&data, &session).unwrap(), 0);

    assert_eq!(
        upsert_outline_chapter_at(&data, &session, chapter("c1", &m[0], &m[1])).unwrap(),
        1
    );
    assert_eq!(
        upsert_outline_section_at(&data, &session, &m[0], section("s1", &m[0], &m[1])).unwrap(),
        2
    );
    // The same chapter again, with no sections named, keeps s1 and changes nothing.
    assert_eq!(
        upsert_outline_chapter_at(&data, &session, chapter("c1", &m[0], &m[1])).unwrap(),
        2
    );
    assert_eq!(
        upsert_outline_section_at(&data, &session, &m[0], section("s1", &m[0], &m[1])).unwrap(),
        2
    );

    let closed =
        close_outline_chapter_at(&data, &session, &m[0], &m[2], "2026-10-10T09:10:00Z").unwrap();
    assert_eq!(closed, 3);
    assert_eq!(
        close_outline_chapter_at(&data, &session, &m[0], &m[2], "2026-10-10T09:10:00Z").unwrap(),
        3,
        "closing with the stored values is a no-op"
    );

    let outline = session_outline_at(&data, &session).unwrap();
    assert_eq!(outline.chapters.len(), 1);
    let stored = &outline.chapters[0];
    assert!(!stored.ongoing);
    assert_eq!(stored.last_message_id, m[2]);
    assert_eq!(stored.ended_at.as_deref(), Some("2026-10-10T09:10:00Z"));
    assert_eq!(stored.sections.len(), 1);
    assert_eq!(session_outline_revision_at(&data, &session).unwrap(), 3);
}

#[test]
fn anchors_must_belong_to_the_session_and_run_forward() {
    let fx = Fixture::new();
    let data = fx.data_dir();
    let store = fx.store();
    let a = store.create_session("a").unwrap().id;
    let b = store.create_session("b").unwrap().id;
    let ma = fx.messages(&a, 3);
    let mb = fx.messages(&b, 2);
    upsert_outline_chapter_at(&data, &a, chapter("c1", &ma[0], &ma[1])).unwrap();

    // A message of another session cannot anchor a chapter of this one.
    let err = upsert_outline_chapter_at(&data, &a, chapter("c2", &ma[2], &mb[0])).unwrap_err();
    assert!(
        err.to_string().contains("not a message of this session"),
        "{err}"
    );
    // Nor can a session's own message anchor another session's chapter.
    let err = upsert_outline_chapter_at(&data, &b, chapter("c3", &ma[0], &mb[1])).unwrap_err();
    assert!(
        err.to_string().contains("not a message of this session"),
        "{err}"
    );
    // A chapter ends before it starts.
    let err = upsert_outline_chapter_at(&data, &a, chapter("c4", &ma[2], &ma[0])).unwrap_err();
    assert!(err.to_string().contains("ends before it starts"), "{err}");
    // A section anchored in another session.
    let err =
        upsert_outline_section_at(&data, &a, &ma[0], section("x", &ma[0], &mb[0])).unwrap_err();
    assert!(
        err.to_string().contains("not a message of this session"),
        "{err}"
    );
    // Closing a chapter that does not start at the given message.
    let err =
        close_outline_chapter_at(&data, &a, &ma[2], &ma[2], "2026-10-10T09:10:00Z").unwrap_err();
    assert!(err.to_string().contains("no chapter starts at"), "{err}");

    // Nothing was written by the refused edits.
    assert_eq!(session_outline_revision_at(&data, &a).unwrap(), 1);
    assert_eq!(session_outline_revision_at(&data, &b).unwrap(), 0);
    let outline_a = session_outline_at(&data, &a).unwrap();
    assert_eq!(
        outline_a
            .chapters
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        ["c1"]
    );
    assert!(session_outline_at(&data, &b).unwrap().chapters.is_empty());
}

#[test]
fn an_unknown_session_or_a_missing_store_writes_nothing() {
    let fx = Fixture::new();
    let data = fx.data_dir();
    assert!(upsert_outline_chapter_at(&data, "no-such-session", chapter("c", "m1", "m2")).is_err());
    assert!(
        !fx.agent_dir().join(FILE_NAME).exists(),
        "a refused edit creates no outline file"
    );

    let empty = tempfile::tempdir().unwrap();
    let err = upsert_outline_chapter_at(empty.path(), "s", chapter("c", "m1", "m2")).unwrap_err();
    assert!(err.to_string().contains("no agent store"), "{err}");
    assert!(!empty.path().join("agent").exists(), "no store is made");
    assert_eq!(
        session_outline_revision_at(empty.path(), "s").unwrap(),
        0,
        "a missing store has revision 0"
    );
    assert!(
        !empty.path().join("agent").exists(),
        "a revision query makes no store"
    );
}
