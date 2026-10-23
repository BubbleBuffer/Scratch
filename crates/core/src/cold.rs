//! Device-local cold history: one file per page, `.vault/cold/<id>.cold`.
//!
//! A cold file holds the page's original Loro updates up to a root (the
//! frontiers and version vector where the hot snapshot starts), so it reads on
//! its own. Layout, little endian:
//!
//! - magic `VCLD`, version `1`, dictionary tag (`0` none, `1` dictionary) and
//!   dictionary id (`0` when none), 8 bytes;
//! - frontiers and version vector, each a `u32` length and Loro's encoding;
//! - original updates: length and FNV-1a `u64`;
//! - zstd payload: length and FNV-1a `u64`;
//! - FNV-1a `u64` over every header byte before it;
//! - the zstd payload, to the end of the file.
//!
//! The checksums catch torn writes and bit rot. They are not a security measure.
//!
//! Dictionaries live in `.vault/cold/dictionaries/<id>.dict`, where `<id>` is
//! the FNV-1a hash of the dictionary as 16 hex digits. Once a vault has
//! `DICT_SAMPLES` readable plain cold files, one dictionary is trained from the
//! first `SAMPLE_MAX_BYTES` of each, and capped at `DICT_MAX_BYTES`. It is
//! written once, never changed, never deleted, and never retrained after it
//! exists, because cold files name the dictionary they need. Until then cold
//! files are plain zstd.
//!
//! Durability: a cold file is synced before its rename. The rename is durable
//! when `write` returns: on Unix the directory is then synced; on Windows the
//! rename is `MoveFileExW` with `MOVEFILE_WRITE_THROUGH`.
//!
//! Limits: this is not a proof against every crash. Unix `fsync` on macOS does
//! not flush the drive cache (that needs `F_FULLFSYNC`, not used here). Both
//! platforms depend on the drive honouring flushes; a drive that lies about
//! them can lose a write that returned. A crash between two files of one
//! operation (the cold file, then the hot snapshot) is ordered by the caller,
//! not by this module.

use std::fs::{self, File};
use std::io::{BufReader, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use loro::{Frontiers, VersionVector};

use crate::error::{invalid, not_found, Result};
#[cfg(feature = "storage-metrics")]
use crate::storage_metrics;

const MAGIC: &[u8; 4] = b"VCLD";
const VERSION: u8 = 1;
const TAG_NONE: u8 = 0;
const TAG_DICT: u8 = 1;
/// zstd's default level.
const LEVEL: i32 = 3;
/// Cold files that must be read before a dictionary is trained.
const DICT_SAMPLES: usize = 20;
/// Most decompressed update bytes taken from one cold file for training.
const SAMPLE_MAX_BYTES: u64 = 64 * 1024;
/// Largest dictionary training may produce.
const DICT_MAX_BYTES: usize = 8 * 1024;
/// Largest frontiers or version-vector blob a header may claim.
const MAX_BLOB: usize = 1 << 20;

/// Where a page's hot snapshot starts.
#[derive(Debug, Clone)]
pub(crate) struct ColdRoot {
    pub(crate) frontiers: Frontiers,
    pub(crate) vv: VersionVector,
}

/// A page's cold history: its original updates, all of them up to `root`.
#[derive(Debug, Clone)]
pub(crate) struct ColdPart {
    pub(crate) root: ColdRoot,
    pub(crate) updates: Vec<u8>,
}

/// The page's one cold file.
pub(crate) fn path(root: &Path, id: &str) -> PathBuf {
    cold_dir(root).join(format!("{id}.cold"))
}

fn cold_dir(root: &Path) -> PathBuf {
    root.join(".vault").join("cold")
}

fn dict_dir(root: &Path) -> PathBuf {
    cold_dir(root).join("dictionaries")
}

fn dict_path(root: &Path, id: u64) -> PathBuf {
    dict_dir(root).join(format!("{id:016x}.dict"))
}

/// Reads and checks a page's cold file, all of its updates. A missing,
/// malformed or damaged file, or one whose dictionary is missing, is an error.
pub(crate) fn read(root: &Path, id: &str) -> Result<ColdPart> {
    #[cfg(feature = "storage-metrics")]
    let _r = storage_metrics::span("cold.read");
    let bytes = {
        #[cfg(feature = "storage-metrics")]
        let _io = storage_metrics::span("cold.read_io");
        match fs::read(path(root, id)) {
            Ok(b) => b,
            Err(e) if e.kind() == ErrorKind::NotFound => return Err(not_found("cold history", id)),
            Err(e) => return Err(e.into()),
        }
    };
    #[cfg(feature = "storage-metrics")]
    storage_metrics::add_bytes("cold.read_io", bytes.len() as u64);
    decode(root, &bytes)
}

/// Writes the page's cold file atomically, replacing any earlier one. Uses the
/// vault's dictionary when one exists. Returns once the rename is durable.
#[cfg(test)]
pub(crate) fn write(root: &Path, id: &str, part: &ColdPart) -> Result<()> {
    write_prepared(root, id, &prepare(root, part)?)
}

/// Compress outside the vault publication lock. A prepared file can keep an
/// older dictionary choice because dictionaries are immutable and retained.
pub(crate) fn prepare(root: &Path, part: &ColdPart) -> Result<Vec<u8>> {
    #[cfg(feature = "storage-metrics")]
    let _p = storage_metrics::span("archive.prepare_cold");
    let dict = current_dict(root)?;
    let payload = compress(&part.updates, dict.as_ref().map(|(_, d)| d.as_slice()))?;
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    match &dict {
        Some((dict_id, _)) => {
            out.push(TAG_DICT);
            out.extend_from_slice(&dict_id.to_le_bytes());
        }
        None => {
            out.push(TAG_NONE);
            out.extend_from_slice(&0u64.to_le_bytes());
        }
    }
    push_blob(&mut out, &part.root.frontiers.encode())?;
    push_blob(&mut out, &part.root.vv.encode())?;
    out.extend_from_slice(&(part.updates.len() as u64).to_le_bytes());
    out.extend_from_slice(&fnv1a(&part.updates).to_le_bytes());
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(&fnv1a(&payload).to_le_bytes());
    let header_hash = fnv1a(&out);
    out.extend_from_slice(&header_hash.to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

pub(crate) fn write_prepared(root: &Path, id: &str, bytes: &[u8]) -> Result<()> {
    #[cfg(feature = "storage-metrics")]
    let _w = storage_metrics::span("archive.write_cold_io");
    #[cfg(feature = "storage-metrics")]
    storage_metrics::add_bytes("archive.write_cold_io", bytes.len() as u64);
    write_durable_atomic(&path(root, id), bytes)
}

/// Publish an already synced hot stage with the same rename protocol as cold.
pub(crate) fn publish_hot(from: &Path, to: &Path) -> Result<()> {
    rename_over(from, to)?;
    sync_dir(
        to.parent()
            .ok_or_else(|| invalid("hot path has no directory"))?,
    )
}

/// Trains the vault's dictionary once, when `DICT_SAMPLES` plain cold files
/// can be sampled and no dictionary exists. Cold files are visited one at a
/// time and sampling stops at `DICT_SAMPLES`, so memory stays at most
/// `DICT_SAMPLES * SAMPLE_MAX_BYTES`. If zstd refuses the samples, nothing is
/// written: cold files stay plain and a later pass retries. A successful
/// dictionary is never retrained.
pub(crate) fn maybe_train_dictionary(root: &Path) -> Result<()> {
    #[cfg(feature = "storage-metrics")]
    let _t = storage_metrics::span("archive.dict_train");
    if !dict_ids(root)?.is_empty() {
        return Ok(());
    }
    let entries = match fs::read_dir(cold_dir(root)) {
        Ok(e) => e,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let mut samples: Vec<Vec<u8>> = Vec::with_capacity(DICT_SAMPLES);
    for entry in entries {
        if samples.len() == DICT_SAMPLES {
            break;
        }
        let name = entry?.file_name();
        let Some(id) = name.to_str().and_then(|n| n.strip_suffix(".cold")) else {
            continue;
        };
        // A damaged or dictionary-coded file is not a sample. Its real read
        // still reports the damage.
        if let Ok(sample) = read_sample(root, id) {
            samples.push(sample);
        }
    }
    if samples.len() < DICT_SAMPLES {
        return Ok(());
    }
    if let Ok(dict) = zstd::dict::from_samples(&samples, DICT_MAX_BYTES) {
        write_dict(root, &dict)?;
    }
    Ok(())
}

/// The first `SAMPLE_MAX_BYTES` of a plain cold file's updates, for training.
/// It checks the header but reads only the start of the payload, so the
/// payload checksum is not checked. A sample is advisory and never a read result.
fn read_sample(root: &Path, id: &str) -> Result<Vec<u8>> {
    let mut file = BufReader::new(File::open(path(root, id))?);
    let header = read_header(&mut file)?;
    if header.dict.is_some() {
        return Err(invalid("cold file uses a dictionary"));
    }
    let mut sample = Vec::new();
    zstd::stream::Decoder::new(file)?
        .take(SAMPLE_MAX_BYTES)
        .read_to_end(&mut sample)?;
    Ok(sample)
}

fn decode(root: &Path, bytes: &[u8]) -> Result<ColdPart> {
    #[cfg(feature = "storage-metrics")]
    let _d = storage_metrics::span("cold.decode");
    let mut rest = bytes;
    let header = read_header(&mut rest)?;
    // Checked before any blob is decoded or dictionary loaded.
    if rest.len() as u64 != header.payload_len || fnv1a(rest) != header.payload_hash {
        return Err(invalid("cold payload checksum mismatch"));
    }
    let frontiers = Frontiers::decode(&header.frontiers)
        .map_err(|e| invalid(format!("cold frontiers: {e}")))?;
    let vv = VersionVector::decode(&header.vv)
        .map_err(|e| invalid(format!("cold version vector: {e}")))?;
    let dict_bytes = header.dict.map(|id| load_dict(root, id)).transpose()?;
    let updates = decompress(rest, dict_bytes.as_deref(), header.raw_len)?;
    if updates.len() as u64 != header.raw_len || fnv1a(&updates) != header.raw_hash {
        return Err(invalid("cold updates checksum mismatch"));
    }
    Ok(ColdPart {
        root: ColdRoot { frontiers, vv },
        updates,
    })
}

/// The checked fields of a cold header. The blobs are not decoded yet.
struct Header {
    dict: Option<u64>,
    frontiers: Vec<u8>,
    vv: Vec<u8>,
    raw_len: u64,
    raw_hash: u64,
    payload_len: u64,
    payload_hash: u64,
}

/// Reads and checks a header, leaving `r` at the first payload byte. It reads
/// only the header, so a sampler can stop there.
fn read_header<R: Read>(r: &mut R) -> Result<Header> {
    // Every header byte read so far, for the header checksum.
    let mut raw = Vec::new();
    if field(r, &mut raw, MAGIC.len())? != MAGIC {
        return Err(invalid("cold file: bad magic"));
    }
    if field(r, &mut raw, 1)?[0] != VERSION {
        return Err(invalid("cold file: unsupported version"));
    }
    let tag = field(r, &mut raw, 1)?[0];
    let dict_id = u64_field(r, &mut raw)?;
    let dict = match tag {
        TAG_NONE if dict_id == 0 => None,
        TAG_DICT => Some(dict_id),
        _ => return Err(invalid("cold file: bad dictionary field")),
    };
    let frontiers = blob(r, &mut raw)?;
    let vv = blob(r, &mut raw)?;
    let raw_len = u64_field(r, &mut raw)?;
    let raw_hash = u64_field(r, &mut raw)?;
    let payload_len = u64_field(r, &mut raw)?;
    let payload_hash = u64_field(r, &mut raw)?;
    let mut stored = [0u8; 8];
    read_exact_or_truncated(r, &mut stored)?;
    if u64::from_le_bytes(stored) != fnv1a(&raw) {
        return Err(invalid("cold header checksum mismatch"));
    }
    Ok(Header {
        dict,
        frontiers,
        vv,
        raw_len,
        raw_hash,
        payload_len,
        payload_hash,
    })
}

/// Reads `n` header bytes and appends them to `raw`.
fn field<R: Read>(r: &mut R, raw: &mut Vec<u8>, n: usize) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; n];
    read_exact_or_truncated(r, &mut buf)?;
    raw.extend_from_slice(&buf);
    Ok(buf)
}

fn u64_field<R: Read>(r: &mut R, raw: &mut Vec<u8>) -> Result<u64> {
    Ok(u64::from_le_bytes(
        field(r, raw, 8)?.try_into().expect("eight bytes"),
    ))
}

fn blob<R: Read>(r: &mut R, raw: &mut Vec<u8>) -> Result<Vec<u8>> {
    let len = u32::from_le_bytes(field(r, raw, 4)?.try_into().expect("four bytes")) as usize;
    if len > MAX_BLOB {
        return Err(invalid("cold header blob too large"));
    }
    field(r, raw, len)
}

fn read_exact_or_truncated<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<()> {
    r.read_exact(buf).map_err(|e| {
        if e.kind() == ErrorKind::UnexpectedEof {
            invalid("cold file truncated")
        } else {
            e.into()
        }
    })
}

fn push_blob(out: &mut Vec<u8>, blob: &[u8]) -> Result<()> {
    if blob.len() > MAX_BLOB {
        return Err(invalid("cold header blob too large"));
    }
    out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
    out.extend_from_slice(blob);
    Ok(())
}

/// FNV-1a: a fixed checksum with no dependency. Changing any one byte changes it.
fn fnv1a(bytes: &[u8]) -> u64 {
    #[cfg(feature = "storage-metrics")]
    let _h = storage_metrics::span("hash.fnv1a");
    #[cfg(feature = "storage-metrics")]
    storage_metrics::add_bytes("hash.fnv1a", bytes.len() as u64);
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}

fn compress(raw: &[u8], dict: Option<&[u8]>) -> Result<Vec<u8>> {
    let out = match dict {
        Some(d) => {
            #[cfg(feature = "storage-metrics")]
            let _z = storage_metrics::span("zstd.compress_dict");
            let mut enc = zstd::stream::Encoder::with_dictionary(Vec::<u8>::new(), LEVEL, d)?;
            enc.write_all(raw)?;
            let out = enc.finish()?;
            #[cfg(feature = "storage-metrics")]
            storage_metrics::add_bytes("zstd.compress_dict", out.len() as u64);
            out
        }
        None => {
            #[cfg(feature = "storage-metrics")]
            let _z = storage_metrics::span("zstd.compress_plain");
            let mut enc = zstd::stream::Encoder::new(Vec::<u8>::new(), LEVEL)?;
            enc.write_all(raw)?;
            let out = enc.finish()?;
            #[cfg(feature = "storage-metrics")]
            storage_metrics::add_bytes("zstd.compress_plain", out.len() as u64);
            out
        }
    };
    Ok(out)
}

/// Decompresses at most `raw_len + 1` bytes, so a wrong header cannot make it run on.
fn decompress(payload: &[u8], dict: Option<&[u8]>, raw_len: u64) -> Result<Vec<u8>> {
    let limit = raw_len.saturating_add(1);
    let mut out = Vec::new();
    match dict {
        Some(d) => {
            #[cfg(feature = "storage-metrics")]
            let _z = storage_metrics::span("zstd.decompress_dict");
            zstd::stream::Decoder::with_dictionary(payload, d)?
                .take(limit)
                .read_to_end(&mut out)?;
            #[cfg(feature = "storage-metrics")]
            storage_metrics::add_bytes("zstd.decompress_dict", out.len() as u64);
        }
        None => {
            #[cfg(feature = "storage-metrics")]
            let _z = storage_metrics::span("zstd.decompress_plain");
            zstd::stream::Decoder::new(payload)?
                .take(limit)
                .read_to_end(&mut out)?;
            #[cfg(feature = "storage-metrics")]
            storage_metrics::add_bytes("zstd.decompress_plain", out.len() as u64);
        }
    }
    Ok(out)
}

/// File names in `dir`. A missing directory has none.
fn file_names(dir: &Path) -> Result<Vec<String>> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut names = Vec::new();
    for entry in entries {
        if let Ok(name) = entry?.file_name().into_string() {
            names.push(name);
        }
    }
    Ok(names)
}

/// Ids of the stored dictionaries.
fn dict_ids(root: &Path) -> Result<Vec<u64>> {
    Ok(file_names(&dict_dir(root))?
        .iter()
        .filter_map(|n| n.strip_suffix(".dict"))
        .filter(|s| s.len() == 16)
        .filter_map(|s| u64::from_str_radix(s, 16).ok())
        .collect())
}

/// The dictionary new cold files use. Only one should exist; if several do,
/// the lowest id is chosen so the choice stays the same from run to run.
fn current_dict(root: &Path) -> Result<Option<(u64, Vec<u8>)>> {
    match dict_ids(root)?.into_iter().min() {
        Some(id) => Ok(Some((id, load_dict(root, id)?))),
        None => Ok(None),
    }
}

fn load_dict(root: &Path, id: u64) -> Result<Vec<u8>> {
    let bytes = {
        #[cfg(feature = "storage-metrics")]
        let _io = storage_metrics::span("cold.dict_read_io");
        match fs::read(dict_path(root, id)) {
            Ok(b) => b,
            Err(e) if e.kind() == ErrorKind::NotFound => {
                return Err(invalid(format!("cold dictionary {id:016x} is missing")))
            }
            Err(e) => return Err(e.into()),
        }
    };
    #[cfg(feature = "storage-metrics")]
    storage_metrics::add_bytes("cold.dict_read_io", bytes.len() as u64);
    if fnv1a(&bytes) != id {
        return Err(invalid(format!("cold dictionary {id:016x} is damaged")));
    }
    Ok(bytes)
}

/// Stores a dictionary under its hash. A file already there must match, since
/// dictionaries are never changed.
fn write_dict(root: &Path, dict: &[u8]) -> Result<u64> {
    let id = fnv1a(dict);
    let path = dict_path(root, id);
    match fs::read(&path) {
        Ok(existing) => {
            if existing != dict {
                return Err(invalid(format!("cold dictionary {id:016x} is damaged")));
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => write_durable_atomic(&path, dict)?,
        Err(e) => return Err(e.into()),
    }
    Ok(id)
}

/// Writes `bytes` through a synced temp file, renames it over `path` and makes
/// the rename durable. A crash leaves the old file or the new one, never a part.
fn write_durable_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| invalid("cold path has no directory"))?;
    fs::create_dir_all(dir)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let result = (|| -> Result<()> {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        // The data is on disk before the rename can make it the visible file.
        file.sync_all()?;
        drop(file);
        rename_over(&tmp, path)?;
        sync_dir(dir)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Renames `from` over `to`. On Windows the move is written through to disk
/// before this returns. On Unix the caller syncs the directory afterwards.
fn rename_over(from: &Path, to: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        win32::move_file_write_through(from, to)?;
    }
    #[cfg(not(windows))]
    {
        fs::rename(from, to)?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_dir(dir: &Path) -> Result<()> {
    File::open(dir)?.sync_all()?;
    Ok(())
}

/// The directory sync is Unix-only; Windows makes the rename durable itself.
#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> Result<()> {
    Ok(())
}

#[cfg(windows)]
mod win32 {
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }

    /// The path as NUL-terminated UTF-16, the form the Win32 API takes.
    fn wide(p: &Path) -> Vec<u16> {
        p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// Moves `from` over `to` and does not return until the move is written
    /// through to disk. Relies on `FlushFileBuffers` for the file data, which
    /// `File::sync_all` performs before this call.
    pub(super) fn move_file_write_through(from: &Path, to: &Path) -> io::Result<()> {
        let from = wide(from);
        let to = wide(to);
        // SAFETY: both buffers are NUL-terminated and outlive the call.
        let ok = unsafe {
            MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::VaultError;
    use crate::ids::new_id;
    use loro::{ExportMode, LoroDoc};

    /// A temp vault root with its cold folder. Removed on drop.
    struct Root(PathBuf);

    impl Root {
        fn new(tag: &str) -> Root {
            let p = std::env::temp_dir().join(format!("vault_cold_unit_{tag}_{}", new_id()));
            fs::create_dir_all(cold_dir(&p)).unwrap();
            Root(p)
        }
    }

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// One commit of `words` pseudo-random words, from peer `seed`.
    fn doc(seed: u64, words: usize) -> LoroDoc {
        let doc = LoroDoc::new();
        doc.set_peer_id(seed).unwrap();
        let mut x = seed;
        let mut text = String::new();
        for _ in 0..words {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            text.push_str(&format!("w{} ", (x >> 33) % 5000));
        }
        doc.get_text("body").insert(0, &text).unwrap();
        doc.commit();
        doc
    }

    /// The cold part of a whole document: its own frontiers, vv and all updates.
    fn part(doc: &LoroDoc) -> ColdPart {
        ColdPart {
            root: ColdRoot {
                frontiers: doc.oplog_frontiers(),
                vv: doc.oplog_vv(),
            },
            updates: doc.export(ExportMode::all_updates()).unwrap(),
        }
    }

    #[test]
    fn round_trip_keeps_root_and_op_ids_without_dictionary() {
        let root = Root::new("plain");
        let src = doc(7, 200);
        let want = part(&src);
        write(&root.0, "page", &want).unwrap();
        let file = fs::read(path(&root.0, "page")).unwrap();
        assert_eq!(&file[..4], &MAGIC[..]);
        assert_eq!(file[4], VERSION);
        assert_eq!(file[5], TAG_NONE);
        let back = read(&root.0, "page").unwrap();
        assert_eq!(back.updates, want.updates);
        assert_eq!(back.root.frontiers.encode(), want.root.frontiers.encode());
        assert_eq!(back.root.vv.encode(), want.root.vv.encode());
        // The updates alone rebuild the same history with the same op IDs.
        let rebuilt = LoroDoc::new();
        rebuilt.import(&back.updates).unwrap();
        assert_eq!(rebuilt.oplog_vv().encode(), src.oplog_vv().encode());
        assert_eq!(rebuilt.get_deep_value(), src.get_deep_value());
    }

    #[test]
    fn any_flipped_byte_or_truncation_is_rejected() {
        let root = Root::new("damage");
        write(&root.0, "p", &part(&doc(3, 300))).unwrap();
        let file = path(&root.0, "p");
        let good = fs::read(&file).unwrap();
        for i in 0..good.len() {
            let mut bad = good.clone();
            bad[i] ^= 0x01;
            fs::write(&file, &bad).unwrap();
            assert!(read(&root.0, "p").is_err(), "flip at byte {i} was accepted");
        }
        for len in 0..good.len() {
            fs::write(&file, &good[..len]).unwrap();
            assert!(
                read(&root.0, "p").is_err(),
                "truncation to {len} bytes was accepted"
            );
        }
        fs::write(&file, &good).unwrap();
        assert!(read(&root.0, "p").is_ok());
    }

    #[test]
    fn bad_version_and_dictionary_fields_are_rejected_even_when_resealed() {
        let root = Root::new("header");
        write(&root.0, "p", &part(&doc(4, 100))).unwrap();
        let file = path(&root.0, "p");
        let good = fs::read(&file).unwrap();
        let f = u32::from_le_bytes(good[14..18].try_into().unwrap()) as usize;
        let v = u32::from_le_bytes(good[18 + f..22 + f].try_into().unwrap()) as usize;
        // Fixed fields, then 4 x u64 for the lengths and hashes, then the header hash.
        let header_end = 22 + f + v + 32;
        // Re-seal the header after one field changes, so only that field is wrong.
        let reseal = |mut bytes: Vec<u8>| {
            let h = fnv1a(&bytes[..header_end]);
            bytes[header_end..header_end + 8].copy_from_slice(&h.to_le_bytes());
            bytes
        };
        let mut version = good.clone();
        version[4] = 2;
        let mut tag = good.clone();
        tag[5] = 7;
        let mut id_without_tag = good.clone();
        id_without_tag[6] = 1;
        for bad in [reseal(version), reseal(tag), reseal(id_without_tag)] {
            fs::write(&file, &bad).unwrap();
            assert!(read(&root.0, "p").is_err());
        }
        fs::write(&file, &good).unwrap();
        assert!(read(&root.0, "p").is_ok());
    }

    #[test]
    fn rewrite_replaces_the_one_file_and_ignores_a_torn_temp() {
        let root = Root::new("atomic");
        let first = part(&doc(5, 80));
        let second = part(&doc(5, 160));
        write(&root.0, "p", &first).unwrap();
        // A torn temp file from an earlier crash is neither read nor kept.
        let tmp = path(&root.0, "p").with_extension("cold.tmp");
        fs::write(&tmp, b"torn").unwrap();
        assert_eq!(read(&root.0, "p").unwrap().updates, first.updates);
        write(&root.0, "p", &second).unwrap();
        assert_eq!(read(&root.0, "p").unwrap().updates, second.updates);
        let names: Vec<String> = fs::read_dir(cold_dir(&root.0))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        // No dictionary was written, so the page's one file is the only entry.
        assert_eq!(names, vec!["p.cold".to_owned()]);
    }

    #[test]
    fn rename_over_replaces_an_existing_file() {
        let root = Root::new("rename");
        let from = root.0.join("from.tmp");
        let to = root.0.join("to.bin");
        fs::write(&to, b"old").unwrap();
        fs::write(&from, b"new").unwrap();
        rename_over(&from, &to).unwrap();
        assert_eq!(fs::read(&to).unwrap(), b"new");
        assert!(!from.exists());
    }

    #[test]
    fn missing_page_is_not_found() {
        let root = Root::new("missing");
        assert!(matches!(
            read(&root.0, "nope"),
            Err(VaultError::NotFound(_))
        ));
    }

    #[test]
    fn sample_is_capped_while_a_full_read_is_not() {
        let root = Root::new("sample");
        let big = part(&doc(9, 20000));
        assert!(big.updates.len() as u64 > SAMPLE_MAX_BYTES);
        write(&root.0, "big", &big).unwrap();
        let sample = read_sample(&root.0, "big").unwrap();
        assert_eq!(sample.len() as u64, SAMPLE_MAX_BYTES);
        assert_eq!(sample[..], big.updates[..sample.len()]);
        // Reads stay uncapped: the full history comes back.
        assert_eq!(read(&root.0, "big").unwrap().updates, big.updates);
        // A damaged header is not sampled either.
        let mut bytes = fs::read(path(&root.0, "big")).unwrap();
        bytes[0] ^= 0x01;
        fs::write(path(&root.0, "big"), &bytes).unwrap();
        assert!(read_sample(&root.0, "big").is_err());
    }

    #[test]
    fn dictionary_trains_once_after_twenty_files_and_is_required_to_read() {
        let root = Root::new("dict");
        let docs: Vec<ColdPart> = (1..=DICT_SAMPLES as u64)
            .map(|s| part(&doc(s, 1500)))
            .collect();
        for (i, p) in docs.iter().enumerate().take(DICT_SAMPLES - 1) {
            write(&root.0, &format!("p{i}"), p).unwrap();
        }
        maybe_train_dictionary(&root.0).unwrap();
        assert!(
            dict_ids(&root.0).unwrap().is_empty(),
            "19 files are not enough"
        );
        write(&root.0, "p19", &docs[19]).unwrap();
        maybe_train_dictionary(&root.0).unwrap();
        let ids = dict_ids(&root.0).unwrap();
        assert_eq!(ids.len(), 1);
        let dict_file = dict_path(&root.0, ids[0]);
        assert!(dict_file.starts_with(root.0.join(".vault/cold/dictionaries")));
        let dict_bytes = fs::read(&dict_file).unwrap();
        assert!(dict_bytes.len() <= DICT_MAX_BYTES);
        assert_eq!(fnv1a(&dict_bytes), ids[0]);
        // Training again changes nothing, even with more cold files present.
        for i in 0..DICT_SAMPLES {
            write(&root.0, &format!("q{i}"), &docs[i]).unwrap();
        }
        maybe_train_dictionary(&root.0).unwrap();
        assert_eq!(dict_ids(&root.0).unwrap(), ids);
        assert_eq!(fs::read(&dict_file).unwrap(), dict_bytes);
        // New files use the dictionary; the plain files written before still read.
        write(&root.0, "fresh", &docs[0]).unwrap();
        assert_eq!(fs::read(path(&root.0, "fresh")).unwrap()[5], TAG_DICT);
        assert_eq!(read(&root.0, "fresh").unwrap().updates, docs[0].updates);
        assert_eq!(read(&root.0, "p0").unwrap().updates, docs[0].updates);
        // A file that needs the dictionary fails without it, and when it is damaged.
        let good_dict = fs::read(&dict_file).unwrap();
        fs::rename(&dict_file, root.0.join("moved.dict")).unwrap();
        assert!(
            read(&root.0, "fresh").is_err(),
            "missing dictionary was accepted"
        );
        fs::write(&dict_file, b"damaged").unwrap();
        fs::remove_file(root.0.join("moved.dict")).unwrap();
        assert!(
            read(&root.0, "fresh").is_err(),
            "damaged dictionary was accepted"
        );
        fs::write(&dict_file, &good_dict).unwrap();
        assert_eq!(read(&root.0, "fresh").unwrap().updates, docs[0].updates);
    }
}
