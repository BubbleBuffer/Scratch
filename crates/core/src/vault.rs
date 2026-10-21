//! `Vault`: the public entry point. See `wiki/architecture/core-state.md` for
//! the storage design (update logs, background writer, index).
//!
//! The API is split over several files, each an `impl Vault` block:
//! this one (open, pages, blocks, flush, reindex), `tree.rs` (order, moves,
//! duplicates, trash, favorites, icon and cover), `collections.rs`
//! (properties, schemas, views, templates, queries), `media.rs` (links,
//! attachments, history, change feed) and `import.rs` (markdown import and
//! rescan).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use loro::ExportMode;

use crate::attach;
use crate::doc::{plain_text, PageDoc, PageMeta};
use crate::error::{invalid, not_found, Result, VaultError};
use crate::feed::Feed;
use crate::ids::{new_id, now_ms, stamp_ms};
use crate::index::{ContentRow, Index, MetaRow};
use crate::markdown::{self, RenderCtx};
use crate::order;
#[cfg(feature = "storage-metrics")]
use crate::storage_metrics;
use crate::types::{
    Block, BlockKind, BlockOp, Change, ExternalSearchRow, MarkKind, Page, PageId, PageSummary,
    PropValue, Property, SearchHit, SearchIndexStats, SearchResults,
};

#[path = "cold.rs"]
pub(crate) mod cold;

/// Default retention window, measured using Loro commit timestamps.
const COLD_AFTER: Duration = Duration::from_secs(30 * 86400);
/// Minimum uncompressed eligible history for a live page.
const COLD_MIN_BYTES: usize = 16 * 1024;

/// Device-local policy; changing it never changes the retained history.
#[derive(Clone, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ColdPolicy {
    age_seconds: i64,
    minimum_bytes: usize,
    pages_per_pass: usize,
    idle_seconds: u64,
}

impl Default for ColdPolicy {
    fn default() -> Self {
        Self { age_seconds: COLD_AFTER.as_secs() as i64, minimum_bytes: COLD_MIN_BYTES,
            pages_per_pass: 2, idle_seconds: 5 }
    }
}

/// Quiet time after the last change before the writer flushes.
const DEBOUNCE: Duration = Duration::from_millis(300);
/// Longest time a change waits for the writer during a continuous burst.
const MAX_DELAY: Duration = Duration::from_millis(2000);
/// Loaded page documents kept in memory (dirty ones are never evicted).
const CACHE_CAP: usize = 32;
/// Pages with at least this many blocks get a block cache file.
const BLOCK_CACHE_MIN: usize = 256;
/// Pages the writer flushes per index transaction.
const WRITER_BATCH: usize = 64;
/// Only time waiting for another OS lock holder counts toward this deadline.
const FLUSH_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const FLUSH_LOCK_WARN_AFTER: Duration = Duration::from_secs(2);
const FLUSH_LOCK_POLL: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileIdentity {
    volume: u64,
    index: u128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileStamp {
    identity: Option<FileIdentity>,
    len: u64,
}

#[derive(Clone, Debug)]
struct SeenFile {
    stamp: Option<FileStamp>,
}

impl SeenFile {
    fn observed(stamp: Option<FileStamp>) -> Self {
        Self { stamp }
    }

    fn unchanged(&self, stamp: &Option<FileStamp>) -> bool {
        self.stamp == *stamp
    }
}

#[derive(Default)]
pub(crate) struct SeenFiles {
    snapshot: Option<SeenFile>,
    old_log: Option<SeenFile>,
    log: Option<SeenFile>,
    sequence: Option<DiskSequence>,
}

/// The epoch changes when repairing an empty/torn record, so repair cannot
/// reset the counter to a value a surviving instance already trusted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DiskSequence {
    epoch: [u8; 26],
    counter: u64,
}

const SEQUENCE_BYTES: usize = 26 + 8 + 8;

fn read_sequence(file: &File) -> Result<Option<DiskSequence>> {
    let mut file = file;
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = [0; SEQUENCE_BYTES];
    match file.read_exact(&mut bytes) {
        Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
        Ok(()) => {}
    }
    let mut extra = [0];
    if file.read(&mut extra)? != 0 {
        return Ok(None);
    }
    let counter = u64::from_le_bytes(bytes[26..34].try_into().expect("counter"));
    let inverse = u64::from_le_bytes(bytes[34..42].try_into().expect("inverse"));
    if inverse != !counter || !bytes[..26].iter().all(u8::is_ascii_alphanumeric) {
        return Ok(None);
    }
    Ok(Some(DiskSequence {
        epoch: bytes[..26].try_into().expect("epoch"),
        counter,
    }))
}

fn sync_parent_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn open_flush_file(root: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(".vault").join("flush.lock"))
}

type FlushFile = Mutex<Option<File>>;

/// All instances of a vault in this process join the same blocking queue,
/// including instances opened through relative paths or directory symlinks.
fn shared_flush_file(root: &Path) -> std::io::Result<Arc<FlushFile>> {
    static FILES: OnceLock<Mutex<HashMap<PathBuf, Weak<FlushFile>>>> = OnceLock::new();
    let file = open_flush_file(root)?;
    let path = fs::canonicalize(root.join(".vault").join("flush.lock"))?;
    let mut files = lock(FILES.get_or_init(|| Mutex::new(HashMap::new())));
    files.retain(|_, file| file.strong_count() != 0);
    if let Some(shared) = files.get(&path).and_then(Weak::upgrade) {
        return Ok(shared);
    }
    let shared = Arc::new(Mutex::new(Some(file)));
    files.insert(path, Arc::downgrade(&shared));
    Ok(shared)
}

fn flush_lock_timeout() -> VaultError {
    std::io::Error::new(
        ErrorKind::TimedOut,
        "timed out waiting for the vault flush lock",
    )
    .into()
}

struct AppendLog {
    file: File,
    identity: Option<FileIdentity>,
}

/// A fresh handle per acquisition excludes both other vault instances and
/// threads in this process, including on platforms with flock ownership.
pub(crate) struct DailyNotesLock {
    file: File,
}

impl Drop for DailyNotesLock {
    fn drop(&mut self) {
        // Closing the owned handle also releases the lock if unlock fails.
        let _ = unlock_exclusive(&self.file);
    }
}

/// A shared OS handle needs a same-process mutex as well: flock ownership
/// belongs to the open file description, so locking it again alone would
/// not exclude another thread using this cached handle.
struct FlushLock<'a> {
    file: MutexGuard<'a, Option<File>>,
    root: &'a Path,
}

thread_local! {
    static HELD_FLUSH_ROOTS: std::cell::RefCell<HashSet<PathBuf>> = std::cell::RefCell::new(HashSet::new());
}

/// Forks outside the writer (for example markdown rescan) also need an
/// atomic hot/cold read. Reuse an enclosing flush lock on the same thread.
pub(crate) fn with_cold_read_lock<T>(root: &Path, read: impl FnOnce() -> Result<T>) -> Result<T> {
    if HELD_FLUSH_ROOTS.with(|held| held.borrow().contains(root)) { return read(); }
    let file = open_flush_file(root)?;
    let started = Instant::now();
    {
        #[cfg(feature = "storage-metrics")]
        let _w = storage_metrics::span("wait.cold_read_lock");
        while !try_lock_exclusive(&file)? {
            if started.elapsed() >= FLUSH_LOCK_TIMEOUT { return Err(flush_lock_timeout()); }
            std::thread::sleep(FLUSH_LOCK_POLL);
        }
    }
    struct ReadLock(File);
    impl Drop for ReadLock { fn drop(&mut self) { let _ = unlock_exclusive(&self.0); } }
    let _guard = ReadLock(file);
    read()
}

impl std::ops::Deref for FlushLock<'_> {
    type Target = File;

    fn deref(&self) -> &File {
        self.file.as_ref().expect("acquired flush handle")
    }
}

impl Drop for FlushLock<'_> {
    fn drop(&mut self) {
        HELD_FLUSH_ROOTS.with(|held| { held.borrow_mut().remove(self.root); });
        if self
            .file
            .as_ref()
            .is_some_and(|file| unlock_exclusive(file).is_err())
        {
            // Closing is the final release mechanism if explicit unlock
            // fails. The next acquisition safely opens a new handle.
            drop(self.file.take());
        }
    }
}

#[cfg(test)]
struct FlushPause {
    entered: Sender<()>,
    resume: Receiver<()>,
}

#[cfg(test)]
#[derive(Default)]
struct DiskIo {
    // Snapshot, rotated log, live log, respectively. Includes postwrite
    // snapshot metadata probes, rather than hiding them from the counts.
    stats: [std::sync::atomic::AtomicUsize; 3],
    reads: [std::sync::atomic::AtomicUsize; 3],
    interrupt_after_snapshot: std::sync::atomic::AtomicBool,
    startup_metadata: std::sync::atomic::AtomicUsize,
    startup_recoveries: std::sync::atomic::AtomicUsize,
    pause_after_merge: Mutex<Option<FlushPause>>,
    lock_blocked: Mutex<Option<Sender<()>>>,
    pause_before_os_retry: Mutex<Option<FlushPause>>,
    /// Installed snapshots and elapsed advisory-lock hold for each batch.
    batch_holds: Mutex<Vec<(usize, Duration)>>,
    pause_after_batch_finish: Mutex<Option<FlushPause>>,
}

/// `.vault/docs/<id>.blocks.json`: the page's blocks as of one document
/// version, so a cold `load_page` of a big page need not wait for Loro to
/// decode every block's state. Used only when `version` matches.
#[derive(serde::Deserialize)]
struct BlockCache {
    version: Vec<u8>,
    blocks: Vec<Block>,
}

#[derive(serde::Serialize)]
struct BlockCacheRef<'a> {
    version: &'a [u8],
    blocks: &'a [Block],
}

fn count_blocks(b: &[Block]) -> usize {
    b.iter().map(|x| 1 + count_blocks(&x.children)).sum()
}

/// An open vault. `Send + Sync`; share it behind an `Arc`. Every method is
/// synchronous and meant to be called off the UI thread.
///
/// Dropping the vault flushes pending writes and stops the writer thread.
pub struct Vault {
    pub(crate) inner: Arc<Inner>,
    writer: Mutex<Option<JoinHandle<()>>>,
    live_worker: Mutex<Option<JoinHandle<()>>>,
}

thread_local! {
    // A synchronous request's actor belongs to one vault on one thread.
    // Other callers and the background writer retain the device identity.
    static ACTORS: RefCell<HashMap<usize, String>> = RefCell::new(HashMap::new());
}

struct ActorGuard {
    vault: usize,
    previous: Option<String>,
}

impl Drop for ActorGuard {
    fn drop(&mut self) {
        ACTORS.with(|actors| {
            let mut actors = actors.borrow_mut();
            if let Some(previous) = self.previous.take() {
                actors.insert(self.vault, previous);
            } else {
                actors.remove(&self.vault);
            }
        });
    }
}

impl Vault {
    /// Attributes synchronous writes in `call` to `actor` in the existing
    /// page peer map. The label is scoped to this vault and thread, restored
    /// on return or panic, and must not cross threads or an async suspension.
    pub fn with_actor<T>(&self, actor: &str, call: impl FnOnce() -> T) -> T {
        let vault = Arc::as_ptr(&self.inner) as usize;
        let previous = ACTORS.with(|actors| actors.borrow_mut().insert(vault, actor.to_owned()));
        let _guard = ActorGuard { vault, previous };
        call()
    }
}

pub(crate) struct Inner {
    pub root: PathBuf,
    pub db: Mutex<Index>,
    pub(crate) search_metadata: Mutex<crate::search_api::SearchMetadata>,
    cache: Mutex<HashMap<PageId, Arc<Slot>>>,
    tx: Sender<Msg>,
    writer_error: Mutex<Option<String>>,
    /// This vault copy's device id (`.vault/device`), for page history.
    pub device: String,
    /// The Loro peer id of this device's changes, derived from `device`,
    /// so a device adds one entry to version vectors, not one per session.
    pub peer: u64,
    feed: Mutex<Feed>,
    /// Notified on every change feed push (`wait_changes`).
    feed_cv: Condvar,
    pub(crate) index_generation: AtomicU64,
    pub(crate) live_queries: Mutex<crate::live_query::Registry>,
    pub(crate) live_stop: AtomicBool,
    /// Pages purged here or on another device (`.vault/purged`), in order.
    pub purged: Mutex<Purged>,
    pub purge_sync: Mutex<()>,
    /// `.vault/lock`, locked by the first instance open on the vault (the
    /// one that uses the device's peer id).
    _lock: File,
    /// Cached control-file handle, shared by this process's vault instances
    /// before taking the cross-process advisory lock.
    flush_file: Arc<FlushFile>,
    /// Folders mounted on this device (`mounts.rs`).
    pub mounts: Mutex<Vec<crate::types::Mount>>,
    /// Mounted files as `rescan_mounts` last saw them.
    pub mount_seen: Mutex<crate::mounts::Seen>,
    pub daily_notes: Mutex<()>,
    cold_policy: ColdPolicy,
    cold_pages: Mutex<HashSet<PageId>>,
    #[cfg(test)]
    cold_before_publish: Mutex<Option<Box<dyn FnOnce(&Inner, &str) + Send>>>,
    #[cfg(test)]
    disk_io: DiskIo,
    /// Serializes lazy calendar settings creation and list edits.
    pub calendars: Mutex<()>,
    /// Serializes lazy creation of the Nodes page and node record writes.
    pub nodes: Mutex<()>,
    /// The synced conflict switch, readable while database/page locks are held.
    pub(crate) conflict_auto_resolve_cache: std::sync::atomic::AtomicBool,
}

/// Active tombstones and durable acknowledgements of restored generations.
#[derive(Default, Clone)]
pub(crate) struct Purged {
    pub list: Vec<PageId>,
    pub set: HashSet<PageId>,
    pub versions: HashMap<PageId, Option<Vec<u8>>>,
    /// The purging device of each active tombstone whose device is known.
    pub devices: HashMap<PageId, String>,
    pub restored: Vec<crate::sync::RestoreRecord>,
}

impl Purged {
    /// The tombstone `record` leaves on its page: its coverage and purging
    /// device after the merge rule. Restore checks and persistence both use it.
    pub(crate) fn effect(&self, record: &crate::sync::PurgeRecord) -> Result<crate::sync::PurgeRecord> {
        let active = self.versions.get(&record.page).cloned().flatten();
        let known = self.devices.get(&record.page).cloned();
        let (version, device) = crate::sync::merge_purge_identity(active, known, record)?;
        Ok(crate::sync::PurgeRecord { page: record.page.clone(), version, device })
    }
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct PurgeState {
    records: Vec<crate::sync::PurgeRecord>,
    #[serde(default)]
    restored: Vec<crate::sync::RestoreRecord>,
}

pub(crate) struct Slot {
    pub state: Mutex<PageState>,
    /// Held while the page's files are written or removed.
    pub io: Mutex<()>,
}

pub(crate) struct PageState {
    pub doc: PageDoc,
    /// Changed since the writer last took a snapshot.
    dirty: bool,
    pub deleted: bool,
    log: Option<AppendLog>,
    seen: SeenFiles,
    changed_on_disk: bool,
    // The last snapshot this instance read or wrote. Existing logs still
    // require compaction even when this version matches the in-memory doc.
    snapshot_version: Vec<u8>,
    /// Changes of this page were exported for sync since its last durable
    /// snapshot: the next flush syncs the snapshot to disk before dropping
    /// the log, so ops another device has are never lost here (their ids
    /// would be reused by this device's fixed peer id).
    pub exported: bool,
}

enum Msg {
    Dirty(PageId),
    /// Flush even if the page is not loaded or not dirty (crash recovery,
    /// reindex, re-rendering after a rename elsewhere).
    Recover(PageId),
    /// Decode a loaded page's Loro state ahead of the first edit.
    Warm(PageId),
    Flush(Sender<()>),
    Stop,
    #[cfg(test)]
    Crash,
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

pub(crate) fn remove_if_exists(p: &Path) -> Result<()> {
    match fs::remove_file(p) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Reads length-prefixed update frames; a torn last frame is ignored.
pub(crate) fn decode_frames(bytes: &[u8], out: &mut Vec<Vec<u8>>) {
    let mut i = 0;
    while i + 4 <= bytes.len() {
        let len = u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        if i + 4 + len > bytes.len() {
            break;
        }
        out.push(bytes[i + 4..i + 4 + len].to_vec());
        i += 4 + len;
    }
}

/// The index's metadata row for a page document.
pub(crate) fn meta_row(doc: &PageDoc) -> MetaRow {
    let m = doc.meta();
    let task_status = (m.page_type == "task").then(|| {
        let data: serde_json::Value = serde_json::from_str(&m.type_data).unwrap_or_default();
        data.get("tasks")
            .and_then(|tasks| tasks.get("status"))
            .and_then(serde_json::Value::as_str)
            .filter(|status| matches!(*status, "todo" | "doing" | "done"))
            .unwrap_or("todo")
            .to_owned()
    });
    let schema = if m.page_type == crate::types::PAGE_TYPE_DATABASE {
        serde_json::to_string(&doc.schema().unwrap_or_default()).ok()
    } else {
        None
    };
    MetaRow {
        id: m.id,
        title: m.title,
        parent: m.parent,
        created: m.created,
        updated: m.updated,
        sort_key: m.order,
        icon: m.icon,
        color: m.color,
        role: m.role,
        agenda_role: m.agenda_role,
        agenda_date: m.agenda_date,
        cover: m.cover,
        favorite: m.favorite,
        trashed: m.trashed,
        page_type: m.page_type,
        schema,
        template: m.template,
        is_template: m.is_template,
        props: doc.props(),
        task_status,
    }
}

/// Pages a page links to: `PageLink` marks and `LinkToPage` blocks.
pub(crate) fn links_of(blocks: &[Block], out: &mut Vec<String>) {
    for b in blocks {
        for m in &b.marks {
            if let MarkKind::PageLink(id) = &m.kind {
                out.push(id.clone());
            }
        }
        if matches!(
            b.kind,
            BlockKind::LinkToPage | BlockKind::Database | BlockKind::Embed
        ) {
            if let Some(t) = &b.target {
                out.push(t.clone());
            }
        }
        links_of(&b.children, out);
    }
}

/// Attachments a page uses: `Image` and `File` blocks, and the cover.
pub(crate) fn refs_of(blocks: &[Block], out: &mut Vec<String>) {
    for b in blocks {
        if matches!(b.kind, BlockKind::Image | BlockKind::File) {
            if let Some(t) = b.target.as_ref().filter(|t| attach::is_attachment_id(t)) {
                out.push(t.clone());
            }
        }
        refs_of(&b.children, out);
    }
}

/// Whether a block tree holds an embed (its search text depends on other titles).
pub(crate) fn has_embed(blocks: &[Block]) -> bool {
    blocks
        .iter()
        .any(|b| b.kind == BlockKind::Embed || has_embed(&b.children))
}

pub(crate) fn content_row(
    meta: &PageMeta,
    props: &[Property],
    blocks: &[Block],
    titles: &dyn Fn(&str) -> Option<String>,
) -> ContentRow {
    let mut links = Vec::new();
    links_of(blocks, &mut links);
    links.sort();
    links.dedup();
    let mut refs = Vec::new();
    refs_of(blocks, &mut refs);
    refs.extend(meta.cover.iter().cloned());
    for p in props {
        if let PropValue::Files(ids) = &p.value {
            refs.extend(ids.iter().filter(|t| attach::is_attachment_id(t)).cloned());
        }
    }
    refs.sort();
    refs.dedup();
    ContentRow {
        body: plain_text(blocks, titles),
        md_path: None,
        md_hash: None,
        md_version: None,
        links,
        refs,
        vv: None,
        todos: crate::agenda::page_agenda(blocks, &meta.type_data),
    }
}

/// A `Page` from a document and its blocks.
pub(crate) fn page_of(doc: &PageDoc, blocks: Vec<Block>, in_trash: bool) -> Page {
    let m = doc.meta();
    Page {
        id: m.id,
        title: m.title,
        blocks,
        parent: m.parent,
        icon: m.icon,
        cover: m.cover,
        properties: doc.props(),
        page_type: m.page_type,
        favorite: m.favorite.is_some(),
        in_trash,
        created_ms: m.created,
        updated_ms: m.updated,
        prefs: m.prefs,
        color: m.color,
        role: m.role,
    }
}

/// `write_atomic`, with the file synced to disk before the rename.
pub(crate) fn write_durable(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut f = File::create(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    fs::rename(&tmp, path)?;
    sync_parent_dir(path)
}

/// The Loro peer id of a device: the first 8 bytes of the SHA-256 of its
/// id, kept to 63 bits.
pub(crate) fn peer_of(device: &str) -> u64 {
    let h = attach::sha256_hex(device.as_bytes());
    u64::from_str_radix(&h[..16], 16).unwrap_or(1) & 0x7fff_ffff_ffff_ffff
}

/// Reads `.vault/purged` (one page id per line).
fn read_purged(root: &Path) -> Result<Purged> {
    let mut p = Purged::default();
    match fs::read_to_string(root.join(".vault").join("purged")) {
        Ok(s) => {
            for id in s.lines().map(str::trim).filter(|l| !l.is_empty()) {
                if p.set.insert(id.to_string()) {
                    p.list.push(id.to_string());
                }
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    match fs::read(root.join(".vault").join("purge-state.json")) {
        Ok(bytes) => {
            let state: PurgeState = serde_json::from_slice(&bytes)
                .map_err(|e| invalid(format!("bad purge state: {e}")))?;
            p.restored = state.restored;
            for record in state.records {
                if p.set.insert(record.page.clone()) {
                    p.list.push(record.page.clone());
                }
                match record.device {
                    Some(device) => p.devices.insert(record.page.clone(), device),
                    None => p.devices.remove(&record.page),
                };
                p.versions.insert(record.page, record.version);
            }
            // A crash between the sidecar and ID-list rewrite can leave a
            // restored ID in the old list. The acknowledgement is durable.
            p.list.retain(|id| {
                p.versions.contains_key(id) || !p.restored.iter().any(|r| &r.page == id)
            });
            p.set = p.list.iter().cloned().collect();
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(p)
}

/// Reads or creates `.vault/device`.
fn device_id(root: &Path) -> Result<String> {
    let path = root.join(".vault").join("device");
    if let Ok(s) = fs::read_to_string(&path) {
        let s = s.trim();
        if !s.is_empty() {
            return Ok(s.to_string());
        }
    }
    let id = format!("{:016x}", {
        let a = new_id();
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in a.bytes().chain(now_ms().to_le_bytes()) {
            h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
        h ^ (std::process::id() as u64).rotate_left(32)
    });
    write_atomic(&path, id.as_bytes())?;
    Ok(id)
}

impl Inner {
    fn actor(&self) -> Option<String> {
        ACTORS.with(|actors| {
            actors
                .borrow()
                .get(&(self as *const Self as usize))
                .cloned()
        })
    }

    fn actor_label(&self) -> String {
        self.actor().unwrap_or_else(|| self.device.clone())
    }

    pub(crate) fn actor_peer(&self) -> u64 {
        match self.actor() {
            Some(actor) => peer_of(&format!("{}:{}:actor:{actor}", self.device, self.peer)),
            None => self.peer,
        }
    }

    pub(crate) fn docs_dir(&self) -> PathBuf {
        self.root.join(".vault").join("docs")
    }
    fn logs_dir(&self) -> PathBuf {
        self.root.join(".vault").join("logs")
    }
    pub(crate) fn pages_dir(&self) -> PathBuf {
        self.root.join("pages")
    }
    /// A restoration may discard exported counters from this device's old
    /// branch. Its durable generation marker gives every device a fresh,
    /// stable editing peer, preventing those counters from being reused.
    pub(crate) fn page_peer(&self, doc: &PageDoc) -> u64 {
        let peer = self.actor_peer();
        match doc.doc.get_map("page").get("purge_restore") {
            Some(loro::ValueOrContainer::Value(loro::LoroValue::String(marker))) => {
                peer_of(&format!("{}:{}:{}", self.device, peer, marker.as_str()))
            }
            _ => peer,
        }
    }

    pub(crate) fn doc_path(&self, id: &str) -> PathBuf {
        self.docs_dir().join(format!("{id}.loro"))
    }
    pub(crate) fn log_path(&self, id: &str) -> PathBuf {
        self.logs_dir().join(format!("{id}.log"))
    }

    fn conflict_archive_dir(&self) -> PathBuf {
        self.root.join(".vault").join("conflict-history")
    }

    fn conflict_archive_path(&self, id: &str) -> Result<PathBuf> {
        if id.is_empty() || id.contains(['/', '\\']) || id == "." || id == ".." {
            return Err(invalid("invalid conflict archive page id"));
        }
        Ok(self.conflict_archive_dir().join(format!("{id}.loro")))
    }

    /// Read a retained page history without installing an active page slot.
    pub(crate) fn read_conflict_archive(&self, id: &str) -> Result<PageDoc> {
        let flush = self.flush_lock()?;
        self.read_conflict_archive_locked(id, &flush)
    }

    fn read_conflict_archive_locked(&self, id: &str, _flush: &FlushLock<'_>) -> Result<PageDoc> {
        let bytes = match fs::read(self.conflict_archive_path(id)?) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == ErrorKind::NotFound => return Err(not_found("conflict archive", id)),
            Err(e) => return Err(e.into()),
        };
        let doc = PageDoc::from_bytes(&bytes, &[])?;
        crate::conflict_archive::validate(id, &doc)?;
        Ok(doc)
    }

    /// Retain all known history. The advisory lock serializes archive writers
    /// across Vault instances/processes; no page cache or purge lock is taken.
    pub(crate) fn store_conflict_archive(&self, id: &str, doc: &PageDoc) -> Result<()> {
        let changed = {
            let flush = self.flush_lock()?;
            self.store_conflict_archive_locked(id, doc, &flush)?
        };
        if changed {
            self.emit([Change::PageChanged(id.to_owned())]);
        }
        Ok(())
    }

    /// Call only while already holding flush.lock, including from purge_files.
    /// Merge the existing archive before replacement so a stale reader cannot
    /// discard another writer's conflict operations or historical frontiers.
    fn store_conflict_archive_locked(
        &self,
        id: &str,
        doc: &PageDoc,
        flush: &FlushLock<'_>,
    ) -> Result<bool> {
        let old = match self.read_conflict_archive_locked(id, flush) {
            Ok(old) => Some(old),
            Err(VaultError::NotFound(_)) => None,
            Err(e) => return Err(e),
        };
        let merged = crate::conflict_archive::merge(id, doc, old.as_ref())?;
        if old.as_ref().is_some_and(|old| old.doc.oplog_vv() == merged.doc.oplog_vv()) {
            return Ok(false);
        }
        let directory = self.conflict_archive_dir();
        fs::create_dir_all(&directory)?;
        sync_parent_dir(&directory)?;
        self.bump_sequence(flush, read_sequence(flush)?)?;
        write_durable(&self.conflict_archive_path(id)?, &merged.snapshot()?)?;
        Ok(true)
    }

    pub(crate) fn archive_page_ids(&self) -> Result<Vec<String>> {
        let _flush = self.flush_lock()?;
        let entries = match fs::read_dir(self.conflict_archive_dir()) {
            Ok(entries) => entries,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut ids = Vec::new();
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(id) = name.strip_suffix(".loro") {
                self.conflict_archive_path(id)?;
                ids.push(id.to_owned());
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// Edit the latest retained document under the advisory lock. The caller
    /// owns lifecycle and conflict-mirror updates; this never creates a page row.
    pub(crate) fn edit_conflict_archive<T>(
        &self,
        id: &str,
        edit: impl FnOnce(&mut PageDoc) -> Result<T>,
    ) -> Result<T> {
        let (result, changed) = {
            let flush = self.flush_lock()?;
            let mut doc = self.read_conflict_archive_locked(id, &flush)?;
            doc.set_peer(self.page_peer(&doc))?;
            doc.register_peer(&self.device)?;
            let result = edit(&mut doc)?;
            doc.doc.commit();
            let changed = self.store_conflict_archive_locked(id, &doc, &flush)?;
            (result, changed)
        };
        if changed {
            self.emit([Change::PageChanged(id.to_owned())]);
        }
        Ok(result)
    }

    fn cache_path(&self, id: &str) -> PathBuf {
        self.docs_dir().join(format!("{id}.blocks.json"))
    }
    fn old_log_path(&self, id: &str) -> PathBuf {
        self.logs_dir().join(format!("{id}.log.old"))
    }

    /// Writes the block cache for a big page (removes it for a small one).
    pub(crate) fn store_block_cache(
        &self,
        id: &str,
        version: &[u8],
        blocks: &[Block],
    ) -> Result<()> {
        if count_blocks(blocks) < BLOCK_CACHE_MIN {
            return remove_if_exists(&self.cache_path(id));
        }
        let json = serde_json::to_vec(&BlockCacheRef { version, blocks })
            .map_err(|e| VaultError::Io(e.to_string()))?;
        write_atomic(&self.cache_path(id), &json)
    }

    /// The page's blocks from its cache file, if the file is for exactly the
    /// document's current version.
    fn cached_blocks(&self, id: &str, doc: &PageDoc) -> Option<Vec<Block>> {
        let bytes = fs::read(self.cache_path(id)).ok()?;
        let cache: BlockCache = serde_json::from_slice(&bytes).ok()?;
        (cache.version == doc.version()).then_some(cache.blocks)
    }

    fn send(&self, msg: Msg) {
        let _ = self.tx.send(msg);
    }

    /// Re-renders a page's markdown and index content in the background.
    pub(crate) fn rerender(&self, id: &str) {
        self.send(Msg::Recover(id.to_string()));
    }

    pub(crate) fn emit(&self, changes: impl IntoIterator<Item = Change>) {
        let mut f = lock(&self.feed);
        for c in changes {
            f.push(c);
        }
        drop(f);
        self.feed_cv.notify_all();
    }

    /// Publish only after SQLite has successfully committed. Keep generation
    /// allocation and feed insertion under the same lock so feed order is
    /// monotonic even when the foreground and background writers race.
    pub(crate) fn index_committed(&self, db: &Index, mut pages: Vec<PageId>) {
        // Computed properties can update rows beyond the caller's pages.
        // Their existing buffer stays on the connection until commit succeeds.
        let touched = db.take_touched();
        if !pages.is_empty() && !touched.is_empty() {
            pages.extend(touched.iter().cloned());
            pages.sort_unstable();
            pages.dedup();
        }
        let mut f = lock(&self.feed);
        for id in touched {
            f.push(Change::PageChanged(id));
        }
        // The feed lock serializes increments across both index connections.
        let generation = self.index_generation.load(Ordering::Relaxed)
            .checked_add(1).expect("index generation exhausted");
        self.index_generation.store(generation, Ordering::Release);
        f.push(Change::IndexCommitted { generation, pages });
        drop(f);
        self.feed_cv.notify_all();
    }

    pub(crate) fn wait_live_changes(&self, since: u32) {
        let mut f = lock(&self.feed);
        loop {
            if self.live_stop.load(Ordering::Acquire) || f.has_changes_since(since) {
                return;
            }
            f = self.feed_cv.wait(f).unwrap_or_else(|p| p.into_inner());
        }
    }

    pub(crate) fn poll(&self, since: u32) -> crate::types::ChangeBatch {
        lock(&self.feed).poll(since)
    }

    /// `poll`, waiting up to `timeout` for a change after `since`.
    pub(crate) fn wait(&self, since: u32, timeout: Duration) -> crate::types::ChangeBatch {
        self.wait_feed(since, timeout, false)
    }

    pub(crate) fn wait_documents(&self, since: u32, timeout: Duration) -> crate::types::ChangeBatch {
        self.wait_feed(since, timeout, true)
    }

    pub(crate) fn poll_documents(&self, since: u32) -> crate::types::ChangeBatch {
        lock(&self.feed).poll_documents(since)
    }

    fn wait_feed(&self, since: u32, timeout: Duration, documents_only: bool) -> crate::types::ChangeBatch {
        let deadline = Instant::now() + timeout;
        let mut f = lock(&self.feed);
        loop {
            let b = if documents_only { f.poll_documents(since) } else { f.poll(since) };
            if b.reset || !b.changes.is_empty() || b.seq != since {
                return b;
            }
            let now = Instant::now();
            if now >= deadline {
                return b;
            }
            f = self
                .feed_cv
                .wait_timeout(f, deadline - now)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }

    /// Writes metadata first, then the compatible one-ID-per-line file.
    pub(crate) fn save_purged(&self, p: &Purged) -> Result<()> {
        let state = PurgeState {
            records: p
                .list
                .iter()
                .map(|page| crate::sync::PurgeRecord {
                    page: page.clone(),
                    version: p.versions.get(page).cloned().flatten(),
                    device: p.devices.get(page).cloned(),
                })
                .collect(),
            restored: p.restored.clone(),
        };
        let bytes = serde_json::to_vec(&state).map_err(|e| invalid(e.to_string()))?;
        write_durable(&self.root.join(".vault/purge-state.json"), &bytes)?;
        let text: String = p.list.iter().map(|id| format!("{id}\n")).collect();
        write_durable(&self.root.join(".vault/purged"), text.as_bytes())
    }

    pub(crate) fn add_purge_records(&self, records: &[crate::sync::PurgeRecord]) -> Result<()> {
        let mut p = lock(&self.purged);
        let mut next = p.clone();
        for r in records {
            // An ID-only record neither re-tombstones a page with restoration
            // history nor weakens an active causal tombstone.
            if r.version.is_none()
                && (next.restored.iter().any(|x| x.page == r.page)
                    || matches!(next.versions.get(&r.page), Some(Some(_))))
            {
                continue;
            }
            if next.set.insert(r.page.clone()) {
                next.list.push(r.page.clone());
            }
            let effect = next.effect(r)?;
            next.versions.insert(r.page.clone(), effect.version);
            match effect.device {
                Some(device) => next.devices.insert(r.page.clone(), device),
                None => next.devices.remove(&r.page),
            };
        }
        self.save_purged(&next)?;
        *p = next;
        Ok(())
    }

    pub(crate) fn add_purged(&self, ids: &[PageId]) -> Result<()> {
        self.add_purge_records(
            &ids.iter()
                .map(|id| crate::sync::PurgeRecord {
                    page: id.clone(),
                    version: None,
                    device: None,
                })
                .collect::<Vec<_>>(),
        )
    }

    /// Replaces a validated full document without retaining the purger's
    /// update logs. The snapshot is durable before any old data is removed.
    pub(crate) fn replace_page_doc(
        &self,
        record: &crate::sync::RestoreRecord,
        bytes: &[u8],
        coverage: &loro::VersionVector,
    ) -> Result<()> {
        let id = &record.page;
        // ID-only tombstones cannot identify a causal frontier. Files left
        // behind by an already recorded legacy purge are deleted history,
        // unlike a live local copy receiving its first legacy tombstone.
        let legacy_leftover = {
            let p = lock(&self.purged);
            p.set.contains(id) && p.versions.get(id).cloned().flatten().is_none()
        };
        // Ensure any existing disk document participates in the same state
        // lock as ordinary edits before checking causal coverage.
        if self.doc_path(id).exists() {
            self.slot(id)?;
        }
        let mut cache = lock(&self.cache);
        let slot = cache.get(id).cloned();
        let mut state = slot.as_ref().map(|s| lock(&s.state));
        if let Some(st) = state.as_ref() {
            if !legacy_leftover && !crate::sync::restore_coverage_includes(&st.doc, coverage) {
                return Err(invalid(format!(
                    "page {id}: restore would discard independent local edits"
                )));
            }
        }
        let _io = slot.as_ref().map(|s| lock(&s.io));
        let flush = self.flush_lock()?;
        // Another Vault/process may have appended after this instance read
        // its cache. Validate those durable operations under the same lock.
        if self.doc_path(id).exists() {
            let (disk, _) = self.read_doc_locked(id, &flush)?;
            if !legacy_leftover && !crate::sync::restore_coverage_includes(&disk, coverage) {
                return Err(invalid(format!(
                    "page {id}: restore would discard independent disk edits"
                )));
            }
        }
        let directory = self.root.join(".vault/restores");
        fs::create_dir_all(&directory)?;
        sync_parent_dir(&directory)?;
        let journal_path = directory.join(format!("{id}.json"));
        let mut journal = crate::sync::RestoreJournal {
            record: record.clone(),
            bytes: bytes.to_vec(),
            installed: false,
        };
        write_durable(
            &journal_path,
            &serde_json::to_vec(&journal).map_err(|e| invalid(e.to_string()))?,
        )?;
        self.bump_sequence(&flush, read_sequence(&flush)?)?;
        write_durable(&self.doc_path(id), bytes)?;
        for path in [
            self.log_path(id),
            self.old_log_path(id),
            self.cache_path(id),
            cold::path(&self.root, id),
        ] {
            remove_if_exists(&path)?;
        }
        lock(&self.cold_pages).remove(id);
        sync_parent_dir(&self.log_path(id))?;
        sync_parent_dir(&self.doc_path(id))?;
        sync_parent_dir(&cold::path(&self.root, id))?;
        journal.installed = true;
        write_durable(
            &journal_path,
            &serde_json::to_vec(&journal).map_err(|e| invalid(e.to_string()))?,
        )?;
        if let Some(st) = state.as_mut() {
            st.deleted = true;
            st.dirty = false;
            st.log = None;
        }
        cache.remove(id);
        Ok(())
    }

    pub(crate) fn finish_restore_journal(&self, id: &str) -> Result<()> {
        let path = self.root.join(".vault/restores").join(format!("{id}.json"));
        remove_if_exists(&path)?;
        sync_parent_dir(&path)
    }

    pub(crate) fn is_purged(&self, id: &str) -> bool {
        lock(&self.purged).set.contains(id)
    }

    /// Acquire before document or flush locks. Never acquire this lock while
    /// holding flush.lock: daily-note creation itself needs that flush lock.
    pub(crate) fn daily_notes_lock(&self) -> Result<DailyNotesLock> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join(".vault").join("daily-notes.lock"))?;
        let started = Instant::now();
        loop {
            if try_lock_exclusive(&file)? {
                return Ok(DailyNotesLock { file });
            }
            if started.elapsed() >= FLUSH_LOCK_TIMEOUT {
                return Err(std::io::Error::new(
                    ErrorKind::TimedOut,
                    "timed out waiting for the vault daily-notes lock",
                )
                .into());
            }
            std::thread::sleep(FLUSH_LOCK_POLL);
        }
    }

    fn flush_lock(&self) -> Result<FlushLock<'_>> {
        #[cfg(feature = "storage-metrics")]
        let _w = storage_metrics::span("wait.flush_lock");
        // One non-blocking probe preserves the test contention notification;
        // a busy mutex then waits normally, without a deadline or polling.
        let mut file = match self.flush_file.try_lock() {
            Ok(file) => file,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                #[cfg(test)]
                self.notify_lock_blocked();
                lock(&self.flush_file)
            }
        };
        if file.is_none() {
            *file = Some(open_flush_file(&self.root)?);
        }
        let started = Instant::now();
        let mut warned = false;
        let mut contended = false;
        loop {
            let acquired = try_lock_exclusive(file.as_ref().expect("flush handle"))?;
            let elapsed = started.elapsed();
            if !warned && (contended || !acquired) && elapsed >= FLUSH_LOCK_WARN_AFTER {
                tracing::warn!(
                    vault = %self.root.display(),
                    wait_ms = elapsed.as_millis() as u64,
                    "waiting for another process to release the vault flush lock"
                );
                warned = true;
            }
            if acquired {
                HELD_FLUSH_ROOTS.with(|held| { held.borrow_mut().insert(self.root.clone()); });
                return Ok(FlushLock { file, root: &self.root });
            }
            contended = true;
            #[cfg(test)]
            self.notify_lock_blocked();
            if elapsed >= FLUSH_LOCK_TIMEOUT {
                return Err(flush_lock_timeout());
            }
            #[cfg(test)]
            if let Some(pause) = lock(&self.disk_io.pause_before_os_retry).take() {
                let _ = pause.entered.send(());
                pause.resume.recv().map_err(|_| {
                    std::io::Error::new(
                        ErrorKind::Interrupted,
                        "test OS retry pause ended without resume",
                    )
                })?;
            }
            std::thread::sleep(FLUSH_LOCK_POLL);
        }
    }

    #[cfg(test)]
    fn notify_lock_blocked(&self) {
        if let Some(blocked) = lock(&self.disk_io.lock_blocked).take() {
            let _ = blocked.send(());
        }
    }

    /// Called only under flush.lock. The record is durable before any page
    /// mutation; a crash inside the following batch therefore invalidates
    /// every older page observation even if file identity/size are equal.
    fn bump_sequence(&self, file: &File, current: Option<DiskSequence>) -> Result<DiskSequence> {
        let sequence = current
            .and_then(|known| {
                known.counter.checked_add(1).map(|counter| DiskSequence {
                    epoch: known.epoch,
                    counter,
                })
            })
            .unwrap_or_else(|| DiskSequence {
                epoch: new_id().as_bytes().try_into().expect("26-byte id"),
                counter: 1,
            });
        let mut bytes = [0; SEQUENCE_BYTES];
        bytes[..26].copy_from_slice(&sequence.epoch);
        bytes[26..34].copy_from_slice(&sequence.counter.to_le_bytes());
        bytes[34..42].copy_from_slice(&(!sequence.counter).to_le_bytes());
        let mut file = file;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&bytes)?;
        file.set_len(SEQUENCE_BYTES as u64)?;
        file.sync_all()?;
        sync_parent_dir(&self.root.join(".vault").join("flush.lock"))?;
        Ok(sequence)
    }

    /// Every document creation/replacement participates in the same sequence
    /// protocol, including install and cold sync import.
    pub(crate) fn write_page_doc(&self, id: &str, bytes: &[u8]) -> Result<SeenFiles> {
        #[cfg(feature = "storage-metrics")]
        let _write = storage_metrics::span("disk.write_page_doc");
        let flush = self.flush_lock()?;
        let sequence = self.bump_sequence(&flush, read_sequence(&flush)?)?;
        write_durable(&self.doc_path(id), bytes)?;
        let old_log = self.stat_page_file(id, 1)?;
        let log = self.stat_page_file(id, 2)?;
        Ok(SeenFiles {
            snapshot: Some(SeenFile::observed(self.stat_page_file(id, 0)?)),
            // Writing a document does not prove existing logs were read.
            old_log: old_log.is_none().then(|| SeenFile::observed(None)),
            log: log.is_none().then(|| SeenFile::observed(None)),
            sequence: Some(sequence),
        })
    }

    /// Installs new-page snapshots and commits their index rows under one
    /// acquisition. Start the SQLite write transaction before calling this
    /// method, keeping the same transaction-before-flush lock order as the
    /// background writer. A failure leaves durable snapshots for recovery.
    pub(crate) fn write_page_docs<'a>(
        &self,
        docs: impl IntoIterator<Item = (&'a str, &'a [u8])>,
        finish: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        #[cfg(feature = "storage-metrics")]
        let _batch = storage_metrics::span("disk.write_page_docs");
        let flush = self.flush_lock()?;
        #[cfg(test)]
        let held_since = Instant::now();
        #[cfg(test)]
        let mut installed = 0;
        let result = (|| {
            self.bump_sequence(&flush, read_sequence(&flush)?)?;
            for (id, bytes) in docs {
                #[cfg(feature = "storage-metrics")]
                let _write = storage_metrics::span("disk.write_page_doc");
                write_durable(&self.doc_path(id), bytes)?;
                #[cfg(test)]
                {
                    installed += 1;
                }
            }
            finish()
        })();
        #[cfg(test)]
        if result.is_ok() {
            if let Some(pause) = lock(&self.disk_io.pause_after_batch_finish).take() {
                let _ = pause.entered.send(());
                pause.resume.recv().map_err(|_| {
                    std::io::Error::new(
                        ErrorKind::Interrupted,
                        "test batch pause ended without resume",
                    )
                })?;
            }
        }
        drop(flush);
        #[cfg(test)]
        lock(&self.disk_io.batch_holds).push((installed, held_since.elapsed()));
        result
    }

    fn page_file_path(&self, id: &str, kind: usize) -> PathBuf {
        match kind {
            0 => self.doc_path(id),
            1 => self.old_log_path(id),
            2 => self.log_path(id),
            _ => unreachable!(),
        }
    }

    fn stat_page_file(&self, id: &str, kind: usize) -> Result<Option<FileStamp>> {
        #[cfg(feature = "storage-metrics")]
        let _s = storage_metrics::span("stat.page_file");
        #[cfg(test)]
        self.disk_io.stats[kind].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(stamp_path(&self.page_file_path(id, kind))?)
    }

    fn read_page_file(&self, id: &str, kind: usize) -> Result<Vec<u8>> {
        #[cfg(feature = "storage-metrics")]
        let _r = storage_metrics::span("disk.read_page_file");
        #[cfg(test)]
        self.disk_io.reads[kind].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let bytes = fs::read(self.page_file_path(id, kind))?;
        #[cfg(feature = "storage-metrics")]
        storage_metrics::add_bytes("disk.read_page_file", bytes.len() as u64);
        Ok(bytes)
    }

    /// Snapshot and sequence have already been synced before this call.
    /// Preserve the inode/file id so other instances' append handles remain
    /// valid; append mode writes at the new EOF after a peer's truncation.
    fn truncate_page_log(
        &self,
        id: &str,
        kind: usize,
        stamp: &Option<FileStamp>,
    ) -> Result<Option<FileStamp>> {
        let Some(stamp) = stamp else {
            return Ok(None);
        };
        let mut after = stamp.clone();
        if stamp.len > 0 {
            let file = OpenOptions::new()
                .write(true)
                .open(self.page_file_path(id, kind))?;
            file.set_len(0)?;
            file.sync_all()?;
            after.len = 0;
        }
        Ok(Some(after))
    }

    /// Reads a page document and records only metadata whose contents were
    /// read under the same cross-instance lock. Snapshot, then both logs.
    fn read_doc_tracked(&self, id: &str) -> Result<(PageDoc, SeenFiles)> {
        let flush = self.flush_lock()?;
        self.read_doc_locked(id, &flush)
    }

    /// The caller owns flush.lock through importing logs and reading cold.
    /// Index recovery keeps the same lock through its index commit.
    fn read_doc_locked(&self, id: &str, flush: &FlushLock<'_>) -> Result<(PageDoc, SeenFiles)> {
        #[cfg(feature = "storage-metrics")]
        let _r = storage_metrics::span("vault.read_doc_locked");
        let sequence = Some(match read_sequence(flush)? {
            Some(sequence) => sequence,
            None => self.bump_sequence(flush, None)?,
        });
        let snapshot_stamp = self.stat_page_file(id, 0)?;
        if snapshot_stamp.is_none() {
            return Err(not_found("page", id));
        }
        let old_stamp = self.stat_page_file(id, 1)?;
        let log_stamp = self.stat_page_file(id, 2)?;
        let snapshot = self.read_page_file(id, 0)?;
        let mut updates = Vec::new();
        if old_stamp.as_ref().is_some_and(|s| s.len > 0) {
            decode_frames(&self.read_page_file(id, 1)?, &mut updates);
        }
        if log_stamp.as_ref().is_some_and(|s| s.len > 0) {
            decode_frames(&self.read_page_file(id, 2)?, &mut updates);
        }
        let mut doc = PageDoc::from_bytes(&snapshot, &[])?;
        // A second instance may have split this page after our inventory was
        // read. Attaching the location is free and only affects shallow docs.
        if doc.doc.is_shallow() && (lock(&self.cold_pages).contains(id)
            || cold::path(&self.root, id).exists()) {
            doc.attach_cold(&self.root, id);
        }
        for bytes in updates { doc.import_updates(&bytes)?; }
        let peer = self.page_peer(&doc);
        doc.set_peer(peer)?;
        let seen = SeenFiles {
            snapshot: Some(SeenFile::observed(snapshot_stamp)),
            old_log: Some(SeenFile::observed(old_stamp)),
            log: Some(SeenFile::observed(log_stamp)),
            sequence,
        };
        Ok((doc, seen))
    }

    /// Reads a page document from disk: snapshot, then rotated and live logs.
    pub(crate) fn read_doc(&self, id: &str) -> Result<PageDoc> {
        self.read_doc_tracked(id).map(|(doc, _)| doc)
    }

    /// A failed import may leave durable snapshots before committing any
    /// index rows. Recover only missing rows, using disk rather than a cache
    /// that might predate another writer's snapshot or pending logs.
    pub(crate) fn recover_missing_rows(&self, db: &Index, ids: &[PageId]) -> Result<()> {
        for chunk in ids.chunks(WRITER_BATCH) {
            // Match the writer's order: SQLite write transaction, then the
            // advisory lock. Never acquire page state under the advisory lock.
            let tx = db.write_tx()?;
            let flush = self.flush_lock()?;
            let purged = read_purged(&self.root)?;
            let mut docs = Vec::new();
            for id in chunk {
                if db.exists(id)? || self.is_purged(id) || purged.set.contains(id) {
                    continue;
                }
                let (doc, _) = match self.read_doc_locked(id, &flush) {
                    Ok(doc) => doc,
                    // A peer may have removed the snapshot since the scan.
                    Err(VaultError::NotFound(_)) => continue,
                    Err(e) => return Err(e),
                };
                let row = meta_row(&doc);
                if row.id != *id {
                    return Err(invalid(format!(
                        "snapshot for page {id} holds page {:?}",
                        row.id
                    )));
                }
                docs.push((doc, row));
            }
            if docs.is_empty() {
                tx.commit()?;
                continue;
            }
            let recovered: Vec<PageId> = docs.iter().map(|(_, row)| row.id.clone()).collect();
            let defer = db.defer_guard();
            // All recovered titles, parents and schemas must be present
            // before deriving content (notably embeds and collection rows).
            for (_, row) in &docs {
                db.upsert(row, &ContentRow::default())?;
            }
            let mut missing_md = Vec::new();
            for (mut doc, row) in docs {
                let meta = doc.meta();
                let blocks = doc.blocks();
                let titles = |pid: &str| db.title_of(pid).ok().flatten();
                let mut content = content_row(&meta, &row.props, &blocks, &titles);
                content.vv = Some(doc.doc.oplog_vv().encode());
                // As in reindex, keep an existing markdown file. Mounted
                // files are reconciled by rescan_mounts rather than guessed.
                if self
                    .mount_folder(meta.parent.as_deref(), meta.is_template)
                    .is_none()
                {
                    let name = markdown::file_name(&meta.title, &row.id);
                    match fs::read(self.pages_dir().join(&name)) {
                        Ok(bytes) => {
                            content.md_hash = Some(attach::sha256_hex(&bytes));
                            content.md_path = Some(name);
                        }
                        Err(e) if e.kind() == ErrorKind::NotFound => {
                            missing_md.push(row.id.clone())
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
                db.update_content(&row.id, meta.updated, &content)?;
                db.set_conflicts(&row.id, &crate::sync::mirror_rows(&row.id, &doc.conflicts()))?;
                db.refresh_trash(&row.id)?;
            }
            defer.finish()?;
            tx.commit()?;
            self.index_committed(db, recovered.clone());
            drop(flush);
            // A recovered title or schema can affect an indexed page,
            // including one recovered in an earlier chunk.
            let mut again: HashSet<PageId> = missing_md.into_iter().collect();
            for id in &recovered {
                again.extend(db.link_sources(id)?);
                if db.schema_of(id)?.is_some() {
                    again.extend(db.child_ids(id)?);
                }
            }
            for id in again {
                self.send(Msg::Recover(id));
            }
            self.emit(
                recovered
                    .into_iter()
                    .map(Change::PageCreated)
                    .chain([Change::TreeChanged]),
            );
        }
        self.emit_touched(db);
        Ok(())
    }

    /// Syncs a page's snapshot and logs to disk (before its changes are
    /// exported to another device).
    pub(crate) fn sync_files(&self, id: &str) -> Result<()> {
        let _flush = self.flush_lock()?;
        for p in [self.doc_path(id), self.old_log_path(id), self.log_path(id)] {
            match OpenOptions::new().write(true).open(&p) {
                Ok(f) => f.sync_all()?,
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    pub(crate) fn cached_slot(&self, id: &str) -> Option<Arc<Slot>> {
        lock(&self.cache).get(id).cloned()
    }

    pub(crate) fn slot(&self, id: &str) -> Result<Arc<Slot>> {
        let mut cache = lock(&self.cache);
        if let Some(s) = cache.get(id) {
            return Ok(s.clone());
        }
        let (doc, seen) = self.read_doc_tracked(id)?;
        let snapshot_version = doc.version();
        let slot = Arc::new(Slot {
            state: Mutex::new(PageState {
                doc,
                dirty: false,
                deleted: false,
                log: None,
                seen,
                changed_on_disk: false,
                snapshot_version,
                exported: false,
            }),
            io: Mutex::new(()),
        });
        cache.insert(id.to_string(), slot.clone());
        if cache.len() > CACHE_CAP {
            let evict: Vec<String> = cache
                .iter()
                .filter(|(k, s)| {
                    k.as_str() != id
                        && Arc::strong_count(s) == 1
                        && s.state.try_lock().map(|st| !st.dirty).unwrap_or(false)
                })
                .map(|(k, _)| k.clone())
                .take(cache.len() - CACHE_CAP)
                .collect();
            for k in evict {
                cache.remove(&k);
            }
        }
        Ok(slot)
    }

    /// Runs `f` on the page's document (read only; no change is recorded).
    pub(crate) fn read<T>(&self, id: &str, f: impl FnOnce(&mut PageDoc) -> T) -> Result<T> {
        let slot = self.slot(id)?;
        let mut st = lock(&slot.state);
        if st.deleted {
            return Err(not_found("page", id));
        }
        Ok(f(&mut st.doc))
    }

    pub(crate) fn read_history<T>(&self, id: &str, f: impl FnOnce(&mut PageDoc) -> T) -> Result<T> {
        let slot = self.slot(id)?;
        let mut st = {
            #[cfg(feature = "storage-metrics")]
            let _w = storage_metrics::span("wait.slot_state");
            lock(&slot.state)
        };
        if st.deleted { return Err(not_found("page", id)); }
        self.refresh_cold_state(id, &mut st)?;
        Ok(f(&mut st.doc))
    }

    /// Cold callers refresh under the same lock as flush and append. Recent
    /// opens/exports never call this or read a cold file. Keep observations
    /// conservative: the ordinary writer still merges/indexes foreign ops.
    pub(crate) fn refresh_cold_state(&self, id: &str, st: &mut PageState) -> Result<()> {
        #[cfg(feature = "storage-metrics")]
        let _r = storage_metrics::span("vault.refresh_cold_state");
        let _flush = self.flush_lock()?;
        let sequence = read_sequence(&_flush)?;
        let stamps = [self.stat_page_file(id, 0)?, self.stat_page_file(id, 1)?, self.stat_page_file(id, 2)?];
        let changed = sequence.is_none() || sequence != st.seen.sequence || st.changed_on_disk
            || [&st.seen.snapshot, &st.seen.old_log, &st.seen.log].iter().zip(&stamps)
                .any(|(seen, stamp)| !seen.as_ref().is_some_and(|s| s.unchanged(stamp)));
        let before = st.doc.doc.oplog_vv();
        if changed {
            let (mut disk, _) = self.read_doc_locked(id, &_flush)?;
            disk.rehydrate()?;
            disk.import_updates(&{
                #[cfg(feature = "storage-metrics")]
                let _e = storage_metrics::span("loro.export_updates");
                st.doc.doc.export(ExportMode::all_updates())?
            })?;
            st.doc = disk;
        } else { st.doc.rehydrate()?; }
        if st.doc.doc.oplog_vv() != before { self.mark_dirty(st, id); }
        Ok(())
    }

    /// Refuse the reverse-order write: a durable independently loadable
    /// cold prefix must exist before even staging a shallow replacement.
    fn stage_cold_hot(&self, id: &str, bytes: &[u8]) -> Result<PathBuf> {
        let part = cold::read(&self.root, id)?;
        let hot = PageDoc::from_bytes(bytes, &[])?;
        let root = hot.doc.frontiers_to_vv(&hot.doc.shallow_since_frontiers())
            .ok_or_else(|| invalid("hot root missing"))?;
        if !hot.doc.is_shallow() || !part.root.vv.includes_vv(&root)
            || !hot.doc.oplog_vv().includes_vv(&part.root.vv) {
            return Err(invalid("cold prefix must cover the shallow hot root"));
        }
        let prefix = loro::LoroDoc::new();
        prefix.import(&part.updates)?;
        if prefix.is_shallow() || prefix.oplog_vv() != part.root.vv {
            return Err(invalid("cold file must be an independent full prefix"));
        }
        let stage = self.docs_dir().join(format!("{id}.cold-stage"));
        write_durable(&stage, bytes)?;
        Ok(stage)
    }

    /// Prepare compression off the page/cache locks; publish only if nobody
    /// acquired or changed the page in the meantime. The writer is the only
    /// caller, so it cannot race a flush or a second cold pass.
    fn cold_page(&self, index: &Index, id: &str, policy: &ColdPolicy, at_ms: i64) -> Result<bool> {
        #[cfg(feature = "storage-metrics")]
        let _c = storage_metrics::span("archive.cold_page");
        if self.is_purged(id) { return Ok(false); }
        let original = {
            let cache = lock(&self.cache);
            if let Some(slot) = cache.get(id) {
                if Arc::strong_count(slot) != 1 { return Ok(false); }
                let Ok(st) = slot.state.try_lock() else { return Ok(false); };
                if st.dirty || st.deleted { return Ok(false); }
                Some((slot.clone(), st.doc.doc.oplog_vv()))
            } else { None }
        };
        // All clean cached changes are already on disk. Read a separate doc
        // so compression and Loro's root search cannot delay foreground use.
        let (full, was_shallow) = match (|| {
            let flush = self.flush_lock()?;
            let (mut doc, seen) = self.read_doc_locked(id, &flush)?;
            // A foreign instance's nonempty log is a dirty page too. Leave
            // its recovery data for the normal writer/index flush protocol.
            if [&seen.old_log, &seen.log].iter().any(|seen|
                seen.as_ref().and_then(|s| s.stamp.as_ref()).is_some_and(|s| s.len > 0)) {
                return Ok(None);
            }
            let was_shallow = doc.doc.is_shallow();
            doc.rehydrate()?;
            Ok(Some((doc, was_shallow)))
        })() {
            Ok(Some(doc)) => doc,
            Ok(None) => return Ok(false),
            Err(VaultError::NotFound(_)) => return Ok(false),
            Err(e) => return Err(e),
        };
        let before = full.doc.oplog_vv();
        let in_trash = index.in_trash(id)?.unwrap_or(false);
        let root = if in_trash {
            full.doc.oplog_frontiers()
        } else {
            let Some(root) = crate::history::cutoff_root(&full, at_ms / 1000 - policy.age_seconds)?
                else { return Ok(false); };
            root
        };
        let hot_bytes = crate::history::hot_snapshot(&full, &root)?;
        let mut hot = PageDoc::from_bytes(&hot_bytes, &[])?;
        if !hot.doc.is_shallow() { return Ok(false); }
        // Always use Loro's actual boundary, including conservative retreats.
        let actual = hot.doc.shallow_since_frontiers();
        let vv = full.doc.frontiers_to_vv(&actual)
            .ok_or_else(|| invalid("cold root missing from page"))?;
        if lock(&self.cold_pages).contains(id) {
            let old = cold::read(&self.root, id)?;
            // The replacement cold prefix must cover the old one, since the
            // old hot file is still authoritative until the final rename.
            if !vv.includes_vv(&old.root.vv) || (was_shallow && vv == old.root.vv) { return Ok(false); }
        }
        let prefix = full.doc.fork_at(&actual)?;
        let updates = prefix.export(ExportMode::all_updates())?;
        if !in_trash && updates.len() < policy.minimum_bytes { return Ok(false); }
        let part = cold::ColdPart { root: cold::ColdRoot { frontiers: actual, vv }, updates };
        let prepared = cold::prepare(&self.root, &part)?;
        #[cfg(test)]
        {
            let hook = lock(&self.cold_before_publish).take();
            if let Some(hook) = hook { hook(self, id); }
        }
        let result = (|| {
            let cache = lock(&self.cache);
            if self.is_purged(id) { return Ok(false); }
            let current = cache.get(id);
            let slot = match (&original, current) {
                (Some((a, _)), Some(b)) if Arc::ptr_eq(a, b) && Arc::strong_count(b) == 2 => Some(b),
                (None, None) => None,
                _ => return Ok(false),
            };
            let mut state = match slot {
                Some(s) => match s.state.try_lock() {
                    Ok(st) if !st.dirty && !st.deleted
                        && original.as_ref().is_some_and(|(_, vv)| st.doc.doc.oplog_vv() == *vv) => Some(st),
                    _ => return Ok(false),
                },
                None => None,
            };
            let _io = slot.map(|s| {
                #[cfg(feature = "storage-metrics")]
                let _w = storage_metrics::span("wait.slot_io");
                lock(&s.io)
            });
            let flush = self.flush_lock()?;
            let (mut disk, seen) = self.read_doc_locked(id, &flush)?;
            disk.rehydrate()?;
            if disk.doc.oplog_vv() != before {
                if let Some(st) = &mut state {
                    disk.import_updates(&st.doc.doc.export(ExportMode::all_updates())?)?;
                    st.doc = disk;
                    self.mark_dirty(st, id);
                } else { self.send(Msg::Recover(id.to_owned())); }
                return Ok(false);
            }
            if [&seen.old_log, &seen.log].iter().any(|seen|
                seen.as_ref().and_then(|s| s.stamp.as_ref()).is_some_and(|s| s.len > 0)) {
                return Ok(false);
            }
            // Even an equal-VV publication by another instance may advance
            // its root. Never replace that prefix with a shorter one.
            if cold::path(&self.root, id).exists() {
                let old = cold::read(&self.root, id)?;
                if !part.root.vv.includes_vv(&old.root.vv) { return Ok(false); }
            }
            // An uncached page cannot change while the cache lock is held;
            // every edit/import obtains a slot through this lock.
            // Invalidate observations before either durable replacement. A
            // crash after cold alone leaves the previous hot + logs intact.
            let sequence = self.bump_sequence(&flush, read_sequence(&flush)?)?;
            cold::write_prepared(&self.root, id, &prepared)?;
            lock(&self.cold_pages).insert(id.to_owned());
            let stage = self.stage_cold_hot(id, &hot_bytes)?;
            cold::publish_hot(&stage, &self.doc_path(id))?;
            let old_stamp = seen.old_log.as_ref().and_then(|s| s.stamp.clone());
            let log_stamp = seen.log.as_ref().and_then(|s| s.stamp.clone());
            let seen = SeenFiles {
                snapshot: Some(SeenFile::observed(self.stat_page_file(id, 0)?)),
                old_log: Some(SeenFile::observed(self.truncate_page_log(id, 1, &old_stamp)?)),
                log: Some(SeenFile::observed(self.truncate_page_log(id, 2, &log_stamp)?)),
                sequence: Some(sequence),
            };
            let peer = self.page_peer(&hot);
            hot.set_peer(peer)?;
            hot.attach_cold(&self.root, id);
            if let Some(st) = &mut state {
                st.snapshot_version = hot.version();
                st.doc = hot;
                st.seen = seen;
                st.changed_on_disk = false;
            }
            Ok(true)
        })();
        remove_if_exists(&self.docs_dir().join(format!("{id}.cold-stage")))?;
        if self.is_purged(id) {
            remove_if_exists(&cold::path(&self.root, id))?;
            lock(&self.cold_pages).remove(id);
        }
        result
    }

    fn cold_pass(&self, index: &Index, cursor: &mut Option<fs::ReadDir>, policy: &ColdPolicy, at_ms: i64) -> Result<usize> {
        if cursor.is_none() { *cursor = Some(fs::read_dir(self.docs_dir())?); }
        let mut moved = 0;
        for _ in 0..policy.pages_per_pass {
            let Some(entry) = cursor.as_mut().expect("cursor").next() else {
                *cursor = None;
                break;
            };
            let entry = entry?;
            let name = entry.file_name();
            let Some(id) = name.to_str().and_then(|s| s.strip_suffix(".loro")) else { continue; };
            match self.cold_page(index, id, policy, at_ms) {
                Ok(did_move) => moved += did_move as usize,
                Err(_) if self.is_purged(id) => {
                    // Purge may remove a prefix while preparation is reading
                    // it. Its durable tombstone wins; this is a cancelled
                    // maintenance pass, not a background write failure.
                    remove_if_exists(&cold::path(&self.root, id))?;
                    lock(&self.cold_pages).remove(id);
                }
                Err(e) => return Err(e),
            }
        }
        if moved > 0 { cold::maybe_train_dictionary(&self.root)?; }
        Ok(moved)
    }

    pub(crate) fn append_log(&self, st: &mut PageState, id: &str, bytes: &[u8]) -> Result<()> {
        // The caller runs its edit closure before reaching this lock.
        let flush = self.flush_lock()?;
        let sequence = read_sequence(&flush)?;
        self.append_log_locked(st, id, bytes, sequence)
    }

    fn append_log_locked(&self, st: &mut PageState, id: &str, bytes: &[u8], sequence: Option<DiskSequence>) -> Result<()> {
        // A second instance can hold a cached page after another purges it.
        // Do not acknowledge an orphan delta that cannot reconstruct history.
        if !self.doc_path(id).exists() {
            return Err(not_found("page", id));
        }
        if sequence != st.seen.sequence
            && !st.seen.snapshot.as_ref().is_some_and(|seen| {
                stamp_path(&self.doc_path(id))
                    .ok()
                    .is_some_and(|stamp| seen.unchanged(&stamp))
            })
        {
            let disk = PageDoc::from_bytes(&fs::read(self.doc_path(id))?, &[])?;
            let marker = |d: &PageDoc| {
                d.doc
                    .get_map("page")
                    .get("purge_restore")
                    .map(|value| value.get_deep_value())
            };
            if marker(&disk) != marker(&st.doc) {
                return Err(invalid(format!(
                    "page {id}: restored by another instance; reload before editing"
                )));
            }
        }
        let current = self.stat_page_file(id, 2)?;
        let known = !st.changed_on_disk
            && sequence.is_some()
            && sequence == st.seen.sequence
            && st.seen.log.as_ref().is_some_and(|s| s.unchanged(&current));
        if !known {
            st.changed_on_disk = true;
        }
        let after = self.append_log_at_stamp(st, id, bytes, &current)?;
        if known {
            st.seen.log = Some(SeenFile::observed(Some(after)));
        }
        // Append never adopts a foreign sequence. A truncate/reappend can
        // have exactly the old identity and length, so that sequence change
        // must still force a merge at this page's next flush.
        Ok(())
    }

    /// Keep a cold rebuild and its durable append against one disk view.
    /// The cached merge also carries master's normalization and peer rules.
    pub(crate) fn merge_sync_import(
        &self,
        index: &Index,
        id: &str,
        st: &mut PageState,
        bytes: &[u8],
    ) -> Result<Option<crate::sync::ImportEffect>> {
        let flush = self.flush_lock()?;
        let sequence = read_sequence(&flush)?;
        self.merge_cached_import(index, id, st, &[bytes.to_vec()], |st, ops, _| {
            self.append_log_locked(st, id, ops, sequence)
        })
    }

    fn append_log_at_stamp(
        &self,
        st: &mut PageState,
        id: &str,
        bytes: &[u8],
        current: &Option<FileStamp>,
    ) -> Result<FileStamp> {
        let same_file = st.log.as_ref().is_some_and(|log| {
            log.identity.is_some()
                && current
                    .as_ref()
                    .is_some_and(|stamp| stamp.identity == log.identity)
        });
        let mut after = if same_file {
            current.as_ref().expect("current log").clone()
        } else {
            st.log = None;
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.log_path(id))?;
            #[cfg(test)]
            self.disk_io.stats[2].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let stamp = stamp_handle(&file)?;
            st.log = Some(AppendLog {
                file,
                identity: stamp.identity,
            });
            stamp
        };
        let mut buf = Vec::with_capacity(4 + bytes.len());
        buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(bytes);
        after.len = after.len.checked_add(buf.len() as u64).ok_or_else(|| {
            std::io::Error::new(ErrorKind::InvalidData, "page log length overflow")
        })?;
        st.log.as_mut().expect("log").file.write_all(&buf)?;
        Ok(after)
    }

    /// Runs `edit` on the page's document as one committed change, logs the
    /// change, marks the page dirty and reports it to the change feed. An
    /// edit that changes nothing records nothing.
    pub(crate) fn edit<T>(
        &self,
        id: &str,
        edit: impl FnOnce(&mut PageDoc) -> Result<T>,
    ) -> Result<T> {
        self.edit_tagged(id, None, edit)
    }

    pub(crate) fn edit_sync<T>(
        &self,
        id: &str,
        edit: impl FnOnce(&mut PageDoc) -> Result<T>,
    ) -> Result<T> {
        self.edit_tagged(id, Some("sync:adopt-orphan"), edit)
    }

    pub(crate) fn edit_tagged<T>(
        &self,
        id: &str,
        message: Option<&str>,
        edit: impl FnOnce(&mut PageDoc) -> Result<T>,
    ) -> Result<T> {
        let slot = self.slot(id)?;
        let mut st = lock(&slot.state);
        if st.deleted {
            return Err(not_found("page", id));
        }
        let peer = self.page_peer(&st.doc);
        st.doc.set_peer(peer)?;
        let vv = st.doc.doc.oplog_vv();
        let out = match edit(&mut st.doc) {
            Ok(v) => v,
            Err(e) => {
                // Undo partial changes of a failed edit.
                if let Some(message) = message {
                    st.doc.doc.set_next_commit_message(message);
                }
                st.doc.doc.commit();
                if st.doc.doc.oplog_vv() != vv {
                    // Failed tools can still have committed partial edits;
                    // retain their actor just as for successful edits.
                    st.doc.register_peer(&self.actor_label())?;
                    st.doc.doc.commit();
                    let bytes = st.doc.doc.export(ExportMode::updates(&vv))?;
                    self.append_log(&mut st, id, &bytes)?;
                    st.dirty = true;
                }
                return Err(e);
            }
        };
        if let Some(message) = message {
            st.doc.doc.set_next_commit_message(message);
        }
        st.doc.doc.commit();
        if st.doc.doc.oplog_vv() == vv {
            return Ok(out);
        }
        st.doc.register_peer(&self.actor_label())?;
        st.doc.set_updated(stamp_ms())?;
        if let Some(message) = message {
            st.doc.doc.set_next_commit_message(message);
        }
        st.doc.doc.commit();
        let bytes = st.doc.doc.export(ExportMode::updates(&vv))?;
        self.append_log(&mut st, id, &bytes)?;
        let was_dirty = std::mem::replace(&mut st.dirty, true);
        drop(st);
        if !was_dirty {
            self.send(Msg::Dirty(id.to_string()));
        }
        self.emit([Change::PageChanged(id.to_string())]);
        Ok(out)
    }

    /// Marks a page changed for the writer.
    pub(crate) fn mark_dirty(&self, st: &mut PageState, id: &str) {
        if !std::mem::replace(&mut st.dirty, true) {
            self.send(Msg::Dirty(id.to_string()));
        }
    }

    /// Version vectors of the loaded documents (they can be ahead of the
    /// index until the writer flushes).
    pub(crate) fn live_versions(&self) -> Vec<(PageId, Vec<u8>)> {
        let slots: Vec<(PageId, Arc<Slot>)> = lock(&self.cache)
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let mut out = Vec::with_capacity(slots.len());
        for (id, slot) in slots {
            let st = lock(&slot.state);
            if !st.deleted {
                out.push((id, st.doc.doc.oplog_vv().encode()));
            }
        }
        out
    }

    /// Writes the page's metadata and properties to the index (the caller's
    /// connection), so queries see them at once.
    pub(crate) fn sync_meta(&self, db: &Index, id: &str) -> Result<()> {
        let slot = self.slot(id)?;
        let mut row = {
            let st = lock(&slot.state);
            if st.deleted {
                return Ok(());
            }
            meta_row(&st.doc)
        };
        if row.role.as_deref() == Some("calendars")
            && self.read(id, |doc| crate::calendars::needs_normalization(doc))?
        {
            self.edit(id, crate::calendars::normalize_document)?;
            row = self.read(id, |doc| meta_row(doc))?;
        }
        db.update_meta(&row)?;
        if db.conn.is_autocommit() {
            self.index_committed(db, vec![id.to_owned()]);
        }
        Ok(())
    }

    /// Reports rows whose formulas or rollups changed with another page.
    pub(crate) fn emit_touched(&self, db: &Index) {
        // A surrounding writer/import transaction publishes these together
        // with its commit. Publishing now would expose uncommitted values.
        if !db.conn.is_autocommit() {
            return;
        }
        let pages = db.touched.borrow().clone();
        if !pages.is_empty() {
            self.index_committed(db, pages);
        }
    }

    /// Adds a new page document: its snapshot, index row and cache slot.
    pub(crate) fn install(&self, db: &Index, doc: PageDoc) -> Result<PageId> {
        self.install_with_md(db, doc, None)
    }

    /// `install`, recording an existing markdown file (name, hash) as the
    /// page's file, so the writer replaces (and renames) it.
    pub(crate) fn install_with_md(
        &self,
        db: &Index,
        mut doc: PageDoc,
        md: Option<(String, String)>,
    ) -> Result<PageId> {
        // Imports and newly discovered markdown files also create top-level
        // pages, which start uncoloured ("none"). Existing supplied colours are
        // retained; sync and reindex do not install local pages through this
        // path.
        let meta = doc.meta();
        if meta.parent.is_none() && !meta.is_template && meta.color.is_none() {
            doc.set_meta_str("color", Some("none"))?;
        }
        doc.register_peer(&self.actor_label())?;
        doc.doc.commit();
        let row = meta_row(&doc);
        let id = row.id.clone();
        let blocks = doc.blocks();
        let seen = self.write_page_doc(&id, &doc.snapshot()?)?;
        let snapshot_version = doc.version();
        let titles = |pid: &str| db.title_of(pid).ok().flatten();
        let mut content = content_row(&doc.meta(), &row.props, &blocks, &titles);
        content.vv = Some(doc.doc.oplog_vv().encode());
        if let Some((name, hash)) = md {
            content.md_path = Some(name);
            content.md_hash = Some(hash);
            // The file is the document as made: the base for merging later
            // outside edits with edits made in the app meanwhile.
            content.md_version = Some(doc.version());
        }
        db.upsert(&row, &content)?;
        let slot = Arc::new(Slot {
            state: Mutex::new(PageState {
                doc,
                dirty: true,
                deleted: false,
                log: None,
                seen,
                changed_on_disk: false,
                snapshot_version,
                exported: false,
            }),
            io: Mutex::new(()),
        });
        lock(&self.cache).insert(id.clone(), slot);
        self.send(Msg::Dirty(id.clone()));
        self.emit([Change::PageCreated(id.clone())]);
        if db.conn.is_autocommit() {
            self.index_committed(db, vec![id.clone()]);
        }
        Ok(id)
    }

    /// Causal purge removal holds the same state lock as edits until the
    /// document is marked deleted. Local purges capture their final version;
    /// remote purges reject a newly arrived edit instead of deleting it.
    pub(crate) fn purge_files(
        &self,
        id: &str,
        md: Option<&str>,
        version: Option<&loro::VersionVector>,
    ) -> Result<()> {
        let slot = self.slot(id)?;
        let mut cache = lock(&self.cache);
        let mut st = lock(&slot.state);
        let _io = lock(&slot.io);
        let flush = self.flush_lock()?;
        let disk = if self.doc_path(id).exists() {
            Some(self.read_doc_locked(id, &flush)?.0)
        } else {
            None
        };
        if let Some(version) = version {
            if !crate::sync::restore_coverage_includes(&st.doc, version)
                || disk
                    .as_ref()
                    .is_some_and(|d| !crate::sync::restore_coverage_includes(d, version))
            {
                return Err(invalid(format!(
                    "page {id}: edited while applying purge; retry with full editor state"
                )));
            }
        } else {
            let mut vv = st.doc.doc.oplog_vv();
            if let Some(disk) = &disk {
                vv.merge(&disk.doc.oplog_vv());
            }
            // A local purge: this device is the purging device.
            self.add_purge_records(&[crate::sync::PurgeRecord {
                page: id.to_string(),
                version: Some(vv.encode()),
                device: Some(self.device.clone()),
            }])?;
        }
        // Keep conflict-bearing page history outside active docs before any
        // destructive step. Include reread disk logs and any previous archive.
        // The existing flush/state/I/O locks protect this union and the unlink.
        if !st.doc.conflicts().is_empty()
            || disk.as_ref().is_some_and(|doc| !doc.conflicts().is_empty())
            || crate::sync::has_conflict_history(&st.doc)
            || disk.as_ref().is_some_and(crate::sync::has_conflict_history)
            || self.conflict_archive_path(id)?.exists()
        {
            let archive = crate::conflict_archive::merge(id, &st.doc, disk.as_ref())?;
            if self.store_conflict_archive_locked(id, &archive, &flush)? {
                self.emit([Change::PageChanged(id.to_owned())]);
            }
        }
        self.bump_sequence(&flush, read_sequence(&flush)?)?;
        st.deleted = true;
        st.dirty = false;
        st.log = None;
        drop(st);
        cache.remove(id);
        remove_if_exists(&self.doc_path(id))?;
        remove_if_exists(&self.log_path(id))?;
        remove_if_exists(&self.old_log_path(id))?;
        remove_if_exists(&self.cache_path(id))?;
        remove_if_exists(&cold::path(&self.root, id))?;
        lock(&self.cold_pages).remove(id);
        sync_parent_dir(&self.doc_path(id))?;
        sync_parent_dir(&self.log_path(id))?;
        sync_parent_dir(&cold::path(&self.root, id))?;
        if let Some(md) = md {
            self.drop_md(id, md, None)?;
        }
        Ok(())
    }

    /// Removes a page's files and marks its loaded document deleted.
    pub(crate) fn remove_files(&self, id: &str, md: Option<&str>) -> Result<()> {
        // ID-only tombstones remain ID-only. Retain conflict history before
        // unlinking, under the same advisory lock used by archive writers.
        let slot = lock(&self.cache).remove(id);
        let mut cached = slot.as_ref().map(|slot| lock(&slot.state));
        let _io = slot.as_ref().map(|slot| lock(&slot.io));
        let flush = self.flush_lock()?;
        let disk = match self.read_doc_locked(id, &flush) {
            Ok((doc, _)) => Some(doc),
            Err(VaultError::NotFound(_)) => None,
            Err(error) => return Err(error),
        };
        if let Some(st) = cached.as_mut() {
            if !st.doc.conflicts().is_empty()
                || crate::sync::has_conflict_history(&st.doc)
                || disk.as_ref().is_some_and(|doc| !doc.conflicts().is_empty()
                    || crate::sync::has_conflict_history(doc))
                || self.conflict_archive_path(id)?.exists()
            {
                let archive = crate::conflict_archive::merge(id, &st.doc, disk.as_ref())?;
                self.store_conflict_archive_locked(id, &archive, &flush)?;
            }
            st.deleted = true;
            st.dirty = false;
            st.log = None;
        } else {
            if let Some(doc) = &disk {
                if !doc.conflicts().is_empty() || crate::sync::has_conflict_history(doc)
                    || self.conflict_archive_path(id)?.exists() {
                    self.store_conflict_archive_locked(id, doc, &flush)?;
                }
            }
        }
        drop(cached);
        remove_if_exists(&self.doc_path(id))?;
        remove_if_exists(&self.log_path(id))?;
        remove_if_exists(&self.old_log_path(id))?;
        remove_if_exists(&self.cache_path(id))?;
        remove_if_exists(&cold::path(&self.root, id))?;
        lock(&self.cold_pages).remove(id);
        if let Some(md) = md {
            // A mounted file is left alone (its content is not known here).
            self.drop_md(id, md, None)?;
        }
        Ok(())
    }

    /// Removes the markdown file of a page moved to the trash.
    pub(crate) fn remove_md(&self, db: &Index, id: &str, md: Option<&str>) -> Result<()> {
        // No SQLite write while the io lock is held (the writer may hold the
        // SQLite write lock while waiting for it).
        let hash = db.md(id)?.1;
        {
            let slot = self.cached_slot(id);
            let _io = slot.as_ref().map(|s| lock(&s.io));
            if let Some(md) = md {
                self.drop_md(id, md, hash.as_deref())?;
            }
        }
        db.clear_md(id)
    }

    /// Writes one page's snapshot, markdown file and index row.
    fn flush_page(&self, index: &Index, id: &str, force: bool) -> Result<()> {
        let slot = if force {
            match self.slot(id) {
                Ok(s) => s,
                Err(VaultError::NotFound(_)) => {
                    // No covering snapshot exists. Leave orphan logs alone
                    // rather than destroying content that was never merged.
                    return Ok(());
                }
                Err(e) => return Err(e),
            }
        } else {
            match lock(&self.cache).get(id) {
                Some(s) => s.clone(),
                None => return Ok(()),
            }
        };
        let was_in_trash = index.in_trash(id)?.unwrap_or(false);
        let mut durable = false;
        let mut failed_import_effect = None;
        let result = (|| {
            let (version, vv, meta, row, blocks, effect) = {
                // Append holds state before acquiring the advisory lock.
                // Never reverse that order, or reacquire state under the
                // advisory lock after releasing it.
                let mut st = lock(&slot.state);
                if st.deleted || (!st.dirty && !force) {
                    return Ok(());
                }
                durable = st.exported;
                let _io = lock(&slot.io);
                let _flush = self.flush_lock()?;
                let sequence = read_sequence(&_flush)?;
                let sequence_changed =
                    sequence.is_none() || sequence != st.seen.sequence || st.changed_on_disk;
                let snapshot_stamp = self.stat_page_file(id, 0)?;
                if snapshot_stamp.is_none() {
                    return Err(not_found("page", id));
                }
                let old_stamp = self.stat_page_file(id, 1)?;
                let mut log_stamp = self.stat_page_file(id, 2)?;
                let mut blobs = Vec::new();
                let mut disk_cold_root = None;
                if sequence_changed
                    || !st
                        .seen
                        .snapshot
                        .as_ref()
                        .is_some_and(|s| s.unchanged(&snapshot_stamp))
                {
                    let snapshot = self.read_page_file(id, 0)?;
                    // Legacy full encodings can import successfully even
                    // when this metadata-only decoder does not support them.
                    if let Ok(meta) = loro::LoroDoc::decode_import_blob_meta(&snapshot, false) {
                        if meta.mode == loro::EncodedBlobMode::ShallowSnapshot
                            && (lock(&self.cold_pages).contains(id) || cold::path(&self.root, id).exists()) {
                            // A full cache opened before a foreign split can
                            // also lack operations archived by its newer root.
                            // Merge that prefix before the retained snapshot.
                            let known = st.doc.doc.oplog_vv();
                            if !meta.start_frontiers.iter().all(|op| known.includes_id(op)) {
                                blobs.push(cold::read(&self.root, id)?.updates);
                            }
                            disk_cold_root = Some(meta.start_frontiers);
                        }
                    }
                    blobs.push(snapshot);
                }
                if old_stamp.as_ref().is_some_and(|s| s.len > 0)
                    && (sequence_changed
                        || !st
                            .seen
                            .old_log
                            .as_ref()
                            .is_some_and(|s| s.unchanged(&old_stamp)))
                {
                    decode_frames(&self.read_page_file(id, 1)?, &mut blobs);
                }
                if log_stamp.as_ref().is_some_and(|s| s.len > 0)
                    && (sequence_changed
                        || !st
                            .seen
                            .log
                            .as_ref()
                            .is_some_and(|s| s.unchanged(&log_stamp)))
                {
                    decode_frames(&self.read_page_file(id, 2)?, &mut blobs);
                }
                let effect = if blobs.is_empty() {
                    None
                } else {
                    self.merge_cached_import(index, id, &mut st, &blobs, |st, ops, received| {
                        if received {
                            // These ops were just read from the named files.
                            Ok(())
                        } else {
                            log_stamp = Some(self.append_log_at_stamp(st, id, ops, &log_stamp)?);
                            Ok(())
                        }
                    })?
                };
                if let Some(error) = effect
                    .as_ref()
                    .and_then(|e| e.imported.as_ref().err())
                    .cloned()
                {
                    // Keep the original files on a rejected import, but
                    // still publish the valid operations Loro applied.
                    failed_import_effect = effect;
                    return Err(error);
                }
                if let Some(root) = disk_cold_root {
                    st.doc.retain_cold_root(&self.root, id, root);
                }
                #[cfg(test)]
                let pause = lock(&self.disk_io.pause_after_merge).take();
                #[cfg(test)]
                if let Some(pause) = pause {
                    let _ = pause.entered.send(());
                    pause
                        .resume
                        .recv_timeout(Duration::from_secs(5))
                        .map_err(|_| {
                            std::io::Error::new(
                                ErrorKind::Interrupted,
                                "test flush pause ended without resume",
                            )
                        })?;
                }
                let version = st.doc.version();
                let vv = st.doc.doc.oplog_vv().encode();
                let has_logs = old_stamp.as_ref().is_some_and(|s| s.len > 0)
                    || log_stamp.as_ref().is_some_and(|s| s.len > 0);
                let write_snapshot = durable || has_logs || version != st.snapshot_version;
                let mut after_sequence = sequence;
                if write_snapshot {
                    let snapshot = st.doc.snapshot()?;
                    // One durable bump covers the whole batch: no other lock
                    // holder can observe between snapshot and truncation.
                    after_sequence = Some(self.bump_sequence(&_flush, sequence)?);
                    write_durable(&self.doc_path(id), &snapshot)?;
                    // Establish the replacement file's identity and size.
                    // This extra postwrite stat is explicitly counted.
                    st.seen.snapshot = Some(SeenFile::observed(self.stat_page_file(id, 0)?));
                    #[cfg(test)]
                    if self
                        .disk_io
                        .interrupt_after_snapshot
                        .swap(false, std::sync::atomic::Ordering::Relaxed)
                    {
                        return Err(std::io::Error::new(
                            ErrorKind::Interrupted,
                            "test crash after durable snapshot",
                        )
                        .into());
                    }
                } else {
                    st.seen.snapshot = Some(SeenFile::observed(snapshot_stamp));
                }
                if has_logs {
                    // Keep both file identities and cached append handles.
                    // Snapshot contents and its directory entry are durable
                    // before any log is shortened; replay after a crash in
                    // between is idempotent.
                    st.seen.old_log = Some(SeenFile::observed(
                        self.truncate_page_log(id, 1, &old_stamp)?,
                    ));
                    st.seen.log = Some(SeenFile::observed(
                        self.truncate_page_log(id, 2, &log_stamp)?,
                    ));
                } else {
                    st.seen.old_log = Some(SeenFile::observed(old_stamp));
                    st.seen.log = Some(SeenFile::observed(log_stamp));
                }
                st.seen.sequence = after_sequence;
                st.changed_on_disk = false;
                st.snapshot_version = version.clone();
                st.dirty = false;
                st.exported = false;
                drop(_flush);
                drop(_io);
                // Decoding blocks and serializing the cache can dominate a
                // large page flush. Other instances can append while this
                // instance finishes that work from its captured version.
                let meta = st.doc.meta();
                let row = meta_row(&st.doc);
                let blocks = st.doc.blocks();
                (version, vv, meta, row, blocks, effect)
            };
            // Writer callers already own a SQLite transaction. Use that
            // connection, after releasing every page/file lock.
            if let Some(effect) = &effect {
                self.apply_import_index(index, id, effect, was_in_trash)?;
            } else {
                index.update_meta(&row)?;
                self.emit_touched(index);
            }
            let _io = lock(&slot.io);
            if lock(&slot.state).deleted {
                return Ok(());
            }
            self.store_block_cache(id, &version, &blocks)?;
            self.finish_flush(index, id, &version, Some(vv), &meta, &row, &blocks)
        })();
        if let Some(effect) = &failed_import_effect {
            self.apply_import_index(index, id, effect, was_in_trash)?;
        }
        if result.is_err() {
            let mut st = lock(&slot.state);
            st.dirty = true;
            st.exported |= durable;
        }
        result
    }
    /// Markdown file and index content of a flushed page. Pages in the trash
    /// have no markdown file. A file edited outside the app since it was
    /// written is left alone until `rescan` reads it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn finish_flush(
        &self,
        index: &Index,
        id: &str,
        version: &[u8],
        vv: Option<Vec<u8>>,
        meta: &PageMeta,
        row: &MetaRow,
        blocks: &[Block],
    ) -> Result<()> {
        let titles = |pid: &str| index.title_of(pid).ok().flatten();
        let mut content = content_row(meta, &row.props, blocks, &titles);
        content.vv = vv;
        let (old_md, old_hash) = index.md(id)?;
        let in_trash = match index.in_trash(id)? {
            Some(t) => t,
            None => {
                meta.trashed.is_some()
                    || match &meta.parent {
                        Some(p) => index.in_trash(p)?.unwrap_or(false),
                        None => false,
                    }
            }
        };
        let mount = self.mount_folder(meta.parent.as_deref(), meta.is_template);
        if in_trash {
            if let Some(old) = &old_md {
                self.drop_md(id, old, old_hash.as_deref())?;
            }
            index.clear_md(id)?;
        } else if let Some(folder) = mount {
            self.write_mounted(index, &folder, version, meta, row, blocks, &mut content)?;
        } else {
            let schema = match &meta.parent {
                Some(p) => index.schema_of(p)?,
                None => None,
            };
            let titles = |pid: &str| index.title_of(pid).ok().flatten();
            let ctx = RenderCtx {
                titles: &titles,
                schema: schema.as_deref(),
            };
            let text = markdown::render(meta, &row.props, blocks, &ctx);
            let name = markdown::file_name(&meta.title, id);
            let external = match (&old_md, &old_hash) {
                (Some(p), Some(h)) => match fs::read(self.pages_dir().join(p)) {
                    Ok(b) => attach::sha256_hex(&b) != *h,
                    Err(_) => false,
                },
                _ => false,
            };
            if !external {
                write_atomic(&self.pages_dir().join(&name), text.as_bytes())?;
                if let Some(old) = &old_md {
                    if *old != name {
                        self.drop_md(id, old, old_hash.as_deref())?;
                    }
                }
                content.md_hash = Some(attach::sha256_hex(text.as_bytes()));
                content.md_path = Some(name);
                content.md_version = Some(version.to_vec());
            }
        }
        if !index.update_content(id, meta.updated, &content)? {
            index.upsert(row, &content)?;
        }
        if index.conn.is_autocommit() {
            self.index_committed(index, vec![id.to_owned()]);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/support/cold_history_unit.rs"]
mod cold_history_tests;

#[cfg(test)]
#[path = "../tests/support/cold_history_bench.rs"]
mod cold_history_bench;

#[cfg(test)]
#[path = "../tests/support/cold_interleaving.rs"]
mod cold_interleaving_tests;

fn writer_loop(inner: Arc<Inner>, rx: Receiver<Msg>, index: Index) {
    let mut pending: HashSet<PageId> = HashSet::new();
    let mut forced: HashSet<PageId> = HashSet::new();
    let mut waiters: Vec<Sender<()>> = Vec::new();
    let mut first = Instant::now();
    let mut cold_cursor = None;
    loop {
        let mut flush_now = false;
        let mut stop = false;
        let msg = if pending.is_empty() {
            match rx.recv_timeout(Duration::from_secs(inner.cold_policy.idle_seconds)) {
                Ok(msg) => Some(msg),
                Err(RecvTimeoutError::Disconnected) => None,
                Err(RecvTimeoutError::Timeout) => {
                    if let Err(e) = inner.cold_pass(&index, &mut cold_cursor, &inner.cold_policy, now_ms()) {
                        *lock(&inner.writer_error) = Some(format!("cold history: {e}"));
                    }
                    continue;
                }
            }
        } else {
            match rx.recv_timeout(DEBOUNCE) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => {
                    flush_now = true;
                    None
                }
                Err(RecvTimeoutError::Disconnected) => None,
            }
        };
        match msg {
            #[cfg(test)]
            Some(Msg::Crash) => break,
            Some(Msg::Dirty(id)) => {
                if pending.is_empty() {
                    first = Instant::now();
                }
                pending.insert(id);
            }
            Some(Msg::Recover(id)) => {
                if pending.is_empty() {
                    first = Instant::now();
                }
                forced.insert(id.clone());
                pending.insert(id);
            }
            Some(Msg::Flush(ack)) => {
                waiters.push(ack);
                flush_now = true;
            }
            Some(Msg::Warm(id)) => {
                let slot = lock(&inner.cache).get(&id).cloned();
                if let Some(slot) = slot {
                    let mut st = lock(&slot.state);
                    if !st.deleted && !st.doc.is_warm() {
                        st.doc.blocks();
                    }
                }
            }
            Some(Msg::Stop) => stop = true,
            None => stop = !flush_now,
        }
        if !pending.is_empty() && first.elapsed() >= MAX_DELAY {
            flush_now = true;
        }
        if flush_now || stop {
            let mut failed = Vec::new();
            // Pages flush in a stable order; one transaction for the batch.
            let mut ids: Vec<PageId> = pending.drain().collect();
            ids.sort();
            // Short transactions, so callers' index writes are not held up.
            for chunk in ids.chunks(WRITER_BATCH) {
                let tx = match index.write_tx() {
                    Ok(tx) => tx,
                    Err(e) => {
                        *lock(&inner.writer_error) = Some(format!("starting index write: {e}"));
                        failed.extend(chunk.iter().map(|id| (id.clone(), forced.contains(id))));
                        continue;
                    }
                };
                for id in chunk {
                    let force = forced.contains(id);
                    if let Err(e) = inner.flush_page(&index, id, force) {
                        *lock(&inner.writer_error) = Some(format!("writing page {id}: {e}"));
                        failed.push((id.clone(), force));
                    }
                }
                if let Err(e) = tx.commit() {
                    *lock(&inner.writer_error) = Some(format!("writing the index: {e}"));
                    for id in chunk {
                        if let Some(slot) = inner.cached_slot(id) {
                            lock(&slot.state).dirty = true;
                        }
                        failed.push((id.clone(), forced.contains(id)));
                    }
                } else {
                    // A failed flush can still apply a valid partial import
                    // to this transaction. Publish every affected batch ID,
                    // including those queued for retry, after it commits.
                    inner.index_committed(&index, chunk.to_vec());
                }
            }
            forced.clear();
            // Retry failed pages on the next cycle (not after a stop).
            if !stop && !failed.is_empty() {
                first = Instant::now();
                for (id, force) in failed {
                    if force {
                        forced.insert(id.clone());
                    }
                    pending.insert(id);
                }
            }
            for w in waiters.drain(..) {
                let _ = w.send(());
            }
        }
        if stop {
            break;
        }
    }
}

/// Copies of `blocks` with fresh block ids.
pub(crate) fn fresh_ids(blocks: &[Block]) -> Vec<Block> {
    blocks
        .iter()
        .map(|b| Block {
            id: new_id(),
            children: fresh_ids(&b.children),
            ..b.clone()
        })
        .collect()
}

impl Vault {
    /// Opens the vault at `root`, creating the layout if missing. Rebuilds the
    /// index if it is missing or from an older version, recovers missing rows
    /// before returning, and finishes interrupted file writes in the background.
    pub fn open(root: String) -> Result<Vault> {
        let root = PathBuf::from(root);
        for d in [
            root.join("pages"),
            root.join(".vault").join("docs"),
            root.join(".vault").join("logs"),
            root.join(".vault").join("cold"),
        ] {
            fs::create_dir_all(d)?;
        }
        let lock_file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join(".vault").join("lock"))?;
        // Only the first instance open on a vault writes with the device's
        // peer id; another one (a second process, or a crash-recovery test)
        // gets a random peer, as two writers with one peer id would corrupt
        // documents.
        let owner = try_lock_exclusive(&lock_file)?;
        let device = device_id(&root)?;
        let peer = if owner {
            peer_of(&device)
        } else {
            loro::LoroDoc::new().peer_id()
        };
        let purged = read_purged(&root)?;
        let mounts = crate::mounts::read_table(&root);
        let db_path = root.join(".vault").join("index.sqlite");
        let (db, fresh) = Index::open(&db_path)?;
        if !fresh {
            crate::calendars::refresh_legacy_calendar_rows(&db)?;
        }
        let (writer_db, _) = Index::open(&db_path)?;
        let flush_file = shared_flush_file(&root)?;
        let (tx, rx) = mpsc::channel();
        let cold_pages = fs::read_dir(root.join(".vault/cold"))?
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_str()?.strip_suffix(".cold").map(str::to_owned))
            .collect();
        let cold_policy = match fs::read(root.join(".vault/cold-policy.json")) {
            Ok(bytes) => serde_json::from_slice::<ColdPolicy>(&bytes)
                .map_err(|e| invalid(format!("cold policy: {e}")))?,
            Err(e) if e.kind() == ErrorKind::NotFound => ColdPolicy::default(),
            Err(e) => return Err(e.into()),
        };
        if cold_policy.age_seconds < 0 || cold_policy.pages_per_pass == 0
            || cold_policy.idle_seconds == 0 {
            return Err(invalid("cold policy needs nonnegative age and positive pass/idle bounds"));
        }
        let inner = Arc::new(Inner {
            root,
            db: Mutex::new(db),
            search_metadata: Mutex::new(crate::search_api::SearchMetadata::default()),
            cache: Mutex::new(HashMap::new()),
            tx,
            writer_error: Mutex::new(None),
            device,
            peer,
            feed: Mutex::new(Feed::default()),
            feed_cv: Condvar::new(),
            index_generation: AtomicU64::new(0),
            live_queries: Mutex::new(crate::live_query::Registry::default()),
            live_stop: AtomicBool::new(false),
            purged: Mutex::new(purged),
            purge_sync: Mutex::new(()),
            _lock: lock_file,
            flush_file,
            mounts: Mutex::new(mounts),
            mount_seen: Mutex::new(HashMap::new()),
            daily_notes: Mutex::new(()),
            cold_policy,
            cold_pages: Mutex::new(cold_pages),
            #[cfg(test)]
            cold_before_publish: Mutex::new(None),
            #[cfg(test)]
            disk_io: DiskIo::default(),
            calendars: Mutex::new(()),
            nodes: Mutex::new(()),
            conflict_auto_resolve_cache: std::sync::atomic::AtomicBool::new(false),
        });
        let worker = inner.clone();
        let handle = std::thread::Builder::new()
            .name("vault-writer".into())
            .spawn(move || writer_loop(worker, rx, writer_db))?;
        let vault = Vault {
            inner,
            writer: Mutex::new(Some(handle)),
            live_worker: Mutex::new(None),
        };
        if owner {
            vault.recover_restorations()?;
            vault.recover_purged_leftovers()?;
        }
        if fresh && fs::read_dir(vault.inner.docs_dir())?.next().is_some() {
            vault.reindex()?;
        } else if !fresh {
            let db = lock(&vault.inner.db);
            let mut missing = Vec::new();
            for entry in fs::read_dir(vault.inner.docs_dir())? {
                let name = entry?.file_name().to_string_lossy().into_owned();
                if let Some(id) = name.strip_suffix(".loro") {
                    if !vault.inner.is_purged(id) && !db.exists(id)? {
                        missing.push(id.to_string());
                    }
                }
            }
            vault.inner.recover_missing_rows(&db, &missing)?;
        }
        // Typo correction's titles and vocabulary are built now, not on the
        // first misspelled query.
        lock(&vault.inner.db).refresh_fuzzy()?;
        let mut seen = HashSet::new();
        for entry in fs::read_dir(vault.inner.logs_dir())? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(id) = name
                .strip_suffix(".log.old")
                .or_else(|| name.strip_suffix(".log"))
            {
                if vault.inner.is_purged(id) {
                    continue;
                }
                #[cfg(test)]
                vault
                    .inner
                    .disk_io
                    .startup_metadata
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                // Truncated logs are intentionally retained. Listing gives
                // names only on Unix, so one metadata probe is necessary;
                // an empty file needs neither recovery nor a content read.
                if entry.metadata()?.len() == 0 {
                    continue;
                }
                if seen.insert(id.to_string()) {
                    #[cfg(test)]
                    vault
                        .inner
                        .disk_io
                        .startup_recoveries
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    vault.inner.send(Msg::Recover(id.to_string()));
                }
            }
        }
        vault.repair_day_notes_on_open()?;
        vault.rebuild_conflict_archive_mirror()?;
        vault.refresh_conflict_policy()?;
        vault.sweep_conflict_policy()?;
        let (live_db, _) = Index::open(&db_path)?;
        let worker = vault.inner.clone();
        let live_worker = std::thread::Builder::new().name("vault-live-queries".into())
            .spawn(move || crate::live_query::run(worker, live_db))?;
        *lock(&vault.live_worker) = Some(live_worker);
        Ok(vault)
    }

    /// The vault's root folder.
    pub fn root(&self) -> String {
        self.inner.root.to_string_lossy().into_owned()
    }

    /// This vault copy's device id (random, kept in `.vault/device`).
    pub fn device_id(&self) -> String {
        self.inner.device.clone()
    }

    /// Pages not in the trash.
    pub fn page_count(&self) -> Result<u32> {
        lock(&self.inner.db).page_count()
    }

    /// Pages sorted by last update, newest first. `parent: None` lists all
    /// pages; `Some(id)` lists the children of that page. Pages in the trash
    /// are left out.
    pub fn list_pages(
        &self,
        parent: Option<PageId>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<PageSummary>> {
        lock(&self.inner.db).list(parent.as_deref(), offset, limit)
    }

    /// Adaptive prefix search. Broad queries match titles until further
    /// typing makes a full-text search selective. Use `search_adaptive` to
    /// receive the narrowing flag, or `search_full` for exhaustive matching.
    pub fn search(&self, query: String, limit: u32) -> Result<Vec<SearchHit>> {
        Ok(self.search_adaptive(query, limit)?.hits)
    }

    /// Adaptive search over vault pages only. External rows are excluded
    /// before ranking, the limit and corrections, so they cannot crowd out
    /// page hits. Page pickers use this; `search` also returns external rows.
    pub fn search_pages(&self, query: String, limit: u32) -> Result<Vec<SearchHit>> {
        let parsed = self.parse_search(query)?;
        Ok(lock(&self.inner.db)
            .search_spec_pages(&parsed.spec, limit, 0)?
            .hits)
    }

    /// Adaptive search with metadata, including when no title matches exist.
    pub fn search_adaptive(&self, query: String, limit: u32) -> Result<SearchResults> {
        let parsed = self.parse_search(query)?;
        let result = self.search_spec(parsed.spec, limit, 0)?;
        Ok(SearchResults {
            hits: result.hits,
            narrowed: result.narrowed,
        })
    }

    /// Full ranked title/body prefix search, without adaptive narrowing.
    /// Exact/prefix title and body matches precede typo corrections when
    /// query words are absent from the vocabulary (last word: no prefix). Trash and templates
    /// are excluded. Intended for API callers that need full body recall.
    pub fn search_full(&self, query: String, limit: u32) -> Result<Vec<SearchHit>> {
        let parsed = self.parse_search(query)?;
        Ok(lock(&self.inner.db)
            .search_spec(&parsed.spec, limit, 0, true)?
            .hits)
    }

    /// Adds or replaces a row of an external search source (for example agent
    /// chats). External rows are searched, filtered and ranked with the pages,
    /// but are never listed, shown in the tree or changed by page operations.
    /// `id` is local to `source`; it may equal a page id.
    pub fn upsert_search_row(&self, source: String, row: ExternalSearchRow) -> Result<()> {
        check_search_identity(&source, Some(&row.id))?;
        let db = lock(&self.inner.db);
        db.upsert_search_row(&source, &row)?;
        self.inner.index_committed(&db, Vec::new());
        self.rerender_linking(&db, &row.id)
    }

    /// Removes one external row. A row that does not exist is not an error.
    pub fn delete_search_row(&self, source: String, id: String) -> Result<()> {
        check_search_identity(&source, Some(&id))?;
        let db = lock(&self.inner.db);
        db.delete_search_row(&source, &id)?;
        self.inner.index_committed(&db, Vec::new());
        self.rerender_linking(&db, &id)
    }

    /// Removes every external row of `source`. Reindexing and clearing the
    /// index keep all external rows.
    pub fn clear_search_source(&self, source: String) -> Result<()> {
        check_search_identity(&source, None)?;
        let db = lock(&self.inner.db);
        // Found before the rows go: afterwards nothing says what they were.
        let linking = db.link_sources_of_source(&source)?;
        db.clear_search_source(&source)?;
        self.inner.index_committed(&db, Vec::new());
        for src in linking {
            self.inner.rerender(&src);
        }
        Ok(())
    }

    /// Re-renders the pages linking to `id`: an embed shows the current title
    /// of its target, which an external row may have just changed or removed.
    fn rerender_linking(&self, db: &Index, id: &str) -> Result<()> {
        for src in db.link_sources(id)? {
            self.inner.rerender(&src);
        }
        Ok(())
    }

    /// Resolves external note metadata without searching title/body text.
    pub fn find_indexed_note(&self, id: String) -> Result<Option<SearchHit>> {
        lock(&self.inner.db).find_indexed_note(&id)
    }

    /// Size and build time of the typo-correction word index. Reports the
    /// cache as last built: zeros before the first build, and not refreshed
    /// by writes until a fuzzy query (or open) rebuilds it.
    pub fn search_index_stats(&self) -> SearchIndexStats {
        lock(&self.inner.db).search_index_stats()
    }

    /// Creates a page at the end of its siblings and returns its id. When
    /// the parent has a default template, the new page starts as a copy of
    /// the template's blocks, properties, icon and cover (`templates.rs`).
    pub fn create_page(&self, parent: Option<PageId>, title: String) -> Result<PageId> {
        self.create_child(parent, title, crate::templates::Start::Default, false)
    }

    /// Creates an empty page at a preferred identity without replacing any
    /// existing document. Used only while holding the daily-note lock.
    pub(crate) fn create_identified_page(
        &self,
        preferred: &str,
        parent: Option<&str>,
        title: &str,
        matches: impl FnOnce(&PageDoc) -> bool,
        initialize: impl FnOnce(&mut PageDoc) -> Result<()>,
    ) -> Result<(PageId, bool)> {
        let db = lock(&self.inner.db);
        let occupied = |id: &str| -> Result<bool> {
            Ok(db.exists(id)?
                || self.inner.doc_path(id).exists()
                || self.inner.log_path(id).exists()
                || self.inner.old_log_path(id).exists())
        };
        let mut id = preferred.to_owned();
        if self.inner.is_purged(&id) {
            // Purge tombstones are permanent. A replacement gets a new key.
            loop {
                id = new_id();
                if !self.inner.is_purged(&id) && !occupied(&id)? {
                    break;
                }
            }
        } else if occupied(&id)? {
            let same = self
                .inner
                .read(&id, |doc| doc.meta().id == id && matches(doc))?;
            if !same {
                return Err(invalid(format!(
                    "daily-note identity {id} is occupied by an unrelated page"
                )));
            }
            // A document can survive without its index row. Index the existing
            // document before restoring/reusing it, retaining its CRDT history.
            if !db.exists(&id)? {
                self.inner.flush_page(&db, &id, true)?;
            }
            let in_trash = db.in_trash(&id)? == Some(true);
            drop(db);
            if in_trash {
                self.restore_page(id.clone())?;
            }
            return Ok((id, false));
        }
        if let Some(parent) = parent {
            if db.in_trash(parent)? != Some(false) {
                return Err(not_found("page", parent));
            }
        }
        let key = order::between(db.last_child_key(parent)?.as_deref(), None);
        let mut doc = PageDoc::new_page(self.inner.actor_peer(), &id, title, parent, stamp_ms())?;
        doc.set_meta_str("order", Some(&key))?;
        initialize(&mut doc)?;
        let id = self.inner.install(&db, doc)?;
        drop(db);
        self.inner.emit([Change::TreeChanged]);
        Ok((id, true))
    }

    /// Creates a page at the caller's `id` under `parent`, copying `template`
    /// when given (`None`: an empty page). Nothing changes when `id` is in use:
    /// by a page in the index or the trash, by a file, or by a purge (purge
    /// tombstones are permanent, so no other id is used). Returns the id and
    /// whether this call created the page. Serialized like `daily_note`: the
    /// daily-notes file lock, then its mutex, then the index.
    pub(crate) fn create_unused_page(
        &self,
        id: &str,
        parent: &str,
        title: &str,
        template: Option<String>,
        initialize: impl FnOnce(&mut PageDoc) -> Result<()>,
    ) -> Result<(String, bool)> {
        if !crate::ids::is_page_id(id) {
            return Err(invalid("a page id is 26 characters of the id alphabet"));
        }
        let _file_guard = self.inner.daily_notes_lock()?;
        let _guard = lock(&self.inner.daily_notes);
        let db = lock(&self.inner.db);
        if self.inner.is_purged(id)
            || db.exists(id)?
            || self.inner.doc_path(id).exists()
            || self.inner.log_path(id).exists()
            || self.inner.old_log_path(id).exists()
        {
            return Ok((id.to_owned(), false));
        }
        match db.in_trash(parent)? {
            None => return Err(not_found("page", parent)),
            Some(true) => return Err(invalid(format!("page {parent} is in the trash"))),
            Some(false) => {}
        }
        if let Some(t) = &template {
            crate::templates::check_template(&db, parent, t)?;
        }
        let key = order::between(db.last_child_key(Some(parent))?.as_deref(), None);
        let mut doc = PageDoc::new_page(self.inner.actor_peer(), id, title, Some(parent), stamp_ms())?;
        doc.set_meta_str("order", Some(&key))?;
        if let Some(t) = template {
            // A template that cannot be read is an error: nothing is installed
            // yet, so no empty page is left behind and reported as created.
            let copy = self.inner.read(&t, crate::templates::TemplateCopy::of)?;
            copy.apply_to(&mut doc)?;
        }
        initialize(&mut doc)?;
        let id = self.inner.install(&db, doc)?;
        drop(db);
        self.inner.emit([Change::TreeChanged]);
        Ok((id, true))
    }

    /// `create_page` with a page type. The type is set after the template's
    /// blocks are copied and before the page is written, so the new page
    /// appears once, with its type. `None` makes a document.
    pub fn create_page_with_type(
        &self,
        parent: Option<PageId>,
        title: String,
        page_type: Option<String>,
    ) -> Result<PageId> {
        self.create_child_typed(
            parent,
            title,
            crate::templates::Start::Default,
            false,
            page_type,
        )
    }

    /// `create_page` starting from `start`; `template` marks the new page
    /// as a template of its parent.
    pub(crate) fn create_child(
        &self,
        parent: Option<PageId>,
        title: String,
        start: crate::templates::Start,
        template: bool,
    ) -> Result<PageId> {
        self.create_child_typed(parent, title, start, template, None)
    }

    /// `create_child` with an optional page type (see `create_page_with_type`).
    pub(crate) fn create_child_typed(
        &self,
        parent: Option<PageId>,
        title: String,
        start: crate::templates::Start,
        template: bool,
        page_type: Option<String>,
    ) -> Result<PageId> {
        use crate::templates::Start;
        let db = lock(&self.inner.db);
        let mut from = None;
        if let Some(p) = &parent {
            match db.in_trash(p)? {
                None => return Err(not_found("page", p)),
                Some(true) => return Err(invalid(format!("page {p} is in the trash"))),
                Some(false) => {}
            }
            from = match start {
                Start::Default => db.template_of(p)?,
                Start::Empty => None,
                Start::Template(t) => {
                    crate::templates::check_template(&db, p, &t)?;
                    Some(t)
                }
            };
        }
        let key = order::between(db.last_child_key(parent.as_deref())?.as_deref(), None);
        let id = new_id();
        let mut doc = PageDoc::new_page(
            self.inner.actor_peer(),
            &id,
            &title,
            parent.as_deref(),
            stamp_ms(),
        )?;
        doc.set_meta_str("order", Some(&key))?;
        if parent.is_none() && !template {
            // New top-level pages start uncoloured; the user picks a colour.
            doc.set_meta_str("color", Some("none"))?;
        }
        if template {
            doc.set_is_template(true)?;
        }
        if let Some(t) = from {
            // A template that cannot be read leaves the page empty.
            if let Ok(copy) = self.inner.read(&t, crate::templates::TemplateCopy::of) {
                copy.apply_to(&mut doc)?;
            }
        }
        if let Some(t) = &page_type {
            doc.set_page_type(t)?;
        }
        let id = self.inner.install(&db, doc)?;
        drop(db);
        self.inner.emit([Change::TreeChanged]);
        Ok(id)
    }

    /// Changes the page's type to any type string (see `PageDoc::set_page_type`).
    /// The page's add-on data is kept.
    pub fn set_page_type(&self, page: PageId, page_type: String) -> Result<()> {
        self.inner
            .edit(&page, |doc| doc.set_page_type(&page_type))?;
        let db = lock(&self.inner.db);
        self.inner.sync_meta(&db, &page)?;
        drop(db);
        self.inner.emit([Change::TreeChanged]);
        Ok(())
    }

    pub fn rename_page(&self, id: PageId, title: String) -> Result<()> {
        self.inner.edit(&id, |doc| doc.set_title(&title))?;
        let db = lock(&self.inner.db);
        self.inner.sync_meta(&db, &id)?;
        // Pages linking here show the title in their markdown.
        for src in db.link_sources(&id)? {
            self.inner.rerender(&src);
        }
        drop(db);
        self.inner.emit([Change::TreeChanged]);
        Ok(())
    }

    /// Moves the page and the pages below it to the trash. They leave lists,
    /// search and queries and their markdown files are removed; `restore_page`
    /// brings them back and `purge_page` deletes them for good.
    pub fn delete_page(&self, id: PageId) -> Result<()> {
        self.trash_page(id)
    }

    /// The page with its blocks, properties and details. Pages in the trash
    /// load too (`in_trash` is set).
    pub fn load_page(&self, id: PageId) -> Result<Page> {
        let in_trash = lock(&self.inner.db).in_trash(&id)?.unwrap_or(false);
        let slot = self.inner.slot(&id)?;
        let mut st = lock(&slot.state);
        if st.deleted {
            return Err(not_found("page", &id));
        }
        if !st.doc.is_warm() {
            if let Some(blocks) = self.inner.cached_blocks(&id, &st.doc) {
                let page = page_of(&st.doc, blocks, in_trash);
                drop(st);
                self.inner.send(Msg::Warm(id.clone()));
                return self.with_computed(page);
            }
        }
        let blocks = st.doc.blocks();
        let page = page_of(&st.doc, blocks, in_trash);
        drop(st);
        self.with_computed(page)
    }

    /// Renders a page using the same Markdown format, title resolver and
    /// parent schema as the vault writer, including a stored type body.
    pub fn export_markdown(&self, page: String) -> Result<String> {
        let (meta, props, blocks) = self
            .inner
            .read(&page, |d| (d.meta(), d.props(), d.blocks()))?;
        let db = lock(&self.inner.db);
        let schema = match &meta.parent {
            Some(parent) => db.schema_of(parent)?,
            None => None,
        };
        let titles = |id: &str| db.title_of(id).ok().flatten();
        let ctx = RenderCtx {
            titles: &titles,
            schema: schema.as_deref(),
        };
        Ok(markdown::render(&meta, &props, &blocks, &ctx))
    }

    /// Adds the page's formula and rollup values (kept in the index only).
    fn with_computed(&self, mut page: Page) -> Result<Page> {
        let extra = lock(&self.inner.db).computed_values(&page.id)?;
        if !extra.is_empty() {
            page.properties
                .retain(|p| !extra.iter().any(|e| e.key == p.key));
            page.properties.extend(extra);
            page.properties.sort_by(|a, b| a.key.cmp(&b.key));
        }
        Ok(page)
    }

    /// Applies the ops as one change: all of them or, on error, none.
    pub fn apply(&self, id: PageId, ops: Vec<BlockOp>) -> Result<()> {
        if ops.is_empty() {
            return Ok(());
        }
        self.inner.edit(&id, |doc| doc.apply(&ops))
    }

    /// Waits until every change so far is written to the Loro snapshots,
    /// markdown files and index. Returns the last background write error, if
    /// any. Call it before the app is suspended or quits.
    pub fn flush(&self) -> Result<()> {
        let (ack, done) = mpsc::channel();
        self.inner.send(Msg::Flush(ack));
        let _ = done.recv();
        match lock(&self.inner.writer_error).take() {
            Some(e) => Err(VaultError::Io(e)),
            None => Ok(()),
        }
    }

    /// Rebuilds the index from the Loro documents. Returns the page count
    /// (pages in the trash included).
    pub fn reindex(&self) -> Result<u32> {
        self.recover_purged_leftovers()?;
        self.flush()?;
        let db = lock(&self.inner.db);
        let tx = db.write_tx()?;
        // Files in mounted folders keep their pages: their names cannot be
        // told from the titles (a taken name gets a number).
        let mounted: HashMap<String, crate::index::MdRow> = db
            .md_rows()?
            .into_iter()
            .filter(|(_, md, _, _)| Path::new(md).is_absolute())
            .map(|r| (r.0.clone(), r))
            .collect();
        db.clear()?;
        let defer = db.defer_guard();
        let mut count = 0;
        let mut missing_md = Vec::new();
        let mut file_names: HashMap<String, String> = HashMap::new();
        // Pages with embeds: their search text needs titles that may not be
        // indexed yet, so they are written again once all pages are in.
        let mut embedding = Vec::new();
        for entry in fs::read_dir(self.inner.docs_dir())? {
            let name = entry?.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".loro") else {
                continue;
            };
            if self.inner.is_purged(id) {
                continue;
            }
            let cached = self.inner.cached_slot(id);
            let (row, meta, blocks, vv, conflicts) = match cached {
                Some(slot) => {
                    let mut st = lock(&slot.state);
                    let vv = st.doc.doc.oplog_vv().encode();
                    (meta_row(&st.doc), st.doc.meta(), st.doc.blocks(), vv, st.doc.conflicts())
                }
                None => {
                    let mut doc = self.inner.read_doc(id)?;
                    let vv = doc.doc.oplog_vv().encode();
                    (meta_row(&doc), doc.meta(), doc.blocks(), vv, doc.conflicts())
                }
            };
            db.set_conflicts(id, &crate::sync::mirror_rows(id, &conflicts))?;
            collect_file_names(&blocks, &mut file_names);
            if has_embed(&blocks) {
                embedding.push((row.clone(), meta.clone(), blocks.clone(), vv.clone()));
            }
            let titles = |pid: &str| db.title_of(pid).ok().flatten();
            let mut content = content_row(&meta, &row.props, &blocks, &titles);
            content.vv = Some(vv);
            if let Some(folder) = self
                .inner
                .mount_folder(meta.parent.as_deref(), meta.is_template)
            {
                // A mounted page keeps the file the index knew, if it is
                // still in the folder. Without one (a lost index) the page
                // has no file: `rescan_mounts` attaches a file of the same
                // name or imports it as a new page, so nothing is
                // overwritten.
                if let Some((_, md, hash, v)) = mounted.get(id) {
                    let f = Path::new(md);
                    if f.is_file() && f.starts_with(&folder) {
                        content.md_path = Some(md.clone());
                        content.md_hash = hash.clone();
                        content.md_version = v.clone();
                    }
                }
                db.upsert(&row, &content)?;
                count += 1;
                continue;
            }
            let md = markdown::file_name(&meta.title, id);
            match fs::read(self.inner.pages_dir().join(&md)) {
                Ok(bytes) => {
                    content.md_hash = Some(attach::sha256_hex(&bytes));
                    content.md_path = Some(md);
                }
                Err(_) => missing_md.push(id.to_string()),
            }
            db.upsert(&row, &content)?;
            count += 1;
        }
        let titles = |pid: &str| db.title_of(pid).ok().flatten();
        for (row, meta, blocks, vv) in embedding {
            // Markdown fields are kept: the content row leaves them unset.
            let mut content = content_row(&meta, &row.props, &blocks, &titles);
            content.vv = Some(vv);
            db.upsert(&row, &content)?;
        }
        db.refresh_all_trash()?;
        defer.finish()?;
        // Attachment details for blobs without a row (a fresh index).
        let known: HashSet<String> = db.all_attachment_ids()?.into_iter().collect();
        let now = now_ms();
        for (blob, size) in attach::list_blobs(&self.inner.root)? {
            if known.contains(&blob) {
                continue;
            }
            let name = file_names
                .get(&blob)
                .cloned()
                .unwrap_or_else(|| blob.clone());
            db.put_attachment(
                &crate::types::Attachment {
                    mime: attach::mime_of(&name).to_string(),
                    id: blob,
                    name,
                    size,
                },
                now,
            )?;
        }
        tx.commit()?;
        self.inner.index_committed(&db, Vec::new());
        drop(db);
        for id in missing_md {
            self.inner.send(Msg::Recover(id));
        }
        self.rebuild_conflict_archive_mirror()?;
        self.inner.emit([Change::TreeChanged]);
        Ok(count)
    }

    pub(crate) fn inner(&self) -> &Inner {
        &self.inner
    }

    pub(crate) fn with_db<T>(&self, f: impl FnOnce(&Index) -> Result<T>) -> Result<T> {
        f(&lock(&self.inner.db))
    }
}

/// Attachment id -> file name, from `File` blocks (their text is the name).
fn collect_file_names(blocks: &[Block], out: &mut HashMap<String, String>) {
    for b in blocks {
        if let (BlockKind::File, Some(t)) = (&b.kind, &b.target) {
            if !b.text.is_empty() {
                out.entry(t.clone()).or_insert_with(|| b.text.clone());
            }
        }
        collect_file_names(&b.children, out);
    }
}

/// Properties as a map, for comparisons.
pub(crate) fn prop_map(props: &[Property]) -> HashMap<String, crate::types::PropValue> {
    props
        .iter()
        .map(|p| (p.key.clone(), p.value.clone()))
        .collect()
}

impl Drop for Vault {
    fn drop(&mut self) {
        self.inner.send(Msg::Stop);
        if let Some(h) = lock(&self.writer).take() {
            let _ = h.join();
        }
        self.inner.live_stop.store(true, Ordering::Release);
        // Take the feed lock before notifying to close the check/wait race.
        self.inner.emit([]);
        if let Some(h) = lock(&self.live_worker).take() {
            let _ = h.join();
        }
        lock(&self.inner.live_queries).close();
    }
}

#[allow(dead_code)]
fn assert_send_sync() {
    fn check<T: Send + Sync>() {}
    check::<Vault>();
}

/// Takes an exclusive, non-blocking lock on `file`. Returns false if another
/// handle holds it. Android's std has no `File::try_lock`, so it uses flock.
#[cfg(not(target_os = "android"))]
fn try_lock_exclusive(file: &fs::File) -> std::io::Result<bool> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(fs::TryLockError::WouldBlock) => Ok(false),
        Err(fs::TryLockError::Error(e)) => Err(e),
    }
}

#[cfg(not(target_os = "android"))]
fn unlock_exclusive(file: &File) -> std::io::Result<()> {
    file.unlock()
}

#[cfg(not(windows))]
fn stamp_metadata(meta: &fs::Metadata) -> FileStamp {
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        Some(FileIdentity {
            volume: meta.dev(),
            index: u128::from(meta.ino()),
        })
    };
    #[cfg(not(unix))]
    let identity = None;
    FileStamp {
        identity,
        len: meta.len(),
    }
}

#[cfg(not(windows))]
fn stamp_handle(file: &File) -> std::io::Result<FileStamp> {
    file.metadata().map(|meta| stamp_metadata(&meta))
}

#[cfg(windows)]
fn stamp_handle(file: &File) -> std::io::Result<FileStamp> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: file owns a valid handle and info has the SDK-provided layout.
    // A successful call initializes the complete output structure.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let info = unsafe { info.assume_init() };
    Ok(FileStamp {
        identity: Some(FileIdentity {
            volume: u64::from(info.dwVolumeSerialNumber),
            index: u128::from(info.nFileIndexLow) | (u128::from(info.nFileIndexHigh) << 32),
        }),
        len: u64::from(info.nFileSizeLow) | (u64::from(info.nFileSizeHigh) << 32),
    })
}
fn stamp_path(path: &Path) -> std::io::Result<Option<FileStamp>> {
    #[cfg(windows)]
    let result = {
        use std::os::windows::fs::OpenOptionsExt;
        // Query the current directory entry, without reading file contents.
        // std's default share mode permits read, write and delete/rename.
        OpenOptions::new()
            .access_mode(0x80) // FILE_READ_ATTRIBUTES
            .open(path)
            .and_then(|file| stamp_handle(&file))
    };
    #[cfg(not(windows))]
    let result = fs::metadata(path).map(|meta| stamp_metadata(&meta));
    match result {
        Ok(stamp) => Ok(Some(stamp)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(target_os = "android")]
fn try_lock_exclusive(file: &fs::File) -> std::io::Result<bool> {
    use std::os::fd::AsRawFd;
    // SAFETY: flock on a valid, open file descriptor owned by `file`.
    let r = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if r == 0 {
        return Ok(true);
    }
    let e = std::io::Error::last_os_error();
    if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
        Ok(false)
    } else {
        Err(e)
    }
}

#[cfg(target_os = "android")]
fn unlock_exclusive(file: &File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: file owns the valid descriptor whose advisory lock we hold.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// External identities are `(source, local id)` pairs. The source `vault` is
/// reserved for pages, and neither part may be empty or contain the unit
/// separator, which joins them into one internal search id.
fn check_search_identity(source: &str, id: Option<&str>) -> Result<()> {
    if source.is_empty() {
        return Err(invalid("search source must not be empty"));
    }
    if source == crate::types::VAULT_SOURCE {
        return Err(invalid("search source \"vault\" is reserved for pages"));
    }
    if source.contains(crate::index::SEP) {
        return Err(invalid("search source must not contain U+001F"));
    }
    if let Some(id) = id {
        if id.is_empty() {
            return Err(invalid("search row id must not be empty"));
        }
        if id.contains(crate::index::SEP) {
            return Err(invalid("search row id must not contain U+001F"));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "flush_batch_tests.rs"]
mod flush_batch_tests;

#[cfg(test)]
#[path = "flush_lock_tests.rs"]
mod flush_lock_tests;

#[cfg(test)]
mod two_writer_storage_tests {
    use super::*;
    use crate::types::{NewBlock, PageUpdate};
    use std::sync::atomic::Ordering;

    struct Dir(PathBuf);

    impl Dir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("vault_flush_private_{}", new_id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn open_stopped(&self) -> Vault {
            let vault = Vault::open(self.0.to_string_lossy().into_owned()).unwrap();
            // Tests drive the real flush implementation synchronously. The
            // test-only stop joins without flushing pending writes, modeling
            // the crash boundary while safely releasing the writer thread.
            vault.inner.send(Msg::Crash);
            if let Some(writer) = lock(&vault.writer).take() {
                writer.join().unwrap();
            }
            vault
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn flush_now(vault: &Vault, id: &str, force: bool) {
        let index = lock(&vault.inner.db);
        let tx = index.write_tx().unwrap();
        vault.inner.flush_page(&index, id, force).unwrap();
        tx.commit().unwrap();
    }

    fn insert(vault: &Vault, page: &str, id: &str, text: &str) {
        vault
            .apply(
                page.to_string(),
                vec![BlockOp::Insert {
                    parent: None,
                    after: None,
                    block: NewBlock {
                        id: id.to_string(),
                        kind: BlockKind::Paragraph,
                        text: text.to_string(),
                        marks: vec![],
                        checked: false,
                    },
                }],
            )
            .unwrap();
    }

    fn texts(vault: &Vault, page: &str) -> Vec<String> {
        let mut out: Vec<_> = vault
            .load_page(page.to_string())
            .unwrap()
            .blocks
            .into_iter()
            .map(|block| block.text)
            .collect();
        out.sort();
        out
    }

    fn reset_io(vault: &Vault) {
        for counter in vault
            .inner
            .disk_io
            .stats
            .iter()
            .chain(&vault.inner.disk_io.reads)
        {
            counter.store(0, Ordering::Relaxed);
        }
    }

    fn io(vault: &Vault) -> ([usize; 3], [usize; 3]) {
        (
            std::array::from_fn(|i| vault.inner.disk_io.stats[i].load(Ordering::Relaxed)),
            std::array::from_fn(|i| vault.inner.disk_io.reads[i].load(Ordering::Relaxed)),
        )
    }

    #[test]
    fn unchanged_flush_stats_once_and_reads_no_sources() {
        let dir = Dir::new();
        let vault = dir.open_stopped();
        let page = vault.create_page(None, "stable".into()).unwrap();
        flush_now(&vault, &page, false);
        reset_io(&vault);
        flush_now(&vault, &page, true);
        assert_eq!(io(&vault), ([1, 1, 1], [0, 0, 0]));

        let slot = vault.inner.slot(&page).unwrap();
        vault.inner.mark_dirty(&mut lock(&slot.state), &page);
        reset_io(&vault);
        flush_now(&vault, &page, false);
        assert_eq!(io(&vault), ([1, 1, 1], [0, 0, 0]));
    }

    #[test]
    fn startup_skips_empty_logs_and_deduplicates_only_nonempty_logs() {
        let dir = Dir::new();
        let a = dir.open_stopped();
        let page = a.create_page(None, "startup".into()).unwrap();
        insert(&a, &page, "seed", "seed");
        let frames = fs::read(a.inner.log_path(&page)).unwrap();
        flush_now(&a, &page, false); // Retains a zero-length live log.
        {
            let _flush = a.inner.flush_lock().unwrap();
            File::create(a.inner.old_log_path(&page)).unwrap();
        }
        drop(a);
        let reopened = dir.open_stopped();
        assert_eq!(
            reopened
                .inner
                .disk_io
                .startup_metadata
                .load(Ordering::Relaxed),
            2
        );
        assert_eq!(
            reopened
                .inner
                .disk_io
                .startup_recoveries
                .load(Ordering::Relaxed),
            0
        );
        assert_eq!(io(&reopened), ([0, 0, 0], [0, 0, 0]));
        {
            let _flush = reopened.inner.flush_lock().unwrap();
            fs::write(reopened.inner.old_log_path(&page), &frames).unwrap();
        }
        drop(reopened);
        // Empty live log must not suppress recovery of its nonempty legacy
        // sibling, regardless of directory listing order.
        let legacy = dir.open_stopped();
        assert_eq!(
            legacy
                .inner
                .disk_io
                .startup_recoveries
                .load(Ordering::Relaxed),
            1
        );
        {
            let _flush = legacy.inner.flush_lock().unwrap();
            fs::write(legacy.inner.log_path(&page), &frames).unwrap();
        }
        drop(legacy);
        // Two nonempty siblings still queue just one recovery for the page.
        let both = dir.open_stopped();
        assert_eq!(
            both.inner
                .disk_io
                .startup_recoveries
                .load(Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn own_append_stats_once_and_dirty_flush_reads_no_sources() {
        let dir = Dir::new();
        let vault = dir.open_stopped();
        let page = vault.create_page(None, "stable".into()).unwrap();
        insert(&vault, &page, "one", "one"); // Open and cache the log handle.
        reset_io(&vault);
        insert(&vault, &page, "two", "two");
        assert_eq!(io(&vault), ([0, 0, 1], [0, 0, 0]));
        reset_io(&vault);
        flush_now(&vault, &page, false);
        // Two snapshot probes (before/after replacement), one per log path;
        // no document/log content reads, including our own pending frames.
        assert_eq!(io(&vault), ([2, 1, 1], [0, 0, 0]));
        assert_eq!(texts(&vault, &page), ["one", "two"]);
        let log_id = stamp_path(&vault.inner.log_path(&page))
            .unwrap()
            .unwrap()
            .identity;
        reset_io(&vault);
        insert(&vault, &page, "three", "three");
        assert_eq!(io(&vault), ([0, 0, 1], [0, 0, 0]));
        reset_io(&vault);
        flush_now(&vault, &page, false);
        assert_eq!(io(&vault), ([2, 1, 1], [0, 0, 0]));
        let after = stamp_path(&vault.inner.log_path(&page)).unwrap().unwrap();
        assert_eq!(after.identity, log_id);
        assert_eq!(after.len, 0);
    }

    #[test]
    fn foreign_sequence_refreshes_equal_identity_and_size_then_own_edits_skip_reads() {
        let dir = Dir::new();
        let a = dir.open_stopped();
        let page = a.create_page(None, "same files".into()).unwrap();
        insert(&a, &page, "seed", "seed");
        flush_now(&a, &page, false);
        let before: Vec<_> = (0..3)
            .map(|kind| stamp_path(&a.inner.page_file_path(&page, kind)).unwrap())
            .collect();
        let b = dir.open_stopped();
        let other = b.create_page(None, "other page".into()).unwrap();
        insert(&b, &other, "foreign", "foreign");
        flush_now(&b, &other, false);
        let after: Vec<_> = (0..3)
            .map(|kind| stamp_path(&a.inner.page_file_path(&page, kind)).unwrap())
            .collect();
        assert_eq!(before, after); // No identity or size changed on our page.
        reset_io(&a);
        // Append must not adopt the peer's sequence even though our log
        // looks unchanged before this edit. The next flush must refresh.
        insert(&a, &page, "next", "next");
        reset_io(&a);
        flush_now(&a, &page, false);
        assert_eq!(io(&a), ([2, 1, 1], [1, 0, 1]));
        assert_eq!(texts(&a, &page), ["next", "seed"]);
        // Once that sequence is merged, ordinary owned edits remain cheap.
        insert(&a, &page, "last", "last");
        reset_io(&a);
        flush_now(&a, &page, false);
        assert_eq!(io(&a), ([2, 1, 1], [0, 0, 0]));
        assert_eq!(texts(&a, &page), ["last", "next", "seed"]);
    }

    #[test]
    fn legacy_empty_or_torn_sequence_bootstraps_before_owned_edits() {
        for corrupt in [&b""[..], &b"torn record"[..]] {
            let dir = Dir::new();
            let a = dir.open_stopped();
            let page = a.create_page(None, "legacy".into()).unwrap();
            insert(&a, &page, "seed", "seed");
            flush_now(&a, &page, false);
            {
                let flush = a.inner.flush_lock().unwrap();
                let mut file: &File = &flush;
                file.seek(SeekFrom::Start(0)).unwrap();
                file.write_all(corrupt).unwrap();
                file.set_len(corrupt.len() as u64).unwrap();
                file.sync_all().unwrap();
            }
            drop(a);
            let reopened = dir.open_stopped();
            reopened.load_page(page.clone()).unwrap();
            insert(&reopened, &page, "own", "own");
            reset_io(&reopened);
            flush_now(&reopened, &page, false);
            assert_eq!(io(&reopened), ([2, 1, 1], [0, 0, 0]));
            assert_eq!(texts(&reopened, &page), ["own", "seed"]);
        }
    }
    #[test]
    fn dropping_an_unfinished_lock_holder_allows_flush_and_preserves_edits() {
        let dir = Dir::new();
        let a = dir.open_stopped();
        let page = a.create_page(None, "shared".into()).unwrap();
        flush_now(&a, &page, false);
        let b = dir.open_stopped();
        b.load_page(page.clone()).unwrap();
        insert(&a, &page, "a", "from a");
        insert(&b, &page, "b", "from b");
        let mut unfinished = a.inner.flush_lock().unwrap();
        let (blocked_tx, blocked_rx) = mpsc::channel();
        *lock(&b.inner.disk_io.lock_blocked) = Some(blocked_tx);
        let (done_tx, done_rx) = mpsc::channel();
        let other_page = page.clone();
        let other = std::thread::spawn(move || {
            flush_now(&b, &other_page, false);
            done_tx.send(()).unwrap();
            b
        });
        blocked_rx.recv_timeout(FLUSH_LOCK_TIMEOUT).unwrap();
        assert!(matches!(done_rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
        // Simulate handle-close release rather than normal RAII unlock.
        // The cache reopens this file at A's next acquisition.
        drop(unfinished.file.take());
        drop(unfinished);
        done_rx.recv_timeout(FLUSH_LOCK_TIMEOUT).unwrap();
        let b = other.join().unwrap();
        flush_now(&a, &page, false);
        assert_eq!(texts(&a, &page), ["from a", "from b"]);
        drop(b);
        drop(a);
        let reopened = dir.open_stopped();
        assert_eq!(texts(&reopened, &page), ["from a", "from b"]);
    }

    #[test]
    fn retained_handle_appends_at_new_eof_after_peer_truncate_without_final_flush() {
        let dir = Dir::new();
        let a = dir.open_stopped();
        let page = a.create_page(None, "shared".into()).unwrap();
        flush_now(&a, &page, false);
        let b = dir.open_stopped();
        b.load_page(page.clone()).unwrap();
        insert(&a, &page, "a", "before peer flush");
        assert!(lock(&a.inner.slot(&page).unwrap().state).log.is_some());
        let identity = stamp_path(&a.inner.log_path(&page))
            .unwrap()
            .unwrap()
            .identity;
        insert(&b, &page, "b", "peer edit");
        flush_now(&b, &page, false);
        let truncated = stamp_path(&a.inner.log_path(&page)).unwrap().unwrap();
        assert_eq!(truncated.identity, identity);
        assert_eq!(truncated.len, 0);
        assert!(lock(&a.inner.slot(&page).unwrap().state).log.is_some());
        reset_io(&a);
        insert(&a, &page, "late", "after peer flush");
        assert_eq!(io(&a), ([0, 0, 1], [0, 0, 0]));
        assert!(a.inner.log_path(&page).exists());
        // Both writer threads were stopped without flushing. Dropping A
        // therefore releases its cached file/owner lock without a final
        // snapshot, exactly the durability boundary under test.
        drop(a);
        drop(b);
        let reopened = dir.open_stopped();
        assert_eq!(
            texts(&reopened, &page),
            ["after peer flush", "before peer flush", "peer edit"]
        );
    }

    #[test]
    fn cached_flush_handle_excludes_same_instance_threads_and_unlocks_on_drop() {
        let dir = Dir::new();
        let vault = dir.open_stopped();
        let held = vault.inner.flush_lock().unwrap();
        let (blocked_tx, blocked_rx) = mpsc::channel();
        *lock(&vault.inner.disk_io.lock_blocked) = Some(blocked_tx);
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let inner = vault.inner.clone();
        let waiter = std::thread::spawn(move || {
            let _guard = inner.flush_lock().unwrap();
            acquired_tx.send(()).unwrap();
        });
        // This notification comes from an actual failed mutex acquisition,
        // not a sleep or a thread-start signal before attempting the lock.
        blocked_rx.recv_timeout(FLUSH_LOCK_TIMEOUT).unwrap();
        assert!(matches!(
            acquired_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        drop(held);
        acquired_rx.recv_timeout(FLUSH_LOCK_TIMEOUT).unwrap();
        waiter.join().unwrap();
        // Cached descriptor remains open, so this proves explicit OS unlock
        // happened before the same-process mutex became available again.
        assert!(lock(&vault.inner.flush_file).is_some());
        let external = open_flush_file(&dir.0).unwrap();
        assert!(try_lock_exclusive(&external).unwrap());
    }

    #[test]
    fn append_blocked_after_flush_merge_survives_truncation_and_no_final_flush() {
        let dir = Dir::new();
        let a = dir.open_stopped();
        let page = a.create_page(None, "paused flush".into()).unwrap();
        insert(&a, &page, "seed", "seed");
        flush_now(&a, &page, false);
        let b = dir.open_stopped();
        b.load_page(page.clone()).unwrap();
        insert(&b, &page, "b-before", "from B before merge");
        insert(&a, &page, "a", "before flush");
        let before = stamp_path(&a.inner.log_path(&page)).unwrap().unwrap();
        let (merged_tx, merged_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        *lock(&a.inner.disk_io.pause_after_merge) = Some(FlushPause {
            entered: merged_tx,
            resume: resume_rx,
        });
        let flush_page = page.clone();
        let flusher = std::thread::spawn(move || {
            flush_now(&a, &flush_page, false);
            a
        });
        merged_rx.recv_timeout(FLUSH_LOCK_TIMEOUT).unwrap();
        let (blocked_tx, blocked_rx) = mpsc::channel();
        let (appended_tx, appended_rx) = mpsc::channel();
        *lock(&b.inner.disk_io.lock_blocked) = Some(blocked_tx);
        let append_page = page.clone();
        let appender = std::thread::spawn(move || {
            insert(&b, &append_page, "b", "during flush");
            appended_tx.send(()).unwrap();
            b
        });
        // B's edit closure and export run before append tries the OS lock.
        // A still holds that lock after merge and before truncation.
        blocked_rx.recv_timeout(FLUSH_LOCK_TIMEOUT).unwrap();
        assert!(matches!(
            appended_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        let path = dir
            .0
            .join(".vault")
            .join("logs")
            .join(format!("{page}.log"));
        assert_eq!(stamp_path(&path).unwrap().unwrap(), before);
        resume_tx.send(()).unwrap();
        appended_rx.recv_timeout(FLUSH_LOCK_TIMEOUT).unwrap();
        let a = flusher.join().unwrap();
        let b = appender.join().unwrap();
        assert_eq!(
            texts(&a, &page),
            ["before flush", "from B before merge", "seed"]
        );
        let after = stamp_path(&path).unwrap().unwrap();
        assert_eq!(after.identity, before.identity);
        assert!(after.len > 0);
        // Both background writers are stopped; B has no final flush that
        // could rescue an append made to the wrong offset or stale handle.
        drop(a);
        drop(b);
        let reopened = dir.open_stopped();
        assert_eq!(
            texts(&reopened, &page),
            [
                "before flush",
                "during flush",
                "from B before merge",
                "seed"
            ]
        );
    }

    #[test]
    #[ignore = "manual 5-run lock + path identity/size microbenchmark"]
    fn lock_identity_size_microbenchmark() {
        const ITERATIONS: u32 = 10_000;
        const RUNS: usize = 5;
        let dir = Dir::new();
        let vault = dir.open_stopped();
        let page = vault.create_page(None, "microbenchmark".into()).unwrap();
        insert(&vault, &page, "seed", "seed");
        flush_now(&vault, &page, false);
        let path = vault.inner.log_path(&page);
        let expected = stamp_path(&path).unwrap();
        let mut ns_per_op = [0; RUNS];
        for measured in &mut ns_per_op {
            let started = Instant::now();
            for _ in 0..ITERATIONS {
                let guard = vault.inner.flush_lock().unwrap();
                let actual = std::hint::black_box(stamp_path(&path).unwrap());
                assert_eq!(actual, expected);
                drop(guard);
            }
            *measured = started.elapsed().as_nanos() / u128::from(ITERATIONS);
        }
        println!("lock + identity/size check + unlock: {ns_per_op:?} ns/op ({ITERATIONS} iterations/run)");
        ns_per_op.sort_unstable();
        println!(
            "median: {} ns/op; excludes sequence reads, Loro and writes",
            ns_per_op[RUNS / 2]
        );
    }

    #[test]
    fn crash_after_durable_snapshot_before_truncate_replays_without_duplicates() {
        let dir = Dir::new();
        let a = dir.open_stopped();
        let page = a.create_page(None, "crash boundary".into()).unwrap();
        flush_now(&a, &page, false);
        let b = dir.open_stopped();
        b.load_page(page.clone()).unwrap();
        insert(&a, &page, "a", "from a");
        insert(&b, &page, "b", "from b");
        let before = stamp_path(&a.inner.log_path(&page)).unwrap().unwrap();
        let before_sequence = lock(&a.inner.slot(&page).unwrap().state).seen.sequence;
        a.inner
            .disk_io
            .interrupt_after_snapshot
            .store(true, Ordering::Relaxed);
        {
            let index = lock(&a.inner.db);
            let tx = index.write_tx().unwrap();
            let error = a.inner.flush_page(&index, &page, false).unwrap_err();
            assert!(error
                .to_string()
                .contains("test crash after durable snapshot"));
            tx.commit().unwrap();
        }
        // Snapshot replacement and the preceding sequence bump succeeded,
        // but the entire original log is still present at the crash point.
        assert_eq!(
            stamp_path(&a.inner.log_path(&page)).unwrap().unwrap(),
            before
        );
        {
            let flush = a.inner.flush_lock().unwrap();
            assert_ne!(read_sequence(&flush).unwrap(), before_sequence);
        }
        drop(a);
        drop(b);
        let reopened = dir.open_stopped();
        assert_eq!(texts(&reopened, &page), ["from a", "from b"]);
        flush_now(&reopened, &page, true);
        assert_eq!(texts(&reopened, &page), ["from a", "from b"]);
        let after = stamp_path(&reopened.inner.log_path(&page))
            .unwrap()
            .unwrap();
        assert_eq!(after.identity, before.identity);
        assert_eq!(after.len, 0);
    }

    #[test]
    fn legacy_rotated_log_is_recovered_then_truncated_without_replacing_it() {
        let dir = Dir::new();
        let a = dir.open_stopped();
        let page = a.create_page(None, "legacy rotated".into()).unwrap();
        insert(&a, &page, "old", "only in legacy log");
        {
            let flush = a.inner.flush_lock().unwrap();
            a.inner
                .bump_sequence(&flush, read_sequence(&flush).unwrap())
                .unwrap();
            // Construct an old-version crash fixture. Production writers
            // never rotate; recovery must still accept the historic layout.
            fs::rename(a.inner.log_path(&page), a.inner.old_log_path(&page)).unwrap();
        }
        let old = stamp_path(&a.inner.old_log_path(&page)).unwrap().unwrap();
        drop(a);
        let recovered = dir.open_stopped();
        assert_eq!(texts(&recovered, &page), ["only in legacy log"]);
        flush_now(&recovered, &page, true);
        let after = stamp_path(&recovered.inner.old_log_path(&page))
            .unwrap()
            .unwrap();
        assert_eq!(after.identity, old.identity);
        assert_eq!(after.len, 0);
        drop(recovered);
        let reopened = dir.open_stopped();
        assert_eq!(texts(&reopened, &page), ["only in legacy log"]);
    }

    #[test]
    fn cached_handle_identity_check_reopens_replaced_log_without_final_flush() {
        let dir = Dir::new();
        let a = dir.open_stopped();
        let page = a
            .create_page(None, "replacement safety net".into())
            .unwrap();
        insert(&a, &page, "first", "first");
        let before = stamp_path(&a.inner.log_path(&page)).unwrap().unwrap();
        {
            let _flush = a.inner.flush_lock().unwrap();
            let path = a.inner.log_path(&page);
            let replacement = path.with_extension("test-replacement");
            fs::copy(&path, &replacement).unwrap();
            fs::rename(replacement, &path).unwrap();
        }
        let replaced = stamp_path(&a.inner.log_path(&page)).unwrap().unwrap();
        assert_ne!(replaced.identity, before.identity);
        assert_eq!(replaced.len, before.len);
        // Identity changes are detectable even without a sequence bump.
        insert(&a, &page, "next", "next");
        drop(a); // No writer/final flush can rescue a stale-handle append.
        let reopened = dir.open_stopped();
        assert_eq!(texts(&reopened, &page), ["first", "next"]);
    }

    #[test]
    fn cold_batch_import_waits_past_deadline_for_held_same_instance_flush_guard() {
        // Forty new pages exercise parallel cold decoding. The held guard
        // makes the importer queue before installing its prepared pages.
        const PAGES: usize = 40;
        let source_dir = Dir::new();
        let source = Vault::open(source_dir.0.to_string_lossy().into_owned()).unwrap();
        let mut updates = Vec::new();
        let mut expected = Vec::new();
        for n in 0..PAGES {
            let title = format!("cold batch {n:02}");
            let text = format!("cold body {n:02}");
            let page = source.create_page(None, title.clone()).unwrap();
            insert(&source, &page, "body", &text);
            let bytes = source.export_updates(page.clone(), Vec::new()).unwrap();
            updates.push(PageUpdate {
                page: page.clone(),
                bytes,
            });
            expected.push((page, title, text));
        }
        drop(source);

        let target_dir = Dir::new();
        let target = Vault::open(target_dir.0.to_string_lossy().into_owned()).unwrap();
        // Scoped threads borrow `target`; the guard and the import both use it.
        let (contended, import) = std::thread::scope(|sc| {
            let target = &target;
            let guard = target.inner.flush_lock().unwrap();
            let (blocked_tx, blocked_rx) = mpsc::channel();
            *lock(&target.inner.disk_io.lock_blocked) = Some(blocked_tx);
            let importer = sc.spawn(move || target.import_updates_batch(updates));
            // The first contention notification proves the importer is
            // waiting on the held guard. The receive is bounded, and on every
            // path below the guard is dropped and the importer joined.
            let contended = blocked_rx.recv_timeout(Duration::from_secs(10)).is_ok();
            if contended {
                // Fixed hold, independent of FLUSH_LOCK_TIMEOUT: long enough
                // to pass the 2 s in-process deadline.
                std::thread::sleep(Duration::from_millis(2_500));
            }
            drop(guard);
            let import = importer.join().expect("import thread panicked");
            (contended, import)
        });
        let report = import.expect(
            "cold batch import must survive a same-instance flush guard held past the deadline",
        );
        assert!(
            contended,
            "the importer never reached the held flush guard (no lock_blocked notification)"
        );
        assert_eq!(report.created, PAGES as u32);
        assert_eq!(report.changed + report.conflicts + report.skipped, 0);
        drop(target);

        let reopened = Vault::open(target_dir.0.to_string_lossy().into_owned()).unwrap();
        assert_eq!(reopened.page_count().unwrap(), PAGES as u32);
        for (page, title, text) in &expected {
            let loaded = reopened.load_page(page.clone()).unwrap();
            assert_eq!(&loaded.title, title);
            assert!(
                loaded.blocks.iter().any(|b| &b.text == text),
                "page {page} lost its body after reopen"
            );
        }
    }
}
