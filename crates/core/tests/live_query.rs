use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use vault_core::*;

struct TempVault(PathBuf);

impl TempVault {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("vault_live_query_{}", new_id())))
    }

    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).unwrap()
    }
}

impl Drop for TempVault {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn paragraph(v: &Vault, page: &str, text: &str) -> String {
    let id = new_id();
    v.apply(
        page.into(),
        vec![BlockOp::Insert {
            parent: None,
            after: None,
            block: NewBlock {
                id: id.clone(),
                kind: BlockKind::Paragraph,
                text: text.into(),
                marks: vec![],
                checked: false,
            },
        }],
    )
    .unwrap();
    id
}

fn replace(v: &Vault, page: &str, block: &str, text: &str) {
    v.apply(
        page.into(),
        vec![BlockOp::SetText {
            id: block.into(),
            text: text.into(),
            marks: vec![],
        }],
    )
    .unwrap();
}

fn watch(v: &Vault, query: &str, limit: u32) -> (LiveQuerySubscription, LiveQueryUpdate) {
    let subscription = v
        .watch_query(v.parse_search(query.into()).unwrap().spec, limit)
        .unwrap();
    let initial = subscription.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(initial.initial);
    assert!(initial.removed.is_empty());
    assert!(initial.changed.is_empty());
    assert!(initial.moved.is_empty());
    (subscription, initial)
}

fn next(subscription: &LiveQuerySubscription) -> LiveQueryUpdate {
    let update = subscription.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(!update.initial);
    update
}

fn quiet(subscription: &LiveQuerySubscription) {
    assert_eq!(
        subscription.recv_timeout(Duration::from_millis(150)),
        Err(RecvTimeoutError::Timeout)
    );
}

fn key(source: &str, id: &str) -> HitKey {
    HitKey {
        source: source.into(),
        id: id.into(),
    }
}

fn external(id: &str, title: &str, body: &str) -> ExternalSearchRow {
    ExternalSearchRow {
        id: id.into(),
        title: title.into(),
        body: body.into(),
        page_type: "chat".into(),
        target: Some("{\"anchor\":1}".into()),
        updated_ms: 100,
    }
}

#[test]
fn initial_snapshot_is_complete_and_empty_queries_also_receive_one() {
    let dir = TempVault::new();
    let v = dir.open();
    let id = v.create_page(None, "Apricot".into()).unwrap();
    paragraph(&v, &id, "Harvest notes");
    v.flush().unwrap();
    let (subscription, initial) = watch(&v, "apricot", 5);
    assert_eq!(
        initial.added,
        v.search_spec(v.parse_search("apricot".into()).unwrap().spec, 5, 0)
            .unwrap()
            .hits
    );
    assert_eq!(initial.added[0].id, id);
    assert_eq!(initial.total_bounded, 1);
    assert_eq!(initial.generation, v.index_generation());
    quiet(&subscription);
    let (_, empty) = watch(&v, "watermelon", 5);
    assert!(empty.added.is_empty());
    assert_eq!(empty.total_bounded, 0);
}

#[test]
fn text_enters_and_leaves_after_writer_commit_with_no_early_page_changed_push() {
    let dir = TempVault::new();
    let v = dir.open();
    let id = v.create_page(None, "Orchard".into()).unwrap();
    let block = paragraph(&v, &id, "ordinary notes");
    v.flush().unwrap();
    let (subscription, initial) = watch(&v, "xylophone", 5);
    let before = v.poll_changes(0).seq;
    replace(&v, &id, &block, "xylophone melody");
    assert!(v
        .poll_changes(before)
        .changes
        .contains(&Change::PageChanged(id.clone())));
    // The debounce is 300ms: a short wait exercises the earlier PageChanged
    // notification without waiting for the normal writer commit.
    assert_eq!(
        subscription.recv_timeout(Duration::from_millis(20)),
        Err(RecvTimeoutError::Timeout)
    );
    v.flush().unwrap();
    let added = next(&subscription);
    assert!(added.generation > initial.generation);
    assert_eq!(added.added.len(), 1);
    assert_eq!(added.added[0].id, id);
    assert_eq!(added.total_bounded, 1);
    replace(&v, &id, &block, "ordinary notes");
    v.flush().unwrap();
    let removed = next(&subscription);
    assert!(removed.generation > added.generation);
    assert_eq!(removed.removed, vec![key(VAULT_SOURCE, &id)]);
    assert_eq!(removed.total_bounded, 0);
    quiet(&subscription);
}

#[test]
fn new_pages_enter_and_unrelated_commits_produce_no_update() {
    let dir = TempVault::new();
    let v = dir.open();
    let (subscription, _) = watch(&v, "title:nectarine", 10);
    let unrelated = v.create_page(None, "Notebook".into()).unwrap();
    paragraph(&v, &unrelated, "unrelated text");
    v.flush().unwrap();
    quiet(&subscription);
    let id = v.create_page(None, "Nectarine".into()).unwrap();
    v.flush().unwrap();
    assert_eq!(next(&subscription).added[0].id, id);
    quiet(&subscription);
}

#[test]
fn changed_snippets_and_sort_moves_are_delivered_without_duplicate_additions() {
    let dir = TempVault::new();
    let v = dir.open();
    let a = v.create_page(None, "Alpha".into()).unwrap();
    let b = v.create_page(None, "Bravo".into()).unwrap();
    let block = paragraph(&v, &b, "original preview");
    v.flush().unwrap();
    let (preview, _) = watch(&v, "bravo", 10);
    replace(&v, &b, &block, "updated preview");
    v.flush().unwrap();
    let changed = next(&preview);
    assert!(changed.added.is_empty());
    assert!(changed
        .changed
        .iter()
        .any(|hit| hit.id == b && hit.snippet.contains("updated preview")));
    drop(preview);
    let (subscription, initial) = watch(&v, "sort:title", 10);
    assert_eq!(
        initial.added.iter().map(|h| &h.id).collect::<Vec<_>>(),
        vec![&a, &b]
    );
    v.rename_page(b.clone(), "Aardvark".into()).unwrap();
    v.flush().unwrap();
    let moved = next(&subscription);
    assert!(moved.added.is_empty());
    assert!(moved.moved.contains(&(key(VAULT_SOURCE, &b), 0)));
    assert!(moved.moved.contains(&(key(VAULT_SOURCE, &a), 1)));
    assert!(moved
        .changed
        .iter()
        .any(|hit| hit.id == b && hit.title == "Aardvark"));
    quiet(&subscription);
}

#[test]
fn limit_window_replacement_and_count_only_changes_are_preserved() {
    let dir = TempVault::new();
    let v = dir.open();
    let a = v.create_page(None, "Apple".into()).unwrap();
    v.flush().unwrap();
    let (subscription, initial) = watch(&v, "sort:title", 1);
    assert_eq!(initial.added[0].id, a);
    let b = v.create_page(None, "Banana".into()).unwrap();
    v.flush().unwrap();
    let count = next(&subscription);
    assert_eq!(count.total_bounded, 2);
    assert!(count.added.is_empty() && count.removed.is_empty() && count.changed.is_empty());
    v.delete_page(a.clone()).unwrap();
    v.flush().unwrap();
    let replacement = next(&subscription);
    assert_eq!(replacement.removed, vec![key(VAULT_SOURCE, &a)]);
    assert_eq!(replacement.added[0].id, b);
    assert!(replacement.moved.contains(&(key(VAULT_SOURCE, &b), 0)));
    assert_eq!(replacement.total_bounded, 1);
}

#[test]
fn tree_moves_wake_in_queries_for_unchanged_descendants() {
    let dir = TempVault::new();
    let v = dir.open();
    let home = v.create_page(None, "Home".into()).unwrap();
    let branch = v.create_page(None, "Branch".into()).unwrap();
    let leaf = v.create_page(Some(branch.clone()), "Leaf".into()).unwrap();
    paragraph(&v, &leaf, "papaya");
    v.flush().unwrap();
    let (subscription, initial) = watch(&v, &format!("papaya in:[[{home}]]"), 10);
    assert!(initial.added.is_empty());
    v.move_page(branch.clone(), Some(home.clone()), None)
        .unwrap();
    v.flush().unwrap();
    assert_eq!(next(&subscription).added[0].id, leaf);
    v.move_page(branch, None, None).unwrap();
    v.flush().unwrap();
    assert_eq!(next(&subscription).removed, vec![key(VAULT_SOURCE, &leaf)]);
    quiet(&subscription);
}

#[test]
fn trash_and_restore_refresh_descendant_results_without_in_filter() {
    let dir = TempVault::new();
    let v = dir.open();
    let parent = v.create_page(None, "Branch".into()).unwrap();
    let leaf = v
        .create_page(Some(parent.clone()), "Papaya".into())
        .unwrap();
    v.flush().unwrap();
    let (subscription, initial) = watch(&v, "title:papaya", 10);
    assert_eq!(initial.added[0].id, leaf);
    v.delete_page(parent.clone()).unwrap();
    v.flush().unwrap();
    assert_eq!(next(&subscription).removed, vec![key(VAULT_SOURCE, &leaf)]);
    v.restore_page(parent).unwrap();
    v.flush().unwrap();
    assert_eq!(next(&subscription).added[0].id, leaf);
    quiet(&subscription);
}

#[test]
fn sync_import_hot_and_cold_notify_existing_subscriptions() {
    let source_dir = TempVault::new();
    let target_dir = TempVault::new();
    let source = source_dir.open();
    let target = target_dir.open();
    let (subscription, _) = watch(&target, "title:kiwi", 10);
    let id = source.create_page(None, "Kiwi".into()).unwrap();
    source.flush().unwrap();
    target
        .import_updates_batch(vec![PageUpdate {
            page: id.clone(),
            bytes: source.export_updates(id.clone(), vec![]).unwrap(),
        }])
        .unwrap();
    assert_eq!(next(&subscription).added[0].id, id);
    let version = target
        .page_versions()
        .unwrap()
        .into_iter()
        .find(|v| v.page == id)
        .unwrap()
        .version;
    source.rename_page(id.clone(), "Plum".into()).unwrap();
    target
        .import_updates_batch(vec![PageUpdate {
            page: id.clone(),
            bytes: source.export_updates(id.clone(), version).unwrap(),
        }])
        .unwrap();
    // apply_import_index commits metadata synchronously, without relying on
    // the debounce writer to make a title filter notice the removal.
    assert_eq!(next(&subscription).removed, vec![key(VAULT_SOURCE, &id)]);
    target.flush().unwrap();
    quiet(&subscription);
}

#[test]
fn source_local_collisions_are_distinct_during_change_move_and_removal() {
    let dir = TempVault::new();
    let v = dir.open();
    let id = v.create_page(None, "Bravo".into()).unwrap();
    v.upsert_search_row("agent".into(), external(&id, "Charlie", "agent summary"))
        .unwrap();
    v.upsert_search_row("other".into(), external(&id, "Delta", "other summary"))
        .unwrap();
    v.flush().unwrap();
    let (subscription, initial) = watch(&v, "sort:title", 10);
    assert_eq!(initial.added.len(), 3);
    assert_eq!(
        initial
            .added
            .iter()
            .map(HitKey::from)
            .collect::<HashSet<_>>()
            .len(),
        3
    );
    let mut row = external(&id, "Alpha", "changed agent summary");
    row.target = Some("{\"anchor\":2}".into());
    v.upsert_search_row("agent".into(), row).unwrap();
    let changed = next(&subscription);
    assert!(changed.added.is_empty() && changed.removed.is_empty());
    assert_eq!(changed.changed.len(), 1);
    assert_eq!(HitKey::from(&changed.changed[0]), key("agent", &id));
    assert!(changed.moved.contains(&(key("agent", &id), 0)));
    v.delete_search_row("agent".into(), id.clone()).unwrap();
    assert_eq!(next(&subscription).removed, vec![key("agent", &id)]);
    v.clear_search_source("other".into()).unwrap();
    assert_eq!(next(&subscription).removed, vec![key("other", &id)]);
    quiet(&subscription);
}

#[test]
fn unwatch_disconnects_even_queued_events_and_vault_drop_closes_live_receivers() {
    let dir = TempVault::new();
    let v = dir.open();
    let subscription = v.watch_query(SearchSpec::default(), 10).unwrap();
    v.unwatch_query(subscription.handle);
    assert_eq!(
        subscription.recv_timeout(Duration::from_secs(1)),
        Err(RecvTimeoutError::Disconnected)
    );
    v.unwatch_query(subscription.handle);
    let (remaining, _) = watch(&v, "sort:title", 10);
    let started = Instant::now();
    drop(v);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(
        remaining.recv_timeout(Duration::from_secs(1)),
        Err(RecvTimeoutError::Disconnected)
    );
}

#[test]
fn generations_and_writer_committed_ids_are_monotonic_and_follow_content_commit() {
    let dir = TempVault::new();
    let v = dir.open();
    let a = v.create_page(None, "One".into()).unwrap();
    let b = v.create_page(None, "Two".into()).unwrap();
    v.flush().unwrap();
    let since = v.poll_changes(0).seq;
    let generation = v.index_generation();
    paragraph(&v, &a, "first committed text");
    paragraph(&v, &b, "second committed text");
    v.flush().unwrap();
    let commits: Vec<_> = v
        .poll_changes(since)
        .changes
        .into_iter()
        .filter_map(|change| match change {
            Change::IndexCommitted { generation, pages } => Some((generation, pages)),
            _ => None,
        })
        .collect();
    assert!(!commits.is_empty());
    assert!(commits[0].0 > generation);
    assert!(commits.windows(2).all(|pair| pair[0].0 < pair[1].0));
    let ids = commits
        .iter()
        .flat_map(|(_, pages)| pages.iter().cloned())
        .collect::<HashSet<_>>();
    assert_eq!(ids, HashSet::from([a.clone(), b.clone()]));
    assert_eq!(v.search("committed".into(), 10).unwrap().len(), 2);
    assert_eq!(commits.last().unwrap().0, v.index_generation());
}

#[test]
fn updates_converge_when_registration_and_commits_race() {
    let dir = TempVault::new();
    let v = std::sync::Arc::new(dir.open());
    let id = v.create_page(None, "Seed".into()).unwrap();
    v.flush().unwrap();
    let edit = v.clone();
    let page = id.clone();
    let worker = std::thread::spawn(move || {
        for name in ["Apple", "Banana", "Cherry", "Date", "Elderberry"] {
            edit.rename_page(page.clone(), name.into()).unwrap();
            edit.flush().unwrap();
        }
    });
    let (subscription, initial) = watch(&v, "sort:title", 10);
    worker.join().unwrap();
    let mut title = initial.added[0].title.clone();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_generation = initial.generation;
    while title != "Elderberry" {
        let update = subscription
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        assert!(update.generation >= last_generation);
        last_generation = update.generation;
        for hit in update.added.into_iter().chain(update.changed) {
            if hit.id == id {
                title = hit.title;
            }
        }
    }
    quiet(&subscription);
}

#[test]
fn related_rollup_rows_are_included_in_committed_ids_and_live_updates() {
    let dir = TempVault::new();
    let v = dir.open();
    let tasks = v.create_page(None, "Tasks".into()).unwrap();
    let projects = v.create_page(None, "Projects".into()).unwrap();
    v.set_schema(
        tasks.clone(),
        vec![PropDef {
            key: "pts".into(),
            name: "Points".into(),
            kind: PropKind::Number,
            options: vec![],
            config: String::new(),
        }],
    )
    .unwrap();
    v.set_schema(
        projects.clone(),
        vec![
            PropDef {
                key: "tasks".into(),
                name: "Tasks".into(),
                kind: PropKind::Relation,
                options: vec![],
                config: serde_json::json!({ "relation": tasks }).to_string(),
            },
            PropDef {
                key: "sum".into(),
                name: "Sum".into(),
                kind: PropKind::Rollup,
                options: vec![],
                config: serde_json::json!({ "rollupRelation": "tasks",
                "rollupProperty": "pts", "rollupFunction": "sum" })
                .to_string(),
            },
        ],
    )
    .unwrap();
    let task = v.create_page(Some(tasks), "Task".into()).unwrap();
    let project = v.create_page(Some(projects), "Project".into()).unwrap();
    v.set_property(task.clone(), "pts".into(), PropValue::Number(1.0))
        .unwrap();
    v.set_property(
        project.clone(),
        "tasks".into(),
        PropValue::Relation(vec![task.clone()]),
    )
    .unwrap();
    v.flush().unwrap();
    let (subscription, initial) = watch(&v, "sum:>5", 10);
    assert!(initial.added.is_empty());
    let since = v.poll_changes(0).seq;
    v.set_property(task.clone(), "pts".into(), PropValue::Number(10.0))
        .unwrap();
    v.flush().unwrap();
    assert_eq!(next(&subscription).added[0].id, project);
    assert!(v
        .poll_changes(since)
        .changes
        .into_iter()
        .any(|change| match change {
            Change::IndexCommitted { pages, .. } => pages.contains(&project),
            _ => false,
        }));
    quiet(&subscription);
}

#[test]
fn markdown_bulk_commit_notifies_watchers_and_repeat_import_stays_quiet() {
    let source = TempVault::new();
    std::fs::create_dir_all(&source.0).unwrap();
    std::fs::write(
        source.0.join("Fruit.md"),
        "# Fruit\n\nPapaya harvest notes.\n",
    )
    .unwrap();
    let dir = TempVault::new();
    let v = dir.open();
    let (subscription, initial) = watch(&v, "papaya", 10);
    assert!(initial.added.is_empty());
    let since = v.poll_changes(0).seq;
    let report = v
        .import_markdown(source.0.to_string_lossy().into_owned(), None)
        .unwrap();
    assert_eq!(report.created, 1);
    let page = report.top_pages[0].clone();
    assert!(
        v.poll_changes(since)
            .changes
            .into_iter()
            .any(|change| match change {
                Change::IndexCommitted { generation, pages } =>
                    generation > initial.generation && pages.contains(&page),
                _ => false,
            }),
        "the bulk transaction publishes without waiting for the page writer"
    );
    let update = next(&subscription);
    assert_eq!(update.added[0].id, page);
    v.flush().unwrap();
    quiet(&subscription);
    let generation = v.index_generation();
    let since = v.poll_changes(0).seq;
    let repeat = v
        .import_markdown(source.0.to_string_lossy().into_owned(), None)
        .unwrap();
    assert_eq!(repeat.created + repeat.updated, 0);
    assert_eq!(v.index_generation(), generation);
    assert!(v.poll_changes(since).changes.is_empty());
    quiet(&subscription);
}
