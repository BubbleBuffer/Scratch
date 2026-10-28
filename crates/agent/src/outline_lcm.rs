//! Display projection of BubblePaw's episode and subject chapter memory.

use std::collections::HashMap;

use anyhow::{Context, Result};
use bubblepaw_sdk::core::{ContextEpisode, ContextSubjectChapter, SubjectChapterStatus};
use bubblepaw_sdk::store::Store;
use vault_tools::outline::{ChatOutline, OutlineChapter, OutlineSection};

/// Closed chapters use their generated description; open chapters use their
/// first episode's title, or an empty title until an episode is available.
fn chapter_title(
    chapter: &ContextSubjectChapter,
    first_episode: Option<&ContextEpisode>,
) -> String {
    match chapter.status {
        SubjectChapterStatus::Closed => chapter.description.clone().unwrap_or_default(),
        SubjectChapterStatus::Open => first_episode
            .map(|episode| episode.title.clone())
            .unwrap_or_default(),
    }
}

/// None means no LCM source exists, so the caller may try legacy chapters.
pub(crate) fn latest_title(store: &Store, session: &str) -> Result<Option<String>> {
    let chapters = store.context_subject_chapters(session)?;
    let Some(chapter) = chapters.last() else {
        return Ok(None);
    };
    let episodes = store.context_episodes(session)?;
    let first = episodes
        .iter()
        .find(|episode| episode.chapter_id == chapter.id);
    Ok(Some(chapter_title(chapter, first)))
}

pub(crate) fn read(store: &Store, session: &str) -> Result<Option<ChatOutline>> {
    let chapters = store.context_subject_chapters(session)?;
    if chapters.is_empty() {
        return Ok(None);
    }
    let episodes = store.context_episodes(session)?;
    let created: HashMap<String, String> = store
        .messages(session)?
        .into_iter()
        .map(|message| (message.id, message.created_at))
        .collect();
    let time = |id: &str| -> Result<String> {
        created
            .get(id)
            .cloned()
            .with_context(|| format!("chapter anchor {id} is not in the session"))
    };
    let chapters = chapters
        .into_iter()
        .map(|chapter| {
            // Membership bounds were read first. An episode appended meanwhile
            // belongs to the next read, keeping this chapter's bounds coherent.
            let members: Vec<_> = episodes
                .iter()
                .filter(|episode| {
                    episode.chapter_id == chapter.id
                        && episode.ordinal >= chapter.first_episode_ordinal
                        && episode.ordinal <= chapter.last_episode_ordinal
                })
                .collect();
            let first = members
                .first()
                .copied()
                .with_context(|| format!("subject chapter {} has no episodes", chapter.id))?;
            let last = members.last().copied().unwrap();
            let started_at = time(&first.first_message_id)?;
            let ended_at = time(&last.last_message_id)?;
            let ongoing = chapter.status == SubjectChapterStatus::Open;
            let sections = members
                .iter()
                .map(|episode| OutlineSection {
                    id: episode.id.clone(),
                    title: episode.title.clone(),
                    summary: episode.summary.clone(),
                    decisions: Vec::new(),
                    first_message_id: episode.first_message_id.clone(),
                    last_message_id: episode.last_message_id.clone(),
                })
                .collect();
            Ok(OutlineChapter {
                title: chapter_title(&chapter, Some(first)),
                summary: chapter.summary.unwrap_or_default(),
                started_at,
                ended_at: if ongoing { None } else { Some(ended_at) },
                id: chapter.id,
                decisions: Vec::new(),
                ongoing,
                first_message_id: first.first_message_id.clone(),
                last_message_id: last.last_message_id.clone(),
                sections,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(ChatOutline { chapters }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapter(status: SubjectChapterStatus) -> ContextSubjectChapter {
        ContextSubjectChapter {
            id: "chapter".into(),
            session_id: "session".into(),
            ordinal: 1,
            first_episode_ordinal: 1,
            last_episode_ordinal: 1,
            status,
            description: (status == SubjectChapterStatus::Closed)
                .then(|| "Generated chapter description".into()),
            summary: (status == SubjectChapterStatus::Closed).then(|| "Generated summary".into()),
            related_ids: Vec::new(),
            related_evidence: Vec::new(),
            model: (status == SubjectChapterStatus::Closed).then(|| "test".into()),
            usage: None,
            reasoning: None,
        }
    }

    fn episode() -> ContextEpisode {
        ContextEpisode {
            id: "episode".into(),
            session_id: "session".into(),
            ordinal: 1,
            first_message_id: "first".into(),
            last_message_id: "last".into(),
            source_tokens: 1,
            cap: 1,
            origin_request_id: None,
            title: "First episode title".into(),
            summary: "Episode summary".into(),
            model: "test".into(),
            usage: None,
            reasoning: None,
            created_at: "2026-10-28T00:00:00Z".into(),
            chapter_id: "chapter".into(),
        }
    }

    #[test]
    fn closed_title_uses_generated_description() {
        assert_eq!(
            chapter_title(&chapter(SubjectChapterStatus::Closed), Some(&episode())),
            "Generated chapter description"
        );
    }

    #[test]
    fn open_title_uses_first_episode() {
        assert_eq!(
            chapter_title(&chapter(SubjectChapterStatus::Open), Some(&episode())),
            "First episode title"
        );
    }

    #[test]
    fn open_title_without_episodes_is_empty() {
        assert_eq!(
            chapter_title(&chapter(SubjectChapterStatus::Open), None),
            ""
        );
    }
}
