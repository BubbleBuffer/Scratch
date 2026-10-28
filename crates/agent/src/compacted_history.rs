//! Read-only display projection of durable context summaries.
//!
//! Every range is zero-based and half-open in [`Store::messages`] canonical
//! session order, never a global SQLite sequence or a filtered UI message index.
//! Notes-owned outlines are intentionally unrelated to this projection.

use std::collections::HashMap;

use anyhow::{ensure, Context, Result};
use bubblepaw_sdk::core::{ContextEpisode, SubjectChapterStatus};
use bubblepaw_sdk::store::{Message, Store};
use serde::{Deserialize, Serialize};

use crate::episode_actions::{parse_actions, parse_gist, ActionKind, ActionVerb};

/// Canonical message indices: `start` is inclusive and `end` is exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageRange {
    pub start: usize,
    pub end: usize,
}

impl MessageRange {
    pub fn message_count(self) -> usize {
        self.end - self.start
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactedHistory {
    /// False when durable summaries overlap. In that case `segments` is empty.
    pub compactable: bool,
    /// Start of the final `Messages` segment; zero when non-compactable.
    pub boundary: usize,
    pub message_count: usize,
    /// IDs in canonical range order. Use these to match a Messages range against
    /// UI messages, which may omit runtime notices and have different indices.
    pub message_ids: Vec<String>,
    /// Ordered coverage, including gaps and an always-present trailing Messages.
    pub segments: Vec<HistorySegment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum HistorySegment {
    LegacyCard(LegacyCard),
    ChapterCard(ChapterCard),
    Messages(MessageRange),
}

impl HistorySegment {
    fn range(&self) -> MessageRange {
        match self {
            Self::LegacyCard(card) => card.range,
            Self::ChapterCard(card) => card.range,
            Self::Messages(range) => *range,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyKind {
    Checkpoint,
    Chapter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyCard {
    pub id: String,
    pub kind: LegacyKind,
    pub title: Option<String>,
    pub summary: String,
    pub started_at: String,
    pub ended_at: String,
    pub first_message_id: String,
    pub last_message_id: String,
    pub range: MessageRange,
    pub message_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterCard {
    pub id: String,
    /// One-based part index. Internal episode gaps split a chapter into parts.
    pub part: usize,
    pub parts: usize,
    pub status: ChapterStatus,
    pub title: String,
    /// Open chapters have no generated summary yet.
    pub summary: Option<String>,
    /// Times of the first and last covered canonical messages, even while open.
    pub started_at: String,
    pub ended_at: String,
    pub first_message_id: String,
    pub last_message_id: String,
    pub range: MessageRange,
    /// Whole-chapter totals, present only on the final part. Uncovered gaps do
    /// not contribute to the message total.
    pub message_count: Option<usize>,
    pub episode_count: Option<usize>,
    pub episodes: Vec<EpisodeCard>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChapterStatus {
    Open,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpisodeCard {
    pub id: String,
    pub started_at: String,
    pub ended_at: String,
    pub first_message_id: String,
    pub last_message_id: String,
    pub range: MessageRange,
    pub message_count: usize,
    pub title: String,
    pub gist: Option<String>,
    pub actions: Vec<EpisodeAction>,
    pub action_count: usize,
    /// All explicit points of interest, including errors.
    pub flagged_count: usize,
    /// Actions explicitly marked `! error:`, never inferred from result text.
    pub error_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpisodeAction {
    pub verb: String,
    pub target: String,
    pub target_kind: TargetKind,
    pub change_marker: Option<String>,
    pub point_of_interest: Option<PointOfInterest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Path,
    Page,
    Command,
    Question,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointOfInterest {
    pub kind: String,
    pub reason: String,
}

struct Messages<'a> {
    all: &'a [Message],
    positions: HashMap<&'a str, usize>,
}

impl<'a> Messages<'a> {
    fn new(all: &'a [Message]) -> Self {
        Self {
            all,
            positions: all
                .iter()
                .enumerate()
                .map(|(i, m)| (m.id.as_str(), i))
                .collect(),
        }
    }

    fn position(&self, id: &str) -> Result<usize> {
        self.positions
            .get(id)
            .copied()
            .with_context(|| format!("compacted history anchor {id} is not in the session"))
    }

    fn range(&self, first: &str, last: &str) -> Result<MessageRange> {
        let start = self.position(first)?;
        let end = self.position(last)? + 1;
        ensure!(
            start < end,
            "compacted history range {first}..{last} runs backwards"
        );
        Ok(MessageRange { start, end })
    }

    fn legacy(
        &self,
        id: String,
        kind: LegacyKind,
        title: Option<String>,
        summary: String,
        range: MessageRange,
    ) -> LegacyCard {
        let first = &self.all[range.start];
        let last = &self.all[range.end - 1];
        LegacyCard {
            id,
            kind,
            title,
            summary,
            started_at: first.created_at.clone(),
            ended_at: last.created_at.clone(),
            first_message_id: first.id.clone(),
            last_message_id: last.id.clone(),
            range,
            message_count: range.message_count(),
        }
    }

    fn episode(&self, episode: &ContextEpisode) -> Result<EpisodeCard> {
        let range = self.range(&episode.first_message_id, &episode.last_message_id)?;
        let log = parse_actions(&episode.summary);
        let actions: Vec<_> = log
            .rows
            .into_iter()
            .map(|row| {
                let target_kind = target_kind(&row.verb, &row.target);
                let change_marker = match (row.change, row.outcome) {
                    (Some(change), Some(outcome)) => Some(format!("{change} · {outcome}")),
                    (change, outcome) => change.or(outcome),
                };
                EpisodeAction {
                    verb: row.verb.as_str().into(),
                    target: row.target,
                    target_kind,
                    change_marker,
                    point_of_interest: row.kind.map(|kind| PointOfInterest {
                        kind: match kind {
                            ActionKind::Decision => "decision",
                            ActionKind::Error => "error",
                            ActionKind::Correction => "correction",
                            ActionKind::Question => "question",
                            ActionKind::Risk => "risk",
                        }
                        .into(),
                        reason: row.reason.unwrap_or_default(),
                    }),
                }
            })
            .collect();
        Ok(EpisodeCard {
            id: episode.id.clone(),
            started_at: self.all[range.start].created_at.clone(),
            ended_at: self.all[range.end - 1].created_at.clone(),
            first_message_id: episode.first_message_id.clone(),
            last_message_id: episode.last_message_id.clone(),
            range,
            message_count: range.message_count(),
            title: episode.title.clone(),
            gist: parse_gist(&episode.summary),
            action_count: actions.len(),
            flagged_count: actions
                .iter()
                .filter(|a| a.point_of_interest.is_some())
                .count(),
            error_count: actions
                .iter()
                .filter(|a| {
                    a.point_of_interest
                        .as_ref()
                        .is_some_and(|p| p.kind == "error")
                })
                .count(),
            actions,
        })
    }
}

fn target_kind(verb: &ActionVerb, target: &str) -> TargetKind {
    if target.starts_with("https://")
        || target.starts_with("http://")
        || target
            .strip_prefix("page:")
            .is_some_and(|page| !page.trim().is_empty())
    {
        return TargetKind::Page;
    }
    match verb {
        ActionVerb::Ran => TargetKind::Command,
        ActionVerb::Asked => TargetKind::Question,
        ActionVerb::Read
        | ActionVerb::Edited
        | ActionVerb::Created
        | ActionVerb::Deleted
        | ActionVerb::Moved
            if !target.contains(char::is_whitespace)
                && (target.starts_with('.')
                    || target.contains('/')
                    || target.contains('\\')
                    || target.rsplit_once('.').is_some_and(|(name, ext)| {
                        !name.is_empty()
                            && !ext.is_empty()
                            && ext.chars().all(|c| c.is_ascii_alphanumeric())
                    })) =>
        {
            TargetKind::Path
        }
        _ => TargetKind::Other,
    }
}

/// Build summary cards and raw gaps without changing titles, summaries or serving
/// state. Overlapping summary coverage yields `compactable: false` and no segments.
pub fn read(store: &Store, session: &str) -> Result<CompactedHistory> {
    store.session(session)?;
    // Older context sources precede chapter bounds; chapter bounds precede
    // episodes. Concurrent episode growth belongs to the next projection.
    let checkpoint = store.context_checkpoint(session)?;
    let legacy = store.context_chapters(session)?;
    let chapters = store.context_subject_chapters(session)?;
    let last_episode_ordinal = chapters
        .iter()
        .map(|c| c.last_episode_ordinal)
        .max()
        .unwrap_or(0);
    let episodes: Vec<_> = store
        .context_episodes(session)?
        .into_iter()
        .filter(|e| e.ordinal <= last_episode_ordinal)
        .collect();
    // Messages last, so newly visible summary anchors are available.
    let all = store.messages(session)?;
    let messages = Messages::new(&all);
    let mut cards = Vec::new();
    if let Some(checkpoint) = checkpoint {
        let range = MessageRange {
            start: 0,
            end: messages.position(&checkpoint.through_message_id)? + 1,
        };
        cards.push(HistorySegment::LegacyCard(messages.legacy(
            checkpoint.id,
            LegacyKind::Checkpoint,
            None,
            checkpoint.summary,
            range,
        )));
    }
    for chapter in legacy {
        let range = messages.range(&chapter.first_message_id, &chapter.last_message_id)?;
        cards.push(HistorySegment::LegacyCard(messages.legacy(
            chapter.id,
            LegacyKind::Chapter,
            Some(chapter.title),
            chapter.summary,
            range,
        )));
    }
    let mut summary_ranges: Vec<_> = cards.iter().map(HistorySegment::range).collect();
    let mut assigned_episodes = 0;
    for chapter in chapters {
        let members: Vec<_> = episodes
            .iter()
            .filter(|e| {
                e.chapter_id == chapter.id
                    && e.ordinal >= chapter.first_episode_ordinal
                    && e.ordinal <= chapter.last_episode_ordinal
            })
            .collect();
        let first = members
            .first()
            .copied()
            .with_context(|| format!("subject chapter {} has no episodes", chapter.id))?;
        assigned_episodes += members.len();
        let projected = members
            .iter()
            .map(|e| messages.episode(e))
            .collect::<Result<Vec<_>>>()?;
        if projected
            .windows(2)
            .any(|pair| pair[0].range.end > pair[1].range.start)
        {
            return Ok(not_compactable(&all));
        }
        // Validate original chapter envelopes too: splitting must not allow an
        // overlapping imported chapter to hide inside another chapter's gap.
        summary_ranges.push(MessageRange {
            start: projected[0].range.start,
            end: projected.last().unwrap().range.end,
        });
        let status = if chapter.status == SubjectChapterStatus::Open {
            ChapterStatus::Open
        } else {
            ChapterStatus::Closed
        };
        let title = crate::outline_lcm::chapter_title(&chapter, Some(first));
        let summary = if status == ChapterStatus::Open {
            None
        } else {
            chapter.summary
        };
        let message_count = projected.iter().map(|e| e.message_count).sum();
        let episode_count = projected.len();
        let mut groups: Vec<Vec<EpisodeCard>> = Vec::new();
        for episode in projected {
            if let Some(group) = groups.last_mut().filter(|group| {
                group
                    .last()
                    .is_some_and(|previous| previous.range.end == episode.range.start)
            }) {
                group.push(episode);
            } else {
                groups.push(vec![episode]);
            }
        }
        let parts = groups.len();
        for (index, episodes) in groups.into_iter().enumerate() {
            let part = index + 1;
            let start = episodes[0].range.start;
            let end = episodes.last().unwrap().range.end;
            cards.push(HistorySegment::ChapterCard(ChapterCard {
                id: chapter.id.clone(),
                part,
                parts,
                title: title.clone(),
                summary: summary.clone(),
                status,
                started_at: all[start].created_at.clone(),
                ended_at: all[end - 1].created_at.clone(),
                first_message_id: all[start].id.clone(),
                last_message_id: all[end - 1].id.clone(),
                range: MessageRange { start, end },
                message_count: (part == parts).then_some(message_count),
                episode_count: (part == parts).then_some(episode_count),
                episodes,
            }));
        }
    }
    ensure!(
        assigned_episodes == episodes.len(),
        "compacted history episodes have no matching subject chapter bounds"
    );
    summary_ranges.sort_by_key(|range| range.start);
    if summary_ranges
        .windows(2)
        .any(|pair| pair[0].end > pair[1].start)
    {
        return Ok(not_compactable(&all));
    }
    cards.sort_by_key(|card| card.range().start);
    let mut segments = Vec::new();
    let mut boundary = 0;
    for card in cards {
        let range = card.range();
        if range.start < boundary {
            return Ok(not_compactable(&all));
        }
        if boundary < range.start {
            segments.push(HistorySegment::Messages(MessageRange {
                start: boundary,
                end: range.start,
            }));
        }
        boundary = range.end;
        segments.push(card);
    }
    segments.push(HistorySegment::Messages(MessageRange {
        start: boundary,
        end: all.len(),
    }));
    Ok(CompactedHistory {
        compactable: true,
        boundary,
        message_count: all.len(),
        message_ids: all.iter().map(|m| m.id.clone()).collect(),
        segments,
    })
}

fn not_compactable(messages: &[Message]) -> CompactedHistory {
    CompactedHistory {
        compactable: false,
        boundary: 0,
        message_count: messages.len(),
        message_ids: messages.iter().map(|m| m.id.clone()).collect(),
        segments: Vec::new(),
    }
}
