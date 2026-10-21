use std::sync::Arc;

use vault_agent::bubblepaw_core::{ContextChapter, Message, Role, Store};
use vault_agent::{read_session_outline, session_outline_at, Agent, AgentConfig, TestHooks};
use vault_core::Vault;
use vault_tools::outline::{ChatOutline, OutlineChapter, OutlineSection, OutlineStore};

struct Fixture {
    dir: tempfile::TempDir,
    agent: Agent,
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
        Self { dir, agent }
    }

    fn data_dir(&self) -> std::path::PathBuf {
        self.dir.path().join("data")
    }

    fn agent_dir(&self) -> std::path::PathBuf {
        self.data_dir().join("agent")
    }

    // Seed rows through BubblePaw's batch append on the open agent's store.
    fn message(&self, session: &str, role: Role, created_at: &str) -> Message {
        let message = Message {
            id: vault_core::new_id(),
            session_id: session.into(),
            role,
            content: format!("message at {created_at}"),
            tool_calls: vec![],
            tool_call_id: None,
            provider_state: None,
            origin: None,
            is_error: false,
            created_at: created_at.into(),
        };
        Store::open(self.agent_dir().join("agent.sqlite"))
            .unwrap()
            .append_batch(std::slice::from_ref(&message))
            .unwrap();
        message
    }
}

fn notes_chapter(first: &str, last: &str) -> OutlineChapter {
    OutlineChapter {
        id: "notes-chapter".into(),
        title: "Notes chapter".into(),
        summary: "Owned by the notes outline".into(),
        decisions: vec!["Context boundary selected from 2 candidates".into()],
        ongoing: true,
        first_message_id: first.into(),
        last_message_id: last.into(),
        started_at: "2026-10-08T01:00:00Z".into(),
        ended_at: None,
        sections: vec![OutlineSection {
            id: "notes-section".into(),
            title: "Notes section".into(),
            summary: "Part".into(),
            decisions: vec![],
            first_message_id: first.into(),
            last_message_id: last.into(),
        }],
    }
}

#[test]
fn session_outline_prefers_notes_outline_then_context_chapters_then_empty() {
    let fixture = Fixture::new();
    let session = fixture.agent.create_session("outline").unwrap().id;
    let first = fixture.message(&session, Role::User, "2026-10-08T01:00:00Z");
    fixture.message(&session, Role::Assistant, "2026-10-08T01:01:00Z");
    let last = fixture.message(&session, Role::Tool, "2026-10-08T01:02:00Z");

    // Nothing saved: empty, not an error.
    assert_eq!(
        fixture.agent.session_outline(&session).unwrap(),
        ChatOutline::default()
    );

    // BubblePaw context chapter: no sections, times from its anchor messages.
    let store = Store::open(fixture.agent_dir().join("agent.sqlite")).unwrap();
    store
        .save_context_chapter(&ContextChapter {
            id: "context-chapter".into(),
            session_id: session.clone(),
            ordinal: 1,
            first_message_id: first.id.clone(),
            last_message_id: last.id.clone(),
            title: "Context title".into(),
            summary: "Context summary".into(),
            model: "scripted/scripted".into(),
            usage: None,
            created_at: "2026-10-08T01:03:00Z".into(),
        })
        .unwrap();
    let outline = fixture.agent.session_outline(&session).unwrap();
    assert_eq!(outline.chapters.len(), 1);
    let fallback = &outline.chapters[0];
    assert_eq!(fallback.id, "context-chapter");
    assert_eq!(fallback.title, "Context title");
    assert_eq!(fallback.summary, "Context summary");
    assert!(fallback.sections.is_empty());
    assert!(fallback.decisions.is_empty());
    assert!(!fallback.ongoing);
    assert_eq!(fallback.started_at, "2026-10-08T01:00:00Z");
    assert_eq!(fallback.ended_at.as_deref(), Some("2026-10-08T01:02:00Z"));

    // The notes-owned outline wins once it exists for the session.
    let mut outline_db =
        OutlineStore::open(&fixture.agent_dir().join(vault_tools::outline::FILE_NAME)).unwrap();
    outline_db
        .save_session(&session, &[notes_chapter(&first.id, &last.id)])
        .unwrap();
    let outline = fixture.agent.session_outline(&session).unwrap();
    assert_eq!(outline.chapters.len(), 1);
    let notes = &outline.chapters[0];
    assert_eq!(notes.id, "notes-chapter");
    assert_eq!(notes.sections.len(), 1);
    assert!(notes.ongoing);
    assert_eq!(
        notes.decisions,
        ["Context boundary selected from 2 candidates"]
    );
    assert_eq!(notes.ended_at, None);

    // Another session has neither: empty again.
    let other = fixture.agent.create_session("other").unwrap().id;
    assert!(fixture
        .agent
        .session_outline(&other)
        .unwrap()
        .chapters
        .is_empty());
}

#[test]
fn a_saved_empty_notes_outline_overrides_the_context_chapters() {
    let fixture = Fixture::new();
    let session = fixture.agent.create_session("cleared").unwrap().id;
    let first = fixture.message(&session, Role::User, "2026-10-08T02:00:00Z");
    let store = Store::open(fixture.agent_dir().join("agent.sqlite")).unwrap();
    store
        .save_context_chapter(&ContextChapter {
            id: "context-chapter".into(),
            session_id: session.clone(),
            ordinal: 1,
            first_message_id: first.id.clone(),
            last_message_id: first.id.clone(),
            title: "Context title".into(),
            summary: "Context summary".into(),
            model: "scripted/scripted".into(),
            usage: None,
            created_at: "2026-10-08T02:01:00Z".into(),
        })
        .unwrap();
    assert_eq!(
        fixture
            .agent
            .session_outline(&session)
            .unwrap()
            .chapters
            .len(),
        1
    );

    // An empty outline saved by the notes app is an outline: no fallback shows.
    let mut outline_db =
        OutlineStore::open(&fixture.agent_dir().join(vault_tools::outline::FILE_NAME)).unwrap();
    outline_db.save_session(&session, &[]).unwrap();
    assert_eq!(
        fixture.agent.session_outline(&session).unwrap(),
        ChatOutline::default()
    );
}

#[test]
fn the_data_dir_helper_reads_the_same_outline_as_the_open_agent() {
    let fixture = Fixture::new();
    let session = fixture.agent.create_session("helper").unwrap().id;
    let first = fixture.message(&session, Role::User, "2026-10-08T03:00:00Z");
    let last = fixture.message(&session, Role::Assistant, "2026-10-08T03:01:00Z");

    // Empty, saved notes outline, then the context fallback: the helper agrees each time.
    assert_eq!(
        session_outline_at(&fixture.data_dir(), &session).unwrap(),
        fixture.agent.session_outline(&session).unwrap()
    );
    let mut outline_db =
        OutlineStore::open(&fixture.agent_dir().join(vault_tools::outline::FILE_NAME)).unwrap();
    outline_db
        .save_session(&session, &[notes_chapter(&first.id, &last.id)])
        .unwrap();
    let helper = session_outline_at(&fixture.data_dir(), &session).unwrap();
    assert_eq!(helper, fixture.agent.session_outline(&session).unwrap());
    assert_eq!(helper.chapters[0].id, "notes-chapter");
}

#[test]
fn the_data_dir_helper_errors_without_creating_a_store() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("empty-data");
    assert!(session_outline_at(&data, "any").is_err());
    assert!(
        !data.exists(),
        "no agent folder is made for a missing store"
    );
}

#[test]
fn session_outline_refuses_an_unknown_session() {
    let fixture = Fixture::new();
    assert!(fixture.agent.session_outline("no-such-session").is_err());
    assert!(session_outline_at(&fixture.data_dir(), "no-such-session").is_err());
}

/// Opt-in: reads a real import made by `import-replay`. Set
/// `NOTES_OUTLINE_REPLAY_DATA_DIR` to that import's `--data-dir`; without it the test
/// is skipped, so the normal suite does not need the replay file.
#[test]
fn imported_replay_outline_is_in_source_order() {
    let Some(dir) = std::env::var_os("NOTES_OUTLINE_REPLAY_DATA_DIR") else {
        eprintln!("skipped: set NOTES_OUTLINE_REPLAY_DATA_DIR to an import-replay data dir");
        return;
    };
    let agent_dir = std::path::PathBuf::from(dir).join("agent");
    let store = Store::open(&agent_dir.join("agent.sqlite")).unwrap();
    let sessions = store.list_sessions().unwrap();
    assert_eq!(sessions.len(), 1, "the import holds one session");
    let session = &sessions[0].id;

    let outline = read_session_outline(
        &store,
        &agent_dir.join(vault_tools::outline::FILE_NAME),
        session,
    )
    .unwrap();
    assert_eq!(outline.chapters.len(), 3, "chapters");
    let section_count: usize = outline.chapters.iter().map(|c| c.sections.len()).sum();
    assert_eq!(section_count, 8, "sections");
    let flags: Vec<bool> = outline.chapters.iter().map(|c| c.ongoing).collect();
    assert_eq!(flags, [false, false, true], "only the last chapter is open");

    // Every anchor must be a message of the session; the message order is the source order.
    let messages = store.messages(session).unwrap();
    let position: std::collections::HashMap<&str, usize> = messages
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id.as_str(), i))
        .collect();
    let at = |id: &str| -> usize {
        *position
            .get(id)
            .unwrap_or_else(|| panic!("anchor {id} is not a message of the session"))
    };

    let mut chapter_end: Option<usize> = None;
    for chapter in &outline.chapters {
        let (start, end) = (at(&chapter.first_message_id), at(&chapter.last_message_id));
        assert!(start <= end, "chapter {} runs backwards", chapter.id);
        if let Some(previous) = chapter_end {
            assert!(
                start >= previous,
                "chapter {} is out of source order",
                chapter.id
            );
        }
        chapter_end = Some(end);

        let first_section = chapter
            .sections
            .first()
            .expect("every chapter has sections");
        let last_section = chapter.sections.last().expect("every chapter has sections");
        assert_eq!(first_section.first_message_id, chapter.first_message_id);
        assert_eq!(last_section.last_message_id, chapter.last_message_id);

        let mut section_end: Option<usize> = None;
        for section in &chapter.sections {
            let (s_start, s_end) = (at(&section.first_message_id), at(&section.last_message_id));
            assert!(
                start <= s_start && s_start <= s_end && s_end <= end,
                "section {} is outside chapter {}",
                section.id,
                chapter.id
            );
            if let Some(previous) = section_end {
                assert!(
                    s_start >= previous,
                    "section {} is out of source order",
                    section.id
                );
            }
            section_end = Some(s_end);
        }

        // A chapter lists its sections' decisions, in order, and nothing else.
        let section_decisions: Vec<&String> =
            chapter.sections.iter().flat_map(|s| &s.decisions).collect();
        assert_eq!(
            chapter.decisions.iter().collect::<Vec<_>>(),
            section_decisions,
            "chapter {} decisions",
            chapter.id
        );
    }

    // Decisions are segmentation entries: counted, worded, and free of message ids.
    let decisions: Vec<&String> = outline
        .chapters
        .iter()
        .flat_map(|c| c.sections.iter().flat_map(|s| &s.decisions))
        .collect();
    assert_eq!(decisions.len(), 8, "section decisions");
    for text in decisions.iter().map(|t| t.as_str()).chain(
        outline
            .chapters
            .iter()
            .flat_map(|c| c.decisions.iter().map(|t| t.as_str())),
    ) {
        assert!(
            text.starts_with("Context boundary selected from ")
                || text.starts_with("No context boundary selected from "),
            "unexpected decision wording: {text}"
        );
        assert!(
            !position.keys().any(|id| text.contains(id)),
            "a decision shows a message id: {text}"
        );
    }
}
