//! `NotesVault`: every `vault_core::Vault` method, for Dart. Calls are async
//! on the Dart side: flutter_rust_bridge runs them on its Rust thread pool,
//! so nothing here blocks the UI isolate. Only type conversion happens here
//! (see `crate::convert`).
//!
//! One `NotesVault` per folder at a time (the core's rule): dispose the old
//! one before opening the same folder again, and before seeding it.

use std::sync::Arc;
use std::time::Instant;

use flutter_rust_bridge::frb;
use vault_core::Vault;

use crate::api::agent::{AgentSchedulePreview, AgentScheduleRun, AgentScheduleState};
use crate::api::types::*;
use crate::convert::*;
use crate::frb_generated::StreamSink;
use crate::local_runner::LocalRunner;

/// The vault's synced agent defaults (integration spec, phase 3). Runners
/// are `hub` and `this_device`; a device override lives in the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultAgentDefaults {
    pub runner: Option<String>,
    pub models: Vec<VaultAgentModel>,
}

/// One runner's default `provider/model`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultAgentModel {
    pub runner: String,
    pub model: String,
}

/// A search hit's identity, including the source that owns its local id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HitKey {
    pub source: String,
    pub id: String,
}

/// Places a hit at its zero-based position in the updated result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HitMove {
    pub key: HitKey,
    pub new_index: u32,
}

/// The initial result, then nonempty changes after relevant index commits.
/// Remove keys, append additions, then place all moves at their target indices
/// simultaneously. Replace hit data from `changed`. Initial additions are ordered.
#[derive(Debug, Clone)]
pub struct LiveQueryUpdate {
    /// Pass this to `unwatch_query` to finish this stream.
    pub handle: u64,
    pub generation: u64,
    pub initial: bool,
    pub added: Vec<Hit>,
    pub removed: Vec<HitKey>,
    pub moved: Vec<HitMove>,
    pub changed: Vec<Hit>,
    /// Saturates at 201, displayed as "200+".
    pub total: u32,
}

fn live_hit_key_from(key: vault_core::HitKey) -> HitKey {
    HitKey {
        source: key.source,
        id: key.id,
    }
}

fn live_query_update_from(handle: u64, update: vault_core::LiveQueryUpdate) -> LiveQueryUpdate {
    LiveQueryUpdate {
        handle,
        generation: update.generation,
        initial: update.initial,
        added: update.added.into_iter().map(hit_from).collect(),
        removed: update.removed.into_iter().map(live_hit_key_from).collect(),
        moved: update
            .moved
            .into_iter()
            .map(|(key, new_index)| HitMove {
                key: live_hit_key_from(key),
                new_index,
            })
            .collect(),
        changed: update.changed.into_iter().map(hit_from).collect(),
        total: update.total_bounded,
    }
}

#[cfg(test)]
mod live_query_conversion_tests {
    use super::*;

    #[test]
    fn legacy_polling_ignores_index_only_writes_and_preserves_page_events() {
        struct TempRoot(std::path::PathBuf);
        impl Drop for TempRoot {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let root = TempRoot(
            std::env::temp_dir().join(format!("notes_bridge_live_{}", vault_core::new_id())),
        );
        let vault = NotesVault {
            inner: Arc::new(Vault::open(root.0.to_string_lossy().into_owned()).unwrap()),
            runner: LocalRunner::Unconfigured,
        };
        let before = vault.poll_changes(0);
        vault
            .inner
            .upsert_search_row(
                "chat".into(),
                vault_core::ExternalSearchRow {
                    id: "external".into(),
                    title: "Chat".into(),
                    body: "summary".into(),
                    page_type: "chat".into(),
                    target: None,
                    updated_ms: 1,
                },
            )
            .unwrap();
        let after = vault.poll_changes(before.seq);
        assert_eq!(after.seq, before.seq);
        assert!(after.changes.is_empty() && !after.reset);
        let started = Instant::now();
        assert!(vault.wait_changes(before.seq, 30).changes.is_empty());
        assert!(started.elapsed() >= std::time::Duration::from_millis(25));
        let page = vault.inner.create_page(None, "Page".into()).unwrap();
        let after = vault.poll_changes(before.seq);
        assert!(after.seq > before.seq);
        assert!(after
            .changes
            .iter()
            .any(|change| change.kind == ChangeKind::PageCreated
                && change.page.as_ref() == Some(&page)));
        assert!(after
            .changes
            .iter()
            .all(|change| change.kind != ChangeKind::IndexCommitted));
    }

    #[test]
    fn source_local_keys_and_commit_metadata_cross_the_bridge() {
        let key = vault_core::HitKey {
            source: "chat".into(),
            id: "same".into(),
        };
        let update = live_query_update_from(
            42,
            vault_core::LiveQueryUpdate {
                generation: 7,
                initial: false,
                added: vec![],
                removed: vec![key.clone()],
                moved: vec![(key, 3)],
                changed: vec![],
                total_bounded: 201,
            },
        );
        assert_eq!(update.handle, 42);
        assert_eq!(update.generation, 7);
        assert_eq!(update.total, 201);
        assert_eq!(
            update.removed[0],
            HitKey {
                source: "chat".into(),
                id: "same".into()
            }
        );
        assert_eq!(update.moved[0].key, update.removed[0]);
        assert_eq!(update.moved[0].new_index, 3);

        let batch = change_batch(vault_core::ChangeBatch {
            seq: 1,
            reset: false,
            changes: vec![vault_core::Change::IndexCommitted {
                generation: 7,
                pages: vec!["same".into()],
            }],
        });
        assert_eq!(batch.changes[0].kind, ChangeKind::IndexCommitted);
        assert_eq!(batch.changes[0].generation, Some(7));
        assert_eq!(batch.changes[0].pages, vec!["same"]);
        assert_eq!(batch.changes[0].page, None);
    }
}

/// An open vault. Cheap to share; the Dart object holds an `Arc`.
#[frb(opaque)]
pub struct NotesVault {
    pub(crate) inner: Arc<Vault>,
    /// This vault's local schedule runner, the only desktop schedule timer. Its
    /// timer stops when the vault is dropped, whether or not an agent is open.
    pub(crate) runner: LocalRunner,
}

#[cfg(test)]
mod date_mark_tests {
    use super::*;
    use vault_core as vc;

    #[test]
    fn date_mark_reference_crosses_bridge_without_offset_conversion() {
        struct TempRoot(std::path::PathBuf);
        impl Drop for TempRoot {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let root =
            TempRoot(std::env::temp_dir().join(format!("notes_bridge_date_mark_{}", vc::new_id())));
        let vault = NotesVault {
            inner: Arc::new(Vault::open(root.0.to_string_lossy().into_owned()).unwrap()),
            runner: LocalRunner::Unconfigured,
        };
        let page = vault.inner.create_page(None, "Plan".into()).unwrap();
        vault
            .inner
            .apply(
                page.clone(),
                vec![vc::BlockOp::Insert {
                    parent: None,
                    after: None,
                    block: vc::NewBlock {
                        id: "todo".into(),
                        kind: vc::BlockKind::Todo,
                        text: "🐱 @date".into(),
                        marks: vec![vc::Mark {
                            start: 2,
                            end: 7,
                            kind: vc::MarkKind::Date("2026-10-06T09:30Z".into()),
                        }],
                        checked: false,
                    },
                }],
            )
            .unwrap();
        vault.inner.flush().unwrap();
        let day = 1_791_244_800_000;
        let day_ms = 86_400_000;
        let item = vault
            .calendar_items(day, day + day_ms, day, day + day_ms)
            .unwrap()
            .into_iter()
            .find(|item| item.block.as_deref() == Some("todo"))
            .unwrap();
        let reference = item.date_mark.unwrap();
        // Ordinary BMark offsets are UTF-16. This opaque token stays in the
        // core's scalar units even when non-BMP text precedes the mark.
        assert_eq!((reference.start, reference.end), (2, 7));
        vault
            .move_date_mark(page.clone(), "todo".into(), reference.clone(), day + day_ms)
            .unwrap();
        vault.inner.flush().unwrap();
        let before = vault.inner.load_page(page.clone()).unwrap().blocks;
        let error = vault
            .move_date_mark(page.clone(), "todo".into(), reference, day + 2 * day_ms)
            .unwrap_err();
        assert!(error.contains("stale date mark reference"), "{error}");
        assert_eq!(vault.inner.load_page(page).unwrap().blocks, before);
        let item = vault
            .calendar_items(
                day + day_ms,
                day + 2 * day_ms,
                day + day_ms,
                day + 2 * day_ms,
            )
            .unwrap()
            .into_iter()
            .find(|item| item.block.as_deref() == Some("todo"))
            .unwrap();
        assert_eq!(item.date_mark.unwrap().iso, "2026-10-07T09:30Z");
    }
}

impl NotesVault {
    /// Opens the vault at `root`, creating the layout if missing.
    /// On desktop, a background thread reads outside changes in mounted
    /// folders every 2 seconds while the vault is open.
    ///
    /// `data_dir` is this device's app data folder (the schedule state lives in
    /// `<data_dir>/agent/schedules.json`). With it, a desktop vault starts its local
    /// schedule runner, off until `local_runner_set_available(true)`; a startup
    /// failure is reported by the local controls and never fails the open. Without
    /// it, the vault has no local runner. Neither depends on an agent.
    pub fn open(root: String, data_dir: Option<String>) -> Result<NotesVault, String> {
        let inner = Arc::new(Vault::open(root).map_err(err)?);
        if cfg!(any(
            target_os = "windows",
            target_os = "linux",
            target_os = "macos"
        )) {
            poll_mounts(Arc::downgrade(&inner));
        }
        let runner = LocalRunner::open(inner.clone(), data_dir);
        Ok(NotesVault { inner, runner })
    }

    /// Fills an empty, closed vault folder with `pages` test pages plus a
    /// "Big page" of `big_page_blocks` blocks (the bench vault).
    pub fn seed(root: String, pages: u32, big_page_blocks: u32) -> Result<(), String> {
        vault_core::seed::generate(root, pages, big_page_blocks).map_err(err)
    }

    /// Adds a "Tasks" collection with `rows` rows to a closed vault folder.
    pub fn seed_tasks(root: String, rows: u32) -> Result<String, String> {
        vault_core::seed::generate_tasks(root, rows).map_err(err)
    }

    /// Seeding progress: page documents written so far in `root`.
    #[frb(sync)]
    pub fn seed_progress(root: String) -> u32 {
        let dir = std::path::Path::new(&root).join(".vault").join("docs");
        std::fs::read_dir(dir)
            .map(|d| {
                d.filter(|e| {
                    e.as_ref()
                        .map(|e| e.file_name().to_string_lossy().ends_with(".loro"))
                        .unwrap_or(false)
                })
                .count() as u32
            })
            .unwrap_or(0)
    }

    // ------------------------------------------------------------ basics

    #[frb(sync)]
    pub fn root(&self) -> String {
        self.inner.root()
    }

    #[frb(sync)]
    pub fn device_id(&self) -> String {
        self.inner.device_id()
    }

    // ------------------------------------------------------ local schedules

    /// This device's existing node record id, or null before the record exists
    /// and on platforms without a local runner.
    pub fn local_node_id(&self) -> Option<String> {
        self.runner.node_id(&self.inner)
    }

    /// Turns this device's schedule runner on or off. On starts its one timer, whose
    /// first tick runs at once. Off stops the timer and publishes the off state.
    /// Repeated calls never start a second timer.
    pub fn local_runner_set_available(&self, available: bool) -> Result<(), String> {
        self.runner.set_available(available)
    }

    /// The schedule's state on this device. Fails unless this device runs it.
    pub fn local_schedule_state(&self, page_id: String) -> Result<AgentScheduleState, String> {
        self.runner.state(&page_id)
    }

    /// Accepts the schedule on this device for `hash` (from `schedule_preview` or
    /// `local_schedule_state`). Fails if the definition changed since.
    pub fn local_schedule_accept(
        &self,
        page_id: String,
        hash: String,
    ) -> Result<AgentScheduleState, String> {
        self.runner.accept(&page_id, &hash)
    }

    /// Pauses or resumes an accepted schedule on this device.
    pub fn local_schedule_pause(
        &self,
        page_id: String,
        paused: bool,
    ) -> Result<AgentScheduleState, String> {
        self.runner.pause(&page_id, paused)
    }

    /// Runs the schedule's occurrence now, once, and records the run.
    pub fn local_schedule_run_now(&self, page_id: String) -> Result<AgentScheduleRun, String> {
        self.runner.run_now(&page_id)
    }

    /// Checks a schedule definition (JSON text) and lists its next occurrences.
    /// Saves nothing. Works on every platform, with or without a data folder.
    pub fn schedule_preview(&self, json: String) -> AgentSchedulePreview {
        crate::schedule_format::format::preview(&json, jiff::Timestamp::now()).into()
    }

    pub fn page_count(&self) -> Result<u32, String> {
        self.inner.page_count().map_err(err)
    }

    /// Pages by last update (newest first); `parent` None = all pages.
    pub fn list_pages(
        &self,
        parent: Option<String>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<PageRow>, String> {
        Ok(rows(
            self.inner.list_pages(parent, offset, limit).map_err(err)?,
        ))
    }

    /// Page-only search for existing page pickers and inline home search.
    /// `search_adaptive` and `search_spec` include external sources.
    pub fn search(&self, query: String, limit: u32) -> Result<Vec<Hit>, String> {
        Ok(self
            .inner
            .search_pages(query, limit)
            .map_err(err)?
            .into_iter()
            .map(hit_from)
            .collect())
    }

    /// Adds or replaces a row of an external search source. Its id is local
    /// to `source` and may equal a page id. External rows are searched and
    /// filtered with pages but are never pages: no listing or page change sees them.
    pub fn upsert_search_row(&self, source: String, row: ExternalSearchRow) -> Result<(), String> {
        self.inner
            .upsert_search_row(source, external_row_from(row))
            .map_err(err)
    }

    /// Removes one external row; a missing row is not an error.
    pub fn delete_search_row(&self, source: String, id: String) -> Result<(), String> {
        self.inner.delete_search_row(source, id).map_err(err)
    }

    /// Removes every external row of `source`. Other sources and all pages stay.
    pub fn clear_search_source(&self, source: String) -> Result<(), String> {
        self.inner.clear_search_source(source).map_err(err)
    }

    /// Exact external-note metadata lookup; works even if its add-on is off.
    pub fn find_indexed_note(&self, id: String) -> Result<Option<Hit>, String> {
        self.inner
            .find_indexed_note(id)
            .map(|hit| hit.map(hit_from))
            .map_err(err)
    }

    /// Adaptive typing search, including status even when no titles match.
    pub fn search_adaptive(&self, query: String, limit: u32) -> Result<SearchResults, String> {
        let results = self.inner.search_adaptive(query, limit).map_err(err)?;
        Ok(SearchResults {
            hits: results.hits.into_iter().map(hit_from).collect(),
            narrowed: results.narrowed,
        })
    }

    /// Reads a query string into its spec and the byte span of each token
    /// (text, phrase, exclude, operator, error). Cheap enough per keystroke.
    pub fn parse_search(&self, q: String) -> Result<ParsedQuery, String> {
        Ok(parsed_query_from(self.inner.parse_search(q).map_err(err)?))
    }

    /// Completions for the byte `cursor` in `q`: keys, values, page titles
    /// and date words. Each replaces its own byte range.
    pub fn complete_search(&self, q: String, cursor: u32) -> Result<Vec<SearchCompletion>, String> {
        Ok(self
            .inner
            .complete_search(q, cursor)
            .map_err(err)?
            .into_iter()
            .map(completion_from)
            .collect())
    }

    /// Runs a spec (from `parse_search`, or edited chips) with paging.
    pub fn search_spec(
        &self,
        spec: SearchSpec,
        limit: u32,
        offset: u32,
    ) -> Result<SearchSpecResults, String> {
        let spec = spec_to(spec)?;
        Ok(search_results_from(
            self.inner.search_spec(spec, limit, offset).map_err(err)?,
        ))
    }

    /// Streams the initial snapshot and subsequent nonempty query diffs.
    /// Call `unwatch_query` with the initial update's handle before cancelling
    /// an idle Dart stream. A closed Dart port is detected on the next send.
    pub fn watch_query(
        &self,
        query: String,
        limit: u32,
        sink: StreamSink<LiveQueryUpdate>,
    ) -> Result<(), String> {
        let spec = self.inner.parse_search(query).map_err(err)?.spec;
        // The vault registry owns the sink. There is no forwarding thread,
        // pending bridge call or strong Vault reference to keep an idle
        // subscription alive after its vault is disposed.
        self.inner
            .watch_query_with(spec, limit, move |handle, update| {
                sink.add(live_query_update_from(handle, update)).is_ok()
            })
            .map(|_| ())
            .map_err(err)
    }

    /// Removes a registration and finishes its stream, including while idle.
    pub fn unwatch_query(&self, handle: u64) {
        self.inner.unwatch_query(handle);
    }

    pub fn create_page(
        &self,
        parent: Option<String>,
        title: String,
        page_type: Option<String>,
    ) -> Result<String, String> {
        self.inner
            .create_page_with_type(parent, title, page_type)
            .map_err(err)
    }

    pub fn set_page_type(&self, page: String, page_type: String) -> Result<(), String> {
        self.inner.set_page_type(page, page_type).map_err(err)
    }

    /// JSON object for one add-on; an absent area is `{}`.
    pub fn type_data(&self, page: String, addon: String) -> Result<String, String> {
        self.inner.type_data(page, addon).map_err(err)
    }

    /// JSON object containing every add-on's area, including unknown ids.
    pub fn type_data_all(&self, page: String) -> Result<String, String> {
        self.inner.type_data_all(page).map_err(err)
    }

    /// The ready-made readable Markdown body stored in the type-data root.
    pub fn type_markdown_body(&self, page: String) -> Result<Option<String>, String> {
        self.inner.type_markdown_body(page).map_err(err)
    }

    /// Stores a readable body, or removes it when `body` is absent.
    pub fn set_type_markdown_body(&self, page: String, body: Option<String>) -> Result<(), String> {
        self.inner.set_type_markdown_body(page, body).map_err(err)
    }

    /// Uses the vault writer's Markdown renderer, including front matter.
    pub fn export_markdown(&self, page: String) -> Result<String, String> {
        self.inner.export_markdown(page).map_err(err)
    }

    pub fn type_data_set(
        &self,
        page: String,
        addon: String,
        path: Vec<String>,
        json_value: String,
    ) -> Result<(), String> {
        self.inner
            .type_data_set(page, addon, path, json_value)
            .map_err(err)
    }

    pub fn type_data_delete(
        &self,
        page: String,
        addon: String,
        path: Vec<String>,
    ) -> Result<(), String> {
        self.inner.type_data_delete(page, addon, path).map_err(err)
    }

    /// Inserts at `index`, or appends when it is absent.
    pub fn type_data_list_insert(
        &self,
        page: String,
        addon: String,
        path: Vec<String>,
        index: Option<u32>,
        json_value: String,
    ) -> Result<(), String> {
        self.inner
            .type_data_list_insert(page, addon, path, index, json_value)
            .map_err(err)
    }

    pub fn type_data_list_delete(
        &self,
        page: String,
        addon: String,
        path: Vec<String>,
        item: TypeDataListItem,
    ) -> Result<(), String> {
        self.inner
            .type_data_list_delete(page, addon, path, type_data_list_item(item)?)
            .map_err(err)
    }

    /// A move is a delete followed by an insert at the resulting list index.
    pub fn type_data_list_move(
        &self,
        page: String,
        addon: String,
        path: Vec<String>,
        item: TypeDataListItem,
        to_index: u32,
    ) -> Result<(), String> {
        self.inner
            .type_data_list_move(page, addon, path, type_data_list_item(item)?, to_index)
            .map_err(err)
    }

    pub fn rename_page(&self, id: String, title: String) -> Result<(), String> {
        self.inner.rename_page(id, title).map_err(err)
    }

    /// Moves the page and its subtree to the trash.
    pub fn delete_page(&self, id: String) -> Result<(), String> {
        self.inner.delete_page(id).map_err(err)
    }

    pub fn load_page(&self, id: String) -> Result<PageData, String> {
        let t = Instant::now();
        let p = self.inner.load_page(id).map_err(err)?;
        let mut d = page_data(p, 0);
        d.load_us = t.elapsed().as_micros() as u32;
        Ok(d)
    }

    /// One transaction: all ops or none.
    pub fn apply(&self, page_id: String, ops: Vec<BOp>) -> Result<(), String> {
        let ops = ops
            .into_iter()
            .map(block_op)
            .collect::<Result<Vec<_>, _>>()?;
        self.inner.apply(page_id, ops).map_err(err)
    }

    /// Waits for the background writer (call before the app is suspended).
    pub fn flush(&self) -> Result<(), String> {
        self.inner.flush().map_err(err)
    }

    pub fn reindex(&self) -> Result<u32, String> {
        self.inner.reindex().map_err(err)
    }

    // --------------------------------------------------------- page tree

    /// Children of `parent` (None = top level) in sibling order.
    pub fn list_children(
        &self,
        parent: Option<String>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<PageRow>, String> {
        Ok(rows(
            self.inner
                .list_children(parent, offset, limit)
                .map_err(err)?,
        ))
    }

    pub fn move_page(
        &self,
        id: String,
        new_parent: Option<String>,
        after: Option<String>,
    ) -> Result<(), String> {
        self.inner.move_page(id, new_parent, after).map_err(err)
    }

    pub fn duplicate_page(&self, id: String) -> Result<String, String> {
        self.inner.duplicate_page(id).map_err(err)
    }

    pub fn restore_page(&self, id: String) -> Result<(), String> {
        self.inner.restore_page(id).map_err(err)
    }

    pub fn list_trash(&self) -> Result<Vec<TrashedPage>, String> {
        Ok(self
            .inner
            .list_trash()
            .map_err(err)?
            .into_iter()
            .map(|t| TrashedPage {
                page: page_row(t.page),
                trashed_ms: t.trashed_ms,
            })
            .collect())
    }

    pub fn purge_page(&self, id: String) -> Result<(), String> {
        self.inner.purge_page(id).map_err(err)
    }

    pub fn empty_trash(&self) -> Result<u32, String> {
        self.inner.empty_trash().map_err(err)
    }

    pub fn set_favorite(&self, id: String, favorite: bool) -> Result<(), String> {
        self.inner.set_favorite(id, favorite).map_err(err)
    }

    pub fn list_favorites(&self) -> Result<Vec<PageRow>, String> {
        Ok(rows(self.inner.list_favorites().map_err(err)?))
    }

    pub fn set_icon(&self, id: String, icon: Option<String>) -> Result<(), String> {
        self.inner.set_icon(id, icon).map_err(err)
    }

    pub fn set_cover(&self, id: String, cover: Option<String>) -> Result<(), String> {
        self.inner.set_cover(id, cover).map_err(err)
    }

    /// Small text, full width, lock and cover position of a page.
    pub fn set_page_prefs(&self, id: String, prefs: PagePrefsData) -> Result<(), String> {
        let p = vault_core::PagePrefs {
            small_text: prefs.small_text,
            full_width: prefs.full_width,
            locked: prefs.locked,
            cover_position: prefs.cover_position,
        };
        self.inner.set_page_prefs(id, p).map_err(err)
    }

    pub fn set_page_color(&self, page: String, color: Option<String>) -> Result<(), String> {
        self.inner.set_page_color(page, color).map_err(err)
    }

    pub fn set_page_role(&self, page: String, role: Option<String>) -> Result<(), String> {
        self.inner.set_page_role(page, role).map_err(err)
    }

    /// The hidden Feedback page, created on first use.
    pub fn feedback_page(&self) -> Result<String, String> {
        self.inner.feedback_page().map_err(err)
    }

    /// Finds a live page by its stable role, independent of its title.
    pub fn find_by_role(&self, role: String) -> Result<Option<String>, String> {
        self.inner.find_by_role(role).map_err(err)
    }

    /// Assigns colours to top-level pages that do not have one yet.
    pub fn ensure_space_colors(&self) -> Result<u32, String> {
        self.inner.ensure_space_colors().map_err(err)
    }

    // ------------------------------------------------------------- agenda

    pub fn set_agenda_settings(
        &self,
        collection: String,
        settings: AgendaSettings,
    ) -> Result<(), String> {
        self.inner
            .set_agenda_settings(collection, agenda_settings_to(settings))
            .map_err(err)
    }

    pub fn agenda_settings(&self, collection: String) -> Result<AgendaSettings, String> {
        Ok(agenda_settings_from(
            self.inner.agenda_settings(collection).map_err(err)?,
        ))
    }

    /// Timed bounds are local instants; whole-day bounds are UTC midnights.
    pub fn agenda(
        &self,
        timed_from_ms: i64,
        timed_to_ms: i64,
        day_from_ms: i64,
        day_to_ms: i64,
    ) -> Result<Vec<AgendaItem>, String> {
        Ok(self
            .inner
            .agenda(timed_from_ms, timed_to_ms, day_from_ms, day_to_ms)
            .map_err(err)?
            .into_iter()
            .map(agenda_item)
            .collect())
    }

    /// Every dated source, including mentions. Bounds match `agenda`.
    pub fn calendar_items(
        &self,
        timed_from_ms: i64,
        timed_to_ms: i64,
        day_from_ms: i64,
        day_to_ms: i64,
    ) -> Result<Vec<AgendaItem>, String> {
        Ok(self
            .inner
            .calendar_items(timed_from_ms, timed_to_ms, day_from_ms, day_to_ms)
            .map_err(err)?
            .into_iter()
            .map(agenda_item)
            .collect())
    }

    /// Moves only the referenced to-do date mark's day, retaining its time.
    /// A stale reference is rejected without editing the block.
    pub fn move_date_mark(
        &self,
        page: String,
        block: String,
        mark_ref: DateMarkRef,
        new_day_ms: i64,
    ) -> Result<(), String> {
        self.inner
            .move_date_mark(
                page,
                block,
                vault_core::DateMarkRef {
                    start: mark_ref.start,
                    end: mark_ref.end,
                    iso: mark_ref.iso,
                },
                new_day_ms,
            )
            .map_err(err)
    }

    pub fn overdue(
        &self,
        timed_before_ms: i64,
        day_before_ms: i64,
        limit: u32,
    ) -> Result<Vec<AgendaItem>, String> {
        Ok(self
            .inner
            .overdue(timed_before_ms, day_before_ms, limit)
            .map_err(err)?
            .into_iter()
            .map(agenda_item)
            .collect())
    }

    /// Done dated tasks on pages edited since the supplied instant.
    pub fn done_since(
        &self,
        timed_before_ms: i64,
        day_before_ms: i64,
        edited_since_ms: i64,
        limit: u32,
    ) -> Result<Vec<AgendaItem>, String> {
        Ok(self
            .inner
            .done_since(timed_before_ms, day_before_ms, edited_since_ms, limit)
            .map_err(err)?
            .into_iter()
            .map(agenda_item)
            .collect())
    }

    pub fn overdue_count(&self, timed_before_ms: i64, day_before_ms: i64) -> Result<u32, String> {
        self.inner
            .overdue_count(timed_before_ms, day_before_ms)
            .map_err(err)
    }

    pub fn set_agenda_done(
        &self,
        page: String,
        block: Option<String>,
        done: bool,
    ) -> Result<(), String> {
        self.inner.set_agenda_done(page, block, done).map_err(err)
    }

    pub fn in_motion(&self, limit: u32) -> Result<Vec<Progress>, String> {
        Ok(self
            .inner
            .in_motion(limit)
            .map_err(err)?
            .into_iter()
            .map(progress)
            .collect())
    }

    pub fn daily_note(
        &self,
        day_ms: i64,
        title: String,
        create: bool,
    ) -> Result<Option<String>, String> {
        self.inner.daily_note(day_ms, title, create).map_err(err)
    }

    /// Links a task under the day's Tasks toggle, returning whether added.
    pub fn link_task_to_day(&self, day_ms: i64, task_page_id: String) -> Result<bool, String> {
        self.inner
            .link_task_to_day(day_ms, task_page_id)
            .map_err(err)
    }

    /// Appends a timestamped child line to the day's top-level Log toggle.
    pub fn append_log(&self, day_ms: i64, text: String) -> Result<String, String> {
        self.inner.append_log(day_ms, text).map_err(err)
    }

    /// Sorted days with a Journal note in the half-open UTC day range.
    pub fn day_notes_in_range(&self, from_ms: i64, to_ms: i64) -> Result<Vec<i64>, String> {
        self.inner.day_notes_in_range(from_ms, to_ms).map_err(err)
    }

    // ----------------------------------------- properties and collections

    pub fn set_property(&self, page: String, key: String, value: PropValue) -> Result<(), String> {
        self.inner
            .set_property(page, key, value_to(value))
            .map_err(err)
    }

    pub fn remove_property(&self, page: String, key: String) -> Result<(), String> {
        self.inner.remove_property(page, key).map_err(err)
    }

    pub fn make_collection(&self, page: String) -> Result<(), String> {
        self.inner.make_collection(page).map_err(err)
    }

    pub fn get_schema(&self, collection: String) -> Result<Vec<PropDef>, String> {
        Ok(self
            .inner
            .get_schema(collection)
            .map_err(err)?
            .into_iter()
            .map(prop_def_from)
            .collect())
    }

    pub fn set_schema(
        &self,
        collection: String,
        schema: Vec<PropDef>,
    ) -> Result<Vec<PropDef>, String> {
        let schema = schema.into_iter().map(prop_def_to).collect();
        Ok(self
            .inner
            .set_schema(collection, schema)
            .map_err(err)?
            .into_iter()
            .map(prop_def_from)
            .collect())
    }

    pub fn list_views(&self, collection: String) -> Result<Vec<ViewDef>, String> {
        Ok(self
            .inner
            .list_views(collection)
            .map_err(err)?
            .into_iter()
            .map(view_from)
            .collect())
    }

    pub fn add_view(&self, collection: String, view: ViewDef) -> Result<String, String> {
        self.inner.add_view(collection, view_to(view)?).map_err(err)
    }

    pub fn update_view(&self, collection: String, view: ViewDef) -> Result<(), String> {
        self.inner
            .update_view(collection, view_to(view)?)
            .map_err(err)
    }

    pub fn delete_view(&self, collection: String, view_id: String) -> Result<(), String> {
        self.inner.delete_view(collection, view_id).map_err(err)
    }

    pub fn move_view(&self, collection: String, view_id: String, index: u32) -> Result<(), String> {
        self.inner
            .move_view(collection, view_id, index)
            .map_err(err)
    }

    pub fn set_default_template(
        &self,
        collection: String,
        template: Option<String>,
    ) -> Result<(), String> {
        self.inner
            .set_default_template(collection, template)
            .map_err(err)
    }

    pub fn default_template(&self, collection: String) -> Result<Option<String>, String> {
        self.inner.default_template(collection).map_err(err)
    }

    // ------------------------------------------------------------ templates

    /// Templates for new children of `parent` (any page), in sibling order.
    pub fn list_templates(&self, parent: String) -> Result<Vec<PageRow>, String> {
        Ok(rows(self.inner.list_templates(parent).map_err(err)?))
    }

    pub fn create_template(&self, parent: String, title: String) -> Result<String, String> {
        self.inner.create_template(parent, title).map_err(err)
    }

    /// A child of `parent` from its template `template` (`None`: empty).
    pub fn create_page_from(
        &self,
        parent: String,
        title: String,
        template: Option<String>,
    ) -> Result<String, String> {
        self.inner
            .create_page_from(parent, title, template)
            .map_err(err)
    }

    pub fn set_template(&self, page: String, is_template: bool) -> Result<(), String> {
        self.inner.set_template(page, is_template).map_err(err)
    }

    /// Copies a template of the page's parent into the page.
    pub fn apply_template(&self, page: String, template: String) -> Result<(), String> {
        self.inner.apply_template(page, template).map_err(err)
    }

    // ------------------------------------------------------------ mounts

    /// The folders of this device mounted as pages (desktop only).
    #[frb(sync)]
    pub fn list_mounts(&self) -> Vec<MountInfo> {
        self.inner
            .list_mounts()
            .into_iter()
            .map(|m| MountInfo {
                page: m.page,
                folder: m.folder,
            })
            .collect()
    }

    /// Mounts the absolute path `folder` as a new page under `parent`
    /// (`None`: top level); its `.md` files become the page's children.
    pub fn add_mount(&self, folder: String, parent: Option<String>) -> Result<MountInfo, String> {
        let m = self.inner.add_mount(folder, parent).map_err(err)?;
        Ok(MountInfo {
            page: m.page,
            folder: m.folder,
        })
    }

    /// Unmounts; the pages stay, the folder's files are left alone.
    pub fn remove_mount(&self, page: String) -> Result<(), String> {
        self.inner.remove_mount(page).map_err(err)
    }

    /// Reads outside changes in the mounted folders. Returns the number of
    /// pages changed.
    pub fn rescan_mounts(&self) -> Result<u32, String> {
        self.inner.rescan_mounts().map_err(err)
    }

    // --------------------------------------------------------- calendars

    /// Calendar categories and database visibility. The first call creates
    /// the default calendars, so it is a write.
    pub fn list_calendars(&self) -> Result<CalendarSettings, String> {
        Ok(calendar_settings_from(
            self.inner.list_calendars().map_err(err)?,
        ))
    }

    /// Adds a calendar at the end of the list and returns its id.
    pub fn create_calendar(
        &self,
        name: String,
        color: String,
        show_in_calendar: bool,
        show_in_today: bool,
    ) -> Result<String, String> {
        self.inner
            .create_calendar(name, color, show_in_calendar, show_in_today)
            .map_err(err)
    }

    pub fn update_calendar(&self, calendar: CalendarCategory) -> Result<(), String> {
        self.inner
            .update_calendar(calendar_category_to(calendar))
            .map_err(err)
    }

    pub fn delete_calendar(&self, id: String) -> Result<(), String> {
        self.inner.delete_calendar(id).map_err(err)
    }

    /// Reorders every calendar; `ids` must list each id exactly once.
    pub fn reorder_calendars(&self, ids: Vec<String>) -> Result<(), String> {
        self.inner.reorder_calendars(ids).map_err(err)
    }

    pub fn set_database_calendar_visibility(
        &self,
        database: String,
        show_in_calendar: bool,
    ) -> Result<(), String> {
        self.inner
            .set_database_calendar_visibility(database, show_in_calendar)
            .map_err(err)
    }

    // ------------------------------------------------------------ queries

    pub fn query(
        &self,
        scope: Option<String>,
        filter: Option<Filter>,
        sorts: Vec<SortSpec>,
        offset: u32,
        limit: u32,
    ) -> Result<QueryResult, String> {
        let r = self
            .inner
            .query(
                scope,
                opt_filter_to(filter)?,
                sorts_to(sorts),
                offset,
                limit,
            )
            .map_err(err)?;
        Ok(QueryResult {
            rows: rows(r.rows),
            total: r.total,
        })
    }

    pub fn query_groups(
        &self,
        scope: Option<String>,
        filter: Option<Filter>,
        group_by: String,
    ) -> Result<Vec<GroupRow>, String> {
        Ok(self
            .inner
            .query_groups(scope, opt_filter_to(filter)?, group_by)
            .map_err(err)?
            .into_iter()
            .map(group)
            .collect())
    }

    pub fn query_range(
        &self,
        scope: Option<String>,
        date_prop: String,
        from_ms: i64,
        to_ms: i64,
        filter: Option<Filter>,
    ) -> Result<Vec<RangeEntry>, String> {
        Ok(self
            .inner
            .query_range(scope, date_prop, from_ms, to_ms, opt_filter_to(filter)?)
            .map_err(err)?
            .into_iter()
            .map(range_entry)
            .collect())
    }

    // ------------------------------- links, attachments, history, feed

    pub fn backlinks(&self, id: String) -> Result<Vec<PageRow>, String> {
        Ok(rows(self.inner.backlinks(id).map_err(err)?))
    }

    pub fn add_attachment_from_path(&self, path: String) -> Result<String, String> {
        self.inner.add_attachment_from_path(path).map_err(err)
    }

    pub fn add_attachment_bytes(&self, name: String, bytes: Vec<u8>) -> Result<String, String> {
        self.inner.add_attachment_bytes(name, bytes).map_err(err)
    }

    pub fn attachment_path(&self, id: String) -> Result<String, String> {
        self.inner.attachment_path(id).map_err(err)
    }

    pub fn attachment_info(&self, id: String) -> Result<AttachmentInfo, String> {
        let a = self.inner.attachment_info(id).map_err(err)?;
        Ok(AttachmentInfo {
            id: a.id,
            name: a.name,
            mime: a.mime,
            size: a.size,
        })
    }

    pub fn gc_attachments(&self) -> Result<u32, String> {
        self.inner.gc_attachments().map_err(err)
    }

    pub fn list_versions(&self, page: String) -> Result<Vec<VersionInfo>, String> {
        Ok(self
            .inner
            .list_versions(page)
            .map_err(err)?
            .into_iter()
            .map(|v| VersionInfo {
                id: v.id,
                time_ms: v.time_ms,
                device: v.device,
                summary: v.summary,
                changes: v.changes,
            })
            .collect())
    }

    pub fn load_page_at(&self, page: String, version: String) -> Result<PageData, String> {
        Ok(page_data(
            self.inner.load_page_at(page, version).map_err(err)?,
            0,
        ))
    }

    pub fn restore_version(&self, page: String, version: String) -> Result<(), String> {
        self.inner.restore_version(page, version).map_err(err)
    }

    /// Document changes after `since_seq` (each page once), or `reset` when
    /// the feed cannot tell. Index commits belong to the live-query stream.
    pub fn poll_changes(&self, since_seq: u32) -> ChangeBatch {
        change_batch(self.inner.poll_sync_changes(since_seq))
    }

    /// `poll_changes` that blocks up to `timeout_ms` for a change. Holds a
    /// bridge worker thread while it waits.
    pub fn wait_changes(&self, since_seq: u32, timeout_ms: u32) -> ChangeBatch {
        change_batch(self.inner.wait_sync_changes(since_seq, timeout_ms))
    }

    // ------------------------------------------------------------ import

    pub fn import_markdown(
        &self,
        dir: String,
        parent: Option<String>,
    ) -> Result<ImportReport, String> {
        let r = self.inner.import_markdown(dir, parent).map_err(err)?;
        Ok(ImportReport {
            pages: r.pages,
            attachments: r.attachments,
            unresolved_links: r.unresolved_links,
            top_pages: r.top_pages,
        })
    }

    /// Reads outside edits of the markdown files back into the pages.
    pub fn rescan(&self) -> Result<u32, String> {
        self.inner.rescan().map_err(err)
    }

    // -------------------------------------------------------- sync hooks

    pub fn page_versions(&self) -> Result<Vec<PageVersion>, String> {
        Ok(self
            .inner
            .page_versions()
            .map_err(err)?
            .into_iter()
            .map(|v| PageVersion {
                page: v.page,
                version: v.version,
            })
            .collect())
    }

    pub fn export_updates(&self, page: String, since: Vec<u8>) -> Result<Vec<u8>, String> {
        self.inner.export_updates(page, since).map_err(err)
    }

    pub fn import_updates(&self, page: String, bytes: Vec<u8>) -> Result<SyncImport, String> {
        Ok(sync_import(
            self.inner.import_updates(page, bytes).map_err(err)?,
        ))
    }

    pub fn import_updates_batch(&self, updates: Vec<PageUpdate>) -> Result<SyncImport, String> {
        let u = updates
            .into_iter()
            .map(|u| vault_core::PageUpdate {
                page: u.page,
                bytes: u.bytes,
            })
            .collect();
        Ok(sync_import(
            self.inner.import_updates_batch(u).map_err(err)?,
        ))
    }

    pub fn purged_pages(&self) -> Vec<String> {
        self.inner.purged_pages()
    }

    pub fn import_purged(&self, pages: Vec<String>) -> Result<u32, String> {
        self.inner.import_purged(pages).map_err(err)
    }

    /// Pending conflicts, including rule-applied entries awaiting confirmation.
    pub fn conflicts(&self, page: Option<String>) -> Result<Vec<Conflict>, String> {
        let device = self.inner.device_id();
        Ok(self
            .inner
            .list_conflicts(page)
            .map_err(err)?
            .into_iter()
            .map(|c| conflict(c, &device))
            .collect())
    }

    /// Compatibility alias for pending conflicts.
    pub fn list_conflicts(&self, page: Option<String>) -> Result<Vec<Conflict>, String> {
        self.conflicts(page)
    }

    /// Resolution log, including undone applications on reopened pending entries.
    pub fn conflict_log(&self, page: Option<String>) -> Result<Vec<Conflict>, String> {
        let device = self.inner.device_id();
        Ok(self
            .inner
            .conflict_log(page)
            .map_err(err)?
            .into_iter()
            .map(|c| conflict(c, &device))
            .collect())
    }

    pub fn resolve_conflict(
        &self,
        page: String,
        id: String,
        resolution: Resolution,
    ) -> Result<(), String> {
        self.inner
            .resolve_conflict(page, id, resolution_to(resolution))
            .map_err(err)
    }

    /// Restore the scope before its latest resolution. Ordinary entries return
    /// to pending review; journal repairs remain log-only. Both retain an undone
    /// application in their history and in the conflict log.
    pub fn undo_conflict(&self, page: String, id: String) -> Result<(), String> {
        self.inner.undo_conflict(page, id).map_err(err)
    }

    pub fn conflict_diff(&self, page: String, id: String) -> Result<ConflictDiff, String> {
        Ok(conflict_diff(
            self.inner.conflict_diff(page, id).map_err(err)?,
        ))
    }

    pub fn conflict_auto_resolve(&self) -> Result<bool, String> {
        self.inner.conflict_auto_resolve().map_err(err)
    }

    pub fn set_conflict_auto_resolve(&self, enabled: bool) -> Result<(), String> {
        self.inner.set_conflict_auto_resolve(enabled).map_err(err)
    }

    pub fn agent_defaults(&self) -> Result<VaultAgentDefaults, String> {
        let d = self.inner.agent_defaults().map_err(err)?;
        Ok(VaultAgentDefaults {
            runner: d.runner,
            models: d
                .models
                .into_iter()
                .map(|(runner, model)| VaultAgentModel { runner, model })
                .collect(),
        })
    }

    pub fn set_agent_default_runner(&self, runner: Option<String>) -> Result<(), String> {
        self.inner.set_agent_default_runner(runner).map_err(err)
    }

    pub fn set_agent_default_model(
        &self,
        runner: String,
        model: Option<String>,
    ) -> Result<(), String> {
        self.inner
            .set_agent_default_model(runner, model)
            .map_err(err)
    }

    /// The tool set a runner's agent gets: read, write or everything
    /// (the default). The runner reads it at its next engine start.
    pub fn agent_tool_set(&self, runner: String) -> Result<String, String> {
        self.inner.agent_tool_set(&runner).map_err(err)
    }

    /// Sets a runner's tool set; `None` returns it to everything.
    pub fn set_agent_tool_set(&self, runner: String, set: Option<String>) -> Result<(), String> {
        self.inner.set_agent_tool_set(runner, set).map_err(err)
    }

    /// One-time migration of old per-device values: written only when the
    /// vault has no agent defaults. Returns whether it wrote.
    pub fn seed_agent_defaults(&self, seed: VaultAgentDefaults) -> Result<bool, String> {
        self.inner
            .seed_agent_defaults(vault_core::AgentDefaults {
                runner: seed.runner,
                models: seed
                    .models
                    .into_iter()
                    .map(|m| (m.runner, m.model))
                    .collect(),
            })
            .map_err(err)
    }

    pub fn attachment_ids(&self) -> Result<Vec<String>, String> {
        self.inner.attachment_ids().map_err(err)
    }

    pub fn missing_attachments(&self) -> Result<Vec<String>, String> {
        self.inner.missing_attachments().map_err(err)
    }

    pub fn store_attachment(&self, id: String, name: String, bytes: Vec<u8>) -> Result<(), String> {
        self.inner.store_attachment(id, name, bytes).map_err(err)
    }
}

#[cfg(test)]
mod conflict_tests {
    use super::*;
    use vault_core as vc;

    struct TempRoot(std::path::PathBuf);

    impl TempRoot {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("notes_bridge_conflict_{}", vc::new_id())))
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn edit(id: &str, text: &str) -> vc::BlockOp {
        vc::BlockOp::SetText {
            id: id.into(),
            text: text.into(),
            marks: vec![],
        }
    }

    #[test]
    fn purging_device_is_local_and_applied_choices_cross_the_bridge() {
        let purger_root = TempRoot::new();
        let editor_root = TempRoot::new();
        let purger = NotesVault::open(purger_root.0.to_string_lossy().into_owned(), None).unwrap();
        let editor = NotesVault::open(editor_root.0.to_string_lossy().into_owned(), None).unwrap();
        let page = purger
            .create_page(None, "Purge conflict".into(), None)
            .unwrap();
        purger
            .inner
            .apply(
                page.clone(),
                vec![vc::BlockOp::Insert {
                    parent: None,
                    after: None,
                    block: vc::NewBlock {
                        id: "target".into(),
                        kind: vc::BlockKind::Paragraph,
                        text: "Original".into(),
                        marks: vec![],
                        checked: false,
                    },
                }],
            )
            .unwrap();
        editor
            .import_updates(
                page.clone(),
                purger.export_updates(page.clone(), vec![]).unwrap(),
            )
            .unwrap();
        purger.inner.delete_page(page.clone()).unwrap();
        purger.inner.purge_page(page.clone()).unwrap();
        editor
            .inner
            .apply(page.clone(), vec![edit("target", "Offline edit")])
            .unwrap();
        editor
            .inner
            .import_purge_records(purger.inner.purge_records())
            .unwrap();
        purger
            .import_updates(
                page.clone(),
                editor.export_updates(page.clone(), vec![]).unwrap(),
            )
            .unwrap();

        for (replica, local) in [(&purger, true), (&editor, false)] {
            let conflicts = replica.conflicts(Some(page.clone())).unwrap();
            let conflict = conflicts
                .iter()
                .find(|entry| entry.scope.kind == ConflictScopeKind::Page)
                .unwrap();
            let side = conflict
                .sides
                .iter()
                .find(|side| side.state.kind == ConflictStateKind::Purged)
                .unwrap();
            assert_eq!(side.device, purger.inner.device_id());
            assert_eq!(side.local, local);
            let choice = conflict
                .applied
                .last()
                .unwrap()
                .resolution
                .as_ref()
                .unwrap();
            assert_eq!(choice.kind, ResolutionKind::UseSide);
            assert_eq!(
                conflict.sides[choice.side as usize].state.kind,
                ConflictStateKind::Present
            );
        }
        let conflict = purger.conflicts(Some(page.clone())).unwrap().remove(0);
        purger
            .resolve_conflict(
                page.clone(),
                conflict.id.clone(),
                Resolution {
                    kind: ResolutionKind::Keep,
                    side: 0,
                },
            )
            .unwrap();
        let log = purger.conflict_log(Some(page)).unwrap();
        let resolved = log.iter().find(|entry| entry.id == conflict.id).unwrap();
        assert_eq!(
            resolved
                .applied
                .last()
                .unwrap()
                .resolution
                .as_ref()
                .unwrap()
                .kind,
            ResolutionKind::Keep
        );
    }

    #[test]
    fn ordinary_undo_reopens_pending_and_retains_the_undone_log_through_public_api() {
        // Roots outlive the handles, matching the existing bridge test pattern.
        let local_root = TempRoot::new();
        let cloud_root = TempRoot::new();
        let local = NotesVault::open(local_root.0.to_string_lossy().into_owned(), None).unwrap();
        let cloud = NotesVault::open(cloud_root.0.to_string_lossy().into_owned(), None).unwrap();
        let page = local.create_page(None, "Conflict".into(), None).unwrap();
        local
            .inner
            .apply(
                page.clone(),
                vec![
                    vc::BlockOp::Insert {
                        parent: None,
                        after: None,
                        block: vc::NewBlock {
                            id: "target".into(),
                            kind: vc::BlockKind::Paragraph,
                            text: "Original".into(),
                            marks: vec![],
                            checked: false,
                        },
                    },
                    vc::BlockOp::Insert {
                        parent: None,
                        after: Some("target".into()),
                        block: vc::NewBlock {
                            id: "unrelated".into(),
                            kind: vc::BlockKind::Paragraph,
                            text: "Unrelated".into(),
                            marks: vec![],
                            checked: false,
                        },
                    },
                ],
            )
            .unwrap();
        cloud
            .import_updates(
                page.clone(),
                local.export_updates(page.clone(), vec![]).unwrap(),
            )
            .unwrap();
        local
            .inner
            .apply(page.clone(), vec![edit("target", "Local")])
            .unwrap();
        cloud
            .inner
            .apply(page.clone(), vec![edit("target", "Cloud")])
            .unwrap();
        local
            .import_updates(
                page.clone(),
                cloud.export_updates(page.clone(), vec![]).unwrap(),
            )
            .unwrap();
        let conflict = local
            .conflicts(Some(page.clone()))
            .unwrap()
            .into_iter()
            .find(|entry| entry.scope.kind == ConflictScopeKind::Block && entry.block == "target")
            .expect("concurrent edits cross the pending bridge API");
        let side = conflict
            .sides
            .iter()
            .position(|side| {
                side.block
                    .as_ref()
                    .is_some_and(|block| block.text == "Cloud")
            })
            .unwrap();
        let before = local.inner.load_page(page.clone()).unwrap().blocks[0].clone();
        local
            .resolve_conflict(
                page.clone(),
                conflict.id.clone(),
                Resolution {
                    kind: ResolutionKind::UseSide,
                    side: side as u32,
                },
            )
            .unwrap();
        assert!(local
            .conflicts(Some(page.clone()))
            .unwrap()
            .iter()
            .all(|entry| entry.id != conflict.id));
        let resolved = local
            .conflict_log(Some(page.clone()))
            .unwrap()
            .into_iter()
            .find(|entry| entry.id == conflict.id)
            .expect("resolution crosses the log bridge API");
        assert_eq!(resolved.status, ConflictStatus::Resolved);
        assert!(resolved.undoable);
        let choice = resolved
            .applied
            .last()
            .unwrap()
            .resolution
            .as_ref()
            .unwrap();
        assert_eq!(choice.kind, ResolutionKind::UseSide);
        assert_eq!(choice.side, side as u32);
        assert_eq!(resolved.applied.last().unwrap().undone_at, None);
        local
            .inner
            .apply(
                page.clone(),
                vec![edit("unrelated", "Later unrelated edit")],
            )
            .unwrap();
        local
            .undo_conflict(page.clone(), conflict.id.clone())
            .unwrap();
        let pending = local
            .conflicts(Some(page.clone()))
            .unwrap()
            .into_iter()
            .find(|entry| entry.id == conflict.id)
            .expect("ordinary undo reopens pending review");
        let logged = local
            .conflict_log(Some(page.clone()))
            .unwrap()
            .into_iter()
            .find(|entry| entry.id == conflict.id)
            .expect("ordinary undo also keeps the log entry");
        assert_eq!(pending.status, ConflictStatus::Pending);
        assert_eq!(logged.status, ConflictStatus::Pending);
        assert!(!pending.undoable);
        assert!(!logged.undoable);
        let undone = pending.applied.last().unwrap();
        assert_eq!(undone.by, ResolutionBy::User);
        assert_eq!(undone.resolution.as_ref().unwrap().side, side as u32);
        assert!(undone.undone_at.is_some());
        assert_eq!(logged.applied.last().unwrap().undone_at, undone.undone_at);
        let restored = local.inner.load_page(page.clone()).unwrap();
        assert_eq!(restored.blocks[0], before);
        assert_eq!(restored.blocks[1].text, "Later unrelated edit");
        assert!(
            local.undo_conflict(page, conflict.id).is_err(),
            "the consumed application cannot be undone twice"
        );
    }
}

/// A new ULID-style id (the core's page id format).
#[frb(sync)]
pub fn new_id() -> String {
    vault_core::new_id()
}

/// The configured status group, with option order as the fallback.
pub fn status_group(def: PropDef, option_id: String) -> StatusGroup {
    status_group_from(vault_core::status_group(&prop_def_to(def), &option_id))
}

/// The first status option in the requested group, in schema order.
pub fn first_option_in(def: PropDef, group: StatusGroup) -> Option<SelectOption> {
    let def = prop_def_to(def);
    vault_core::first_option_in(&def, status_group_to(group)).map(|option| SelectOption {
        id: option.id.clone(),
        name: option.name.clone(),
        color: option.color.clone(),
    })
}

/// Lowercase hex SHA-256 (an attachment's id).
pub fn sha256_hex(bytes: Vec<u8>) -> String {
    vault_core::sha256_hex(&bytes)
}

/// Whether version `a` includes everything in version `b`.
pub fn version_includes(a: Vec<u8>, b: Vec<u8>) -> Result<bool, String> {
    vault_core::version_includes(&a, &b).map_err(err)
}

#[frb(init)]
pub fn init_app() {
    flutter_rust_bridge::setup_default_user_utils();
}

/// Calls `rescan_mounts` every 2 seconds until the vault is closed. Holds
/// the vault only while a rescan runs (none without mounts).
fn poll_mounts(vault: std::sync::Weak<Vault>) {
    let _ = std::thread::Builder::new()
        .name("vault-mounts".into())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let Some(v) = vault.upgrade() else { return };
            if !v.list_mounts().is_empty() {
                let _ = v.rescan_mounts();
            }
        });
}
