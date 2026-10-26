//! The dev importer on a synthetic replay. Harness-written user text and the
//! orphan reply before it are dropped, the assistant reply after a harness user
//! stays, anchors into dropped messages resolve to kept ones, and `split`
//! decisions are not imported.

use std::path::{Path, PathBuf};
use std::process::Command;

use bubblepaw_core::{Message, Role, Store};
use rusqlite::Connection;
use serde_json::json;
use vault_tools::outline::{self, OutlineStore};

struct Temp(PathBuf);

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn role_name(role: &Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
        Role::System => "system",
    }
}

fn message(id: &str, role: Role, content: &str, seq: usize) -> Message {
    Message {
        id: id.into(),
        session_id: "source".into(),
        role,
        content: content.into(),
        tool_calls: vec![],
        tool_call_id: None,
        provider_state: None,
        origin: None,
        is_error: false,
        created_at: format!("2026-10-10T10:00:{seq:02}.000+00:00"),
    }
}

/// Source seqs 1-8: s1 leading harness user, s2 its orphan reply, s3 and s4 a real
/// turn, s5 a harness environment block with s6 its reply, s7 and s8 a second turn.
/// Episode 1 starts on the skipped s1; the split decision names the skipped s5.
fn replay_file(dir: &Path) -> PathBuf {
    let path = dir.join("replay.sqlite");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE sessions(id TEXT PRIMARY KEY, data TEXT NOT NULL);
         CREATE TABLE messages(seq INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, data TEXT NOT NULL);
         CREATE TABLE context_subject_chapters(id TEXT PRIMARY KEY, session_id TEXT NOT NULL, ordinal INTEGER NOT NULL, first_episode_ordinal INTEGER NOT NULL, last_episode_ordinal INTEGER NOT NULL, status TEXT NOT NULL, data TEXT NOT NULL);
         CREATE TABLE context_episodes(id TEXT PRIMARY KEY, session_id TEXT NOT NULL, ordinal INTEGER NOT NULL, first_message_seq INTEGER NOT NULL, last_message_seq INTEGER NOT NULL, chapter_id TEXT NOT NULL, data TEXT NOT NULL);
         CREATE TABLE context_decisions(id TEXT PRIMARY KEY, session_id TEXT NOT NULL, episode_ordinal INTEGER NOT NULL, kind TEXT NOT NULL, data TEXT NOT NULL);",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO sessions(id,data) VALUES('source',?1)",
        [json!({"id": "source", "title": "Source replay"}).to_string()],
    )
    .unwrap();

    let records = [
        (
            "s1",
            Role::User,
            "# AGENTS.md instructions for /repo\n\n<INSTRUCTIONS>\nBe brief.",
        ),
        ("s2", Role::Assistant, "Orphan reply."),
        ("s3", Role::User, "first question"),
        ("s4", Role::Assistant, "Looking into it."),
        (
            "s5",
            Role::User,
            "<environment_context>\n  <cwd>/repo</cwd>\n</environment_context>",
        ),
        ("s6", Role::Assistant, "Here is the answer."),
        ("s7", Role::User, "second question"),
        ("s8", Role::Assistant, "Final answer."),
    ];
    for (index, (id, role, content)) in records.into_iter().enumerate() {
        conn.execute(
            "INSERT INTO messages(session_id,data) VALUES('source',?1)",
            [serde_json::to_string(&message(id, role, content, index + 1)).unwrap()],
        )
        .unwrap();
    }

    conn.execute(
        "INSERT INTO context_subject_chapters VALUES('chA','source',1,1,2,'closed',?1)",
        [json!({
            "id": "chA",
            "description": "Opening chapter",
            "summary": "Chapter A summary",
            "status": "closed"
        })
        .to_string()],
    )
    .unwrap();
    for (ordinal, title, first, last) in [
        (1, "Opening question", "s1", "s6"),
        (2, "Follow-up", "s7", "s8"),
    ] {
        conn.execute(
            "INSERT INTO context_episodes VALUES(?1,'source',?2,0,0,'chA',?3)",
            rusqlite::params![
                format!("e{ordinal}"),
                ordinal,
                json!({
                    "ordinal": ordinal,
                    "chapter_id": "chA",
                    "title": title,
                    "summary": format!("{title} summary"),
                    "first_message_id": first,
                    "last_message_id": last
                })
                .to_string()
            ],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO context_decisions VALUES('d1','source',1,'split',?1)",
        [json!({
            "episode_ordinal": 1,
            "option_message_ids": ["s1", "s5"],
            "choice": "s1"
        })
        .to_string()],
    )
    .unwrap();
    path
}

#[test]
fn harness_input_and_split_decisions_are_not_imported() {
    let temp =
        Temp(std::env::temp_dir().join(format!("import_replay_test_{}", vault_core::new_id())));
    std::fs::create_dir_all(&temp.0).unwrap();
    let data = temp.0.join("data");
    std::fs::create_dir_all(data.join("agent")).unwrap();
    let replay = replay_file(&temp.0);

    let output = Command::new(env!("CARGO_BIN_EXE_import-replay"))
        .arg(&replay)
        .arg("--data-dir")
        .arg(&data)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "import failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains(
            "messages=5 (user 2, assistant 3, tool 0; system skipped 0; harness user skipped 2, other skipped 1)"
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains("chapters=1 sections=2 decisions=0"),
        "{stdout}"
    );
    assert!(
        stdout.contains("anchors_in_session=ok source_order=ok"),
        "{stdout}"
    );
    let session_id = stdout
        .lines()
        .find_map(|line| line.strip_prefix("session_id="))
        .expect("the importer prints the session ID")
        .to_string();

    // The imported messages: the harness user and the orphan reply are absent, and
    // the reply after the mid-turn harness user is kept in its turn.
    let store = Store::open(data.join("agent").join("agent.sqlite")).unwrap();
    let stored = store.messages(&session_id).unwrap();
    let shape: Vec<(&str, &str)> = stored
        .iter()
        .map(|m| (role_name(&m.role), m.content.as_str()))
        .collect();
    assert_eq!(
        shape,
        [
            ("user", "first question"),
            ("assistant", "Looking into it."),
            ("assistant", "Here is the answer."),
            ("user", "second question"),
            ("assistant", "Final answer."),
        ]
    );

    // Anchors: the section that starts on the skipped s1 starts on the first kept
    // message, and the section that ends on s6 ends on its kept reply.
    let outline_path = data.join("agent").join(outline::FILE_NAME);
    let chapters = OutlineStore::read_session(&outline_path, &session_id)
        .unwrap()
        .expect("the session has an outline");
    assert_eq!(chapters.len(), 1, "{chapters:?}");
    let ids: Vec<&str> = stored.iter().map(|m| m.id.as_str()).collect();
    let chapter = &chapters[0];
    assert_eq!(chapter.first_message_id.as_str(), ids[0]);
    assert_eq!(chapter.last_message_id.as_str(), ids[4]);
    assert!(
        !chapter.ongoing && chapter.ended_at.is_some(),
        "{chapter:?}"
    );
    assert!(chapter.decisions.is_empty(), "{chapter:?}");
    assert_eq!(chapter.sections.len(), 2, "{chapter:?}");
    let first = &chapter.sections[0];
    assert_eq!(first.title, "Opening question");
    assert_eq!(first.first_message_id.as_str(), ids[0]);
    assert_eq!(first.last_message_id.as_str(), ids[2]);
    assert!(first.decisions.is_empty(), "{first:?}");
    let second = &chapter.sections[1];
    assert_eq!(second.title, "Follow-up");
    assert_eq!(second.first_message_id.as_str(), ids[3]);
    assert_eq!(second.last_message_id.as_str(), ids[4]);
    assert!(second.decisions.is_empty(), "{second:?}");
}
