//! The change feed: an in-memory window of the last changes, numbered by a
//! sequence that starts at 0 when the vault is opened.

use std::collections::VecDeque;

use crate::types::{Change, ChangeBatch};

/// Changes kept; a poller further behind gets `reset`.
const WINDOW: usize = 10_000;

#[derive(Default)]
pub(crate) struct Feed {
    all: Window,
    /// Transport has its own cursor and retention window. Index commits
    /// must neither advance it nor evict unsent document changes.
    documents: Window,
}

#[derive(Default)]
struct Window {
    seq: u32,
    /// (seq, change), seq increasing.
    entries: VecDeque<(u32, Change)>,
    /// Highest seq dropped from the window (0 = none).
    dropped: u32,
}

impl Feed {
    /// A cheap wake check: callers can clone the batch after taking their
    /// evaluation lock, so commits accumulated meanwhile are coalesced too.
    pub fn has_changes_since(&self, since: u32) -> bool {
        self.all.seq != since
    }

    pub fn push(&mut self, change: Change) {
        if !matches!(change, Change::IndexCommitted { .. }) {
            self.documents.push(change.clone());
        }
        self.all.push(change);
    }

    pub fn poll(&self, since: u32) -> ChangeBatch {
        self.all.poll(since)
    }

    pub fn poll_documents(&self, since: u32) -> ChangeBatch {
        self.documents.poll(since)
    }
}

impl Window {
    fn push(&mut self, change: Change) {
        self.seq += 1;
        // Coalesce a repeat of the latest change: same content, newer seq.
        if self.entries.back().is_some_and(|(_, c)| *c == change) {
            self.entries.back_mut().expect("entry").0 = self.seq;
            return;
        }
        self.entries.push_back((self.seq, change));
        if self.entries.len() > WINDOW {
            if let Some((s, _)) = self.entries.pop_front() {
                self.dropped = s;
            }
        }
    }

    fn poll(&self, since: u32) -> ChangeBatch {
        if since > self.seq || since < self.dropped {
            return ChangeBatch {
                seq: self.seq,
                changes: Vec::new(),
                reset: true,
            };
        }
        let mut changes: Vec<Change> = Vec::new();
        for (s, c) in self.entries.iter().rev() {
            if *s <= since {
                break;
            }
            changes.push(c.clone());
        }
        changes.reverse();
        // One entry per change, in order of latest occurrence.
        let mut seen = std::collections::HashSet::new();
        let mut out: Vec<Change> = Vec::with_capacity(changes.len());
        for c in changes.into_iter().rev() {
            if seen.insert(c.clone()) {
                out.push(c);
            }
        }
        out.reverse();
        ChangeBatch {
            seq: self.seq,
            changes: out,
            reset: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesces_and_resets() {
        let mut f = Feed::default();
        f.push(Change::PageChanged("a".into()));
        f.push(Change::PageChanged("a".into()));
        f.push(Change::TreeChanged);
        let b = f.poll(0);
        assert_eq!(b.seq, 3);
        assert_eq!(
            b.changes,
            vec![Change::PageChanged("a".into()), Change::TreeChanged]
        );
        assert!(f.poll(3).changes.is_empty());
        assert!(f.poll(9).reset);
        for _ in 0..WINDOW + 5 {
            f.push(Change::PageChanged("x".into()));
            f.push(Change::PageChanged("y".into()));
        }
        assert!(f.poll(1).reset);
        assert!(!f.poll(f.all.seq - 1).reset);
    }

    #[test]
    fn index_churn_does_not_advance_or_evict_document_feed() {
        let mut f = Feed::default();
        f.push(Change::PageChanged("a".into()));
        let before = f.poll_documents(0);
        for generation in 1..=(WINDOW as u64 + 5) {
            f.push(Change::IndexCommitted { generation, pages: vec!["a".into()] });
        }
        assert!(f.poll(0).reset, "the ordinary feed retains its bounded window");
        assert_eq!(f.poll_documents(0), before);
        assert_eq!(f.poll_documents(before.seq), ChangeBatch {
            seq: before.seq, changes: Vec::new(), reset: false,
        });
        f.push(Change::TreeChanged);
        assert_eq!(f.poll_documents(before.seq), ChangeBatch {
            seq: before.seq + 1, changes: vec![Change::TreeChanged], reset: false,
        });
    }
}
