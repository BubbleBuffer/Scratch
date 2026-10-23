//! Feature-only storage telemetry, for measuring where cold-store reads and
//! first syncs spend time. Build with `--features storage-metrics`.
//!
//! Without the feature every function here is an empty inline function and
//! [`SpanGuard`] has no fields and no `Drop`, so instrumented call sites
//! compile to nothing.
//!
//! - [`span`] times a region. Nested spans on one thread subtract the time of
//!   their children, so `exclusive_ns` of a parent excludes its children. The
//!   guard is `!Send`: it must drop on the thread that created it.
//! - [`add_bytes`] adds a byte count to a label; it does not count a call.
//! - [`reset`] starts a new phase. A guard created before a reset is dropped
//!   silently, so old work is never charged to the new phase.
//! - [`report`] returns `{"labels": {...}}` with calls, bytes, `inclusive_ns`
//!   and `exclusive_ns` per label. Times are summed across threads, so they can
//!   exceed the wall time of a phase that uses several threads.
//!
//! Labels are `&'static str` and must name a cost, never a page, ID, path or
//! token.

#[cfg(feature = "storage-metrics")]
mod imp {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::marker::PhantomData;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, MutexGuard};
    use std::time::Instant;

    #[derive(Clone, Copy, Default)]
    struct Stat {
        calls: u64,
        bytes: u64,
        inclusive_ns: u64,
        exclusive_ns: u64,
    }

    /// Phase number. Bumped under the `TOTALS` lock by `reset`.
    static GEN: AtomicU64 = AtomicU64::new(1);
    static TOTALS: Mutex<BTreeMap<&'static str, Stat>> = Mutex::new(BTreeMap::new());

    /// Per-thread open spans. `stack[i]` is the child time already seen by the
    /// i-th open span on this thread.
    struct Local {
        gen: u64,
        stack: Vec<u64>,
    }

    thread_local! {
        static LOCAL: RefCell<Local> = const { RefCell::new(Local { gen: 0, stack: Vec::new() }) };
    }

    fn totals() -> MutexGuard<'static, BTreeMap<&'static str, Stat>> {
        TOTALS.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn nanos(d: std::time::Duration) -> u64 {
        u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
    }

    /// An open span. Dropping it records the span if its phase is still current.
    #[must_use = "bind the guard to a name; a dropped guard times nothing"]
    pub struct SpanGuard {
        label: &'static str,
        gen: u64,
        depth: usize,
        start: Instant,
        _not_send: PhantomData<*const ()>,
    }

    /// Opens a span named `label` on this thread.
    #[must_use = "bind the guard to a name; a dropped guard times nothing"]
    pub fn span(label: &'static str) -> SpanGuard {
        let start = Instant::now();
        let gen = GEN.load(Ordering::SeqCst);
        let depth = LOCAL
            .try_with(|local| {
                let mut local = local.borrow_mut();
                if local.gen != gen {
                    // Frames left by an earlier phase are not ours to keep.
                    local.gen = gen;
                    local.stack.clear();
                }
                local.stack.push(0);
                local.stack.len() - 1
            })
            .unwrap_or(0);
        SpanGuard {
            label,
            gen,
            depth,
            start,
            _not_send: PhantomData,
        }
    }

    impl Drop for SpanGuard {
        fn drop(&mut self) {
            let elapsed = nanos(self.start.elapsed());
            if self.gen != GEN.load(Ordering::SeqCst) {
                let _ = LOCAL.try_with(|local| {
                    let mut local = local.borrow_mut();
                    if local.gen == self.gen && local.stack.len() > self.depth {
                        local.stack.truncate(self.depth);
                    }
                });
                return;
            }
            let child = LOCAL
                .try_with(|local| {
                    let mut local = local.borrow_mut();
                    if local.gen != self.gen || local.stack.len() <= self.depth {
                        return None;
                    }
                    // Guards dropped out of order: spans opened after this one
                    // are not closed yet; their time is discarded with them.
                    local.stack.truncate(self.depth + 1);
                    let child = local.stack.pop().unwrap_or(0);
                    if let Some(parent) = local.stack.last_mut() {
                        *parent = parent.saturating_add(elapsed);
                    }
                    Some(child)
                })
                .ok()
                .flatten();
            if let Some(child) = child {
                let mut totals = totals();
                // Checked under the lock that `reset` holds while it bumps the phase.
                if self.gen != GEN.load(Ordering::SeqCst) {
                    return;
                }
                let stat = totals.entry(self.label).or_default();
                stat.calls += 1;
                stat.inclusive_ns = stat.inclusive_ns.saturating_add(elapsed);
                stat.exclusive_ns = stat
                    .exclusive_ns
                    .saturating_add(elapsed.saturating_sub(child));
            }
        }
    }

    /// Adds `bytes` to `label`. No call is counted.
    pub fn add_bytes(label: &'static str, bytes: u64) {
        let gen = GEN.load(Ordering::SeqCst);
        let current = LOCAL
            .try_with(|local| {
                let local = local.borrow();
                local.stack.is_empty() || local.gen == gen
            })
            .unwrap_or(true);
        if !current {
            return;
        }
        let mut totals = totals();
        if gen != GEN.load(Ordering::SeqCst) {
            return;
        }
        let stat = totals.entry(label).or_default();
        stat.bytes = stat.bytes.saturating_add(bytes);
    }

    /// Starts a new phase: clears every total. Guards opened earlier are
    /// discarded when they drop.
    pub fn reset() {
        let mut totals = totals();
        totals.clear();
        GEN.fetch_add(1, Ordering::SeqCst);
    }

    /// The current totals, one entry per label that has been recorded.
    pub fn report() -> serde_json::Value {
        let totals = totals();
        let labels: serde_json::Map<String, serde_json::Value> = totals
            .iter()
            .map(|(label, s)| {
                (
                    label.to_string(),
                    serde_json::json!({
                        "calls": s.calls,
                        "bytes": s.bytes,
                        "inclusive_ns": s.inclusive_ns,
                        "exclusive_ns": s.exclusive_ns,
                    }),
                )
            })
            .collect();
        serde_json::json!({
            "enabled": true,
            "phase": GEN.load(Ordering::SeqCst),
            "units": "ns",
            "threads": "times are summed across threads and can exceed phase wall time",
            "labels": labels,
        })
    }
}

#[cfg(not(feature = "storage-metrics"))]
mod imp {
    /// Empty without the feature: holds nothing and does nothing on drop.
    #[must_use = "bind the guard to a name; a dropped guard times nothing"]
    pub struct SpanGuard;

    #[inline(always)]
    #[must_use = "bind the guard to a name; a dropped guard times nothing"]
    pub fn span(_label: &'static str) -> SpanGuard {
        SpanGuard
    }

    #[inline(always)]
    pub fn add_bytes(_label: &'static str, _bytes: u64) {}

    #[inline(always)]
    pub fn reset() {}

    pub fn report() -> serde_json::Value {
        serde_json::json!({ "enabled": false })
    }
}

pub use imp::{add_bytes, report, reset, span, SpanGuard};

#[cfg(all(test, feature = "storage-metrics"))]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use std::time::{Duration, Instant};

    // The totals are process-wide, so the tests that reset them run one at a time.
    static SERIAL: StdMutex<()> = StdMutex::new(());

    fn label_of(report: &serde_json::Value, label: &str) -> Option<(u64, u64, u64, u64)> {
        let v = report["labels"].get(label)?;
        Some((
            v["calls"].as_u64().unwrap(),
            v["bytes"].as_u64().unwrap(),
            v["inclusive_ns"].as_u64().unwrap(),
            v["exclusive_ns"].as_u64().unwrap(),
        ))
    }

    fn busy(ms: u64) {
        let until = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < until {
            std::hint::spin_loop();
        }
    }

    #[test]
    fn nested_spans_subtract_child_time_exactly() {
        let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
        reset();
        {
            let _outer = span("test.outer");
            busy(2);
            {
                let _inner = span("test.inner");
                busy(3);
            }
            busy(1);
        }
        let r = report();
        let (oc, _, oi, oe) = label_of(&r, "test.outer").expect("outer recorded");
        let (ic, _, ii, ie) = label_of(&r, "test.inner").expect("inner recorded");
        assert_eq!((oc, ic), (1, 1));
        // The parent's exclusive time is its inclusive time minus the child's
        // inclusive time, exactly, because the child's elapsed is what was subtracted.
        assert_eq!(oe, oi - ii);
        assert_eq!(ie, ii, "a leaf has no children");
        // The outer span's own busy time is 2 ms before and 1 ms after the child.
        assert!(
            oe >= 3_000_000,
            "outer self time is at least its 3 ms of busy waiting"
        );
    }

    #[test]
    fn sibling_spans_on_one_thread_do_not_leak_into_each_other() {
        let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
        reset();
        {
            let _p = span("test.parent");
            {
                let _a = span("test.child");
                busy(1);
            }
            {
                let _b = span("test.child");
                busy(1);
            }
        }
        let r = report();
        let (pc, _, pi, pe) = label_of(&r, "test.parent").unwrap();
        let (cc, _, ci, _) = label_of(&r, "test.child").unwrap();
        assert_eq!(cc, 2);
        assert_eq!(pc, 1);
        assert_eq!(pe, pi - ci, "both children are subtracted from the parent");
    }

    #[test]
    fn reset_discards_a_guard_opened_before_it() {
        let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
        reset();
        let old = span("test.old");
        busy(1);
        reset();
        drop(old);
        assert!(
            label_of(&report(), "test.old").is_none(),
            "old guard was charged to the new phase"
        );

        // A child span from the new phase must not be subtracted from an
        // old-phase parent, and the old parent must not appear afterwards.
        let _fresh = span("test.fresh");
        {
            let _inner = span("test.inner_fresh");
            busy(1);
        }
        drop(_fresh);
        let r = report();
        let (fc, _, fi, fe) = label_of(&r, "test.fresh").unwrap();
        let (_, _, ii, _) = label_of(&r, "test.inner_fresh").unwrap();
        assert_eq!(fc, 1);
        assert_eq!(
            fe,
            fi - ii,
            "new-phase accounting is independent of the discarded guard"
        );
        assert!(label_of(&r, "test.old").is_none());
    }

    #[test]
    fn add_bytes_counts_bytes_without_calls() {
        let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
        reset();
        add_bytes("test.bytes", 10);
        add_bytes("test.bytes", 32);
        assert_eq!(label_of(&report(), "test.bytes"), Some((0, 42, 0, 0)));
    }

    #[test]
    fn reset_discards_bytes_from_an_old_active_span() {
        let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
        reset();
        let old = span("test.old_bytes");
        reset();
        add_bytes("test.old_bytes", 42);
        drop(old);
        assert!(label_of(&report(), "test.old_bytes").is_none());
        add_bytes("test.new_bytes", 7);
        assert_eq!(label_of(&report(), "test.new_bytes"), Some((0, 7, 0, 0)));
    }
}
