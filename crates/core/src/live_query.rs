//! Vault-owned live search subscriptions. Index writers only publish feed
//! events; this worker coalesces them and performs all membership/ranking work.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Weak};
use std::time::Duration;

use crate::index::Index;
use crate::search_query::SearchSpec;
use crate::types::{Change, HitKey, LiveQueryUpdate, PageId, SearchSpecResults, VAULT_SOURCE};
use crate::vault::{lock, Inner, Vault};
use crate::Result;

/// Owns a subscription receiver. Dropping it unregisters the query. The
/// receiver does not keep the vault or its worker alive.
pub struct LiveQuerySubscription {
    pub handle: u64,
    receiver: Receiver<LiveQueryUpdate>,
    active: Arc<AtomicBool>,
    inner: Weak<Inner>,
}

impl LiveQuerySubscription {
    /// The first update is always a snapshot (`initial = true`), including an
    /// empty result. Unwatching or dropping the vault disconnects the stream.
    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> std::result::Result<LiveQueryUpdate, RecvTimeoutError> {
        if !self.active.load(Ordering::Acquire) {
            return Err(RecvTimeoutError::Disconnected);
        }
        let update = self.receiver.recv_timeout(timeout)?;
        if !self.active.load(Ordering::Acquire) {
            return Err(RecvTimeoutError::Disconnected);
        }
        Ok(update)
    }
}

impl Drop for LiveQuerySubscription {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
        if let Some(inner) = self.inner.upgrade() {
            lock(&inner.live_queries).watches.remove(&self.handle);
        }
    }
}

#[derive(Default)]
pub(crate) struct Registry {
    next_handle: u64,
    watches: HashMap<u64, Watch>,
}

struct Watch {
    spec: SearchSpec,
    limit: u32,
    last: SearchSpecResults,
    delivery: Delivery,
    active: Arc<AtomicBool>,
}

enum Delivery {
    Channel(Sender<LiveQueryUpdate>),
    Callback(Box<dyn Fn(LiveQueryUpdate) -> bool + Send>),
}

impl Delivery {
    fn send(&self, update: LiveQueryUpdate) -> bool {
        match self {
            Self::Channel(sender) => sender.send(update).is_ok(),
            Self::Callback(send) => send(update),
        }
    }
}

impl Registry {
    pub(crate) fn close(&mut self) {
        for watch in self.watches.values() {
            watch.active.store(false, Ordering::Release);
        }
        self.watches.clear();
    }
}

impl Vault {
    /// Execute a stored query and register it atomically with respect to the
    /// live worker. The first queued update contains the complete window.
    pub fn watch_query(&self, spec: SearchSpec, limit: u32) -> Result<LiveQuerySubscription> {
        let (sender, receiver) = mpsc::channel();
        let (handle, active) = self.register_query(spec, limit, |_| Delivery::Channel(sender))?;
        Ok(LiveQuerySubscription {
            handle,
            receiver,
            active,
            inner: Arc::downgrade(&self.inner),
        })
    }

    /// Register a bridge sink without a forwarding task retaining the vault.
    /// The initial snapshot is delivered on the calling thread; later updates
    /// run on the live worker. Returning false unregisters the sink. The sink
    /// must return promptly and must not re-enter this vault's query registry.
    pub fn watch_query_with(
        &self,
        spec: SearchSpec,
        limit: u32,
        send: impl Fn(u64, LiveQueryUpdate) -> bool + Send + 'static,
    ) -> Result<u64> {
        self.register_query(spec, limit, |handle| {
            Delivery::Callback(Box::new(move |update| send(handle, update)))
        })
        .map(|(handle, _)| handle)
    }

    fn register_query(
        &self,
        spec: SearchSpec,
        limit: u32,
        delivery: impl FnOnce(u64) -> Delivery,
    ) -> Result<(u64, Arc<AtomicBool>)> {
        // The worker also takes this lock before advancing its feed cursor:
        // a commit racing the initial snapshot cannot fall between the two.
        let mut registry = lock(&self.inner.live_queries);
        let db = lock(&self.inner.db);
        let tx = db.conn.unchecked_transaction()?;
        let generation = self.index_generation();
        let last = db.search_spec(&spec, limit, 0, false)?;
        tx.commit()?;
        drop(db);
        registry.next_handle = registry
            .next_handle
            .checked_add(1)
            .expect("live query handles exhausted");
        let handle = registry.next_handle;
        let delivery = delivery(handle);
        let active = Arc::new(AtomicBool::new(true));
        let delivered = delivery.send(LiveQueryUpdate {
            generation,
            initial: true,
            added: last.hits.clone(),
            removed: Vec::new(),
            moved: Vec::new(),
            changed: Vec::new(),
            total_bounded: last.total_bounded,
        });
        if delivered {
            registry.watches.insert(
                handle,
                Watch {
                    spec,
                    limit,
                    last,
                    delivery,
                    active: active.clone(),
                },
            );
        } else {
            active.store(false, Ordering::Release);
        }
        Ok((handle, active))
    }

    /// Stop a subscription, including any updates already queued for it.
    /// Returning waits for an in-flight evaluation of this registry.
    pub fn unwatch_query(&self, handle: u64) {
        if let Some(watch) = lock(&self.inner.live_queries).watches.remove(&handle) {
            watch.active.store(false, Ordering::Release);
        }
    }

    /// Monotonic within this open vault; starts at zero before its first commit.
    pub fn index_generation(&self) -> u64 {
        self.inner.index_generation.load(Ordering::Acquire)
    }
}

pub(crate) fn run(inner: Arc<Inner>, db: Index) {
    let mut since = 0;
    loop {
        inner.wait_live_changes(since);
        if inner.live_stop.load(Ordering::Acquire) {
            break;
        }
        // Everything accumulated while the previous evaluation ran is one
        // union, with one membership probe per watch.
        let mut registry = lock(&inner.live_queries);
        let batch = inner.poll(since);
        since = batch.seq;
        let mut pages = HashSet::<PageId>::new();
        let mut full = batch.reset;
        let mut tree = batch.reset;
        let mut committed = batch.reset;
        let mut generation = inner.index_generation.load(Ordering::Acquire);
        for change in batch.changes {
            match change {
                Change::IndexCommitted {
                    generation: next,
                    pages: ids,
                } => {
                    generation = generation.max(next);
                    committed = true;
                    full |= ids.is_empty();
                    pages.extend(ids);
                }
                Change::TreeChanged => tree = true,
                _ => {}
            }
        }
        if !committed && !tree {
            continue;
        }
        let pages: Vec<_> = pages.into_iter().collect();
        registry.watches.retain(|_, watch| {
            if !watch.active.load(Ordering::Acquire) {
                return false;
            }
            let evaluate = || -> Result<Option<SearchSpecResults>> {
                // Membership and the refreshed window share one SQLite
                // snapshot even when another writer commits concurrently.
                let tx = db.conn.unchecked_transaction()?;
                let tree_sensitive = tree
                    && watch.spec.operators.iter().any(|op| {
                        !op.property && matches!(op.key.as_str(), "in" | "links" | "from")
                    });
                // Path patterns read ancestor titles: a rename anywhere above
                // can change membership without touching the member page.
                let path_sensitive = committed
                    && watch.spec.operators.iter().any(|op| {
                        !op.property
                            && op.key == "in"
                            && crate::search_path::split_raw(&op.value, ',')
                                .iter()
                                .any(|raw| crate::search_path::is_pattern(raw))
                    });
                let was_hit = watch
                    .last
                    .hits
                    .iter()
                    .any(|hit| hit.source == VAULT_SOURCE && pages.contains(&hit.id));
                // The settled gate remembers the returned window, not every
                // match: removing a previously matching row outside that
                // window can leave a bounded total unchanged until another
                // relevant commit. In particular this applies to limit zero.
                let relevant = full
                    || tree_sensitive
                    || path_sensitive
                    || was_hit
                    || (committed && db.search_spec_matches_ids(&watch.spec, &pages)?);
                let next = if relevant {
                    Some(db.search_spec(&watch.spec, watch.limit, 0, false)?)
                } else {
                    None
                };
                tx.commit()?;
                Ok(next)
            };
            match evaluate() {
                Ok(Some(next)) => {
                    let update = diff(generation, &watch.last, &next);
                    watch.last = next;
                    if let Some(update) = update {
                        if !watch.delivery.send(update) {
                            watch.active.store(false, Ordering::Release);
                            return false;
                        }
                    }
                }
                Ok(None) => {}
                // A later commit retries against the last delivered window.
                // Keep the failure visible in native diagnostics.
                Err(error) => eprintln!("live query evaluation failed: {error}"),
            }
            true
        });
    }
    lock(&inner.live_queries).close();
}

fn diff(
    generation: u64,
    old: &SearchSpecResults,
    next: &SearchSpecResults,
) -> Option<LiveQueryUpdate> {
    let before: HashMap<_, _> = old
        .hits
        .iter()
        .enumerate()
        .map(|(position, hit)| (HitKey::from(hit), (position, hit)))
        .collect();
    let after: HashSet<_> = next.hits.iter().map(HitKey::from).collect();
    let removed = old
        .hits
        .iter()
        .map(HitKey::from)
        .filter(|key| !after.contains(key))
        .collect::<Vec<_>>();
    let mut added = Vec::new();
    let mut changed = Vec::new();
    let mut moved = Vec::new();
    for (position, hit) in next.hits.iter().enumerate() {
        let key = HitKey::from(hit);
        if let Some((previous_position, previous)) = before.get(&key) {
            if *previous_position != position {
                moved.push((key, position as u32));
            }
            if *previous != hit {
                changed.push(hit.clone());
            }
        } else {
            added.push(hit.clone());
            moved.push((key, position as u32));
        }
    }
    if added.is_empty()
        && removed.is_empty()
        && moved.is_empty()
        && changed.is_empty()
        && old.total_bounded == next.total_bounded
    {
        return None;
    }
    Some(LiveQueryUpdate {
        generation,
        initial: false,
        added,
        removed,
        moved,
        changed,
        total_bounded: next.total_bounded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{new_id, BlockKind, BlockOp, NewBlock};

    struct Dir(std::path::PathBuf);

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fixture() -> (Dir, Vault, Vec<(String, String)>) {
        let dir = Dir(std::env::temp_dir().join(format!("vault_live_coalesce_{}", new_id())));
        let v = Vault::open(dir.0.to_string_lossy().into_owned()).unwrap();
        let mut pages = Vec::new();
        for title in ["First", "Second"] {
            let page = v.create_page(None, title.into()).unwrap();
            let block = new_id();
            v.apply(
                page.clone(),
                vec![BlockOp::Insert {
                    parent: None,
                    after: None,
                    block: NewBlock {
                        id: block.clone(),
                        kind: BlockKind::Paragraph,
                        text: "ordinary content".into(),
                        marks: Vec::new(),
                        checked: false,
                    },
                }],
            )
            .unwrap();
            pages.push((page, block));
        }
        v.flush().unwrap();
        (dir, v, pages)
    }

    fn text(v: &Vault, page: &(String, String), content: &str) {
        v.apply(
            page.0.clone(),
            vec![BlockOp::SetText {
                id: page.1.clone(),
                text: content.into(),
                marks: Vec::new(),
            }],
        )
        .unwrap();
        v.flush().unwrap();
    }

    #[test]
    fn coalesces_union_of_committed_ids_and_suppresses_transient_membership() {
        let (_dir, v, pages) = fixture();
        let sub = v
            .watch_query(v.parse_search("xylophone".into()).unwrap().spec, 10)
            .unwrap();
        assert!(sub
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .added
            .is_empty());
        // Pause only the notifier. Commits must complete while this lock is
        // held; it is never touched by the writer or foreground commit path.
        let paused = lock(&v.inner.live_queries);
        text(&v, &pages[0], "xylophone temporary");
        text(&v, &pages[1], "xylophone permanent");
        text(&v, &pages[0], "ordinary content");
        let generation = v.index_generation();
        drop(paused);
        let update = sub.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(update.generation, generation);
        assert_eq!(update.added.len(), 1);
        assert_eq!(update.added[0].id, pages[1].0);
        assert!(update.removed.is_empty());
        assert_eq!(
            sub.recv_timeout(Duration::from_millis(150)),
            Err(RecvTimeoutError::Timeout)
        );
    }

    #[test]
    fn coalesced_batch_keeps_earlier_committed_ids_and_evaluates_final_window() {
        let (_dir, v, pages) = fixture();
        let sub = v
            .watch_query(v.parse_search("xylophone".into()).unwrap().spec, 10)
            .unwrap();
        sub.recv_timeout(Duration::from_secs(5)).unwrap();
        let paused = lock(&v.inner.live_queries);
        text(&v, &pages[0], "xylophone first");
        text(&v, &pages[1], "xylophone second");
        drop(paused);
        let update = sub.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            update
                .added
                .iter()
                .map(|hit| hit.id.clone())
                .collect::<HashSet<_>>(),
            pages.iter().map(|page| page.0.clone()).collect()
        );
        assert_eq!(update.total_bounded, 2);
        assert_eq!(
            sub.recv_timeout(Duration::from_millis(150)),
            Err(RecvTimeoutError::Timeout)
        );
    }

    #[test]
    fn subscriptions_do_not_retain_vault_inner_after_shutdown() {
        let (_dir, v, _) = fixture();
        let sub = v
            .watch_query(v.parse_search("sort:title".into()).unwrap().spec, 10)
            .unwrap();
        let weak = Arc::downgrade(&v.inner);
        drop(v);
        assert!(
            weak.upgrade().is_none(),
            "both owned threads must release Inner"
        );
        assert_eq!(
            sub.recv_timeout(Duration::from_secs(1)),
            Err(RecvTimeoutError::Disconnected)
        );
    }

    #[test]
    fn callback_sinks_release_on_unwatch_rejected_delivery_and_vault_shutdown() {
        let (_dir, v, _) = fixture();
        let (sender, receiver) = mpsc::channel();
        let retained = Arc::new(());
        let weak = Arc::downgrade(&retained);
        let handle = v
            .watch_query_with(
                v.parse_search("sort:title".into()).unwrap().spec,
                10,
                move |handle, update| {
                    let _keep_alive = &retained;
                    sender.send((handle, update)).is_ok()
                },
            )
            .unwrap();
        let (sent_handle, initial) = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(sent_handle, handle);
        assert!(initial.initial);
        assert!(weak.upgrade().is_some());
        v.unwatch_query(handle);
        assert!(weak.upgrade().is_none());
        let retained = Arc::new(());
        let weak = Arc::downgrade(&retained);
        let rejected = v
            .watch_query_with(SearchSpec::default(), 10, move |_, _| {
                let _keep_alive = &retained;
                false
            })
            .unwrap();
        assert!(!lock(&v.inner.live_queries).watches.contains_key(&rejected));
        assert!(weak.upgrade().is_none());
        let retained = Arc::new(());
        let weak = Arc::downgrade(&retained);
        v.watch_query_with(SearchSpec::default(), 10, move |_, _| {
            let _keep_alive = &retained;
            true
        })
        .unwrap();
        drop(v);
        assert!(weak.upgrade().is_none());
    }
}
