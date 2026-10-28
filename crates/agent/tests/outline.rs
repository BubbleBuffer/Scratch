use std::sync::Arc;

use vault_agent::bubblepaw_sdk::core::{
    ContextEpisode, ContextSubjectChapter, EpisodeBatch, SubjectChapterStatus,
};
use vault_agent::bubblepaw_sdk::store::{ContextChapter, Message, Role, Store};
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

// LCM fixture: six messages in three episodes (1-2, 3-4, 5-6) and two chapters.
// Chapter 1 covers episodes 1-2 and is closed when episode 3 is saved. Chapter 2
// covers episode 3 and stays open. `close_last` closes chapter 2 after episode 3
// is saved, and closes chapter 1 after episode 2 so the batches never close two
// chapters at once.

const LCM_TIMES: [&str; 6] = [
    "2026-10-08T04:00:00Z",
    "2026-10-08T04:01:00Z",
    "2026-10-08T04:02:00Z",
    "2026-10-08T04:03:00Z",
    "2026-10-08T04:04:00Z",
    "2026-10-08T04:05:00Z",
];
// Episodes 4 and 5 are added by the growth tests, after the six base messages.
const LCM_EXTRA_TIMES: [&str; 4] = [
    "2026-10-08T04:06:00Z",
    "2026-10-08T04:07:00Z",
    "2026-10-08T04:08:00Z",
    "2026-10-08T04:09:00Z",
];
const LCM_EPISODE_TITLES: [&str; 5] = [
    "Episode one title",
    "Episode two title",
    "Episode three title",
    "Episode four title",
    "Episode five title",
];
const LCM_EPISODE_SUMMARIES: [&str; 5] = [
    "Episode one summary",
    "Episode two summary",
    "Episode three summary",
    "Episode four summary",
    "Episode five summary",
];
const LCM_FIRST_DESCRIPTION: &str = "First chapter description";
const LCM_FIRST_SUMMARY: &str = "First chapter summary";
const LCM_SECOND_DESCRIPTION: &str = "Second chapter description";
const LCM_SECOND_SUMMARY: &str = "Second chapter summary";
const LCM_GROWN_DESCRIPTION: &str = "Grown chapter description";
const LCM_GROWN_SUMMARY: &str = "Grown chapter summary";

struct Lcm {
    fixture: Fixture,
    session: String,
    messages: Vec<Message>,
}

impl Lcm {
    fn new(close_last: bool) -> Self {
        let fixture = Fixture::new();
        let session = fixture.agent.create_session("lcm fixture").unwrap().id;
        let messages: Vec<Message> = LCM_TIMES
            .iter()
            .enumerate()
            .map(|(index, time)| {
                let role = if index.is_multiple_of(2) {
                    Role::User
                } else {
                    Role::Assistant
                };
                fixture.message(&session, role, time)
            })
            .collect();

        let store = Store::open(fixture.agent_dir().join("agent.sqlite")).unwrap();
        lcm_save(
            &store,
            &session,
            1,
            &messages[0..2],
            lcm_open(&session, 1, 1, 1),
            None,
            None,
        );
        let first_closed = lcm_close(
            lcm_open(&session, 1, 1, 2),
            LCM_FIRST_DESCRIPTION,
            LCM_FIRST_SUMMARY,
        );
        if close_last {
            lcm_save(
                &store,
                &session,
                2,
                &messages[2..4],
                lcm_open(&session, 1, 1, 2),
                None,
                Some(first_closed),
            );
            lcm_save(
                &store,
                &session,
                3,
                &messages[4..6],
                lcm_open(&session, 2, 3, 3),
                None,
                Some(lcm_close(
                    lcm_open(&session, 2, 3, 3),
                    LCM_SECOND_DESCRIPTION,
                    LCM_SECOND_SUMMARY,
                )),
            );
        } else {
            lcm_save(
                &store,
                &session,
                2,
                &messages[2..4],
                lcm_open(&session, 1, 1, 2),
                None,
                None,
            );
            lcm_save(
                &store,
                &session,
                3,
                &messages[4..6],
                lcm_open(&session, 2, 3, 3),
                Some(first_closed),
                None,
            );
        }
        Self {
            fixture,
            session,
            messages,
        }
    }

    fn agent(&self) -> &Agent {
        &self.fixture.agent
    }

    fn store(&self) -> Store {
        Store::open(self.fixture.agent_dir().join("agent.sqlite")).unwrap()
    }

    // `sessions` refreshes automatic titles before it lists them.
    fn title(&self) -> String {
        self.agent()
            .sessions()
            .unwrap()
            .into_iter()
            .find(|session| session.id == self.session)
            .unwrap()
            .title
    }

    fn save_notes_outline(&self, chapters: &[OutlineChapter]) {
        let mut outline_db = OutlineStore::open(
            &self
                .fixture
                .agent_dir()
                .join(vault_tools::outline::FILE_NAME),
        )
        .unwrap();
        outline_db.save_session(&self.session, chapters).unwrap();
    }

    // Seeded after the episodes, so the legacy range overlaps LCM coverage.
    fn seed_legacy_chapter(&self) {
        self.store()
            .save_context_chapter(&ContextChapter {
                id: "legacy-chapter".into(),
                session_id: self.session.clone(),
                ordinal: 1,
                first_message_id: self.messages[0].id.clone(),
                last_message_id: self.messages[5].id.clone(),
                title: "Legacy title".into(),
                summary: "Legacy summary".into(),
                model: "scripted/scripted".into(),
                usage: None,
                created_at: "2026-10-08T04:06:00Z".into(),
            })
            .unwrap();
    }

    // Appends messages after the current tail and returns their range in `messages`.
    fn append_messages(&mut self, times: &[&str]) -> std::ops::Range<usize> {
        let start = self.messages.len();
        for (offset, time) in times.iter().enumerate() {
            let role = if (start + offset).is_multiple_of(2) {
                Role::User
            } else {
                Role::Assistant
            };
            let message = self.fixture.message(&self.session, role, time);
            self.messages.push(message);
        }
        start..self.messages.len()
    }

    // Episode 4 joins open chapter 2 and leaves it open.
    fn save_open_episode_four(&mut self) {
        let range = self.append_messages(&LCM_EXTRA_TIMES[0..2]);
        lcm_save(
            &self.store(),
            &self.session,
            4,
            &self.messages[range],
            lcm_open(&self.session, 2, 3, 4),
            None,
            None,
        );
    }

    // Episode 5 is written into chapter 2 and closes it in the same batch.
    fn save_episode_five_closing_chapter_two(&mut self) {
        let range = self.append_messages(&LCM_EXTRA_TIMES[2..4]);
        lcm_save(
            &self.store(),
            &self.session,
            5,
            &self.messages[range],
            lcm_open(&self.session, 2, 3, 5),
            None,
            Some(lcm_close(
                lcm_open(&self.session, 2, 3, 5),
                LCM_GROWN_DESCRIPTION,
                LCM_GROWN_SUMMARY,
            )),
        );
    }
}

fn lcm_open(
    session: &str,
    ordinal: usize,
    first_episode: usize,
    last_episode: usize,
) -> ContextSubjectChapter {
    ContextSubjectChapter {
        id: format!("{session}:subject:{ordinal}"),
        session_id: session.into(),
        ordinal,
        first_episode_ordinal: first_episode,
        last_episode_ordinal: last_episode,
        status: SubjectChapterStatus::Open,
        description: None,
        summary: None,
        related_ids: Vec::new(),
        related_evidence: Vec::new(),
        model: None,
        usage: None,
        reasoning: None,
    }
}

fn lcm_close(
    mut chapter: ContextSubjectChapter,
    description: &str,
    summary: &str,
) -> ContextSubjectChapter {
    chapter.status = SubjectChapterStatus::Closed;
    chapter.description = Some(description.into());
    chapter.summary = Some(summary.into());
    chapter.model = Some("chapter-model".into());
    chapter
}

// Writes one episode through the SDK; the coverage token comes from the store itself.
fn lcm_save(
    store: &Store,
    session: &str,
    ordinal: usize,
    range: &[Message],
    open_chapter: ContextSubjectChapter,
    closed_chapter: Option<ContextSubjectChapter>,
    close_after_episode: Option<ContextSubjectChapter>,
) {
    let episode = ContextEpisode {
        id: format!("{session}:episode:{ordinal}"),
        session_id: session.into(),
        ordinal,
        first_message_id: range[0].id.clone(),
        last_message_id: range[range.len() - 1].id.clone(),
        source_tokens: 1_000,
        cap: 200,
        origin_request_id: None,
        title: LCM_EPISODE_TITLES[ordinal - 1].into(),
        summary: LCM_EPISODE_SUMMARIES[ordinal - 1].into(),
        model: "summary-model".into(),
        usage: None,
        reasoning: None,
        created_at: range[range.len() - 1].created_at.clone(),
        chapter_id: open_chapter.id.clone(),
    };
    let batch = EpisodeBatch {
        episode,
        decisions: Vec::new(),
        open_chapter,
        closed_chapter,
        close_after_episode,
        expected_previous_coverage: store.episode_coverage_token(session).unwrap(),
    };
    store.save_episode_batch(&batch).unwrap();
}

fn assert_section(
    section: &OutlineSection,
    episode: &ContextEpisode,
    first: &Message,
    last: &Message,
) {
    assert_eq!(section.id, episode.id);
    assert_eq!(section.title, episode.title);
    assert_eq!(section.summary, episode.summary);
    assert_eq!(section.first_message_id, first.id);
    assert_eq!(section.last_message_id, last.id);
}

#[test]
fn lcm_outline_builds_chapters_and_episode_sections_from_the_sdk_store() {
    let lcm = Lcm::new(false);
    let m = &lcm.messages;
    let store = lcm.store();
    let episodes = store.context_episodes(&lcm.session).unwrap();
    let subjects = store.context_subject_chapters(&lcm.session).unwrap();
    assert_eq!(episodes.len(), 3);
    assert_eq!(subjects.len(), 2);

    let outline = lcm.agent().session_outline(&lcm.session).unwrap();
    assert_eq!(outline.chapters.len(), 2, "chapters");

    // Chapter 1 is closed: its title is the description, its summary the closing summary.
    let first = &outline.chapters[0];
    assert_eq!(first.id, subjects[0].id);
    assert_eq!(first.title, LCM_FIRST_DESCRIPTION);
    assert_eq!(first.summary, LCM_FIRST_SUMMARY);
    assert!(!first.ongoing);
    assert_eq!(first.first_message_id, m[0].id);
    assert_eq!(first.last_message_id, m[3].id);
    assert_eq!(first.started_at, LCM_TIMES[0]);
    assert_eq!(first.ended_at.as_deref(), Some(LCM_TIMES[3]));
    assert_eq!(first.sections.len(), 2, "episode sections");
    assert_section(&first.sections[0], &episodes[0], &m[0], &m[1]);
    assert_section(&first.sections[1], &episodes[1], &m[2], &m[3]);

    // Chapter 2 is open: its title is the first episode's title and its summary is empty.
    let second = &outline.chapters[1];
    assert_eq!(second.id, subjects[1].id);
    assert_eq!(second.title, LCM_EPISODE_TITLES[2]);
    assert!(second.summary.is_empty(), "an open chapter has no summary");
    assert!(second.ongoing);
    assert_eq!(second.first_message_id, m[4].id);
    assert_eq!(second.last_message_id, m[5].id);
    assert_eq!(second.started_at, LCM_TIMES[4]);
    assert_eq!(second.ended_at, None);
    assert_eq!(second.sections.len(), 1);
    assert_section(&second.sections[0], &episodes[2], &m[4], &m[5]);

    assert_eq!(
        session_outline_at(&lcm.fixture.data_dir(), &lcm.session).unwrap(),
        outline
    );
}

#[test]
fn lcm_outline_reports_a_closed_last_chapter() {
    let lcm = Lcm::new(true);
    let outline = lcm.agent().session_outline(&lcm.session).unwrap();
    assert_eq!(outline.chapters.len(), 2, "chapters");
    assert!(outline.chapters.iter().all(|c| !c.ongoing));

    let last = &outline.chapters[1];
    assert_eq!(last.title, LCM_SECOND_DESCRIPTION);
    assert_eq!(last.summary, LCM_SECOND_SUMMARY);
    assert_eq!(last.first_message_id, lcm.messages[4].id);
    assert_eq!(last.last_message_id, lcm.messages[5].id);
    assert_eq!(last.started_at, LCM_TIMES[4]);
    assert_eq!(last.ended_at.as_deref(), Some(LCM_TIMES[5]));
    assert_eq!(last.sections.len(), 1);
    assert_eq!(last.sections[0].title, LCM_EPISODE_TITLES[2]);
}

#[test]
fn notes_outline_wins_over_lcm_episodes_including_a_saved_empty_one() {
    let lcm = Lcm::new(false);
    let m = &lcm.messages;
    lcm.save_notes_outline(&[notes_chapter(&m[0].id, &m[5].id)]);
    let outline = lcm.agent().session_outline(&lcm.session).unwrap();
    assert_eq!(outline.chapters.len(), 1);
    assert_eq!(outline.chapters[0].id, "notes-chapter");

    lcm.save_notes_outline(&[]);
    assert_eq!(
        lcm.agent().session_outline(&lcm.session).unwrap(),
        ChatOutline::default()
    );
}

#[test]
fn lcm_outline_wins_over_a_seeded_legacy_chapter() {
    let lcm = Lcm::new(false);
    let subjects = lcm.store().context_subject_chapters(&lcm.session).unwrap();
    lcm.seed_legacy_chapter();
    let outline = lcm.agent().session_outline(&lcm.session).unwrap();
    let ids: Vec<&str> = outline.chapters.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        [subjects[0].id.as_str(), subjects[1].id.as_str()],
        "the legacy chapter shows only when no LCM chapter exists"
    );
    assert!(outline.chapters.iter().all(|c| c.title != "Legacy title"));
}

#[test]
fn chat_title_takes_the_open_chapters_first_episode_title() {
    let lcm = Lcm::new(false);
    assert_eq!(lcm.title(), LCM_EPISODE_TITLES[2]);
}

#[test]
fn chat_title_takes_the_closed_chapters_description() {
    let lcm = Lcm::new(true);
    assert_eq!(lcm.title(), LCM_SECOND_DESCRIPTION);
}

#[test]
fn explicit_rename_stays_sticky_over_lcm_titles() {
    let lcm = Lcm::new(false);
    lcm.agent()
        .rename_session(&lcm.session, "Named by the user")
        .unwrap();
    assert_eq!(lcm.title(), "Named by the user");

    let m = &lcm.messages;
    lcm.save_notes_outline(&[notes_chapter(&m[0].id, &m[5].id)]);
    assert_eq!(lcm.title(), "Named by the user");
}

#[test]
fn notes_outline_title_wins_over_the_lcm_title() {
    let lcm = Lcm::new(false);
    let m = &lcm.messages;
    lcm.save_notes_outline(&[notes_chapter(&m[0].id, &m[5].id)]);
    assert_eq!(lcm.title(), "Notes chapter");
}

#[test]
fn open_chapter_keeps_its_first_episode_title_until_the_writer_closes_it() {
    let mut lcm = Lcm::new(false);
    lcm.save_open_episode_four();

    // Episode 4 has a different title, but the open chapter keeps episode 3's.
    let outline = lcm.agent().session_outline(&lcm.session).unwrap();
    assert_eq!(outline.chapters.len(), 2, "chapters");
    let open = &outline.chapters[1];
    assert_eq!(open.title, LCM_EPISODE_TITLES[2]);
    assert!(open.summary.is_empty(), "an open chapter has no summary");
    assert!(open.ongoing);
    assert_eq!(open.ended_at, None);
    assert_eq!(open.first_message_id, lcm.messages[4].id);
    assert_eq!(open.last_message_id, lcm.messages[7].id);
    assert_eq!(open.sections.len(), 2, "episode sections");
    let episodes = lcm.store().context_episodes(&lcm.session).unwrap();
    assert_section(
        &open.sections[0],
        &episodes[2],
        &lcm.messages[4],
        &lcm.messages[5],
    );
    assert_section(
        &open.sections[1],
        &episodes[3],
        &lcm.messages[6],
        &lcm.messages[7],
    );
    assert_eq!(lcm.title(), LCM_EPISODE_TITLES[2]);

    // Episode 5 closes the same chapter through the writer: its title becomes the description.
    lcm.save_episode_five_closing_chapter_two();
    let outline = lcm.agent().session_outline(&lcm.session).unwrap();
    assert_eq!(outline.chapters.len(), 2, "chapters");
    let closed = &outline.chapters[1];
    assert_eq!(closed.title, LCM_GROWN_DESCRIPTION);
    assert_eq!(closed.summary, LCM_GROWN_SUMMARY);
    assert!(!closed.ongoing);
    assert_eq!(closed.ended_at.as_deref(), Some(LCM_EXTRA_TIMES[3]));
    assert_eq!(closed.last_message_id, lcm.messages[9].id);
    assert_eq!(closed.sections.len(), 3, "episode sections");
    assert_eq!(lcm.title(), LCM_GROWN_DESCRIPTION);
}

#[test]
fn lcm_automatic_title_beats_a_seeded_legacy_chapter() {
    let lcm = Lcm::new(false);
    lcm.seed_legacy_chapter();
    assert_eq!(lcm.title(), LCM_EPISODE_TITLES[2]);
}

#[test]
fn saved_empty_notes_outline_suppresses_lcm_renaming() {
    let mut lcm = Lcm::new(false);
    lcm.save_notes_outline(&[]);
    assert_eq!(
        lcm.agent().session_outline(&lcm.session).unwrap(),
        ChatOutline::default()
    );
    assert_eq!(lcm.title(), "lcm fixture");

    // Later LCM episodes still do not rename the chat past a saved empty notes outline.
    lcm.save_open_episode_four();
    assert_eq!(
        lcm.agent().session_outline(&lcm.session).unwrap(),
        ChatOutline::default()
    );
    assert_eq!(lcm.title(), "lcm fixture");
}
