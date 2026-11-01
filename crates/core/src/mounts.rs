//! Folder mounts (desktop): a folder on this device shown as a page whose
//! child pages are the folder's `.md` files (`wiki/core/mounts.md`).
//!
//! - The mount table is device-local: `.vault/mounts.json`, never synced.
//!   The pages themselves are ordinary vault pages and sync as usual.
//! - A mounted page's markdown file is `<folder>/<title>.md` (its index
//!   `md_path` is that absolute path), written by the writer like the files
//!   in `pages/`, without the vault's frontmatter (`render_plain`).
//! - Only the folder's top level: subfolders and other files are left
//!   alone, and pages below a mounted page live in the vault only.
//! - Outside edits come in through `rescan_mounts` (the app polls it):
//!   changed files are merged into their pages, new files become pages, a
//!   page whose file is gone moves to the trash.
//!
//! Safety: the app never deletes a file in a mounted folder, and writes one
//! only if it is the page's own file, unchanged since the app last read or
//! wrote it (else the edits outside are merged first), or a new file. A
//! page leaving the folder (trash) moves its file to `.vault/mount-trash/`
//! if the app knows its content; a file edited outside meanwhile stays and
//! comes back as a new page, so both versions are kept.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::attach;
use crate::doc::PageMeta;
use crate::error::{invalid, not_found, Result};
use crate::index::{fold, ContentRow, Index, MetaRow};
use crate::markdown::{self, plain_stem, RenderCtx};
use crate::types::{Block, Change, Mount, PageId};
use crate::vault::{lock, remove_if_exists, write_atomic, Inner, Vault};

/// What `rescan_mounts` saw of a file last time: (size, modified ns).
pub(crate) type Seen = HashMap<PathBuf, (u64, u128)>;

fn table_path(root: &Path) -> PathBuf {
    root.join(".vault").join("mounts.json")
}

/// Reads `.vault/mounts.json` (none: no mounts).
pub(crate) fn read_table(root: &Path) -> Vec<Mount> {
    fs::read(table_path(root))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn stamp(p: &Path) -> Option<(u64, u128)> {
    let m = fs::metadata(p).ok()?;
    let t = m
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((m.len(), t))
}

/// `.md` files at the top of `dir` (no hidden ones), sorted.
fn md_files_in(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_lowercase();
            n.ends_with(".md") && !n.starts_with('.')
        })
        .map(|e| e.path())
        .collect();
    v.sort();
    Ok(v)
}

fn same_path(a: &Path, b: &Path) -> bool {
    let n = |p: &Path| fold(&p.to_string_lossy().replace('\\', "/"));
    n(a) == n(b)
}

impl Inner {
    /// The mounted folder whose files the children of `parent` are
    /// (templates excluded).
    pub(crate) fn mount_folder(&self, parent: Option<&str>, is_template: bool) -> Option<PathBuf> {
        let p = parent?;
        if is_template {
            return None;
        }
        lock(&self.mounts)
            .iter()
            .find(|m| m.page == p)
            .map(|m| PathBuf::from(&m.folder))
    }

    pub(crate) fn mount_folder_of(&self, db: &Index, id: &str) -> Option<PathBuf> {
        let (parent, tpl) = db.parent_and_template(id).ok()??;
        self.mount_folder(parent.as_deref(), tpl)
    }

    /// The writer's part for a page in a mounted folder: writes
    /// `<folder>/<title>.md` unless its file was changed outside since the
    /// app last read or wrote it (`rescan_mounts` merges that first) or is
    /// gone (the page goes to the trash). A name taken by another file gets
    /// a number: `<title> (2).md`. A renamed page's old file is removed (its
    /// content is in the new one).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write_mounted(
        &self,
        index: &Index,
        folder: &Path,
        version: &[u8],
        meta: &PageMeta,
        row: &MetaRow,
        blocks: &[Block],
        content: &mut ContentRow,
    ) -> Result<()> {
        if !folder.is_dir() {
            return Ok(());
        }
        let (old_md, old_hash) = index.md(&meta.id)?;
        let old = old_md.as_ref().map(|o| self.pages_dir().join(o));
        if let Some(o) = &old {
            match fs::read(o) {
                Ok(b) => {
                    if old_hash.as_deref() != Some(attach::sha256_hex(&b).as_str()) {
                        return Ok(());
                    }
                }
                Err(_) if o.starts_with(folder) => return Ok(()),
                Err(_) => {}
            }
        }
        let schema = match &meta.parent {
            Some(p) => index.schema_of(p)?,
            None => None,
        };
        let titles = |pid: &str| index.title_of(pid).ok().flatten();
        let ctx = RenderCtx {
            titles: &titles,
            schema: schema.as_deref(),
        };
        let text = markdown::render_plain(meta, &row.props, blocks, &ctx);
        let stem = plain_stem(&meta.title);
        let is_old = |t: &Path| old.as_deref().is_some_and(|o| same_path(o, t));
        let mut target = folder.join(format!("{stem}.md"));
        let mut n = 2;
        while target.exists() && !is_old(&target) {
            target = folder.join(format!("{stem} ({n}).md"));
            n += 1;
        }
        write_atomic(&target, text.as_bytes())?;
        if let Some(o) = &old {
            if !is_old(&target) {
                remove_if_exists(o)?;
            }
        }
        let hash = attach::sha256_hex(text.as_bytes());
        if let Some(s) = stamp(&target) {
            lock(&self.mount_seen).insert(target.clone(), s);
        }
        content.md_hash = Some(hash);
        content.md_path = Some(target.to_string_lossy().into_owned());
        content.md_version = Some(version.to_vec());
        Ok(())
    }

    /// The page's markdown file leaves its place (the page went to the
    /// trash, or was moved out of its mounted folder). A file in `pages/` is deleted; a
    /// file in a mounted folder is moved to `.vault/mount-trash/<id>.md` if
    /// its content is what the app last read or wrote (`hash`), else left
    /// where it is.
    pub(crate) fn drop_md(&self, id: &str, md: &str, hash: Option<&str>) -> Result<()> {
        let path = self.pages_dir().join(md);
        if !Path::new(md).is_absolute() {
            return remove_if_exists(&path);
        }
        let Ok(bytes) = fs::read(&path) else {
            return Ok(());
        };
        if hash != Some(attach::sha256_hex(&bytes).as_str()) {
            return Ok(());
        }
        let dest = self.root.join(".vault").join("mount-trash");
        fs::create_dir_all(&dest)?;
        let dest = dest.join(format!("{id}.md"));
        if fs::rename(&path, &dest).is_err() {
            // Another drive: copy, then remove the original.
            write_atomic(&dest, &bytes)?;
            remove_if_exists(&path)?;
        }
        Ok(())
    }
}

impl Vault {
    /// The folders mounted on this device.
    pub fn list_mounts(&self) -> Vec<Mount> {
        lock(&self.inner.mounts).clone()
    }

    /// Mounts `folder` (an absolute path to an existing folder outside the
    /// vault, not inside or around another mount) as a new page under
    /// `parent`, named after the folder, whose children are the folder's
    /// `.md` files. Returns the mount.
    pub fn add_mount(&self, folder: String, parent: Option<PageId>) -> Result<Mount> {
        let path = PathBuf::from(folder.trim());
        if !path.is_absolute() {
            return Err(invalid("the folder must be an absolute path"));
        }
        if !path.is_dir() {
            return Err(not_found("folder", &path.to_string_lossy()));
        }
        let path = fs::canonicalize(&path)
            .map(|p| PathBuf::from(p.to_string_lossy().trim_start_matches(r"\\?\")))
            .unwrap_or(path);
        let root = fs::canonicalize(&self.inner.root).unwrap_or(self.inner.root.clone());
        let root = PathBuf::from(root.to_string_lossy().trim_start_matches(r"\\?\"));
        let inside = |a: &Path, b: &Path| {
            let (a, b) = (
                fold(&a.to_string_lossy().replace('\\', "/")),
                fold(&b.to_string_lossy().replace('\\', "/")),
            );
            a == b || a.starts_with(&format!("{}/", b.trim_end_matches('/')))
        };
        if inside(&path, &root) || inside(&root, &path) {
            return Err(invalid(
                "the folder cannot be inside the vault or contain it",
            ));
        }
        for m in lock(&self.inner.mounts).iter() {
            let f = Path::new(&m.folder);
            if inside(&path, f) || inside(f, &path) {
                return Err(invalid(format!("{} is already mounted", m.folder)));
            }
        }
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let page = self.create_page(parent, title)?;
        let mount = Mount {
            page,
            folder: path.to_string_lossy().into_owned(),
        };
        {
            let mut t = lock(&self.inner.mounts);
            t.push(mount.clone());
            write_atomic(
                &table_path(&self.inner.root),
                &serde_json::to_vec_pretty(&*t).map_err(|e| invalid(e.to_string()))?,
            )?;
        }
        self.rescan_mounts()?;
        Ok(mount)
    }

    /// Unmounts the folder of mount page `page`. The pages stay in the vault
    /// (their markdown moves to `pages/`); the folder's files are left as
    /// they are.
    pub fn remove_mount(&self, page: PageId) -> Result<()> {
        // Pending edits go to the folder first. The table changes and the
        // pages lose their files under the index lock, which the writer
        // holds too: it never sees a page out of its mount with a file
        // still in the folder (it would move the file away).
        self.flush()?;
        let db = lock(&self.inner.db);
        let folder = {
            let mut t = lock(&self.inner.mounts);
            let i = t
                .iter()
                .position(|m| m.page == page)
                .ok_or_else(|| not_found("mount", &page))?;
            let m = t.remove(i);
            write_atomic(
                &table_path(&self.inner.root),
                &serde_json::to_vec_pretty(&*t).map_err(|e| invalid(e.to_string()))?,
            )?;
            m.folder
        };
        lock(&self.inner.mount_seen).retain(|p, _| !p.starts_with(&folder));
        let mut moved = Vec::new();
        for id in db.child_ids(&page)? {
            let (md, _) = db.md(&id)?;
            if md.as_deref().is_some_and(|m| Path::new(m).is_absolute()) {
                db.clear_md(&id)?;
                moved.push(id);
            }
        }
        drop(db);
        for id in moved {
            self.inner.rerender(&id);
        }
        Ok(())
    }

    /// Reads the mounted folders: changed files are merged into their pages
    /// (as `rescan` does for `pages/`), new files become pages, and pages
    /// whose file is gone move to the trash. Files whose size and time did
    /// not change since the last call are not read. A folder that is not
    /// there (an unplugged drive) is skipped. Returns the number of pages
    /// changed, created or trashed.
    pub fn rescan_mounts(&self) -> Result<u32> {
        let mounts = self.list_mounts();
        if mounts.is_empty() {
            return Ok(0);
        }
        self.flush()?;
        let mut changed = 0;
        let mut gone: Vec<PageId> = Vec::new();
        let db = lock(&self.inner.db);
        for m in &mounts {
            let dir = PathBuf::from(&m.folder);
            if !dir.is_dir() || db.in_trash(&m.page)? != Some(false) {
                continue;
            }
            let files = md_files_in(&dir)?;
            // This mount's pages with a file: path -> (id, hash).
            let mut known: Vec<(PathBuf, String, Option<String>)> = Vec::new();
            let mut unfiled: Vec<(String, String)> = Vec::new();
            for id in db.child_ids(&m.page)? {
                if db.in_trash(&id)? != Some(false)
                    || self.inner.mount_folder_of(&db, &id).is_none()
                {
                    continue;
                }
                match db.md(&id)? {
                    (Some(p), h) if Path::new(&p).is_absolute() => {
                        known.push((PathBuf::from(p), id, h))
                    }
                    _ => unfiled.push((id.clone(), db.title_of(&id)?.unwrap_or_default())),
                }
            }
            let mut seen_ids: HashSet<String> = HashSet::new();
            for f in &files {
                let st = stamp(f);
                let k = known.iter().find(|(p, _, _)| same_path(p, f));
                let unchanged_stamp =
                    st.is_some() && lock(&self.inner.mount_seen).get(f) == st.as_ref();
                if let Some((_, id, _)) = k {
                    seen_ids.insert(id.clone());
                    if unchanged_stamp {
                        continue;
                    }
                }
                let bytes = fs::read(f)?;
                let hash = attach::sha256_hex(&bytes);
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let name = f.to_string_lossy().into_owned();
                match k {
                    Some((_, id, h)) => {
                        if h.as_deref() != Some(hash.as_str())
                            && self.update_from_md(&db, id, &text, &name, &hash, &dir)?
                        {
                            changed += 1;
                        }
                    }
                    None => {
                        let stem = f
                            .file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        // A page of the mount without a file whose title
                        // gives this name (after a reindex): the file is its.
                        let owner = unfiled
                            .iter()
                            .position(|(_, t)| fold(&plain_stem(t)) == fold(&stem));
                        match owner {
                            Some(i) => {
                                let (id, _) = unfiled.remove(i);
                                seen_ids.insert(id.clone());
                                self.update_from_md(&db, &id, &text, &name, &hash, &dir)?;
                            }
                            None => {
                                self.page_from_md(
                                    &db,
                                    &text,
                                    &name,
                                    &hash,
                                    Some((&m.page, &stem)),
                                    &dir,
                                )?;
                            }
                        }
                        changed += 1;
                    }
                }
                if let Some(s) = st {
                    lock(&self.inner.mount_seen).insert(f.clone(), s);
                }
            }
            for (p, id, _) in &known {
                if !seen_ids.contains(id) && !p.exists() {
                    gone.push(id.clone());
                }
            }
            // Pages without a file yet (new, or after a reindex): written.
            for (id, _) in &unfiled {
                self.inner.rerender(id);
            }
        }
        drop(db);
        for id in gone {
            self.trash_page(id)?;
            changed += 1;
        }
        if changed > 0 {
            self.inner.emit([Change::TreeChanged]);
        }
        Ok(changed)
    }
}
