//! Conflict choices and retained purge histories through real hub HTTP.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use vault_core::*;
use vault_sync::{Hub, HubServer, SyncClient};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("vault-conflict-history-{tag}-{}", new_id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn open(&self) -> Arc<Vault> {
        Arc::new(Vault::open(self.0.to_string_lossy().into_owned()).unwrap())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// Directories are declared last so all clients/server/Vaults drop before them.
struct Network {
    hub: Arc<Hub>,
    _server: HubServer,
    a: Option<SyncClient>,
    b: Option<SyncClient>,
    url: String,
    b_token: String,
    dirs: [TempDir; 3],
}

impl Network {
    fn new() -> Self {
        let dirs = [TempDir::new("hub"), TempDir::new("a"), TempDir::new("b")];
        let hub = Hub::new(dirs[0].open()).unwrap();
        let server = hub.serve("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.addr());
        let a_token = hub.pair("A").unwrap();
        let b_token = hub.pair("B").unwrap();
        let a = Some(SyncClient::new(dirs[1].open(), &url, &a_token));
        let b = Some(SyncClient::new(dirs[2].open(), &url, &b_token));
        Self { hub, _server: server, a, b, url, b_token, dirs }
    }

    fn a(&self) -> &SyncClient {
        self.a.as_ref().unwrap()
    }

    fn b(&self) -> &SyncClient {
        self.b.as_ref().unwrap()
    }

    fn vaults(&self) -> [&Vault; 3] {
        [self.hub.vault().as_ref(), self.a().vault().as_ref(), self.b().vault().as_ref()]
    }

    fn restart_b(&mut self) {
        self.b().vault().flush().unwrap();
        drop(self.b.take());
        self.b = Some(SyncClient::new(self.dirs[2].open(), &self.url, &self.b_token));
    }
}

fn insert(parent: Option<&str>, after: Option<&str>, id: &str, text: &str) -> BlockOp {
    BlockOp::Insert {
        parent: parent.map(str::to_owned),
        after: after.map(str::to_owned),
        block: NewBlock {
            id: id.into(), kind: BlockKind::Paragraph, text: text.into(),
            marks: vec![], checked: false,
        },
    }
}

fn text(id: &str, text: &str) -> BlockOp {
    BlockOp::SetText { id: id.into(), text: text.into(), marks: vec![] }
}

fn record(vault: &Vault, page: &str, id: &str) -> Conflict {
    let mut records = vault.list_conflicts(Some(page.into())).unwrap();
    records.extend(vault.conflict_log(Some(page.into())).unwrap());
    records.into_iter().find(|record| record.id == id).expect("conflict record survives")
}

fn assert_purged_with_log(vault: &Vault, page: &str, id: &str) -> Conflict {
    assert!(vault.purged_pages().contains(&page.to_owned()));
    assert!(matches!(vault.load_page(page.into()), Err(VaultError::NotFound(_))));
    assert!(!vault.page_versions().unwrap().iter().any(|version| version.page == page),
        "ordinary page versions describe loadable pages only");
    assert!(vault.sync_page_versions().unwrap().iter().any(|version| version.page == page),
        "retained history is still compared by sync");
    assert!(vault.list_conflicts(Some(page.into())).unwrap().iter().all(|record| record.id != id));
    let conflict = vault.conflict_log(Some(page.into())).unwrap().into_iter()
        .find(|record| record.id == id).expect("purged resolution remains in the log");
    assert!(conflict.undoable());
    assert_eq!(conflict.applied.last().unwrap().by, ResolutionBy::User);
    conflict
}

fn assert_pending_with_undone_log(vault: &Vault, page: &str, id: &str) -> Conflict {
    let pending = vault.list_conflicts(Some(page.into())).unwrap().into_iter()
        .find(|record| record.id == id).expect("ordinary undo reopens pending review");
    assert!(matches!(pending.status, ConflictStatus::Pending));
    assert!(!pending.undoable(), "the undone application has been consumed");
    let application = pending.applied.last().expect("undo keeps the resolution history");
    assert_eq!(application.by, ResolutionBy::User);
    assert!(application.undone_at.is_some(), "undo is marked on the retained application");
    let logged = vault.conflict_log(Some(page.into())).unwrap().into_iter()
        .find(|record| record.id == id).expect("reopened entry also keeps its undone log line");
    assert_eq!(logged, pending);
    pending
}

#[test]
fn block_choice_and_scoped_undo_sync_between_two_devices_and_the_hub() {
    let net = Network::new();
    let h = net.hub.vault();
    let page = h.create_page(None, "Conflict".into()).unwrap();
    h.apply(page.clone(), vec![
        insert(None, None, "target", "Original"),
        insert(None, Some("target"), "unrelated", "Unrelated"),
    ]).unwrap();
    net.a().sync_once().unwrap();
    net.b().sync_once().unwrap();
    net.a().vault().apply(page.clone(), vec![text("target", "Local")]).unwrap();
    h.apply(page.clone(), vec![text("target", "Cloud")]).unwrap();
    net.a().sync_once().unwrap();
    let conflict = net.a().vault().list_conflicts(Some(page.clone())).unwrap().into_iter()
        .find(|conflict| matches!(&conflict.scope, ConflictScope::Block { block } if block == "target"))
        .expect("concurrent edits have a block conflict");
    let side_index = conflict.sides.iter().position(|side| {
        matches!(&side.state, SideState::Present { content } if content[0].text == "Cloud")
    }).unwrap();
    let chosen = match &conflict.sides[side_index].state {
        SideState::Present { content } => content[0].clone(),
        _ => unreachable!(),
    };
    let before = net.a().vault().load_page(page.clone()).unwrap().blocks[0].clone();
    net.a().vault().resolve_conflict(page.clone(), conflict.id.clone(),
        ConflictResolution::UseSide(side_index as u32)).unwrap();
    net.a().sync_once().unwrap();
    net.b().sync_once().unwrap();
    net.a().sync_once().unwrap();
    for vault in net.vaults() {
        assert_eq!(vault.load_page(page.clone()).unwrap().blocks[0], chosen);
        assert!(vault.list_conflicts(Some(page.clone())).unwrap().iter().all(|c| c.id != conflict.id));
        assert_eq!(record(vault, &page, &conflict.id).applied.last().unwrap().by, ResolutionBy::User);
    }
    h.apply(page.clone(), vec![text("unrelated", "Later unrelated edit")]).unwrap();
    net.b().sync_once().unwrap();
    net.b().vault().undo_conflict(page.clone(), conflict.id.clone()).unwrap();
    net.b().sync_once().unwrap();
    net.a().sync_once().unwrap();
    net.b().sync_once().unwrap();
    for vault in net.vaults() {
        let restored = vault.load_page(page.clone()).unwrap();
        assert_eq!(restored.blocks[0], before);
        assert_eq!(restored.blocks[1].text, "Later unrelated edit");
        assert_pending_with_undone_log(vault, &page, &conflict.id);
    }
}

#[test]
fn purged_page_log_reaches_a_fresh_replica_and_undo_restores_the_full_before_page() {
    purged_page_choice_and_undo(false);
}

#[test]
fn purged_page_log_reaches_an_offline_warm_replica_and_undo_restores_the_full_before_page() {
    purged_page_choice_and_undo(true);
}

fn purged_page_choice_and_undo(warm_b: bool) {
    let mut net = Network::new();
    let h = net.hub.vault();
    let page = h.create_page(None, "Whole page".into()).unwrap();
    h.apply(page.clone(), vec![
        insert(None, None, "section", "Section"),
        insert(Some("section"), None, "kept", "The editing side kept this child"),
    ]).unwrap();
    net.a().sync_once().unwrap();
    if warm_b {
        net.b().sync_once().unwrap();
        let original = net.b().vault().load_page(page.clone()).unwrap();
        assert_eq!(original.blocks[0].text, "Section");
        assert_eq!(original.blocks[0].children[0].id, "kept");
    }
    // B now stays offline through the hub purge, A's editing-side restoration,
    // and UseSide(Purged). Cold B has no copy; warm B retains the original active
    // page, whose old baseline is deleted by the tombstone-first response before
    // the decision delta arrives. Both must import archive/log history without
    // interpreting retained page content as an active-page restoration.
    net.a().vault().apply(page.clone(), vec![text("section", "Offline section edit")]).unwrap();
    h.apply(page.clone(), vec![BlockOp::Delete { id: "kept".into() }]).unwrap();
    h.delete_page(page.clone()).unwrap();
    h.purge_page(page.clone()).unwrap();
    net.a().sync_once().unwrap();
    let conflict = net.a().vault().list_conflicts(Some(page.clone())).unwrap().into_iter()
        .find(|conflict| matches!(conflict.scope, ConflictScope::Page)
            && conflict.sides.iter().any(|side| matches!(side.state, SideState::Purged { .. })))
        .expect("purge versus offline editing is a page conflict");
    let before = net.a().vault().load_page(page.clone()).unwrap();
    assert!(!before.in_trash);
    assert_eq!(before.blocks[0].children[0].id, "kept");
    let purged_side = conflict.sides.iter().position(|side| matches!(side.state, SideState::Purged { .. })).unwrap();
    net.a().vault().resolve_conflict(page.clone(), conflict.id.clone(),
        ConflictResolution::UseSide(purged_side as u32)).unwrap();
    let old_archive = net.a().vault().export_updates(page.clone(), Vec::new()).unwrap();
    net.a().sync_once().unwrap();
    if warm_b {
        assert_eq!(net.b().vault().load_page(page.clone()).unwrap().blocks[0].text, "Section",
            "offline B still holds its original active baseline before reconnecting");
    }
    let first_b = net.b().sync_once().unwrap();
    assert_eq!(first_b.created, 0, "archive metadata must not create an active page");
    if warm_b {
        assert_eq!(first_b.purged, 1, "reconnect deletes B's old active baseline");
    }
    net.a().sync_once().unwrap();
    let expected = assert_purged_with_log(net.a().vault(), &page, &conflict.id);
    for vault in net.vaults() {
        assert_eq!(assert_purged_with_log(vault, &page, &conflict.id), expected);
        vault.import_updates(page.clone(), old_archive.clone()).unwrap();
        assert_purged_with_log(vault, &page, &conflict.id);
    }
    // Repeat incremental comparisons and reopen B. Both its
    // archive and the pending lifecycle tombstone must survive the restart.
    let repeated = net.b().sync_once().unwrap();
    assert_eq!((repeated.created, repeated.purged), (0, 0));
    net.restart_b();
    assert_eq!(assert_purged_with_log(net.b().vault(), &page, &conflict.id), expected);
    net.b().vault().undo_conflict(page.clone(), conflict.id.clone()).unwrap();
    net.b().sync_once().unwrap();
    net.a().sync_once().unwrap();
    net.b().sync_once().unwrap();
    for vault in net.vaults() {
        let restored = vault.load_page(page.clone()).unwrap();
        assert!(!restored.in_trash);
        assert_eq!(restored.title, before.title);
        assert_eq!(restored.blocks, before.blocks);
        assert!(!vault.purged_pages().contains(&page));
        assert!(vault.page_versions().unwrap().iter().any(|version| version.page == page));
        assert_pending_with_undone_log(vault, &page, &conflict.id);
    }
}
