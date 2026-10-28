use std::sync::Arc;

use vault_agent::bubblepaw_sdk::core::{
    ContextCheckpoint, ContextEpisode, ContextSubjectChapter, EpisodeBatch, SubjectChapterStatus,
};
use vault_agent::bubblepaw_sdk::store::{ContextChapter, Message, MessageOrigin, Role, Store};
use vault_agent::compacted_history::{
    ChapterStatus, CompactedHistory, HistorySegment, LegacyKind, MessageRange, TargetKind,
};
use vault_agent::{
    read_session_compacted_history, session_compacted_history_at, Agent, AgentConfig, TestHooks,
};
use vault_core::Vault;

const V3: &str = "# Fix storage\nFixed the `storage` reader.\nActions:\n- Read https://example.com/spec [r1]\n- Edited src/store.rs · +4 −2 [e1] ! correction: repaired boundary\n- Ran cargo test · failed 2 [t1] ! error: fixture mismatch\n- Asked whether to retry ! question: waiting for answer\n- Searched 500 files\n- nonsense skipped\nOpen: none";
const V2: &str = "# Earlier investigation\n\n## Goal and constraints\nInvestigate the old reader.\n\n## Results\n- Original legacy prose.\nOpen: none";

struct Fixture {
    dir: tempfile::TempDir,
    agent: Agent,
    store: Store,
    session: String,
    messages: Vec<Message>,
}

impl Fixture {
    fn new(count: usize) -> Self {
        Self::with_notice(count, None)
    }

    fn with_notice(count: usize, notice: Option<usize>) -> Self {
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
        let store = Store::open(dir.path().join("data/agent/agent.sqlite")).unwrap();
        let session = agent.create_session("history fixture").unwrap().id;
        let messages: Vec<_> = (0..count)
            .map(|i| Message {
                id: format!("message-{i}"),
                session_id: session.clone(),
                role: if i.is_multiple_of(2) || notice == Some(i) {
                    Role::User
                } else {
                    Role::Assistant
                },
                content: if notice == Some(i) {
                    "[host notice] context changed".into()
                } else {
                    format!("canonical {i}")
                },
                tool_calls: vec![],
                tool_call_id: None,
                provider_state: None,
                origin: (notice == Some(i)).then_some(MessageOrigin::Generated),
                is_error: false,
                created_at: format!("2026-10-28T10:{i:02}:00Z"),
            })
            .collect();
        store.append_batch(&messages).unwrap();
        Self {
            dir,
            agent,
            store,
            session,
            messages,
        }
    }

    fn read(&self) -> CompactedHistory {
        read_session_compacted_history(&self.store, &self.session).unwrap()
    }

    fn checkpoint(&self, end: usize) {
        self.store
            .save_context_checkpoint(&ContextCheckpoint {
                id: "checkpoint".into(),
                session_id: self.session.clone(),
                through_message_id: self.messages[end - 1].id.clone(),
                model: "test".into(),
                summary: "Checkpoint summary".into(),
                files_touched: vec![],
                created_at: "2026-10-28T11:00:00Z".into(),
                usage: None,
            })
            .unwrap();
    }

    fn legacy(&self, start: usize, end: usize) {
        self.store
            .save_context_chapter(&ContextChapter {
                id: "legacy".into(),
                session_id: self.session.clone(),
                ordinal: 1,
                first_message_id: self.messages[start].id.clone(),
                last_message_id: self.messages[end - 1].id.clone(),
                title: "Legacy title".into(),
                summary: "Legacy summary".into(),
                model: "test".into(),
                usage: None,
                created_at: "2026-10-28T11:01:00Z".into(),
            })
            .unwrap();
    }

    fn episode(
        &self,
        ordinal: usize,
        range: std::ops::Range<usize>,
        chapter: usize,
        first_episode: usize,
        close: bool,
        summary: &str,
    ) {
        let open = ContextSubjectChapter {
            id: format!("chapter-{chapter}"),
            session_id: self.session.clone(),
            ordinal: chapter,
            first_episode_ordinal: first_episode,
            last_episode_ordinal: ordinal,
            status: SubjectChapterStatus::Open,
            description: None,
            summary: None,
            related_ids: vec![],
            related_evidence: vec![],
            model: None,
            usage: None,
            reasoning: None,
        };
        let closed = close.then(|| {
            let mut closed = open.clone();
            closed.status = SubjectChapterStatus::Closed;
            closed.description = Some(format!("Chapter {chapter} description"));
            closed.summary = Some(format!("Chapter {chapter} summary"));
            closed.model = Some("test".into());
            closed
        });
        self.store
            .save_episode_batch(&EpisodeBatch {
                episode: ContextEpisode {
                    id: format!("episode-{ordinal}"),
                    session_id: self.session.clone(),
                    ordinal,
                    first_message_id: self.messages[range.start].id.clone(),
                    last_message_id: self.messages[range.end - 1].id.clone(),
                    source_tokens: 100,
                    cap: 40,
                    origin_request_id: None,
                    title: format!("Episode {ordinal}"),
                    summary: summary.into(),
                    model: "test".into(),
                    usage: None,
                    reasoning: None,
                    created_at: "2026-10-28T12:00:00Z".into(),
                    chapter_id: open.id.clone(),
                },
                decisions: vec![],
                open_chapter: open,
                closed_chapter: None,
                close_after_episode: closed,
                expected_previous_coverage: self
                    .store
                    .episode_coverage_token(&self.session)
                    .unwrap(),
            })
            .unwrap();
    }

    fn five_episodes(&self, offset: usize) {
        self.episode(1, offset..offset + 2, 1, 1, false, V3);
        self.episode(2, offset + 2..offset + 4, 1, 1, true, V2);
        self.episode(3, offset + 4..offset + 6, 2, 3, false, V3);
        self.episode(4, offset + 6..offset + 8, 2, 3, true, V2);
        self.episode(5, offset + 8..offset + 10, 3, 5, false, V3);
    }

    // Import/corruption fixture: the real writer correctly refuses gaps/overlaps.
    // Only the selected episode's first anchor changes; all other real-writer rows
    // and canonical ordering remain intact.
    fn change_episode_start(&self, ordinal: usize, start: usize) {
        let conn =
            rusqlite::Connection::open(self.store.chat_path(&self.session).unwrap().unwrap())
                .unwrap();
        conn.execute("UPDATE context_episodes SET first_message_seq=(SELECT seq FROM messages WHERE message_id=?1),data=json_set(data,'$.first_message_id',?1) WHERE ordinal=?2", rusqlite::params![self.messages[start].id, i64::try_from(ordinal).unwrap()]).unwrap();
    }
}

fn range(start: usize, end: usize) -> MessageRange {
    MessageRange { start, end }
}

#[test]
fn real_writer_projects_two_closed_chapters_one_open_and_five_mixed_episodes() {
    let f = Fixture::new(12);
    f.five_episodes(0);
    let original_title = f.store.session(&f.session).unwrap().title;
    let history = f.read();
    assert!(history.compactable);
    assert_eq!(history.boundary, 10);
    assert_eq!(history.message_count, 12);
    assert_eq!(history.segments.len(), 4);
    for (index, segment) in history.segments[..3].iter().enumerate() {
        let HistorySegment::ChapterCard(card) = segment else {
            panic!("expected chapter")
        };
        let (start, end, count) = [(0, 4, 2), (4, 8, 2), (8, 10, 1)][index];
        assert_eq!(card.id, format!("chapter-{}", index + 1));
        assert_eq!(
            card.status,
            if index == 2 {
                ChapterStatus::Open
            } else {
                ChapterStatus::Closed
            }
        );
        assert_eq!(
            card.title,
            if index == 2 {
                "Episode 5".into()
            } else {
                format!("Chapter {} description", index + 1)
            }
        );
        assert_eq!(
            card.summary,
            (index != 2).then(|| format!("Chapter {} summary", index + 1))
        );
        assert_eq!(card.range, range(start, end));
        assert_eq!((card.part, card.parts), (1, 1));
        assert_eq!(card.message_count, Some(end - start));
        assert_eq!(card.episode_count, Some(count));
        assert_eq!(card.episodes.len(), count);
        assert_eq!(card.started_at, f.messages[start].created_at);
        assert_eq!(card.ended_at, f.messages[end - 1].created_at);
        assert_eq!(card.first_message_id, f.messages[start].id);
        assert_eq!(card.last_message_id, f.messages[end - 1].id);
        for (local, episode) in card.episodes.iter().enumerate() {
            let ordinal = start / 2 + local + 1;
            let a = (ordinal - 1) * 2;
            assert_eq!(episode.id, format!("episode-{ordinal}"));
            assert_eq!(episode.title, format!("Episode {ordinal}"));
            assert_eq!(episode.range, range(a, a + 2));
            assert_eq!(episode.message_count, 2);
            assert_eq!(episode.started_at, f.messages[a].created_at);
            assert_eq!(episode.ended_at, f.messages[a + 1].created_at);
            assert_eq!(episode.first_message_id, f.messages[a].id);
            assert_eq!(episode.last_message_id, f.messages[a + 1].id);
            if ordinal.is_multiple_of(2) {
                assert_eq!(episode.gist.as_deref(), Some("Investigate the old reader."));
                assert!(episode.actions.is_empty());
                assert_eq!(
                    (
                        episode.action_count,
                        episode.flagged_count,
                        episode.error_count
                    ),
                    (0, 0, 0)
                );
            } else {
                assert_eq!(episode.gist.as_deref(), Some("Fixed the storage reader."));
                assert_eq!(
                    (
                        episode.action_count,
                        episode.flagged_count,
                        episode.error_count
                    ),
                    (5, 3, 1)
                );
                let actions = &episode.actions;
                assert_eq!(actions[0].verb, "Read");
                assert_eq!(actions[0].target, "https://example.com/spec");
                assert_eq!(actions[0].target_kind, TargetKind::Page);
                assert_eq!(actions[0].change_marker, None);
                assert_eq!(actions[0].point_of_interest, None);
                assert_eq!(actions[1].target_kind, TargetKind::Path);
                assert_eq!(actions[1].change_marker.as_deref(), Some("+4 −2"));
                let poi = actions[1].point_of_interest.as_ref().unwrap();
                assert_eq!(poi.kind, "correction");
                assert_eq!(poi.reason, "repaired boundary");
                assert_eq!(actions[2].target_kind, TargetKind::Command);
                assert_eq!(actions[2].change_marker.as_deref(), Some("failed 2"));
                assert_eq!(actions[3].target_kind, TargetKind::Question);
                assert_eq!(actions[4].target_kind, TargetKind::Other);
                assert_eq!(actions[4].target, "500 files");
            }
        }
    }
    assert_eq!(history.segments[3], HistorySegment::Messages(range(10, 12)));
    assert_eq!(
        f.agent.session_compacted_history(&f.session).unwrap(),
        history
    );
    assert_eq!(
        session_compacted_history_at(&f.dir.path().join("data"), &f.session).unwrap(),
        history
    );
    assert!(!f.dir.path().join("data/agent/chat_outline.sqlite").exists());
    assert_eq!(f.store.session(&f.session).unwrap().title, original_title);
}

#[test]
fn saved_notes_outline_is_not_a_compaction_source() {
    let f = Fixture::new(12);
    f.five_episodes(0);
    let before = f.read();
    let path = f.dir.path().join("data/agent/chat_outline.sqlite");
    vault_tools::outline::OutlineStore::open(&path)
        .unwrap()
        .save_session(&f.session, &[])
        .unwrap();
    assert_eq!(f.read(), before);
}

#[test]
fn migration_retains_checkpoint_legacy_chapter_and_lcm_in_order() {
    let f = Fixture::new(15);
    f.checkpoint(2);
    f.legacy(2, 4);
    f.five_episodes(4);
    let h = f.read();
    assert!(h.compactable);
    assert_eq!(h.boundary, 14);
    assert_eq!(h.segments.len(), 6);
    let HistorySegment::LegacyCard(checkpoint) = &h.segments[0] else {
        panic!("checkpoint")
    };
    assert_eq!(checkpoint.id, "checkpoint");
    assert_eq!(checkpoint.kind, LegacyKind::Checkpoint);
    assert_eq!(checkpoint.title, None);
    assert_eq!(checkpoint.summary, "Checkpoint summary");
    assert_eq!(checkpoint.range, range(0, 2));
    assert_eq!(checkpoint.message_count, 2);
    assert_eq!(checkpoint.started_at, f.messages[0].created_at);
    assert_eq!(checkpoint.ended_at, f.messages[1].created_at);
    assert_eq!(checkpoint.first_message_id, f.messages[0].id);
    assert_eq!(checkpoint.last_message_id, f.messages[1].id);
    let HistorySegment::LegacyCard(legacy) = &h.segments[1] else {
        panic!("legacy")
    };
    assert_eq!(legacy.kind, LegacyKind::Chapter);
    assert_eq!(legacy.title.as_deref(), Some("Legacy title"));
    assert_eq!(legacy.summary, "Legacy summary");
    assert_eq!(legacy.range, range(2, 4));
    assert_eq!(
        h.segments.last(),
        Some(&HistorySegment::Messages(range(14, 15)))
    );
}

#[test]
fn checkpoint_only_and_legacy_only_keep_a_trailing_raw_range() {
    let f = Fixture::new(4);
    f.checkpoint(2);
    let h = f.read();
    assert!(h.compactable);
    assert_eq!(h.boundary, 2);
    assert_eq!(h.segments.len(), 2);
    assert_eq!(h.segments[1], HistorySegment::Messages(range(2, 4)));
    let f = Fixture::new(4);
    f.legacy(0, 4);
    let h = f.read();
    assert!(h.compactable);
    assert_eq!(h.boundary, 4);
    assert_eq!(h.segments[1], HistorySegment::Messages(range(4, 4)));
}

#[test]
fn empty_and_uncompacted_sessions_have_one_raw_range_and_no_boundary() {
    for count in [0, 3] {
        let f = Fixture::new(count);
        assert_eq!(
            f.read(),
            CompactedHistory {
                compactable: true,
                boundary: 0,
                message_count: count,
                message_ids: f.messages.iter().map(|m| m.id.clone()).collect(),
                segments: vec![HistorySegment::Messages(range(0, count))],
            }
        );
    }
    let dir = tempfile::tempdir().unwrap();
    assert!(session_compacted_history_at(dir.path(), "missing").is_err());
    assert!(!dir.path().join("agent").exists());
}

#[test]
fn checkpoint_can_be_followed_directly_by_lcm_without_a_legacy_chapter() {
    let f = Fixture::new(13);
    f.checkpoint(2);
    f.five_episodes(2);
    let h = f.read();
    assert!(h.compactable);
    assert_eq!(h.message_count, 13);
    assert_eq!(h.boundary, 12);
    assert_eq!(h.segments.len(), 5);
    let HistorySegment::LegacyCard(checkpoint) = &h.segments[0] else {
        panic!("checkpoint")
    };
    assert_eq!(checkpoint.range, range(0, 2));
    let HistorySegment::ChapterCard(first) = &h.segments[1] else {
        panic!("chapter")
    };
    assert_eq!(first.range, range(2, 6));
    assert_eq!(h.segments[4], HistorySegment::Messages(range(12, 13)));
}

#[test]
fn canonical_ids_resolve_raw_ranges_when_app_messages_hide_generated_notices() {
    let f = Fixture::with_notice(7, Some(4));
    f.episode(1, 0..2, 1, 1, true, V3);
    f.episode(2, 2..4, 2, 2, false, V2);
    let h = f.read();
    assert_eq!(h.message_count, 7);
    assert_eq!(
        h.message_ids,
        f.messages.iter().map(|m| m.id.clone()).collect::<Vec<_>>()
    );
    assert_eq!(h.boundary, 4);
    assert_eq!(h.segments[2], HistorySegment::Messages(range(4, 7)));
    let app_messages = f.agent.session_state(&f.session).unwrap().messages;
    assert_eq!(app_messages.len(), 6);
    let raw_ids = &h.message_ids[4..7];
    assert_eq!(raw_ids[0], f.messages[4].id);
    let visible_raw: Vec<_> = app_messages
        .iter()
        .filter(|m| raw_ids.contains(&m.id))
        .map(|m| m.id.clone())
        .collect();
    assert_eq!(
        visible_raw,
        [f.messages[5].id.clone(), f.messages[6].id.clone()]
    );
}

#[test]
fn targets_use_explicit_page_prefix_and_deterministic_read_paths() {
    let f = Fixture::new(2);
    f.episode(1, 0..2, 1, 1, false, "# Targets\nRead references.\nActions:\n- Read README.md\n- Searched https://example.com\n- Read 6 files in crates/x\n- Edited page: Notes/Plan\n- Edited the note\nOpen: none");
    let h = f.read();
    let HistorySegment::ChapterCard(chapter) = &h.segments[0] else {
        panic!("chapter")
    };
    let actions = &chapter.episodes[0].actions;
    assert_eq!(actions[0].target_kind, TargetKind::Path);
    assert_eq!(actions[1].target_kind, TargetKind::Page);
    assert_eq!(actions[2].target_kind, TargetKind::Other);
    assert_eq!(actions[3].target_kind, TargetKind::Page);
    assert_eq!(actions[3].target, "page: Notes/Plan");
    assert_eq!(actions[4].target_kind, TargetKind::Other);
}

#[test]
fn internal_episode_gap_splits_chapter_parts_and_keeps_totals_on_last_part() {
    let f = Fixture::new(12);
    f.five_episodes(0);
    f.change_episode_start(2, 3);
    let h = f.read();
    assert!(h.compactable);
    assert_eq!(h.boundary, 10);
    assert_eq!(h.message_count, 12);
    assert_eq!(h.segments.len(), 6);
    let HistorySegment::ChapterCard(first) = &h.segments[0] else {
        panic!("first part")
    };
    assert_eq!(h.segments[1], HistorySegment::Messages(range(2, 3)));
    let HistorySegment::ChapterCard(last) = &h.segments[2] else {
        panic!("last part")
    };
    assert_eq!(first.id, "chapter-1");
    assert_eq!(last.id, first.id);
    assert_eq!(first.title, "Chapter 1 description");
    assert_eq!(last.title, first.title);
    assert_eq!(first.summary.as_deref(), Some("Chapter 1 summary"));
    assert_eq!(last.summary, first.summary);
    assert_eq!(first.status, ChapterStatus::Closed);
    assert_eq!(last.status, first.status);
    assert_eq!((first.part, first.parts), (1, 2));
    assert_eq!((last.part, last.parts), (2, 2));
    assert_eq!(first.range, range(0, 2));
    assert_eq!(last.range, range(3, 4));
    assert_eq!((first.message_count, first.episode_count), (None, None));
    assert_eq!((last.message_count, last.episode_count), (Some(3), Some(2)));
    for (part, start, end, episode_id) in [(first, 0, 2, "episode-1"), (last, 3, 4, "episode-2")] {
        assert_eq!(part.started_at, f.messages[start].created_at);
        assert_eq!(part.ended_at, f.messages[end - 1].created_at);
        assert_eq!(part.first_message_id, f.messages[start].id);
        assert_eq!(part.last_message_id, f.messages[end - 1].id);
        assert_eq!(part.episodes.len(), 1);
        assert_eq!(part.episodes[0].id, episode_id);
        assert_eq!(part.episodes[0].range, range(start, end));
    }
    for (index, id, expected_range) in [
        (3, "chapter-2", range(4, 8)),
        (4, "chapter-3", range(8, 10)),
    ] {
        let HistorySegment::ChapterCard(chapter) = &h.segments[index] else {
            panic!("later chapter")
        };
        assert_eq!(chapter.id, id);
        assert_eq!(chapter.range, expected_range);
        assert_eq!((chapter.part, chapter.parts), (1, 1));
        assert_eq!(
            chapter.message_count,
            Some(expected_range.end - expected_range.start)
        );
    }
    assert_eq!(h.segments[5], HistorySegment::Messages(range(10, 12)));
}

#[test]
fn imported_gap_between_chapters_becomes_raw_messages() {
    let f = Fixture::new(12);
    f.five_episodes(0);
    f.change_episode_start(3, 5);
    let h = f.read();
    assert!(h.compactable);
    assert_eq!(h.segments[1], HistorySegment::Messages(range(4, 5)));
    assert_eq!(h.boundary, 10);
}

#[test]
fn overlapping_episode_chapter_and_legacy_ranges_disable_compaction() {
    for (ordinal, start) in [(2, 1), (3, 3)] {
        let f = Fixture::new(12);
        f.five_episodes(0);
        f.change_episode_start(ordinal, start);
        let h = f.read();
        assert!(!h.compactable);
        assert_eq!(h.boundary, 0);
        assert_eq!(h.message_count, 12);
        assert!(h.segments.is_empty());
    }
    let f = Fixture::new(12);
    f.five_episodes(0);
    f.legacy(0, 2);
    let h = f.read();
    assert!(!h.compactable);
    assert!(h.segments.is_empty());
}
