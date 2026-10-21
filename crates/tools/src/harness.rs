//! Harness-written user text in recorded transcripts, shared by the replay importer
//! and the dev replay.
//!
//! The harness writes some user messages itself: project instructions and
//! environment context. They are not the person's turns, so they are left out of
//! the visible transcript. Assistant replies that follow them stay, as part of the
//! turn before.

use std::collections::HashMap;

/// Whether a user message is harness-written context rather than something the
/// person typed. Matches a message that starts (after leading whitespace) with
/// `# AGENTS.md instructions`, a message that contains `<INSTRUCTIONS>` (case
/// insensitive), or one that starts with an `environment_context` or
/// `environment-context` XML block (case insensitive).
pub fn is_harness_user_text(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("# AGENTS.md instructions")
        || text.to_ascii_lowercase().contains("<instructions>")
        || environment_block(trimmed)
}

fn environment_block(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('<') else {
        return false;
    };
    // ASCII lowercasing keeps byte offsets, so the lowered text can be sliced.
    let lower = rest.to_ascii_lowercase();
    ["environment_context", "environment-context"]
        .iter()
        .any(|name| {
            lower
                .strip_prefix(name)
                .is_some_and(|after| after.starts_with(['>', '/', ' ', '\t', '\r', '\n']))
        })
}

/// Whether a recorded context decision is kept. `split` entries are the replay's
/// own segmentation bookkeeping, not choices a person made, so they are dropped.
pub fn keeps_decision(kind: &str) -> bool {
    !kind.eq_ignore_ascii_case("split")
}

/// One recorded message, as [`visible`] sees it. System records are not passed in.
pub enum Part<'a> {
    User(&'a str),
    /// An assistant record, with the IDs of the tool calls it made.
    Assistant(Vec<&'a str>),
    /// A tool result, with the call ID it answers.
    Tool(&'a str),
}

/// Which recorded messages stay visible, and which kept message stands in for each
/// one that does not.
pub struct Visible {
    pub keep: Vec<bool>,
    /// For each part: itself when kept, otherwise the nearest kept part before it,
    /// or the first kept part after it when none is before. `None` only when no
    /// part is kept.
    pub stand_in: Vec<Option<usize>>,
}

/// Works out the visible transcript from the recorded messages in source order.
///
/// A harness user is dropped. An assistant record is kept only after a real user
/// message, so leading harness input and its replies are dropped. A tool result is
/// kept with the assistant that made its call, and only after a real user message.
pub fn visible(parts: &[Part]) -> Visible {
    let mut keep = vec![false; parts.len()];
    let mut owner: HashMap<&str, usize> = HashMap::new();
    let mut real_turn = false;
    for (i, part) in parts.iter().enumerate() {
        let kept = match part {
            Part::User(text) => {
                let real = !is_harness_user_text(text);
                real_turn |= real;
                real
            }
            Part::Assistant(calls) => {
                for call in calls {
                    owner.insert(call, i);
                }
                real_turn
            }
            Part::Tool(call) => real_turn && owner.get(call).is_none_or(|&o| keep[o]),
        };
        keep[i] = kept;
    }

    let mut stand_in = vec![None; parts.len()];
    let mut before = None;
    for (i, &kept) in keep.iter().enumerate() {
        if kept {
            before = Some(i);
        }
        stand_in[i] = before;
    }
    let mut after = None;
    for i in (0..parts.len()).rev() {
        if keep[i] {
            after = Some(i);
        } else if stand_in[i].is_none() {
            stand_in[i] = after;
        }
    }
    Visible { keep, stand_in }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_harness_user_text() {
        assert!(is_harness_user_text(
            "# AGENTS.md instructions for /repo\n\nBody"
        ));
        assert!(is_harness_user_text("  \n# AGENTS.md instructions"));
        assert!(is_harness_user_text(
            "# AGENTS.md instructions\n\n<INSTRUCTIONS>\nx"
        ));
        assert!(is_harness_user_text("note <instructions> here"));
        assert!(is_harness_user_text("<Instructions>"));
        assert!(is_harness_user_text(
            "<environment_context>\n  <cwd>x</cwd>"
        ));
        assert!(is_harness_user_text("\n  <environment-context>"));
        assert!(is_harness_user_text("<environment_context/>"));
    }

    #[test]
    fn leaves_the_persons_text_alone() {
        assert!(!is_harness_user_text(""));
        assert!(!is_harness_user_text("first question"));
        assert!(!is_harness_user_text("# AGENTS.md"));
        assert!(!is_harness_user_text("# Agents.md instructions"));
        assert!(!is_harness_user_text(
            "Please explain <environment_context>"
        ));
        assert!(!is_harness_user_text("<environment_contexts>"));
        assert!(!is_harness_user_text("<environment-contextual>"));
        assert!(!is_harness_user_text("<div>"));
    }

    #[test]
    fn split_decisions_are_not_kept() {
        assert!(!keeps_decision("split"));
        assert!(!keeps_decision("Split"));
        assert!(keeps_decision("choice"));
    }

    #[test]
    fn harness_user_mid_turn_merges_its_replies_into_the_turn() {
        let calls = vec!["c1"];
        let parts = [
            Part::User("question"),
            Part::Assistant(calls),
            Part::Tool("c1"),
            Part::User("<environment_context>"),
            Part::Assistant(vec![]),
            Part::User("next"),
        ];
        let v = visible(&parts);
        assert_eq!(v.keep, [true, true, true, false, true, true]);
        assert_eq!(
            v.stand_in,
            [Some(0), Some(1), Some(2), Some(2), Some(4), Some(5)]
        );
    }

    #[test]
    fn leading_harness_input_and_its_orphan_replies_are_dropped() {
        let calls = vec!["c0"];
        let parts = [
            Part::User("# AGENTS.md instructions"),
            Part::Assistant(calls),
            Part::Tool("c0"),
            Part::Assistant(vec![]),
            Part::User("question"),
            Part::Assistant(vec![]),
        ];
        let v = visible(&parts);
        assert_eq!(v.keep, [false, false, false, false, true, true]);
        // Dropped leading messages stand in for the first kept message.
        assert_eq!(
            v.stand_in,
            [Some(4), Some(4), Some(4), Some(4), Some(4), Some(5)]
        );
    }

    #[test]
    fn nothing_visible_leaves_no_stand_in() {
        let v = visible(&[Part::User("<INSTRUCTIONS>"), Part::Assistant(vec![])]);
        assert_eq!(v.keep, [false, false]);
        assert_eq!(v.stand_in, [None, None]);
    }
}
