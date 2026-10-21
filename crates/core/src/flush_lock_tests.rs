//! Unit checks for the vault flush lock rules: same-process waits queue
//! without a deadline, OS contention has its own warning and a fresh
//! deadline measured after local queuing, and instances in one process
//! share one queue.

use super::*;
use std::fmt;
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;
use tracing::field::{Field, Visit};
use tracing::{span, Dispatch, Event, Level, Metadata, Subscriber};

/// A temporary vault root, removed on drop.
struct FlushDir(PathBuf);

impl FlushDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("flush_lock_tests_{}", new_id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn root(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for FlushDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Every event one subscriber saw, with its level and message.
#[derive(Clone, Default)]
struct Events(Arc<std::sync::Mutex<Vec<(Level, String, Option<u64>)>>>);

impl Events {
    fn dispatch(&self) -> Dispatch {
        Dispatch::new(Recorder(self.0.clone()))
    }

    fn warnings(&self) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(level, _, _)| *level == Level::WARN)
            .count()
    }

    /// WARN events that name the flush lock; the OS-wait warning says so.
    fn flush_lock_warnings(&self) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(level, message, wait_ms)| {
                if *level != Level::WARN || !message.contains("flush lock") {
                    return false;
                }
                assert!(
                    wait_ms.is_some_and(|ms| ms >= 2_000),
                    "warning before two seconds"
                );
                true
            })
            .count()
    }
}

struct Recorder(Arc<std::sync::Mutex<Vec<(Level, String, Option<u64>)>>>);

impl Subscriber for Recorder {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }

    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}

    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut message = String::new();
        let mut wait_ms = None;
        event.record(&mut MessageVisitor(&mut message, &mut wait_ms));
        self.0
            .lock()
            .unwrap()
            .push((*event.metadata().level(), message, wait_ms));
    }

    fn enter(&self, _: &span::Id) {}

    fn exit(&self, _: &span::Id) {}
}

struct MessageVisitor<'a>(&'a mut String, &'a mut Option<u64>);

impl Visit for MessageVisitor<'_> {
    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "wait_ms" {
            *self.1 = Some(value);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            *self.0 = format!("{value:?}");
        }
    }
}

/// Result of one `flush_lock` call made on a helper thread.
struct Outcome {
    elapsed: Duration,
    /// When the call returned, after any guard it held was released.
    finished: Instant,
    result: std::result::Result<(), String>,
}

/// Calls `flush_lock` on a helper thread that reports to the same dispatch
/// the test reads. The guard is released as soon as it is acquired.
fn spawn_flusher(inner: Arc<Inner>, dispatch: Dispatch) -> (Receiver<Outcome>, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        tracing::dispatcher::with_default(&dispatch, || {
            let started = Instant::now();
            let result = inner
                .flush_lock()
                .map(|_guard| ())
                .map_err(|e| e.to_string());
            let finished = Instant::now();
            let _ = tx.send(Outcome {
                elapsed: started.elapsed(),
                finished,
                result,
            });
        });
    });
    (rx, handle)
}

/// Arms the one-shot contention notification on `inner` before any waiter
/// starts, so the first real blocked attempt is the one received.
fn arm_blocked(inner: &Inner) -> Receiver<()> {
    let (tx, rx) = mpsc::channel();
    *lock(&inner.disk_io.lock_blocked) = Some(tx);
    rx
}

#[test]
fn same_process_blocking_wait_has_no_deadline_and_no_os_warning() {
    let dir = FlushDir::new();
    let vault = Vault::open(dir.root()).unwrap();
    let inner = vault.inner.clone();
    let holder = inner.flush_lock().unwrap();
    let blocked = arm_blocked(&inner);
    let events = Events::default();
    let (rx, waiter) = spawn_flusher(inner.clone(), events.dispatch());
    let noticed = blocked.recv_timeout(Duration::from_secs(10)).is_ok();
    if noticed {
        // Hold well past FLUSH_LOCK_TIMEOUT: a same-process waiter must still
        // queue rather than time out.
        std::thread::sleep(Duration::from_secs(12));
    }
    drop(holder);
    let outcome = rx.recv_timeout(Duration::from_secs(10));
    waiter.join().expect("waiter panicked");
    assert!(noticed, "waiter never reached the held flush lock");
    let outcome = outcome.expect("waiter did not return after the holder dropped");
    outcome
        .result
        .expect("same-process waiter must acquire after the holder drops");
    assert!(
        outcome.elapsed >= Duration::from_secs(12),
        "waiter returned before the holder released: {:?}",
        outcome.elapsed
    );
    assert_eq!(events.warnings(), 0, "same-process queueing must not warn");
}

#[test]
fn os_holder_times_out_after_ten_seconds_and_warns_once() {
    let dir = FlushDir::new();
    let vault = Vault::open(dir.root()).unwrap();
    // An independent open file description holds the OS lock, as another
    // process would.
    let holder = open_flush_file(&dir.0).unwrap();
    assert!(try_lock_exclusive(&holder).unwrap());
    let events = Events::default();
    let (rx, waiter) = spawn_flusher(vault.inner.clone(), events.dispatch());
    let outcome = rx.recv_timeout(FLUSH_LOCK_TIMEOUT + Duration::from_secs(10));
    drop(holder);
    waiter.join().expect("waiter panicked");
    let outcome = outcome.expect("waiter did not return within the bound");
    let error = outcome.result.expect_err("a held OS lock must time out");
    assert!(
        error.contains("timed out waiting for the vault flush lock"),
        "unexpected error: {error}"
    );
    assert!(
        outcome.elapsed >= FLUSH_LOCK_TIMEOUT - Duration::from_millis(100),
        "timed out before the fresh deadline: {:?}",
        outcome.elapsed
    );
    assert!(
        outcome.elapsed < FLUSH_LOCK_TIMEOUT + Duration::from_secs(3),
        "timed out long after the deadline: {:?}",
        outcome.elapsed
    );
    assert_eq!(
        events.flush_lock_warnings(),
        1,
        "OS wait must warn exactly once"
    );
}

#[test]
fn os_wait_warns_once_then_acquires_after_holder_drops() {
    let dir = FlushDir::new();
    let vault = Vault::open(dir.root()).unwrap();
    let holder = open_flush_file(&dir.0).unwrap();
    assert!(try_lock_exclusive(&holder).unwrap());
    let blocked = arm_blocked(&vault.inner);
    let events = Events::default();
    let (rx, waiter) = spawn_flusher(vault.inner.clone(), events.dispatch());
    let noticed = blocked.recv_timeout(Duration::from_secs(10)).is_ok();
    if noticed {
        // Past FLUSH_LOCK_WARN_AFTER (2 s) and well inside the 10 s deadline.
        std::thread::sleep(Duration::from_secs(3));
    }
    let released = Instant::now();
    drop(holder);
    let outcome = rx.recv_timeout(Duration::from_secs(10));
    waiter.join().expect("waiter panicked");
    assert!(noticed, "waiter never reached the held OS lock");
    let outcome = outcome.expect("waiter did not return after the holder dropped");
    outcome
        .result
        .expect("waiter must acquire once the OS holder drops");
    assert!(
        released.elapsed() < Duration::from_secs(5),
        "acquisition after drop took {:?}",
        released.elapsed()
    );
    assert!(outcome.elapsed >= Duration::from_secs(3));
    assert_eq!(
        events.flush_lock_warnings(),
        1,
        "OS wait must warn exactly once"
    );
}

#[test]
fn os_deadline_starts_after_local_queue_ends() {
    // W takes the local queue and then waits on the OS lock for its own
    // deadline. X queues behind W on the local mutex, so X's OS deadline
    // must start only once X gets the local queue, not when X called.
    let dir = FlushDir::new();
    let vault = Vault::open(dir.root()).unwrap();
    let inner = vault.inner.clone();
    let holder = open_flush_file(&dir.0).unwrap();
    assert!(try_lock_exclusive(&holder).unwrap());
    let blocked = arm_blocked(&inner);
    let events_w = Events::default();
    let events_x = Events::default();
    let (w_rx, w) = spawn_flusher(inner.clone(), events_w.dispatch());
    let w_noticed = blocked.recv_timeout(Duration::from_secs(10)).is_ok();
    let mut x = None;
    if w_noticed {
        // The contention notice is sent from inside W's flush_lock, after W
        // took the local queue, so X can start now without a fixed pause: it
        // queues behind W for as long as W waits on the OS lock.
        x = Some(spawn_flusher(inner.clone(), events_x.dispatch()));
    }
    let w_outcome = w_rx.recv_timeout(FLUSH_LOCK_TIMEOUT + Duration::from_secs(10));
    let x_outcome = x.as_ref().map(|(rx, _)| {
        rx.recv_timeout(2 * FLUSH_LOCK_TIMEOUT + Duration::from_secs(10))
    });
    drop(holder);
    w.join().expect("W panicked");
    if let Some((_, x)) = x {
        x.join().expect("X panicked");
    }
    assert!(w_noticed, "W never reached the held OS lock");
    let w_outcome = w_outcome.expect("W did not return within the bound");
    let w_finished = w_outcome.finished;
    let w_error = w_outcome
        .result
        .expect_err("W must time out on the OS lock");
    assert!(w_error.contains("timed out waiting for the vault flush lock"));
    let x_outcome = x_outcome
        .expect("X was never started")
        .expect("X did not return within the bound");
    let x_finished = x_outcome.finished;
    let x_error = x_outcome
        .result
        .expect_err("X must time out on the OS lock");
    assert!(x_error.contains("timed out waiting for the vault flush lock"));
    // X cannot take the local queue before W releases it, so X's own deadline
    // runs from some point after W finished. Measured from W's finish, X
    // must spend its full deadline; a deadline that began at X's call would
    // end only about 0.5 s after W. The 1 s slack covers scheduling delay
    // between W's release and its recorded finish.
    let x_after_w = x_finished.saturating_duration_since(w_finished);
    assert!(
        x_after_w >= FLUSH_LOCK_TIMEOUT - Duration::from_secs(1),
        "X's OS deadline did not start after W left the local queue: {x_after_w:?}"
    );
    assert!(
        x_after_w < FLUSH_LOCK_TIMEOUT + Duration::from_secs(5),
        "X timed out long after its fresh deadline: {x_after_w:?}"
    );
    assert_eq!(events_w.flush_lock_warnings(), 1);
    assert_eq!(events_x.flush_lock_warnings(), 1);
}

#[test]
fn separate_instances_share_one_queue_without_os_warnings() {
    let dir = FlushDir::new();
    let a = Vault::open(dir.root()).unwrap();
    // The same directory through a non-canonical spelling: the shared queue
    // is keyed by canonical path.
    let b = Vault::open(format!("{}/.", dir.root())).unwrap();
    let a_inner = a.inner.clone();
    let b_inner = b.inner.clone();
    let holder = a_inner.flush_lock().unwrap();
    let blocked = arm_blocked(&b_inner);
    let events = Events::default();
    let (rx, waiter) = spawn_flusher(b_inner.clone(), events.dispatch());
    let noticed = blocked.recv_timeout(Duration::from_secs(10)).is_ok();
    if noticed {
        // Longer than the 2 s OS warning threshold. Separate descriptors
        // would contend at the OS level here and warn; a shared queue must not.
        std::thread::sleep(Duration::from_secs(3));
    }
    drop(holder);
    let outcome = rx.recv_timeout(Duration::from_secs(10));
    waiter.join().expect("waiter panicked");
    assert!(noticed, "second instance never reached the held guard");
    let outcome = outcome.expect("second instance did not return after release");
    outcome
        .result
        .expect("second instance must acquire after the first releases");
    assert!(
        outcome.elapsed >= Duration::from_secs(3),
        "second instance did not wait for the first: {:?}",
        outcome.elapsed
    );
    assert_eq!(events.warnings(), 0, "shared-queue waits must not warn");
}

#[test]
fn os_wait_crossing_warning_threshold_before_acquisition_still_warns() {
    let dir = FlushDir::new();
    let vault = Vault::open(dir.root()).unwrap();
    let holder = open_flush_file(&dir.0).unwrap();
    assert!(try_lock_exclusive(&holder).unwrap());
    let (entered_tx, entered_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    *lock(&vault.inner.disk_io.pause_before_os_retry) = Some(FlushPause {
        entered: entered_tx,
        resume: resume_rx,
    });
    let events = Events::default();
    let (rx, waiter) = spawn_flusher(vault.inner.clone(), events.dispatch());
    let paused = entered_rx.recv_timeout(Duration::from_secs(10)).is_ok();
    if paused {
        // The first OS attempt failed before the warning threshold. Release
        // the holder before the next attempt, after crossing that threshold.
        std::thread::sleep(Duration::from_millis(2_200));
    }
    drop(holder);
    let _ = resume_tx.send(());
    let outcome = rx.recv_timeout(Duration::from_secs(10));
    waiter.join().expect("waiter panicked");
    assert!(paused, "waiter did not reach the OS retry pause");
    let outcome = outcome.unwrap();
    outcome.result.unwrap();
    assert!(outcome.elapsed >= FLUSH_LOCK_WARN_AFTER);
    assert_eq!(
        events.flush_lock_warnings(),
        1,
        "a successful acquisition must still warn about its preceding OS wait"
    );
}
