//! `import-replay`: copies a BubblePaw context-replay session into a notes data directory.
//!
//! ```text
//! import-replay REPLAY_SQLITE --data-dir DIR [--title TITLE]
//! ```
//!
//! Creates one new session in `DIR/agent/agent.sqlite` (BubblePaw's store) and
//! its user, assistant and tool messages in source order, with new ids. System
//! messages (provider context) are not copied. The replay's subject chapters,
//! episodes and segmentation decisions go to `DIR/agent/chat_outline.sqlite`, anchored
//! to the new message ids. Running it again adds a second session.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use bubblepaw_sdk::store::{Message, Role, Store};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use vault_tools::harness::{self, Part};
use vault_tools::outline::{self, OutlineChapter, OutlineSection, OutlineStore};

fn fail(msg: &str) -> ! {
    eprintln!("import-replay: {msg}");
    std::process::exit(1);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut replay = None;
    let mut data_dir = None;
    let mut title = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--data-dir" => {
                data_dir = Some(
                    args.next()
                        .unwrap_or_else(|| fail("--data-dir needs a folder")),
                )
            }
            "--title" => title = Some(args.next().unwrap_or_else(|| fail("--title needs a value"))),
            "-h" | "--help" => {
                println!("import-replay REPLAY_SQLITE --data-dir DIR [--title TITLE]");
                return;
            }
            other if other.starts_with("--") => {
                fail(&format!("unknown argument {other} (see --help)"))
            }
            other => {
                if replay.replace(other.to_string()).is_some() {
                    fail("give one replay.sqlite file");
                }
            }
        }
    }
    let replay = replay.unwrap_or_else(|| fail("missing replay.sqlite (see --help)"));
    let data_dir = data_dir.unwrap_or_else(|| fail("--data-dir DIR is required (see --help)"));
    if let Err(e) = run(Path::new(&replay), Path::new(&data_dir), title.as_deref()) {
        fail(&format!("{e:#}"));
    }
}

fn run(replay: &Path, data_dir: &Path, title: Option<&str>) -> Result<()> {
    let source = Connection::open_with_flags(replay, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open {}", replay.display()))?;

    let sessions = json_rows(&source, "SELECT data FROM sessions")?;
    if sessions.len() != 1 {
        bail!("replay has {} sessions; expected one", sessions.len());
    }
    let source_title = sessions[0]["title"]
        .as_str()
        .unwrap_or("Imported replay")
        .to_string();

    // Source order is the messages' seq order; system messages are provider context.
    let mut source_messages = Vec::new();
    let mut system_skipped = 0usize;
    for data in strings(&source, "SELECT data FROM messages ORDER BY seq")? {
        let message: Message = serde_json::from_str(&data).context("parse a replay message")?;
        if message.role == Role::System {
            system_skipped += 1;
        } else {
            source_messages.push(message);
        }
    }

    // Harness-written user text (instructions, environment context) is not shown;
    // its replies stay with the turn before, and anchors into it stand in for a
    // neighbouring kept message. See `harness::visible`.
    let parts: Vec<Part> = source_messages
        .iter()
        .map(|m| match m.role {
            Role::User => Part::User(&m.content),
            Role::Assistant => {
                Part::Assistant(m.tool_calls.iter().map(|c| c.id.as_str()).collect())
            }
            _ => Part::Tool(m.tool_call_id.as_deref().unwrap_or_default()),
        })
        .collect();
    let visible = harness::visible(&parts);
    let harness_skipped = source_messages
        .iter()
        .zip(&visible.keep)
        .filter(|(m, keep)| !**keep && m.role == Role::User)
        .count();
    let other_skipped = source_messages
        .iter()
        .zip(&visible.keep)
        .filter(|(_, keep)| !**keep)
        .count()
        - harness_skipped;
    if !visible.keep.iter().any(|&keep| keep) {
        bail!("replay has no visible messages after removing harness input");
    }

    let agent_dir: PathBuf = data_dir.join("agent");
    let agent_db = agent_dir.join("agent.sqlite");
    let store = Store::open(&agent_db).with_context(|| format!("open {}", agent_db.display()))?;
    let session = store.create_session(title.unwrap_or(&source_title))?;

    // Old id -> new id, and new id -> created_at, for anchors and outline times.
    let mut new_ids: HashMap<String, String> = HashMap::new();
    let mut created_at: HashMap<String, String> = HashMap::new();
    let mut imported = Vec::with_capacity(source_messages.len());
    for (message, _) in source_messages
        .iter()
        .zip(&visible.keep)
        .filter(|(_, keep)| **keep)
    {
        let new = Message {
            id: vault_core::new_id(),
            session_id: session.id.clone(),
            ..message.clone()
        };
        new_ids.insert(message.id.clone(), new.id.clone());
        created_at.insert(new.id.clone(), new.created_at.clone());
        imported.push(new);
    }
    // Every source message maps to a kept one: itself, or the stand-in for a dropped one.
    for (i, message) in source_messages.iter().enumerate() {
        if let Some(target) = visible.stand_in[i] {
            let new = new_ids[&source_messages[target].id].clone();
            new_ids.entry(message.id.clone()).or_insert(new);
        }
    }
    let (mut users, mut assistants, mut tools) = (0usize, 0usize, 0usize);
    for message in &imported {
        match message.role {
            Role::User => users += 1,
            Role::Assistant => assistants += 1,
            Role::Tool => tools += 1,
            Role::System => {}
        }
    }
    store.append_batch(&imported)?;

    let anchor = |old: &str| -> Result<String> {
        new_ids
            .get(old)
            .cloned()
            .with_context(|| format!("replay message {old} was not imported"))
    };

    let episodes = json_rows(
        &source,
        "SELECT data FROM context_episodes ORDER BY ordinal",
    )?;
    // `split` rows are the replay's segmentation bookkeeping, not decisions a person made.
    let decisions = json_rows(
        &source,
        "SELECT data FROM context_decisions WHERE lower(kind) <> 'split' ORDER BY episode_ordinal",
    )?;
    let subject_chapters = json_rows(
        &source,
        "SELECT data FROM context_subject_chapters ORDER BY ordinal",
    )?;

    let mut anchors_checked = 0usize;
    let mut episode_used: HashSet<i64> = HashSet::new();
    let mut outline_chapters = Vec::with_capacity(subject_chapters.len());
    let mut section_count = 0usize;
    let mut decision_count = 0usize;
    for chapter in &subject_chapters {
        let chapter_id = str_field(chapter, "id")?;
        let mut members: Vec<&Value> = episodes
            .iter()
            .filter(|e| e["chapter_id"].as_str() == Some(chapter_id.as_str()))
            .collect();
        members.sort_by_key(|e| e["ordinal"].as_i64().unwrap_or(0));
        if members.is_empty() {
            bail!("subject chapter {chapter_id} has no episodes");
        }

        let mut sections = Vec::with_capacity(members.len());
        for episode in members {
            let ordinal = episode["ordinal"].as_i64().context("episode ordinal")?;
            episode_used.insert(ordinal);
            // A section needs a title; there is no meaningful stand-in for a missing one.
            let section_title = non_empty(episode["title"].as_str()).with_context(|| {
                format!("episode {ordinal} of chapter {chapter_id} has no title")
            })?;
            let mut section_decisions = Vec::new();
            for decision in decisions
                .iter()
                .filter(|d| d["episode_ordinal"].as_i64() == Some(ordinal))
            {
                let options = decision["option_message_ids"]
                    .as_array()
                    .context("decision options")?;
                // Candidate and chosen ids are checked against the import here; the
                // visible decision names only the count, never the ids.
                let mut candidates = Vec::with_capacity(options.len());
                for option in options {
                    let old = option.as_str().context("decision option id")?;
                    anchor(old)?;
                    candidates.push(old);
                    anchors_checked += 1;
                }
                let n = options.len();
                section_decisions.push(match decision["choice"].as_str() {
                    Some(choice) => {
                        anchor(choice)?;
                        if !candidates.contains(&choice) {
                            bail!("episode {ordinal}: the selected boundary is not one of its candidates");
                        }
                        format!("Context boundary selected from {n} candidates")
                    }
                    None => format!("No context boundary selected from {n} candidates"),
                });
                decision_count += 1;
            }
            let first = anchor(&str_field(episode, "first_message_id")?)?;
            let last = anchor(&str_field(episode, "last_message_id")?)?;
            anchors_checked += 2;
            sections.push(OutlineSection {
                id: vault_core::new_id(),
                title: section_title,
                summary: episode["summary"].as_str().unwrap_or("").to_string(),
                decisions: section_decisions,
                first_message_id: first,
                last_message_id: last,
            });
        }
        section_count += sections.len();

        let first = sections[0].first_message_id.clone();
        let last = sections[sections.len() - 1].last_message_id.clone();
        let closed = chapter["status"].as_str() == Some("closed");
        // An open chapter has no description or summary of its own: its title is the
        // first named section's, its summary the section summaries joined. Source
        // text is kept as it is when present.
        let title = non_empty(chapter["description"].as_str()).unwrap_or_else(|| {
            sections
                .iter()
                .find(|s| !s.title.trim().is_empty())
                .map(|s| s.title.clone())
                .unwrap_or_else(|| "Current conversation".to_string())
        });
        let summary = non_empty(chapter["summary"].as_str()).unwrap_or_else(|| {
            sections
                .iter()
                .filter_map(|s| non_empty(Some(s.summary.as_str())))
                .collect::<Vec<_>>()
                .join("\n\n")
        });
        // Every section's segmentation entries, in source order. They are boundary
        // choices made by the replay, not conclusions reached by a person.
        let decisions: Vec<String> = sections
            .iter()
            .flat_map(|s| s.decisions.iter().cloned())
            .collect();
        outline_chapters.push(OutlineChapter {
            id: vault_core::new_id(),
            title,
            summary,
            decisions,
            ongoing: !closed,
            started_at: created_at.get(&first).cloned().unwrap_or_default(),
            ended_at: if closed {
                Some(created_at.get(&last).cloned().unwrap_or_default())
            } else {
                None
            },
            first_message_id: first,
            last_message_id: last,
            sections,
        });
    }
    if episode_used.len() != episodes.len() {
        bail!(
            "{} replay episodes belong to no subject chapter",
            episodes.len() - episode_used.len()
        );
    }

    let mut outline_store = OutlineStore::open(&agent_dir.join(outline::FILE_NAME))?;
    outline_store.save_session(&session.id, &outline_chapters)?;
    let outline_path = agent_dir.join(outline::FILE_NAME);
    let saved = OutlineStore::read_session(&outline_path, &session.id)?
        .context("the saved outline did not read back")?;

    // Verify against the store, through its public API.
    let stored = store.messages(&session.id)?;
    let position: HashMap<&str, usize> = stored
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id.as_str(), i))
        .collect();
    let mut last_position = 0usize;
    for chapter in &saved {
        for anchor in [&chapter.first_message_id, &chapter.last_message_id] {
            position.get(anchor.as_str()).with_context(|| {
                format!("chapter {} anchor missing from the session", chapter.id)
            })?;
        }
        for section in &chapter.sections {
            let first = *position
                .get(section.first_message_id.as_str())
                .context("section anchor missing from the session")?;
            let last = *position
                .get(section.last_message_id.as_str())
                .context("section anchor missing from the session")?;
            if first < last_position || last < first {
                bail!("section {} is out of source order", section.id);
            }
            last_position = last;
        }
    }

    println!("session_id={}", session.id);
    println!("title={}", session.title);
    println!("data_dir={}", data_dir.display());
    println!("agent_db={}", agent_db.display());
    println!("outline_db={}", outline_path.display());
    println!(
        "messages={} (user {users}, assistant {assistants}, tool {tools}; system skipped {system_skipped}; harness user skipped {harness_skipped}, other skipped {other_skipped})",
        imported.len()
    );
    println!(
        "chapters={} sections={section_count} decisions={decision_count} anchors_checked={anchors_checked}",
        saved.len()
    );
    let ongoing: Vec<&str> = saved
        .iter()
        .map(|c| if c.ongoing { "true" } else { "false" })
        .collect();
    let chapter_decisions: usize = saved.iter().map(|c| c.decisions.len()).sum();
    let section_decisions: usize = saved
        .iter()
        .flat_map(|c| &c.sections)
        .map(|s| s.decisions.len())
        .sum();
    println!(
        "ongoing={} chapter_decisions={chapter_decisions} section_decisions={section_decisions}",
        ongoing.join(",")
    );
    println!("anchors_in_session=ok source_order=ok");
    Ok(())
}

fn str_field(value: &Value, key: &str) -> Result<String> {
    value[key]
        .as_str()
        .map(str::to_string)
        .with_context(|| format!("missing {key}"))
}

/// The text, unless it is missing or blank.
fn non_empty(text: Option<&str>) -> Option<String> {
    text.filter(|t| !t.trim().is_empty()).map(str::to_string)
}

fn strings(conn: &Connection, sql: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn json_rows(conn: &Connection, sql: &str) -> Result<Vec<Value>> {
    strings(conn, sql)?
        .iter()
        .map(|data| serde_json::from_str(data).context("parse a replay row"))
        .collect()
}
