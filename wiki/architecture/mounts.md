---
type: topic
layer: state
summary: "Any PC folder can be mounted at a vault path; devices see one logical vault, with no symlinks."
parent: "[[architecture/index]]"
status: current
---

# Folder mounts

## Built (2026-10-01)

The simplest safe version of the design, desktop only. Core: [mounts.rs](../../crates/core/src/mounts.rs); app: Settings → Vault → Mounted folders.

- **What a mount is.** A new page (named after the folder, at the top level) whose children are the folder's top-level `*.md` files, one page each, titled by file name. Subfolders and other files are ignored; pages nested deeper under a mounted page live in the vault only.
- **The mount table** is `.vault/mounts.json` (page id and absolute folder), per device and not synced. Filters, read-only mode, owner and hub replica are not built. The mount page and its pages sync like any other pages; other devices see them as ordinary vault pages.
- **Adding** needs an absolute path to an existing folder outside the vault, not inside or around another mount.
- **Files.** A mounted page is written to `<folder>/<title>.md`, without the vault's own frontmatter keys (`id`, `title`, `parent`, `created`, `updated`, `favorite`, `collection`); the user's other frontmatter (tags, properties) is kept. A new page whose name is taken by another file gets `<title> (2).md`. A renamed page moves to its new name.
- **Outside edits** are read by `rescan_mounts`, which the bridge runs every 2 s on desktop (files whose size and time did not change are skipped). Changed files are merged three-way into their page, as `rescan` does for `pages/`. New files become pages. A page whose file is gone goes to the trash.

### Safety choices

- **Never overwrite a user's change.** The writer writes a file only if its content is what the app last read or wrote (by hash), or the file is new. Otherwise it waits for the rescan, which merges the outside edit with the app's edits (both kept), then writes the merged page.
- **Never delete a user's file.** A page trashed in the app moves its file to `.vault/mount-trash/<page id>.md` only if the file's content came from the app; an outside-edited file stays, and the next rescan imports it as a new page, so both versions are kept. A page moved out of the mount follows the same rule.
- **Unmounting** keeps the pages (their markdown moves to `pages/`) and leaves every file in the folder as it is.
- **An unplugged folder** (missing) is skipped: its pages are not trashed and nothing is written.
- **After a reindex** mounted pages keep the files the old index recorded. With a lost index they have none until a rescan: a file then attaches to a page of the same title with no file (the file's content wins; the page's history keeps the old version), or becomes a new page. Duplicate pages are possible; overwritten files are not.

### Known limits

- Flat only: no subfolders, no filters, no read-only mounts, no mounts on the phone.
- Relative images in mounted files are imported into the blob store; the written file then links the vault's blob instead of the original image.
- Polling, not a file watcher: outside edits show after up to 2 s, and each poll stats every mounted file.

