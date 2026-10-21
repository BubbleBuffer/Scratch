//! Generic add-on data on pages (`type_data`): map keys and list items,
//! two replicas, reopen, Markdown export/import and rescan, invalid writes and
//! typed creation or conversion.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde_json::{json, Value};
use vault_core::*;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("vault_type_data_{tag}_{}", new_id()));
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn root(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
    fn pages(&self) -> String {
        self.0.join("pages").to_string_lossy().into_owned()
    }
    fn open(&self) -> Vault {
        Vault::open(self.root()).expect("open")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const KANBAN: &str = "kanban";

fn path(segs: &[&str]) -> Vec<String> {
    segs.iter().map(|s| s.to_string()).collect()
}

fn parse(json_text: &str) -> Value {
    serde_json::from_str(json_text).unwrap()
}

fn data(v: &Vault, page: &str, addon: &str) -> Value {
    parse(&v.type_data(page.into(), addon.into()).unwrap())
}

fn all(v: &Vault, page: &str) -> Value {
    parse(&v.type_data_all(page.into()).unwrap())
}

fn set(v: &Vault, page: &str, addon: &str, p: &[&str], json_value: &str) -> Result<()> {
    v.type_data_set(page.into(), addon.into(), path(p), json_value.into())
}

fn list_insert(v: &Vault, page: &str, p: &[&str], index: Option<u32>, json_value: &str) -> Result<()> {
    v.type_data_list_insert(page.into(), KANBAN.into(), path(p), index, json_value.into())
}

fn ids(list: &Value) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|x| match x {
            Value::String(s) => s.clone(),
            other => other["id"].as_str().unwrap().to_string(),
        })
        .collect()
}

fn versions(v: &Vault, page: &str) -> Vec<u8> {
    v.page_versions()
        .unwrap()
        .into_iter()
        .find(|x| x.page == page)
        .unwrap()
        .version
}

fn prop(v: &Vault, page: &str, key: &str) -> Option<PropValue> {
    v.load_page(page.into())
        .unwrap()
        .properties
        .into_iter()
        .find(|x| x.key == key)
        .map(|x| x.value)
}

/// Sends `from`'s changes that `to` lacks.
fn push(from: &Vault, to: &Vault) {
    let have: HashMap<String, Vec<u8>> = to
        .page_versions()
        .unwrap()
        .into_iter()
        .map(|v| (v.page, v.version))
        .collect();
    let mut updates = Vec::new();
    for v in from.page_versions().unwrap() {
        let theirs = have.get(&v.page).cloned().unwrap_or_default();
        if !version_includes(&theirs, &v.version).unwrap() {
            let bytes = from.export_updates(v.page.clone(), theirs).unwrap();
            updates.push(PageUpdate {
                page: v.page,
                bytes,
            });
        }
    }
    to.import_updates_batch(updates).unwrap();
}

fn sync(a: &Vault, b: &Vault) {
    push(a, b);
    push(b, a);
}

/// Two replicas of one page with no add-on data; `b` gets the page by sync.
fn replicas(ta: &TempDir, tb: &TempDir) -> (Vault, Vault, String) {
    let a = ta.open();
    let page = a.create_page(None, "Board".into()).unwrap();
    let b = tb.open();
    push(&a, &b);
    (a, b, page)
}

fn md_file(t: &TempDir, id: &str) -> PathBuf {
    fs::read_dir(t.0.join("pages"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(&format!("-{id}.md")))
        .expect("markdown file")
}

/// The raw JSON text of the `type-data:` line in an exported file.
fn raw_type_data(text: &str) -> &str {
    text.lines()
        .find_map(|l| l.strip_prefix("type-data: "))
        .expect("type-data line")
}

/// Rewrites the file's `type-data:` line with `json_value`.
fn rewrite_type_data(file: &PathBuf, json_value: &str) {
    let text = fs::read_to_string(file).unwrap();
    let edited = text
        .lines()
        .map(|l| {
            if l.starts_with("type-data: ") {
                format!("type-data: {json_value}")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(file, edited + "\n").unwrap();
}

#[test]
fn map_keys_set_read_and_delete() {
    let t = TempDir::new("map");
    let v = t.open();
    let p = v.create_page(None, "Page".into()).unwrap();
    assert_eq!(v.type_data(p.clone(), KANBAN.into()).unwrap(), "{}");
    assert_eq!(v.type_data_all(p.clone()).unwrap(), "{}");

    set(&v, &p, KANBAN, &["title"], "\"Work\"").unwrap();
    // Missing intermediate maps are created by set.
    set(&v, &p, KANBAN, &["view", "wip", "limit"], "3").unwrap();
    set(&v, &p, KANBAN, &["view", "wip", "open"], "true").unwrap();
    set(&v, &p, "mod:other", &["n"], "null").unwrap();
    assert_eq!(
        data(&v, &p, KANBAN),
        json!({"title": "Work", "view": {"wip": {"limit": 3, "open": true}}})
    );
    assert_eq!(
        all(&v, &p),
        json!({KANBAN: data(&v, &p, KANBAN), "mod:other": {"n": null}})
    );

    // Replacing a key's value with another kind is a plain write.
    set(&v, &p, KANBAN, &["view"], "[1, 2]").unwrap();
    assert_eq!(data(&v, &p, KANBAN)["view"], json!([1, 2]));

    // A deleted key that held a list or map comes back empty when set again.
    set(&v, &p, KANBAN, &["stale"], "[1, 2, 3]").unwrap();
    set(&v, &p, KANBAN, &["obj"], r#"{"a": 1}"#).unwrap();
    v.type_data_delete(p.clone(), KANBAN.into(), path(&["stale"])).unwrap();
    v.type_data_delete(p.clone(), KANBAN.into(), path(&["obj"])).unwrap();
    set(&v, &p, KANBAN, &["stale"], "[9]").unwrap();
    set(&v, &p, KANBAN, &["obj"], r#"{"b": 2}"#).unwrap();
    assert_eq!(data(&v, &p, KANBAN)["stale"], json!([9]));
    assert_eq!(data(&v, &p, KANBAN)["obj"], json!({"b": 2}));

    v.type_data_delete(p.clone(), KANBAN.into(), path(&["title"])).unwrap();
    assert!(data(&v, &p, KANBAN).get("title").is_none());
    v.type_data_delete(p.clone(), "mod:other".into(), path(&["n"])).unwrap();
    assert_eq!(data(&v, &p, "mod:other"), json!({}));
    // Deleting a key that is not there is an error.
    assert!(matches!(
        v.type_data_delete(p.clone(), KANBAN.into(), path(&["title"])),
        Err(VaultError::NotFound(_))
    ));
}

#[test]
fn list_items_insert_delete_and_move_by_index_and_id() {
    let t = TempDir::new("list");
    let v = t.open();
    let p = v.create_page(None, "Board".into()).unwrap();
    set(&v, &p, KANBAN, &["cards"], "[]").unwrap();
    list_insert(&v, &p, &["cards"], None, r#"{"id":"c1","title":"a"}"#).unwrap();
    list_insert(&v, &p, &["cards"], None, r#"{"id":"c2","title":"b"}"#).unwrap();
    list_insert(&v, &p, &["cards"], Some(0), r#"{"id":"c0","title":"z"}"#).unwrap();
    assert_eq!(ids(&data(&v, &p, KANBAN)["cards"]), ["c0", "c1", "c2"]);

    // A path runs through a list item selected by its id.
    set(&v, &p, KANBAN, &["cards", "c2", "items"], "[]").unwrap();
    list_insert(&v, &p, &["cards", "c2", "items"], None, "\"t1\"").unwrap();
    set(&v, &p, KANBAN, &["cards", "c1", "title"], "\"renamed\"").unwrap();
    assert_eq!(data(&v, &p, KANBAN)["cards"][2]["items"], json!(["t1"]));
    assert_eq!(data(&v, &p, KANBAN)["cards"][1]["title"], "renamed");

    // Delete by id: a map item by its id, a scalar item by its text.
    list_insert(&v, &p, &["cards"], None, "\"loose\"").unwrap();
    v.type_data_list_delete(p.clone(), KANBAN.into(), path(&["cards"]), ListItem::Id("loose".into()))
        .unwrap();
    v.type_data_list_delete(p.clone(), KANBAN.into(), path(&["cards"]), ListItem::Id("c0".into()))
        .unwrap();
    assert_eq!(ids(&data(&v, &p, KANBAN)["cards"]), ["c1", "c2"]);

    // Move: the item ends at `to_index`.
    v.type_data_list_move(
        p.clone(),
        KANBAN.into(),
        path(&["cards"]),
        ListItem::Id("c2".into()),
        0,
    )
    .unwrap();
    assert_eq!(ids(&data(&v, &p, KANBAN)["cards"]), ["c2", "c1"]);
    v.type_data_list_move(p.clone(), KANBAN.into(), path(&["cards"]), ListItem::Index(0), 1)
        .unwrap();
    assert_eq!(ids(&data(&v, &p, KANBAN)["cards"]), ["c1", "c2"]);
    // Moving an item to its own place changes nothing.
    let before = versions(&v, &p);
    v.type_data_list_move(p.clone(), KANBAN.into(), path(&["cards"]), ListItem::Index(1), 1)
        .unwrap();
    assert_eq!(versions(&v, &p), before);

    // The moved item keeps its content.
    assert_eq!(data(&v, &p, KANBAN)["cards"][1]["title"], "b");
}

#[test]
fn invalid_paths_and_values_fail_without_changes() {
    let t = TempDir::new("invalid");
    let v = t.open();
    let p = v.create_page(None, "Board".into()).unwrap();
    set(&v, &p, KANBAN, &["title"], "\"x\"").unwrap();
    set(&v, &p, KANBAN, &["cards"], r#"[{"id":"c1"}]"#).unwrap();
    let before = versions(&v, &p);
    let snapshot = data(&v, &p, KANBAN);

    let failures: Vec<VaultError> = vec![
        // A scalar in the middle of a path.
        set(&v, &p, KANBAN, &["title", "sub"], "1").unwrap_err(),
        // A list item id that does not exist, in a set path.
        set(&v, &p, KANBAN, &["cards", "nope", "x"], "1").unwrap_err(),
        // A set or delete that ends at a list item (the list operations do that).
        set(&v, &p, KANBAN, &["cards", "c1"], "{}").unwrap_err(),
        v.type_data_delete(p.clone(), KANBAN.into(), path(&["cards", "c1"]))
            .unwrap_err(),
        // A list operation on a map or a missing list.
        list_insert(&v, &p, &["title"], None, "1").unwrap_err(),
        list_insert(&v, &p, &["missing"], None, "1").unwrap_err(),
        list_insert(&v, &p, &[], None, "1").unwrap_err(),
        // An insert past the end, a move out of range, a missing item.
        list_insert(&v, &p, &["cards"], Some(5), "1").unwrap_err(),
        v.type_data_list_move(p.clone(), KANBAN.into(), path(&["cards"]), ListItem::Index(0), 3)
            .unwrap_err(),
        v.type_data_list_delete(p.clone(), KANBAN.into(), path(&["cards"]), ListItem::Index(9))
            .unwrap_err(),
        v.type_data_list_delete(
            p.clone(),
            KANBAN.into(),
            path(&["cards"]),
            ListItem::Id("gone".into()),
        )
        .unwrap_err(),
        // Deleting what is not there, and an empty path.
        v.type_data_delete(p.clone(), KANBAN.into(), path(&["cards", "c9"]))
            .unwrap_err(),
        v.type_data_delete(p.clone(), "mod:none".into(), path(&["x"]))
            .unwrap_err(),
        v.type_data_delete(p.clone(), KANBAN.into(), vec![]).unwrap_err(),
        // Set needs a key, and the add-on id must not be empty.
        set(&v, &p, KANBAN, &[], "[1]").unwrap_err(),
        set(&v, &p, KANBAN, &[], "{}").unwrap_err(),
        set(&v, &p, "", &["x"], "1").unwrap_err(),
        // Root names beginning with $ belong to the core, not add-ons.
        set(&v, &p, "$markdown", &["x"], "1").unwrap_err(),
        set(&v, &p, "$future", &["x"], "1").unwrap_err(),
        v.type_data(p.clone(), "$markdown".into()).unwrap_err(),
        v.type_data_delete(p.clone(), "$markdown".into(), path(&["x"]))
            .unwrap_err(),
        v.type_data_list_insert(p.clone(), "$markdown".into(), path(&["x"]), None, "1".into())
            .unwrap_err(),
        v.type_data_list_delete(p.clone(), "$markdown".into(), path(&["x"]), ListItem::Index(0))
            .unwrap_err(),
        v.type_data_list_move(p.clone(), "$markdown".into(), path(&["x"]), ListItem::Index(0), 0)
            .unwrap_err(),
        // Malformed JSON.
        set(&v, &p, KANBAN, &["title"], "{oops").unwrap_err(),
        v.type_data_list_insert(p.clone(), KANBAN.into(), path(&["cards"]), None, "[1".into())
            .unwrap_err(),
        // A page that is not there.
        v.type_data("missing-page".into(), KANBAN.into()).unwrap_err(),
    ];
    for e in &failures {
        assert!(
            matches!(e, VaultError::Invalid(_) | VaultError::NotFound(_)),
            "unexpected error {e:?}"
        );
    }
    assert_eq!(versions(&v, &p), before, "a failed write changed the page");
    assert_eq!(data(&v, &p, KANBAN), snapshot);
}

#[test]
fn two_replicas_initialize_one_absent_add_on_and_keep_disjoint_edits() {
    // Both replicas start from a page with no add-on data, so each creates the
    // add-on map, an intermediate map and the list key on its own.
    let (ta, tb) = (TempDir::new("init_a"), TempDir::new("init_b"));
    let (a, b, p) = replicas(&ta, &tb);
    set(&a, &p, "mod:shared", &["a"], "1").unwrap();
    set(&a, &p, "mod:shared", &["grid", "x"], "10").unwrap();
    set(&a, &p, "mod:shared", &["todo"], r#"["from-a"]"#).unwrap();
    set(&b, &p, "mod:shared", &["b"], "2").unwrap();
    set(&b, &p, "mod:shared", &["grid", "y"], "20").unwrap();
    set(&b, &p, "mod:shared", &["todo"], r#"["from-b"]"#).unwrap();
    sync(&a, &b);

    let (da, db) = (data(&a, &p, "mod:shared"), data(&b, &p, "mod:shared"));
    assert_eq!(da, db, "replicas differ after sync");
    assert_eq!(da["a"], 1);
    assert_eq!(da["b"], 2);
    assert_eq!(da["grid"], json!({"x": 10, "y": 20}));
    let mut todo: Vec<String> = da["todo"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    todo.sort();
    assert_eq!(todo, ["from-a", "from-b"]);
}

#[test]
fn concurrent_inserts_into_one_list_keep_both_items() {
    let (ta, tb) = (TempDir::new("insert_a"), TempDir::new("insert_b"));
    let (a, b, p) = replicas(&ta, &tb);
    set(&a, &p, KANBAN, &["cards"], "[]").unwrap();
    push(&a, &b);

    list_insert(&a, &p, &["cards"], None, r#"{"id":"from-a"}"#).unwrap();
    list_insert(&b, &p, &["cards"], None, r#"{"id":"from-b"}"#).unwrap();
    sync(&a, &b);

    let (da, db) = (data(&a, &p, KANBAN), data(&b, &p, KANBAN));
    assert_eq!(da, db, "replicas differ after sync");
    let mut got = ids(&da["cards"]);
    got.sort();
    assert_eq!(got, ["from-a", "from-b"]);
}

#[test]
fn move_of_one_item_and_rename_of_another_keep_both_edits() {
    let (ta, tb) = (TempDir::new("cross_a"), TempDir::new("cross_b"));
    let (a, b, p) = replicas(&ta, &tb);
    set(&a, &p, KANBAN, &["cards"], r#"[{"id":"c1","title":"a"},{"id":"c2","title":"b"}]"#)
        .unwrap();
    push(&a, &b);

    a.type_data_list_move(p.clone(), KANBAN.into(), path(&["cards"]), ListItem::Id("c2".into()), 0)
        .unwrap();
    set(&b, &p, KANBAN, &["cards", "c1", "title"], "\"renamed\"").unwrap();
    sync(&a, &b);

    let (da, db) = (data(&a, &p, KANBAN), data(&b, &p, KANBAN));
    assert_eq!(da, db, "replicas differ after sync");
    assert_eq!(ids(&da["cards"]), ["c2", "c1"], "the move and the rename both apply");
    assert_eq!(da["cards"][0]["title"], "b");
    assert_eq!(da["cards"][1]["title"], "renamed");
}

#[test]
fn same_item_move_and_rename_conflict_converges_and_drops_the_rename() {
    // A move is a delete and an insert of the item's JSON copy. A rename made
    // on the other replica edits the old item, which the delete removed, so
    // Loro drops that rename. The item still appears once, and both replicas
    // agree on the result.
    let (ta, tb) = (TempDir::new("move_a"), TempDir::new("move_b"));
    let (a, b, p) = replicas(&ta, &tb);
    set(&a, &p, KANBAN, &["cards"], r#"[{"id":"c1","title":"a"},{"id":"c2","title":"b"}]"#)
        .unwrap();
    push(&a, &b);

    a.type_data_list_move(p.clone(), KANBAN.into(), path(&["cards"]), ListItem::Id("c1".into()), 1)
        .unwrap();
    set(&b, &p, KANBAN, &["cards", "c1", "title"], "\"renamed\"").unwrap();
    sync(&a, &b);

    let (da, db) = (data(&a, &p, KANBAN), data(&b, &p, KANBAN));
    assert_eq!(da, db, "replicas differ after sync");
    assert_eq!(ids(&da["cards"]), ["c2", "c1"], "the item appears once");
    assert_eq!(da["cards"][1]["title"], "a", "the concurrent rename is lost (delete + insert)");
}

#[test]
fn add_on_data_survives_reopen() {
    let t = TempDir::new("reopen");
    let before;
    {
        let v = t.open();
        let p = v.create_page(None, "Board".into()).unwrap();
        set(&v, &p, KANBAN, &["cards"], r#"[{"id":"c1"}]"#).unwrap();
        set(&v, &p, "mod:other", &["flag"], "false").unwrap();
        before = v.type_data_all(p.clone()).unwrap();
        v.flush().unwrap();
    }
    let v = t.open();
    let p = v.list_children(None, 0, 10).unwrap()[0].id.clone();
    assert_eq!(v.type_data_all(p).unwrap(), before);
}

#[test]
fn markdown_keeps_unknown_add_on_data_and_reserved_names() {
    let t = TempDir::new("markdown");
    let v = t.open();
    let p = v.create_page(None, "Board".into()).unwrap();
    set(&v, &p, "mod:future.kanban", &["cards"], r#"[{"id":"c1","note":"a # b: \"q\""}]"#).unwrap();
    // A user property named like the reserved key.
    v.set_property(p.clone(), "type-data".into(), PropValue::Text("mine".into()))
        .unwrap();
    v.flush().unwrap();

    let file = md_file(&t, &p);
    let text = fs::read_to_string(&file).unwrap();
    let raw = raw_type_data(&text);
    assert!(raw.starts_with('{') && !raw.contains('\n'), "{raw}");
    assert_eq!(parse(raw), all(&v, &p), "the raw line is the add-on data");
    assert!(text.contains("\"prop:type-data\": \"mine\""), "{text}");

    // An outside edit of the add-on data is read back; other add-ons are kept.
    set(&v, &p, "mod:kept", &["n"], "2").unwrap();
    v.flush().unwrap();
    rewrite_type_data(
        &file,
        &json!({
            "mod:future.kanban": {"cards": [{"id": "c1", "note": "edited"}, {"id": "c2"}]},
            "mod:kept": {"n": 2}
        })
        .to_string(),
    );
    assert!(v.rescan().unwrap() >= 1);
    assert_eq!(
        data(&v, &p, "mod:future.kanban"),
        json!({"cards": [{"id": "c1", "note": "edited"}, {"id": "c2"}]})
    );
    assert_eq!(data(&v, &p, "mod:kept"), json!({"n": 2}));
    // The reserved name is a property, not add-on data.
    assert_eq!(prop(&v, &p, "type-data"), Some(PropValue::Text("mine".into())));
}

#[test]
fn markdown_strings_survive_rescan_and_an_explicit_empty_object_clears() {
    let t = TempDir::new("strings");
    let v = t.open();
    let p = v.create_page(None, "Board".into()).unwrap();
    let tricky = "a # b: c \"q\" \\ back\nline2";
    set(&v, &p, "mod:tricky", &["s"], &json!(tricky).to_string()).unwrap();
    set(&v, &p, "mod:tricky", &["k # 1: x"], "[\"y # z\", 1.5, null]").unwrap();
    v.flush().unwrap();

    let file = md_file(&t, &p);
    let mut raw = parse(raw_type_data(&fs::read_to_string(&file).unwrap()));
    assert_eq!(raw, all(&v, &p));
    assert_eq!(data(&v, &p, "mod:tricky")["s"], tricky);

    // An outside edit that adds a key with the same tricky characters.
    raw["mod:tricky"]["extra"] = json!("z # w: \"v\"\nnext");
    rewrite_type_data(&file, &raw.to_string());
    assert!(v.rescan().unwrap() >= 1);
    assert_eq!(data(&v, &p, "mod:tricky")["s"], tricky);
    assert_eq!(data(&v, &p, "mod:tricky")["extra"], "z # w: \"v\"\nnext");

    // An explicit empty object is data: it removes all the add-on data.
    rewrite_type_data(&file, "{}");
    assert!(v.rescan().unwrap() >= 1);
    assert_eq!(v.type_data_all(p.clone()).unwrap(), "{}");
}

#[test]
fn export_then_import_keeps_add_on_data_page_type_property_and_body() {
    let (ta, tb) = (TempDir::new("export_a"), TempDir::new("export_b"));
    let a = ta.open();
    let p = a
        .create_page_with_type(None, "Trip".into(), Some("mod:example".into()))
        .unwrap();
    a.apply(
        p.clone(),
        vec![BlockOp::Insert {
            parent: None,
            after: None,
            block: NewBlock {
                id: "body1".into(),
                kind: BlockKind::Paragraph,
                text: "Body # text: \"quoted\"".into(),
                marks: vec![],
                checked: false,
            },
        }],
    )
    .unwrap();
    set(&a, &p, "mod:alpha", &["title"], &json!("A # hash: \"q\"\nline2").to_string()).unwrap();
    set(&a, &p, "mod:alpha", &["tags"], &json!(["x", "y # z"]).to_string()).unwrap();
    set(&a, &p, "mod:alpha", &["nested", "deep", "n"], "1.5").unwrap();
    set(&a, &p, "mod:alpha", &["nested", "deep", "flag"], "false").unwrap();
    set(&a, &p, "mod:alpha", &["nested", "deep", "none"], "null").unwrap();
    set(
        &a,
        &p,
        "mod:alpha",
        &["cards"],
        r#"[{"id":"c1","t":"line1\nline2"},{"id":"c2","items":[1,[2,3],{"k":"v"}]}]"#,
    )
    .unwrap();
    set(&a, &p, "mod:alpha", &["cards", "c2", "title"], "\"second\"").unwrap();
    set(&a, &p, "mod:beta.v2", &["list"], r#"[1,[2,3],{"k":"v"}]"#).unwrap();
    a.set_property(p.clone(), "type-data".into(), PropValue::Text("mine: # kept".into()))
        .unwrap();
    a.flush().unwrap();

    let original = all(&a, &p);
    let text = fs::read_to_string(md_file(&ta, &p)).unwrap();
    assert_eq!(parse(raw_type_data(&text)), original, "raw JSON line");

    let b = tb.open();
    let report = b.import_markdown(ta.pages(), None).unwrap();
    assert_eq!(report.pages, 1);
    let q = report.top_pages[0].clone();

    assert_eq!(all(&b, &q), original);
    assert_eq!(b.load_page(q.clone()).unwrap().page_type, "mod:example");
    assert_eq!(b.load_page(q.clone()).unwrap().title, "Trip");
    assert_eq!(b.load_page(q.clone()).unwrap().blocks[0].text, "Body # text: \"quoted\"");
    assert_eq!(
        prop(&b, &q, "type-data"),
        Some(PropValue::Text("mine: # kept".into()))
    );
    assert_eq!(data(&b, &q, "mod:alpha")["cards"][1]["items"], json!([1, [2, 3], {"k": "v"}]));
}

#[test]
fn import_reads_raw_add_on_data_from_a_file() {
    let t = TempDir::new("import");
    let src = TempDir::new("import_src");
    let json_text = r#"{"mod:future":{"cards":["a","b # c"]}}"#;
    fs::write(
        src.0.join("Imported.md"),
        format!("---\ntype-data: {json_text}\n---\nBody text\n"),
    )
    .unwrap();
    let v = t.open();
    let report = v.import_markdown(src.root(), None).unwrap();
    assert_eq!(report.pages, 1);
    let page = report.top_pages[0].clone();
    assert_eq!(data(&v, &page, "mod:future"), json!({"cards": ["a", "b # c"]}));
}

#[test]
fn folder_import_remaps_nested_type_data_page_ids_but_keeps_keys_and_readable_body() {
    let (source_dir, destination_dir) = (TempDir::new("remap_source"), TempDir::new("remap_dest"));
    let source = source_dir.open();
    // The board sorts first, so references must also resolve to later files.
    let board = source.create_page_with_type(None, "Alpha board".into(), Some(KANBAN.into())).unwrap();
    let a = source.create_page(None, "Zulu A".into()).unwrap();
    let b = source.create_page(None, "Zulu B".into()).unwrap();
    let missing = "page-not-in-export";
    let payload = json!({
        "cards": {"todo": [b, a, b], "done": []},
        "nested": [{"refs": [a, {"target": b, "$markdown": a}]}],
        "unrelated": "ordinary label",
        "partial": format!("prefix-{a}"),
        "missing": missing,
        "scalars": [null, false, 7],
        (a.clone()): b
    });
    set(&source, &board, KANBAN, &["payload"], &payload.to_string()).unwrap();
    // An add-on id can itself equal an old page id; root keys stay literal too.
    set(&source, &board, &a, &["target"], &json!(b).to_string()).unwrap();
    // Exact equality with a source id catches an accidental rewrite of the
    // reserved root body, while nested keys with that spelling are ordinary data.
    source.set_type_markdown_body(board.clone(), Some(a.clone())).unwrap();
    source.flush().unwrap();
    let exported = source.export_markdown(board.clone()).unwrap();
    assert_eq!(fs::read(md_file(&source_dir, &board)).unwrap(), exported.as_bytes());
    assert_eq!(markdown_body(&exported), a);
    drop(source);

    let destination = destination_dir.open();
    let report = destination.import_markdown(source_dir.pages(), None).unwrap();
    assert_eq!(report.pages, 3);
    assert_eq!(report.unresolved_links, 0);
    let imported: HashMap<String, String> = destination.list_children(None, 0, 10).unwrap()
        .into_iter().map(|row| (row.title, row.id)).collect();
    let (board_new, a_new, b_new) = (&imported["Alpha board"], &imported["Zulu A"], &imported["Zulu B"]);
    assert_ne!(board_new, &board);
    assert_ne!(a_new, &a);
    assert_ne!(b_new, &b);
    let expected = json!({
        "cards": {"todo": [b_new, a_new, b_new], "done": []},
        "nested": [{"refs": [a_new, {"target": b_new, "$markdown": a_new}]}],
        "unrelated": "ordinary label",
        "partial": format!("prefix-{a}"),
        "missing": missing,
        "scalars": [null, false, 7],
        (a.clone()): b_new
    });
    assert_eq!(data(&destination, board_new, KANBAN), json!({"payload": expected}));
    assert_eq!(data(&destination, board_new, &a), json!({"target": b_new}));
    assert_eq!(destination.type_markdown_body(board_new.clone()).unwrap(), Some(a.clone()));
    assert!(destination.load_page(board_new.clone()).unwrap().blocks.is_empty());
    destination.flush().unwrap();
    drop(destination);
    let reopened = destination_dir.open();
    assert_eq!(data(&reopened, board_new, KANBAN), json!({"payload": expected}));
    assert_eq!(reopened.type_markdown_body(board_new.clone()).unwrap(), Some(a));
}

#[test]
fn folder_import_keeps_ambiguous_type_data_page_ids() {
    let (source_dir, destination_dir) = (TempDir::new("ambiguous_source"), TempDir::new("ambiguous_dest"));
    let source = source_dir.open();
    let board = source.create_page_with_type(None, "Alpha board".into(), Some(KANBAN.into())).unwrap();
    let target = source.create_page(None, "Zulu target".into()).unwrap();
    set(&source, &board, KANBAN, &["refs"], &json!([target, {"target": target}]).to_string()).unwrap();
    source.flush().unwrap();
    // Both real exported files carry the same source front-matter id.
    fs::copy(md_file(&source_dir, &target), source_dir.0.join("pages/Zulu copy.md")).unwrap();
    drop(source);

    let destination = destination_dir.open();
    let report = destination.import_markdown(source_dir.pages(), None).unwrap();
    assert_eq!(report.pages, 3);
    let rows = destination.list_children(None, 0, 10).unwrap();
    let board_new = &rows.iter().find(|row| row.title == "Alpha board").unwrap().id;
    assert_ne!(board_new, &board);
    let targets: Vec<_> = rows.iter().filter(|row| row.title == "Zulu target").collect();
    assert_eq!(targets.len(), 2);
    assert_ne!(targets[0].id, targets[1].id);
    assert!(targets.iter().all(|row| row.id != target));
    assert_eq!(data(&destination, board_new, KANBAN), json!({"refs": [target, {"target": target}]}));
}

#[test]
fn typed_creation_and_conversion() {
    let t = TempDir::new("types");
    let v = t.open();
    let board = v.create_page_with_type(None, "Board".into(), Some(KANBAN.into())).unwrap();
    assert_eq!(v.load_page(board.clone()).unwrap().page_type, KANBAN);
    let plain = v.create_page_with_type(None, "Plain".into(), None).unwrap();
    assert_eq!(v.load_page(plain.clone()).unwrap().page_type, PAGE_TYPE_DOCUMENT);
    // The index row carries the type too.
    let row = v
        .list_children(None, 0, 10)
        .unwrap()
        .into_iter()
        .find(|x| x.id == board)
        .unwrap();
    assert_eq!(row.page_type, KANBAN);

    // A document can become a database and back.
    v.set_page_type(plain.clone(), PAGE_TYPE_DATABASE.into()).unwrap();
    assert_eq!(v.load_page(plain.clone()).unwrap().page_type, PAGE_TYPE_DATABASE);
    v.set_page_type(plain.clone(), PAGE_TYPE_DOCUMENT.into()).unwrap();
    assert_eq!(v.load_page(plain.clone()).unwrap().page_type, PAGE_TYPE_DOCUMENT);

    // Any type converts to any other, and the add-on data stays.
    set(&v, &board, "mod:kept", &["x"], "1").unwrap();
    v.set_page_type(board.clone(), PAGE_TYPE_DATABASE.into()).unwrap();
    assert_eq!(v.load_page(board.clone()).unwrap().page_type, PAGE_TYPE_DATABASE);
    v.set_page_type(board.clone(), "mod:converted".into()).unwrap();
    assert_eq!(v.load_page(board.clone()).unwrap().page_type, "mod:converted");
    assert_eq!(data(&v, &board, "mod:kept"), json!({"x": 1}));

    let example = v
        .create_page_with_type(None, "Example".into(), Some("mod:example".into()))
        .unwrap();
    set(&v, &example, "mod:example", &["n"], "2").unwrap();
    v.set_page_type(example.clone(), "mod:converted".into()).unwrap();
    assert_eq!(v.load_page(example.clone()).unwrap().page_type, "mod:converted");
    v.set_page_type(example.clone(), PAGE_TYPE_DOCUMENT.into()).unwrap();
    assert_eq!(v.load_page(example.clone()).unwrap().page_type, PAGE_TYPE_DOCUMENT);
    assert_eq!(data(&v, &example, "mod:example"), json!({"n": 2}));
    // Setting the type a page already has changes nothing.
    v.set_page_type(example.clone(), PAGE_TYPE_DOCUMENT.into()).unwrap();
    assert_eq!(data(&v, &example, "mod:example"), json!({"n": 2}));

    // A typed child keeps the template's blocks and gets its type.
    let parent = v.create_page(None, "Trips".into()).unwrap();
    let tpl = v.create_template(parent.clone(), "Tpl".into()).unwrap();
    v.apply(
        tpl.clone(),
        vec![BlockOp::Insert {
            parent: None,
            after: None,
            block: NewBlock {
                id: "tb".into(),
                kind: BlockKind::Paragraph,
                text: "from template".into(),
                marks: vec![],
                checked: false,
            },
        }],
    )
    .unwrap();
    v.set_default_template(parent.clone(), Some(tpl)).unwrap();
    let child = v
        .create_page_with_type(Some(parent), "Trip".into(), Some(KANBAN.into()))
        .unwrap();
    let page = v.load_page(child).unwrap();
    assert_eq!(page.page_type, KANBAN);
    assert_eq!(page.blocks.len(), 1);
    assert_eq!(page.blocks[0].text, "from template");
}

fn markdown_body(text: &str) -> &str {
    let rest = text.strip_prefix("---\n").expect("front matter");
    let (_, body) = rest.split_once("\n---\n").expect("front matter end");
    body
}

#[test]
fn readable_type_body_is_reserved_persistent_and_removable() {
    let t = TempDir::new("readable_persist");
    let p;
    let body = "\r\n# Readable board\r\n\r\n- [[Card]]  \r\n\r\n";
    {
        let v = t.open();
        p = v.create_page_with_type(None, "Board".into(), Some(KANBAN.into())).unwrap();
        assert_eq!(v.type_markdown_body(p.clone()).unwrap(), None);
        let before = versions(&v, &p);
        v.set_type_markdown_body(p.clone(), None).unwrap();
        assert_eq!(versions(&v, &p), before, "removing an absent body is a no-op");
        set(&v, &p, KANBAN, &["columns"], "[]").unwrap();
        v.set_type_markdown_body(p.clone(), Some(body.into())).unwrap();
        assert_eq!(v.type_markdown_body(p.clone()).unwrap().as_deref(), Some(body));
        assert_eq!(all(&v, &p), json!({"kanban": {"columns": []}, "$markdown": body}));
        let before = versions(&v, &p);
        v.set_type_markdown_body(p.clone(), Some(body.into())).unwrap();
        assert_eq!(versions(&v, &p), before, "setting the same body is a no-op");
        v.flush().unwrap();
    }
    let v = t.open();
    assert_eq!(v.type_markdown_body(p.clone()).unwrap().as_deref(), Some(body));
    v.set_page_type(p.clone(), PAGE_TYPE_DOCUMENT.into()).unwrap();
    assert_eq!(v.type_markdown_body(p.clone()).unwrap().as_deref(), Some(body));
    v.set_type_markdown_body(p.clone(), None).unwrap();
    assert_eq!(v.type_markdown_body(p.clone()).unwrap(), None);
    assert_eq!(all(&v, &p), json!({"kanban": {"columns": []}}));
    assert!(!parse(raw_type_data(&v.export_markdown(p.clone()).unwrap()))
        .as_object().unwrap().contains_key("$markdown"));
    v.flush().unwrap();
    drop(v);
    assert_eq!(t.open().type_markdown_body(p).unwrap(), None);
}

#[test]
fn readable_body_and_its_removal_sync_through_the_page_document() {
    let (ta, tb) = (TempDir::new("readable_sync_a"), TempDir::new("readable_sync_b"));
    let (a, b, p) = replicas(&ta, &tb);
    set(&a, &p, KANBAN, &["columns"], "[]").unwrap();
    a.set_type_markdown_body(p.clone(), Some("\n# Board\r\n\r\n".into())).unwrap();
    push(&a, &b);
    assert_eq!(b.type_markdown_body(p.clone()).unwrap(), a.type_markdown_body(p.clone()).unwrap());
    assert_eq!(all(&a, &p), all(&b, &p));
    b.set_type_markdown_body(p.clone(), None).unwrap();
    push(&b, &a);
    assert_eq!(a.type_markdown_body(p.clone()).unwrap(), None);
    assert_eq!(data(&a, &p, KANBAN), json!({"columns": []}));
}

#[test]
fn readable_body_exports_marker_and_verbatim_bytes_and_rescans_identically() {
    let t = TempDir::new("readable_export");
    let v = t.open();
    let p = v.create_page_with_type(None, "Board".into(), Some(KANBAN.into())).unwrap();
    set(&v, &p, KANBAN, &["columns"], r#"[{"id":"todo","title":"To do"}]"#).unwrap();
    for body in ["", "# No final newline", "\n\n# Leading and trailing\n\n", "\r\n# CRLF\r\n\r\n"] {
        v.set_type_markdown_body(p.clone(), Some(body.into())).unwrap();
        let exported = v.export_markdown(p.clone()).unwrap();
        assert_eq!(markdown_body(&exported), body);
        assert_eq!(parse(raw_type_data(&exported)), json!({
            "kanban": {"columns": [{"id": "todo", "title": "To do"}]},
            "$markdown": true
        }));
        assert!(v.load_page(p.clone()).unwrap().blocks.is_empty());
        v.flush().unwrap();
        let file = md_file(&t, &p);
        assert_eq!(fs::read(&file).unwrap(), exported.as_bytes(), "writer and API agree");
        // A front-matter-only formatting edit forces the rescan path. It must
        // neither parse the readable body nor change its bytes or timestamp.
        fs::write(&file, exported.replacen("type-data: ", "type-data:   ", 1)).unwrap();
        assert_eq!(v.rescan().unwrap(), 0);
        assert_eq!(v.export_markdown(p.clone()).unwrap(), exported);
        assert_eq!(v.type_markdown_body(p.clone()).unwrap().as_deref(), Some(body));
    }
}

#[test]
fn marked_folder_import_preserves_body_without_blocks_or_link_resolution() {
    let t = TempDir::new("readable_import");
    let src = TempDir::new("readable_import_src");
    let v = t.open();
    for (i, body) in ["", "# No newline", "\n\n# Board\n- [[Missing card]]\n\n", "\r\n# CRLF\r\n\r\n"]
        .into_iter().enumerate()
    {
        let file = src.0.join(format!("Board{i}.md"));
        fs::write(&file, format!(
            "---\r\ntype: kanban\r\ntype-data: {{\"kanban\":{{\"columns\":[]}},\"$markdown\":true}}\r\n---\r\n{body}"
        )).unwrap();
    }
    let report = v.import_markdown(src.root(), None).unwrap();
    assert_eq!(report.pages, 4);
    assert_eq!(report.unresolved_links, 0, "readable Markdown is not parsed");
    for p in report.top_pages {
        let page = v.load_page(p.clone()).unwrap();
        let body = fs::read_to_string(src.0.join(format!("{}.md", page.title))).unwrap();
        let body = body.split_once("\r\n---\r\n").unwrap().1;
        assert!(page.blocks.is_empty());
        assert_eq!(v.type_markdown_body(p.clone()).unwrap().as_deref(), Some(body));
        assert_eq!(markdown_body(&v.export_markdown(p.clone()).unwrap()), body);
        assert_eq!(data(&v, &p, KANBAN), json!({"columns": []}));
    }
}

#[test]
fn empty_paragraph_placeholders_export_verbatim_readable_body_in_both_paths() {
    let t = TempDir::new("readable_placeholders");
    let v = t.open();
    for count in [1, 3] {
        let p = v.create_page_with_type(None, format!("Board {count}"), Some(KANBAN.into())).unwrap();
        let ops = (0..count).map(|i| BlockOp::Insert {
            parent: None,
            after: (i > 0).then(|| format!("placeholder{}", i - 1)),
            block: NewBlock {
                id: format!("placeholder{i}"), kind: BlockKind::Paragraph,
                text: String::new(), marks: vec![], checked: false,
            },
        }).collect();
        v.apply(p.clone(), ops).unwrap();
        for body in ["", "# No final newline", "\n\n# Board\n\n", "\r\n# Board\r\n\r\n"] {
            v.set_type_markdown_body(p.clone(), Some(body.into())).unwrap();
            let before = v.load_page(p.clone()).unwrap().blocks;
            assert_eq!(before.len(), count);
            assert!(before.iter().all(|b| b.kind == BlockKind::Paragraph && b.text.is_empty() && b.children.is_empty()));
            let exported = v.export_markdown(p.clone()).unwrap();
            assert_eq!(markdown_body(&exported).as_bytes(), body.as_bytes(), "{count} placeholders");
            assert_eq!(parse(raw_type_data(&exported))["$markdown"], true);
            v.flush().unwrap();
            assert_eq!(fs::read(md_file(&t, &p)).unwrap(), exported.as_bytes(), "writer and API agree");
            assert_eq!(v.load_page(p.clone()).unwrap().blocks, before, "export keeps editor placeholders");
        }
    }
}

#[test]
fn substantive_or_child_bearing_blocks_override_readable_body_in_both_paths() {
    let t = TempDir::new("readable_block_precedence");
    let v = t.open();
    let fallback = "# Stored fallback must not win\r\n";
    for (name, kind, text, child) in [
        ("text", BlockKind::Paragraph, "Actual blocks", None),
        ("whitespace", BlockKind::Paragraph, " ", None),
        ("child", BlockKind::Paragraph, "", Some("Child text")),
        ("empty_child", BlockKind::Paragraph, "", Some("")),
        ("other_kind", BlockKind::Heading1, "", None),
    ] {
        let p = v.create_page_with_type(None, name.into(), Some(KANBAN.into())).unwrap();
        // An empty sibling must not hide a substantive block elsewhere.
        let mut ops = vec![
            BlockOp::Insert {
                parent: None, after: None,
                block: NewBlock { id: "empty".into(), kind: BlockKind::Paragraph,
                    text: String::new(), marks: vec![], checked: false },
            },
            BlockOp::Insert {
                parent: None, after: Some("empty".into()),
                block: NewBlock { id: "actual".into(), kind,
                    text: text.into(), marks: vec![], checked: false },
            },
        ];
        if let Some(child_text) = child {
            ops.push(BlockOp::Insert {
                parent: Some("actual".into()), after: None,
                block: NewBlock { id: "child".into(), kind: BlockKind::Paragraph,
                    text: child_text.into(), marks: vec![], checked: false },
            });
        }
        v.apply(p.clone(), ops).unwrap();
        // The same blocks without a stored body give an independent expected
        // rendering, including empty nonparagraphs and empty child blocks.
        let normal_export = v.export_markdown(p.clone()).unwrap();
        let expected_body = markdown_body(&normal_export);
        v.set_type_markdown_body(p.clone(), Some(fallback.into())).unwrap();
        let exported = v.export_markdown(p.clone()).unwrap();
        assert_eq!(markdown_body(&exported), expected_body, "{name}");
        assert_ne!(markdown_body(&exported), fallback, "{name}");
        assert_eq!(parse(raw_type_data(&exported))["$markdown"], true);
        assert_eq!(v.type_markdown_body(p.clone()).unwrap().as_deref(), Some(fallback));
        v.flush().unwrap();
        assert_eq!(fs::read(md_file(&t, &p)).unwrap(), exported.as_bytes(), "writer and API agree for {name}");
    }
}

#[test]
fn marked_rescan_creates_and_updates_exact_body_and_clears_existing_blocks() {
    let t = TempDir::new("readable_rescan");
    let v = t.open();
    let file = t.0.join("pages").join("Board.md");
    let original = "\n# First\r\n\r\n";
    fs::write(&file, format!("---\ntype-data: {{\"$markdown\":true}}\n---\n{original}")).unwrap();
    assert_eq!(v.rescan().unwrap(), 1);
    let p = v.list_children(None, 0, 10).unwrap()[0].id.clone();
    assert_eq!(v.type_markdown_body(p.clone()).unwrap().as_deref(), Some(original));
    assert!(v.load_page(p.clone()).unwrap().blocks.is_empty());
    v.flush().unwrap();
    let file = md_file(&t, &p);
    let before = v.export_markdown(p.clone()).unwrap();
    let next = "\n# First\r\n\r\n\r\n"; // Only trailing bytes changed.
    let prefix = &before[..before.len() - original.len()];
    fs::write(&file, format!("{prefix}{next}")).unwrap();
    assert_eq!(v.rescan().unwrap(), 1);
    assert_eq!(v.type_markdown_body(p.clone()).unwrap().as_deref(), Some(next));

    v.apply(p.clone(), vec![BlockOp::Insert {
        parent: None, after: None,
        block: NewBlock { id: "existing".into(), kind: BlockKind::Paragraph,
            text: "Existing blocks".into(), marks: vec![], checked: false },
    }]).unwrap();
    v.flush().unwrap();
    let exported = v.export_markdown(p.clone()).unwrap();
    let rendered_body = markdown_body(&exported);
    // Marker import suppresses parsing even when the prior page had blocks.
    fs::write(&file, exported.replacen("type-data: ", "type-data:   ", 1)).unwrap();
    assert_eq!(v.rescan().unwrap(), 1);
    assert!(v.load_page(p.clone()).unwrap().blocks.is_empty());
    assert_eq!(v.type_markdown_body(p.clone()).unwrap().as_deref(), Some(rendered_body));
}

#[test]
fn blocks_take_precedence_over_stored_body_and_unmarked_markdown_still_parses() {
    let t = TempDir::new("readable_blocks");
    let src = TempDir::new("readable_unmarked_src");
    let v = t.open();
    let p = v.create_page(None, "Board".into()).unwrap();
    v.set_type_markdown_body(p.clone(), Some("# Stored fallback\n".into())).unwrap();
    v.apply(p.clone(), vec![BlockOp::Insert {
        parent: None, after: None,
        block: NewBlock { id: "body".into(), kind: BlockKind::Paragraph,
            text: "Actual blocks".into(), marks: vec![], checked: false },
    }]).unwrap();
    let exported = v.export_markdown(p.clone()).unwrap();
    assert_eq!(markdown_body(&exported), "\nActual blocks\n");
    assert_eq!(parse(raw_type_data(&exported))["$markdown"], true);
    assert_eq!(v.type_markdown_body(p.clone()).unwrap().as_deref(), Some("# Stored fallback\n"));
    fs::write(src.0.join("Plain.md"), "---\ntype-data: {\"kanban\":{}}\n---\n# Normal heading\n").unwrap();
    let imported = v.import_markdown(src.root(), None).unwrap().top_pages[0].clone();
    assert_eq!(v.type_markdown_body(imported.clone()).unwrap(), None);
    let page = v.load_page(imported).unwrap();
    assert_eq!(page.blocks.len(), 1);
    assert_eq!(page.blocks[0].kind, BlockKind::Heading1);
    assert_eq!(page.blocks[0].text, "Normal heading");

    // Removing the marker from an otherwise unchanged fallback must also
    // restore normal parsing in the update path.
    let fallback = v.create_page(None, "Fallback".into()).unwrap();
    v.set_type_markdown_body(fallback.clone(), Some("# Heading\n".into())).unwrap();
    v.flush().unwrap();
    let file = md_file(&t, &fallback);
    rewrite_type_data(&file, "{}");
    assert_eq!(v.rescan().unwrap(), 1);
    let page = v.load_page(fallback.clone()).unwrap();
    assert_eq!(page.blocks.len(), 1);
    assert_eq!(page.blocks[0].kind, BlockKind::Heading1);
    assert_eq!(v.type_markdown_body(fallback).unwrap(), None);
}
