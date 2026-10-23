//! Two devices and a hub in one process, over real HTTP on localhost.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use vault_core::*;
use vault_sync::{Hub, HubRequest, HubServer, SyncClient, SyncError, SyncOptions, DEVICE_HEADER};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("vault_hub_{tag}_{}", new_id()));
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
fn snapshot(v: &Vault) -> BTreeMap<String, Page> {
    v.page_versions()
        .unwrap()
        .into_iter()
        .map(|p| (p.page.clone(), v.load_page(p.page).unwrap()))
        .collect()
}

fn texts(v: &Vault, p: &str) -> Vec<String> {
    v.load_page(p.to_string())
        .unwrap()
        .blocks
        .into_iter()
        .map(|b| b.text)
        .collect()
}

struct Setup {
    _dirs: Vec<TempDir>,
    hub_vault: Arc<Vault>,
    hub: Arc<Hub>,
    server: HubServer,
    a: Arc<SyncClient>,
    b: Arc<SyncClient>,
}

fn setup() -> Setup {
    let (th, ta, tb) = (TempDir::new("hub"), TempDir::new("a"), TempDir::new("b"));
    let hub_vault = th.open();
    let hub = Hub::new(hub_vault.clone()).unwrap();
    let server = hub.serve("127.0.0.1:0").unwrap();
    let url = format!("http://{}", server.addr());
    let a = Arc::new(SyncClient::new(
        ta.open(),
        &url,
        &hub.pair("phone").unwrap(),
    ));
    let b = Arc::new(SyncClient::new(
        tb.open(),
        &url,
        &hub.pair("laptop").unwrap(),
    ));
    Setup {
        _dirs: vec![th, ta, tb],
        hub_vault,
        hub,
        server,
        a,
        b,
    }
}

fn wait_for(what: &str, mut f: impl FnMut() -> bool, diagnose: impl Fn() -> String) {
    let start = Instant::now();
    while !f() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timed out: {what}; {}",
            diagnose()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn two_devices_sync_through_a_hub() {
    let s = setup();
    let (h, a, b) = (&s.hub_vault, s.a.vault(), s.b.vault());

    // Pages made on the hub reach both devices.
    let x = h.create_page(None, "X".into()).unwrap();
    h.apply(
        x.clone(),
        vec![ins(None, nb("1", "one")), ins(Some("1"), nb("2", "two"))],
    )
    .unwrap();
    let m = h.create_page(None, "M".into()).unwrap();
    let t = h.create_page(None, "T".into()).unwrap();
    let r = s.a.sync_once().unwrap();
    assert!(r.full);
    assert_eq!(r.created, 3);
    s.b.sync_once().unwrap();
    assert_eq!(snapshot(a), snapshot(h));
    assert_eq!(snapshot(b), snapshot(h));

    // Nothing to do: a delta sync of one round that moves no page.
    let r = s.a.sync_once().unwrap();
    assert!(!r.full);
    assert_eq!(r.rounds, 1);
    assert_eq!((r.created, r.changed, r.pushed), (0, 0, 0));

    // Offline edits on every side.
    a.apply(x.clone(), vec![set_text("1", "one (phone)")])
        .unwrap();
    let n = a
        .create_page(Some(x.clone()), "New on phone".into())
        .unwrap();
    b.apply(x.clone(), vec![set_text("2", "two (laptop)")])
        .unwrap();
    b.delete_page(t.clone()).unwrap();
    h.move_page(m.clone(), Some(x.clone()), None).unwrap();
    let r = s.a.sync_once().unwrap();
    assert!(!r.full);
    assert!(r.pushed >= 2, "{r:?}");
    s.b.sync_once().unwrap();
    s.a.sync_once().unwrap();
    for v in [a, b, h] {
        assert_eq!(texts(v, &x), vec!["one (phone)", "two (laptop)"]);
        assert_eq!(
            v.load_page(m.clone()).unwrap().parent.as_deref(),
            Some(x.as_str())
        );
        assert_eq!(
            v.load_page(n.clone()).unwrap().parent.as_deref(),
            Some(x.as_str())
        );
        assert!(v.load_page(t.clone()).unwrap().in_trash);
        assert!(v.list_conflicts(None).unwrap().is_empty());
    }
    assert_eq!(snapshot(a), snapshot(h));
    assert_eq!(snapshot(b), snapshot(h));

    // Attachments: uploaded with the page, fetched lazily elsewhere.
    let png = vec![0x89, b'P', b'N', b'G', 1, 2, 3];
    let img = a
        .add_attachment_bytes("Grüße.png".into(), png.clone())
        .unwrap();
    let p = a.create_page(None, "Photo".into()).unwrap();
    a.apply(
        p.clone(),
        vec![
            ins(
                None,
                NewBlock {
                    kind: BlockKind::Image,
                    ..nb("i", "")
                },
            ),
            BlockOp::SetTarget {
                id: "i".into(),
                target: Some(img.clone()),
            },
        ],
    )
    .unwrap();
    let r = s.a.sync_once().unwrap();
    assert_eq!(r.blobs_sent, 1);
    assert_eq!(h.attachment_info(img.clone()).unwrap().name, "Grüße.png");
    s.b.sync_once().unwrap();
    b.flush().unwrap();
    assert_eq!(b.missing_attachments().unwrap(), vec![img.clone()]);
    assert_eq!(s.b.fetch_missing_attachments().unwrap(), 1);
    assert_eq!(
        fs::read(b.attachment_path(img.clone()).unwrap()).unwrap(),
        png
    );
    assert_eq!(b.attachment_info(img.clone()).unwrap().mime, "image/png");
    // Known to the hub now: not offered again.
    assert_eq!(s.a.sync_once().unwrap().blobs_sent, 0);

    // A conflict: both devices change one block.
    a.apply(x.clone(), vec![set_text("1", "phone wins?")])
        .unwrap();
    b.apply(x.clone(), vec![set_text("1", "laptop wins?")])
        .unwrap();
    s.a.sync_once().unwrap();
    let r = s.b.sync_once().unwrap();
    assert_eq!(r.conflicts, 1);
    s.a.sync_once().unwrap();
    for v in [a, b, h] {
        let cs = v.list_conflicts(None).unwrap();
        assert_eq!(cs.len(), 1, "{cs:?}");
        assert_eq!(cs[0].block, "1");
        assert_eq!(cs[0].sides[0].device, b.device_id());
        assert_eq!(cs[0].sides[1].device, a.device_id());
    }
    let conflict_id = a.list_conflicts(Some(x.clone())).unwrap()[0].id.clone();
    a.resolve_conflict(x.clone(), conflict_id, ConflictResolution::UseSide(0))
        .unwrap();
    s.a.sync_once().unwrap();
    s.b.sync_once().unwrap();
    for v in [a, b, h] {
        assert!(v.list_conflicts(None).unwrap().is_empty());
        assert_eq!(texts(v, &x)[0], "laptop wins?");
    }

    // The laptop purges the page; the phone's offline rename, which the
    // laptop never saw, is not covered by the purge. The page comes back
    // whole and live on every device instead of being deleted.
    a.rename_page(t.clone(), "T (edited on phone)".into())
        .unwrap();
    b.purge_page(t.clone()).unwrap();
    s.b.sync_once().unwrap();
    let r = s.a.sync_once().unwrap();
    assert_eq!(r.purged, 0, "restored, not deleted: {r:?}");
    s.b.sync_once().unwrap();
    for v in [a, b, h] {
        let page = v.load_page(t.clone()).expect("the page is back");
        assert_eq!(page.title, "T (edited on phone)");
        assert!(!page.in_trash);
        assert!(v.purged_pages().is_empty());
    }
    assert_eq!(snapshot(a), snapshot(h));
    assert_eq!(snapshot(b), snapshot(h));
    drop(s.server);
}

#[test]
fn small_budgets_take_several_rounds() {
    let s = setup();
    let h = &s.hub_vault;
    for i in 0..20 {
        let p = h.create_page(None, format!("Page {i}")).unwrap();
        h.apply(p, vec![ins(None, nb("b", &"text ".repeat(50)))])
            .unwrap();
    }
    let mut small = SyncClient::new(
        s.a.vault().clone(),
        &format!("http://{}", s.server.addr()),
        &s.hub.pair("small").unwrap(),
    );
    small.set_budget(1000);
    let r = small.sync_once().unwrap();
    assert!(r.rounds > 3, "{r:?}");
    assert_eq!(r.created, 20);
    assert_eq!(snapshot(s.a.vault()), snapshot(h));
    // The phone pushes many pages in several requests.
    for i in 0..10 {
        s.a.vault().create_page(None, format!("Phone {i}")).unwrap();
    }
    small.sync_once().unwrap();
    assert_eq!(h.page_count().unwrap(), 30);
}

#[test]
fn tokens_bind_to_one_device() {
    let s = setup();
    let url = format!("http://{}", s.server.addr());
    let bad = SyncClient::new(s.a.vault().clone(), &url, "not-a-token");
    assert!(matches!(bad.sync_once(), Err(SyncError::Status(401, _))));
    // The phone's token, once used by the phone, fails for the laptop.
    let token = s.hub.pair("shared").unwrap();
    SyncClient::new(s.a.vault().clone(), &url, &token)
        .sync_once()
        .unwrap();
    let other = SyncClient::new(s.b.vault().clone(), &url, &token);
    assert!(matches!(other.sync_once(), Err(SyncError::Status(403, _))));
    // A copy of the hub's vault (same device id) is refused.
    let copy = TempDir::new("copy");
    fs::create_dir_all(copy.0.join(".vault")).unwrap();
    fs::write(copy.0.join(".vault/device"), s.hub_vault.device_id()).unwrap();
    let c = SyncClient::new(copy.open(), &url, &s.hub.pair("copy").unwrap());
    assert!(matches!(c.sync_once(), Err(SyncError::Status(409, _))));
    let devices = s.hub.devices();
    assert_eq!(devices.len(), 4);
    assert_eq!(s.hub.unpair("copy").unwrap(), 1);
}

#[test]
fn hub_restart_leads_to_a_full_sync() {
    let s = setup();
    let h = s.hub_vault.clone();
    let p = h.create_page(None, "Before".into()).unwrap();
    let r = s.a.sync_once().unwrap();
    assert!(r.full);
    let addr = s.server.addr();
    drop(s.server);
    // A new hub run on the same vault and address: a new epoch, so the
    // client's delta base is refused and it syncs in full.
    let hub2 = Hub::new(h.clone()).unwrap();
    // tiny_http closes the old listener on its own thread: retry briefly.
    let mut tries = 0;
    let _server = loop {
        match hub2.serve(&addr.to_string()) {
            Ok(s) => break s,
            Err(e) if tries < 40 => {
                tries += 1;
                let _ = e;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => panic!("{e}"),
        }
    };
    h.rename_page(p.clone(), "After".into()).unwrap();
    let r = s.a.sync_once().unwrap();
    assert!(r.full, "{r:?}");
    assert_eq!(r.changed, 1);
    assert_eq!(s.a.vault().load_page(p.clone()).unwrap().title, "After");
    // Then deltas again.
    h.rename_page(p.clone(), "Again".into()).unwrap();
    let r = s.a.sync_once().unwrap();
    assert!(!r.full);
    assert_eq!(r.changed, 1);
}

#[test]
fn background_sync_follows_changes_on_both_sides() {
    let s = setup();
    let opts = SyncOptions {
        debounce_ms: 50,
        max_delay_ms: 200,
        interval_ms: 60_000,
        long_poll_ms: 2_000,
        prefetch_attachments: true,
    };
    let bg_a = s.a.start(opts.clone());
    let bg_b = s.b.start(opts);
    let (a, b) = (s.a.vault().clone(), s.b.vault().clone());
    let p = a.create_page(None, "Live".into()).unwrap();
    let diagnose = || {
        let blocks = |v: &Vault| {
            v.load_page(p.clone()).map(|page| {
                page.blocks
                    .into_iter()
                    .map(|block| (block.id, block.text))
                    .collect::<Vec<_>>()
            })
        };
        format!(
            "a_status={:?}; b_status={:?}; a_blocks={:?}; hub_blocks={:?}; b_blocks={:?}",
            bg_a.status(),
            bg_b.status(),
            blocks(&a),
            blocks(&s.hub_vault),
            blocks(&b)
        )
    };
    wait_for(
        "page reaches b",
        || b.load_page(p.clone()).is_ok(),
        &diagnose,
    );
    b.apply(p.clone(), vec![ins(None, nb("1", "from b"))])
        .unwrap();
    wait_for(
        "edit reaches a",
        || {
            a.load_page(p.clone())
                .map(|pg| pg.blocks.len() == 1)
                .unwrap_or(false)
        },
        &diagnose,
    );
    let img = b
        .add_attachment_bytes("x.txt".into(), b"hello".to_vec())
        .unwrap();
    b.apply(
        p.clone(),
        vec![
            ins(
                Some("1"),
                NewBlock {
                    kind: BlockKind::File,
                    ..nb("f", "x.txt")
                },
            ),
            BlockOp::SetTarget {
                id: "f".into(),
                target: Some(img.clone()),
            },
        ],
    )
    .unwrap();
    wait_for(
        "attachment prefetched on a",
        || a.attachment_path(img.clone()).is_ok(),
        &diagnose,
    );
    let st = bg_a.status();
    assert!(st.last_error.is_none(), "{st:?}");
    assert!(st.syncs >= 2);
    bg_a.stop();
    bg_b.stop();
}

#[test]
fn index_only_churn_keeps_hub_change_cursor_and_transport_history_unchanged() {
    let s = setup();
    let token = s.hub.pair("index-observer").unwrap();
    let observer_device = new_id();
    let changes = |query: String| {
        let response = s.hub.handle(HubRequest {
            method: "GET".into(),
            path: "/changes".into(),
            query,
            headers: vec![
                ("Authorization".into(), format!("Bearer {token}")),
                (DEVICE_HEADER.into(), observer_device.clone()),
            ],
            body: Box::new(std::io::Cursor::new(Vec::<u8>::new())),
            body_len: Some(0),
            cancel: None,
        });
        assert_eq!(response.status, 200);
        vault_sync::wire::ChangesResponse::decode(&response.body).unwrap()
    };
    let page = s
        .hub_vault
        .create_page(None, "Retained change".into())
        .unwrap();
    s.hub_vault.flush().unwrap();
    let before = changes("wait=0".into());
    let transport = s.hub_vault.poll_sync_changes(0);
    // The raw window holds 10k entries. Even enough index-only commits to
    // overflow it must not evict document transport history or signal sync.
    for _ in 0..10_005 {
        s.hub_vault
            .delete_search_row("test".into(), "already-absent".into())
            .unwrap();
    }
    assert!(s.hub_vault.poll_changes(0).reset);
    assert_eq!(s.hub_vault.poll_sync_changes(0), transport);
    let after = changes(format!("since={}:{}&wait=0", before.epoch, before.seq));
    assert_eq!((after.epoch, after.seq), (before.epoch, before.seq));
    let start = Instant::now();
    let waited = changes(format!("since={}:{}&wait=100", before.epoch, before.seq));
    assert!(start.elapsed() >= Duration::from_millis(90));
    assert_eq!((waited.epoch, waited.seq), (before.epoch, before.seq));
    let report = s.a.sync_once().unwrap();
    assert_eq!(report.created, 1);
    assert_eq!(
        s.a.vault().load_page(page).unwrap().title,
        "Retained change"
    );
}

#[test]
fn background_sync_does_not_run_for_local_or_hub_index_only_commits() {
    let s = setup();
    let bg = s.a.start(SyncOptions {
        debounce_ms: 30,
        max_delay_ms: 100,
        interval_ms: 60_000,
        long_poll_ms: 250,
        prefetch_attachments: false,
    });
    wait_for(
        "startup sync and initial remote notification",
        || {
            let status = bg.status();
            status.syncs >= 2 && !status.syncing
        },
        || format!("{:?}", bg.status()),
    );
    let before = bg.status().syncs;
    for vault in [s.a.vault(), &s.hub_vault] {
        vault
            .upsert_search_row(
                "test".into(),
                ExternalSearchRow {
                    id: "index-only".into(),
                    title: "External".into(),
                    body: "preview".into(),
                    page_type: "chat".into(),
                    target: None,
                    updated_ms: 1,
                },
            )
            .unwrap();
    }
    // Several remote long-poll intervals and local debounce windows elapse.
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(bg.status().syncs, before, "{:?}", bg.status());
    assert!(!bg.status().syncing);
    bg.stop();
}
