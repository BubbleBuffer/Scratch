//! Purges and offline edits through a real hub over HTTP: the two orders a
//! purge and an edit can reach the hub in, legacy ID-only tombstones, and
//! replays and restarts. Also an old client on the v1 wire, which keeps the
//! delete-wins behaviour it always had.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use vault_core::*;
use vault_sync::wire::{PushRequest, PushResponse, SyncRequest, PROTOCOL};
use vault_sync::{Hub, HubRequest, HubResponse, HubServer, SyncClient};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("vault_purge_{tag}_{}", new_id()));
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn root(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
    fn open(&self) -> Arc<Vault> {
        Arc::new(Vault::open(self.root()).expect("open"))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn nb(id: &str, text: &str) -> NewBlock {
    NewBlock {
        id: id.into(),
        kind: BlockKind::Paragraph,
        text: text.into(),
        marks: vec![],
        checked: false,
    }
}

fn ins(after: Option<&str>, block: NewBlock) -> BlockOp {
    BlockOp::Insert {
        parent: None,
        after: after.map(Into::into),
        block,
    }
}

fn set_text(id: &str, text: &str) -> BlockOp {
    BlockOp::SetText {
        id: id.into(),
        text: text.into(),
        marks: vec![],
    }
}

/// Every page (id -> page) of a vault.
fn snapshot(v: &Vault) -> std::collections::BTreeMap<String, Page> {
    v.page_versions()
        .unwrap()
        .into_iter()
        .map(|p| (p.page.clone(), v.load_page(p.page).unwrap()))
        .collect()
}

fn texts(v: &Vault, p: &PageId) -> Vec<String> {
    v.load_page(p.clone())
        .unwrap()
        .blocks
        .into_iter()
        .map(|b| b.text)
        .collect()
}

/// A hub on a loopback port; devices are paired with it.
struct Net {
    _hub_dir: TempDir,
    hub: Arc<Hub>,
    hub_vault: Arc<Vault>,
    _server: HubServer,
    url: String,
}

impl Net {
    fn new() -> Net {
        let dir = TempDir::new("hub");
        let hub_vault = dir.open();
        let hub = Hub::new(hub_vault.clone()).unwrap();
        let server = hub.serve("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.addr());
        Net {
            _hub_dir: dir,
            hub,
            hub_vault,
            _server: server,
            url,
        }
    }

    fn device(&self, label: &str) -> Device {
        Device {
            dir: TempDir::new(label),
            url: self.url.clone(),
            token: self.hub.pair_legacy_token(label).unwrap(),
            client: None,
        }
        .started()
    }
}

/// A device: its own vault directory and a sync client for the hub.
struct Device {
    dir: TempDir,
    url: String,
    token: String,
    client: Option<SyncClient>,
}

impl Device {
    fn started(mut self) -> Device {
        self.client = Some(SyncClient::new(self.dir.open(), &self.url, &self.token));
        self
    }

    fn v(&self) -> &Vault {
        self.client.as_ref().unwrap().vault()
    }

    fn sync(&self) -> vault_sync::SyncReport {
        self.client.as_ref().unwrap().sync_once().unwrap()
    }

    /// Closes the vault and opens it again from disk, as after a restart.
    fn restart(&mut self) {
        self.v().flush().unwrap();
        self.client = None;
        self.client = Some(SyncClient::new(self.dir.open(), &self.url, &self.token));
    }
}

/// A page "T" with blocks "1" and "2", made on the hub and synced to the
/// given devices.
fn shared_page(net: &Net, devices: &[&Device]) -> PageId {
    let h = &net.hub_vault;
    let t = h.create_page(None, "T".into()).unwrap();
    h.apply(
        t.clone(),
        vec![ins(None, nb("1", "one")), ins(Some("1"), nb("2", "two"))],
    )
    .unwrap();
    for d in devices {
        d.sync();
    }
    t
}

/// The phone's offline edit: a rename and a changed first block.
fn phone_edit(a: &Device, t: &PageId) {
    a.v().rename_page(t.clone(), "T (phone)".into()).unwrap();
    a.v()
        .apply(t.clone(), vec![set_text("1", "one (phone)")])
        .unwrap();
}

/// The laptop's offline purge: it deletes block 2, trashes the page and
/// purges it. The phone never saw the deletion.
fn laptop_purge(b: &Device, t: &PageId) {
    b.v()
        .apply(t.clone(), vec![BlockOp::Delete { id: "2".into() }])
        .unwrap();
    b.v().delete_page(t.clone()).unwrap();
    b.v().purge_page(t.clone()).unwrap();
}

/// The page as the phone edited it, block 2 included (the laptop's delete
/// was never applied to it), live and whole.
fn assert_restored_whole(v: &Vault, t: &PageId) {
    let page = v.load_page(t.clone()).expect("the page is back");
    assert_eq!(page.title, "T (phone)");
    assert!(!page.in_trash);
    assert_eq!(texts(v, t), vec!["one (phone)", "two"]);
    assert!(v.purged_pages().is_empty());
    assert!(v.list_versions(t.clone()).unwrap().len() > 1);
    assert!(v
        .search("phone".into(), 20)
        .unwrap()
        .iter()
        .any(|hit| &hit.id == t));
}

fn assert_gone(v: &Vault, t: &PageId) {
    assert!(matches!(
        v.load_page(t.clone()),
        Err(VaultError::NotFound(_))
    ));
    assert_eq!(v.purged_pages(), vec![t.clone()]);
}

/// One v1 `POST` as an old client sends it; `token` is that client's.
fn post_v1(hub: &Hub, token: &str, path: &str, body: &[u8]) -> HubResponse {
    hub.handle(HubRequest {
        method: "POST".into(),
        path: path.into(),
        query: String::new(),
        headers: vec![
            ("Authorization".into(), format!("Bearer {token}")),
            ("X-Device".into(), "old-device".into()),
        ],
        body: Box::new(body),
        body_len: Some(body.len() as u64),
        cancel: None,
    })
}

#[test]
fn purger_first_restores_the_whole_editor_state() {
    let net = Net::new();
    let (a, b) = (net.device("phone"), net.device("laptop"));
    let t = shared_page(&net, &[&a, &b]);
    phone_edit(&a, &t);
    laptop_purge(&b, &t);
    // The purge reaches the hub first; the phone's edit is not covered by
    // it, so the page comes back whole from the phone's copy.
    b.sync();
    a.sync();
    b.sync();
    for v in [a.v(), b.v(), &*net.hub_vault] {
        assert_restored_whole(v, &t);
    }
    assert_eq!(snapshot(a.v()), snapshot(&net.hub_vault));
    assert_eq!(snapshot(b.v()), snapshot(&net.hub_vault));
}

#[test]
fn editor_first_restores_the_whole_editor_state() {
    let net = Net::new();
    let (a, b) = (net.device("phone"), net.device("laptop"));
    let t = shared_page(&net, &[&a, &b]);
    phone_edit(&a, &t);
    // The edit reaches the hub before the purge exists.
    a.sync();
    laptop_purge(&b, &t);
    // The hub's copy has the edit the purge does not cover: it is restored
    // there, and then on both devices.
    b.sync();
    a.sync();
    b.sync();
    for v in [a.v(), b.v(), &*net.hub_vault] {
        assert_restored_whole(v, &t);
    }
    assert_eq!(snapshot(a.v()), snapshot(&net.hub_vault));
    assert_eq!(snapshot(b.v()), snapshot(&net.hub_vault));
}

#[test]
fn purge_after_both_branches_merged_recovers_and_survives_restart() {
    let net = Net::new();
    let (mut a, b) = (net.device("phone"), net.device("laptop"));
    let t = shared_page(&net, &[&a, &b]);
    // The hub learns the deletion branch while the editor is offline.
    b.v()
        .apply(t.clone(), vec![BlockOp::Delete { id: "2".into() }])
        .unwrap();
    b.v().delete_page(t.clone()).unwrap();
    b.sync();
    phone_edit(&a, &t);
    a.sync();
    // Trash versus an offline edit applies the phone's whole page at detection:
    // the phone retained block 2 and never saw the laptop's deletion.
    assert_eq!(texts(&net.hub_vault, &t), vec!["one (phone)", "two"]);
    // The later purge still comes from the laptop's old deletion branch. Whole
    // editing-side restoration must survive that purge, replays and restart.
    b.v().purge_page(t.clone()).unwrap();
    let tombstones = b.v().purge_records();
    b.sync();
    a.sync();
    b.sync();
    for v in [a.v(), b.v(), &*net.hub_vault] {
        assert_restored_whole(v, &t);
        v.import_purge_records(tombstones.clone()).unwrap();
        assert_restored_whole(v, &t);
    }
    a.restart();
    a.sync();
    assert_restored_whole(a.v(), &t);
}

#[test]
fn a_stale_copy_stays_purged_through_replays_and_restarts() {
    let net = Net::new();
    let (mut a, mut b) = (net.device("phone"), net.device("laptop"));
    let t = shared_page(&net, &[&a, &b]);
    // No offline edit on the phone: its copy is covered by the purge.
    b.v().delete_page(t.clone()).unwrap();
    b.v().purge_page(t.clone()).unwrap();
    b.sync();
    assert_eq!(a.sync().purged, 1, "the stale copy is removed");
    b.sync();
    for v in [a.v(), b.v(), &*net.hub_vault] {
        assert_gone(v, &t);
    }
    // Replays: every device syncs again and nothing comes back.
    for _ in 0..2 {
        for d in [&a, &b] {
            let r = d.sync();
            assert_eq!((r.created, r.purged), (0, 0), "{r:?}");
        }
    }
    // Restarts: the tombstones are read back from disk and still hold.
    a.restart();
    b.restart();
    a.sync();
    b.sync();
    for v in [a.v(), b.v(), &*net.hub_vault] {
        assert_gone(v, &t);
    }
    assert_eq!(snapshot(a.v()), snapshot(&net.hub_vault));
}

/// The phone holds a page that an old client purged on the v1 wire (an
/// ID-only tombstone on the hub), and edits it offline.
fn legacy_purge_with_offline_edit(net: &Net) -> (Device, PageId) {
    let a = net.device("phone");
    let t = shared_page(net, &[&a]);
    let token = net.hub.pair_legacy_token("old").unwrap();
    let body = SyncRequest {
        full: true,
        purged: vec![t.clone()],
        ..SyncRequest::default()
    }
    .encode();
    assert_eq!(post_v1(&net.hub, &token, "/sync", &body).status, 200);
    phone_edit(&a, &t);
    (a, t)
}

#[test]
fn a_legacy_purge_brings_an_edited_copy_back_to_trash() {
    let net = Net::new();
    let (mut a, t) = legacy_purge_with_offline_edit(&net);
    a.sync();
    // The edited copy is kept in the trash, marked as came back after a
    // permanent delete. The hub holds the same copy.
    for v in [a.v(), &*net.hub_vault] {
        let page = v.load_page(t.clone()).expect("kept in the trash");
        assert!(page.in_trash);
        assert_eq!(page.title, "T (phone)");
        assert_eq!(texts(v, &t), vec!["one (phone)", "two"]);
        assert_eq!(v.restored_after_purge_pages().unwrap(), vec![t.clone()]);
        assert!(v.purged_pages().is_empty());
    }
    // The marker is durable: it survives a restart.
    a.restart();
    assert!(a.v().load_page(t.clone()).unwrap().in_trash);
    assert_eq!(a.v().restored_after_purge_pages().unwrap(), vec![t.clone()]);
    a.sync();
    assert_eq!(snapshot(a.v()), snapshot(&net.hub_vault));
}

#[test]
fn a_repurge_blocks_a_stale_snapshot_of_a_legacy_copy() {
    let net = Net::new();
    let (a, t) = legacy_purge_with_offline_edit(&net);
    a.sync();
    // The recovered copy as it stood before the second purge.
    let stale = a.v().export_updates(t.clone(), Vec::new()).unwrap();
    a.v().purge_page(t.clone()).unwrap();
    a.sync();
    a.sync();
    for v in [a.v(), &*net.hub_vault] {
        assert_gone(v, &t);
        // Replayed, the stale snapshot is covered by the purge: skipped.
        let r = v.import_updates(t.clone(), stale.clone()).unwrap();
        assert_eq!((r.created, r.skipped), (0, 1));
        assert!(v.load_page(t.clone()).is_err());
    }
    // The legacy record, replayed after its restoration, changes nothing.
    let legacy = PurgeRecord {
        page: t.clone(),
        version: None,
        device: None,
    };
    assert_eq!(a.v().import_purge_records(vec![legacy]).unwrap(), 0);
    let r = a.sync();
    assert_eq!((r.created, r.purged), (0, 0), "{r:?}");
    assert_gone(a.v(), &t);
}

#[test]
fn an_old_client_on_the_v1_wire_keeps_delete_wins() {
    let net = Net::new();
    let a = net.device("phone");
    let t = shared_page(&net, &[&a]);
    // The old client's copy has an edit the hub never saw.
    let old_dir = TempDir::new("old");
    let old = old_dir.open();
    let snap = net.hub_vault.export_updates(t.clone(), Vec::new()).unwrap();
    old.import_updates(t.clone(), snap).unwrap();
    old.rename_page(t.clone(), "T (old client)".into()).unwrap();
    let edited = old.export_updates(t.clone(), Vec::new()).unwrap();

    let token = net.hub.pair_legacy_token("old").unwrap();
    let body = SyncRequest {
        full: true,
        purged: vec![t.clone()],
        ..SyncRequest::default()
    }
    .encode();
    let r = post_v1(&net.hub, &token, "/sync", &body);
    assert_eq!(r.status, 200);
    assert_eq!(r.body[0], PROTOCOL, "v1 answer");
    assert!(matches!(
        net.hub_vault.load_page(t.clone()),
        Err(VaultError::NotFound(_))
    ));

    // Its v1 push of the edited copy is dropped, as it always was.
    let body = PushRequest {
        updates: vec![(t.clone(), edited)],
    }
    .encode();
    let r = post_v1(&net.hub, &token, "/push", &body);
    let pushed = PushResponse::decode(&r.body).unwrap();
    assert_eq!((pushed.created, pushed.changed), (0, 0));
    assert!(matches!(
        net.hub_vault.load_page(t.clone()),
        Err(VaultError::NotFound(_))
    ));
    assert_eq!(net.hub_vault.purged_pages(), vec![t.clone()]);
    assert!(net.hub_vault.restore_records().is_empty());
}

/// An old client's vault as the old code left it: it purged the page, and
/// `.vault/purged` holds only the ID. Its v1 syncs send that list.
fn old_client_purge_set(net: &Net, t: &PageId) -> TempDir {
    let dir = TempDir::new("old_purge_set");
    {
        let old = dir.open();
        let snap = net.hub_vault.export_updates(t.clone(), Vec::new()).unwrap();
        old.import_updates(t.clone(), snap).unwrap();
        old.delete_page(t.clone()).unwrap();
        old.purge_page(t.clone()).unwrap();
        old.flush().unwrap();
    }
    // The old code wrote no purge-state file, only the ID list.
    fs::remove_file(dir.0.join(".vault").join("purge-state.json")).unwrap();
    dir
}

/// The IDs an old client sends, read from its vault as it is on disk now.
fn persisted_purges(dir: &TempDir) -> Vec<PageId> {
    dir.open().purged_pages()
}

/// Stops the hub and opens its vault again. Pairing tokens live in the
/// vault, so the devices' tokens still work after the restart.
fn restart_hub(net: Net) -> (TempDir, Arc<Hub>, Arc<Vault>) {
    let Net {
        _hub_dir: dir,
        hub,
        hub_vault,
        _server: server,
        ..
    } = net;
    hub_vault.flush().unwrap();
    drop(server);
    drop(hub);
    drop(hub_vault);
    let vault = dir.open();
    let hub = Hub::new(vault.clone()).unwrap();
    (dir, hub, vault)
}

#[test]
fn an_old_clients_persisted_purges_never_remove_a_restored_page() {
    let net = Net::new();
    let (a, b) = (net.device("phone"), net.device("laptop"));
    let t = shared_page(&net, &[&a, &b]);
    let old_dir = old_client_purge_set(&net, &t);
    phone_edit(&a, &t);
    laptop_purge(&b, &t);
    // The purge reaches the hub first; the phone's unseen edit then restores
    // the page live, whole, on the hub and both devices.
    b.sync();
    a.sync();
    b.sync();
    for v in [a.v(), b.v(), &*net.hub_vault] {
        assert_restored_whole(v, &t);
    }
    assert_eq!(snapshot(a.v()), snapshot(&net.hub_vault));

    // The old client's v1 syncs replay its persisted purge set, again and
    // again. Each replay leaves the restored page alone.
    let token = net.hub.pair_legacy_token("old").unwrap();
    assert_eq!(persisted_purges(&old_dir), vec![t.clone()]);
    for _ in 0..3 {
        // Reopening the old vault discards its in-memory sync base, so every
        // connection sends the entire persisted purge set again.
        let body = SyncRequest {
            full: true,
            purged: persisted_purges(&old_dir),
            ..SyncRequest::default()
        }
        .encode();
        assert_eq!(post_v1(&net.hub, &token, "/sync", &body).status, 200);
        assert_restored_whole(&net.hub_vault, &t);
        a.sync();
        assert_restored_whole(a.v(), &t);
    }

    // A covering purge made on the laptop after the restore. It is not sent
    // yet: the hub is about to restart.
    b.v().delete_page(t.clone()).unwrap();
    b.v().purge_page(t.clone()).unwrap();
    let repurge = b.v().purge_records();
    assert!(repurge[0].version.is_some());

    // Restart the hub: the restoration history is read back from disk, and
    // the old client's replays still change nothing.
    let (_dir, hub, hub_vault) = restart_hub(net);
    let server = hub.serve("127.0.0.1:0").unwrap();
    let editor = SyncClient::new(
        a.client.as_ref().unwrap().vault().clone(),
        &format!("http://{}", server.addr()),
        &a.token,
    );
    assert_restored_whole(&hub_vault, &t);
    let body = SyncRequest {
        full: true,
        purged: persisted_purges(&old_dir),
        ..SyncRequest::default()
    }
    .encode();
    for _ in 0..2 {
        assert_eq!(post_v1(&hub, &token, "/sync", &body).status, 200);
        assert_restored_whole(&hub_vault, &t);
        editor.sync_once().unwrap();
        assert_restored_whole(a.v(), &t);
    }

    // The new-style covering purge still removes the restored copy.
    assert_eq!(hub_vault.import_purge_records(repurge).unwrap(), 1);
    assert_gone(&hub_vault, &t);
    // After that, the old client's replay is a no-op on a purged page.
    assert_eq!(post_v1(&hub, &token, "/sync", &body).status, 200);
    assert_gone(&hub_vault, &t);
}

#[test]
fn an_unedited_legacy_copy_returns_to_marked_trash_through_the_hub() {
    let net = Net::new();
    let mut a = net.device("phone");
    let t = shared_page(&net, &[&a]);
    // An old client's ID-only purge reaches the hub first. The phone's copy
    // has no offline edit.
    let token = net.hub.pair_legacy_token("old").unwrap();
    let body = SyncRequest {
        full: true,
        purged: vec![t.clone()],
        ..SyncRequest::default()
    }
    .encode();
    assert_eq!(post_v1(&net.hub, &token, "/sync", &body).status, 200);
    assert!(matches!(
        net.hub_vault.load_page(t.clone()),
        Err(VaultError::NotFound(_))
    ));
    // The phone receives the tombstone and restores its copy into the trash,
    // marked. The copy reaches the hub, which holds the tombstone, and is
    // restored the same way. The second sync carries the restored copy.
    a.sync();
    a.sync();
    for v in [a.v(), &*net.hub_vault] {
        let page = v.load_page(t.clone()).expect("kept in the trash");
        assert!(page.in_trash);
        assert_eq!(page.title, "T");
        assert_eq!(texts(v, &t), vec!["one", "two"]);
        assert_eq!(v.restored_after_purge_pages().unwrap(), vec![t.clone()]);
        assert!(v.purged_pages().is_empty());
    }
    // A replay of the legacy purge after the restoration changes nothing.
    assert_eq!(post_v1(&net.hub, &token, "/sync", &body).status, 200);
    assert!(net.hub_vault.load_page(t.clone()).unwrap().in_trash);
    assert!(net.hub_vault.purged_pages().is_empty());
    // The marker and the cleared tombstone survive a restart.
    a.restart();
    assert!(a.v().load_page(t.clone()).unwrap().in_trash);
    assert_eq!(a.v().restored_after_purge_pages().unwrap(), vec![t.clone()]);
    assert!(a.v().purged_pages().is_empty());
    a.sync();
    assert_eq!(snapshot(a.v()), snapshot(&net.hub_vault));
}
