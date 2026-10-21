use serde_json::{json, Value};
use vault_core::{BlockKind, BlockOp, NewBlock, Vault};
use vault_tools::call;

fn run(v: &Vault, name: &str, args: Value) -> String {
    call(v, name, &args).unwrap_or_else(|e| panic!("{name} {args}: {e}"))
}

fn insert(id: &str, text: &str, parent: Option<&str>, after: Option<&str>) -> BlockOp {
    BlockOp::Insert {
        parent: parent.map(String::from),
        after: after.map(String::from),
        block: NewBlock {
            id: id.into(),
            kind: BlockKind::Todo,
            text: text.into(),
            marks: vec![],
            checked: false,
        },
    }
}

fn fixture() -> (tempfile::TempDir, Vault, String) {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::open(dir.path().to_string_lossy().into_owned()).unwrap();
    let page = v.create_page(None, "Nested tasks".into()).unwrap();
    v.apply(
        page.clone(),
        vec![
            insert("before", "Before tree", None, None),
            insert("parent", "Parent task", None, Some("before")),
            insert("after", "After tree", None, Some("parent")),
            insert("left", "Left sibling", Some("parent"), None),
            insert("target", "Nested target", Some("parent"), Some("left")),
            insert("right", "Right sibling", Some("parent"), Some("target")),
            insert("grandchild", "Old grandchild", Some("target"), None),
            insert("cousin", "Keep grandchild", Some("right"), None),
        ],
    )
    .unwrap();
    (dir, v, page)
}

// Obtain the actual reference exposed to callers, rather than relying on the
// fixture's ids when issuing edits.
fn id_for_text(read: &str, text: &str) -> String {
    read.lines()
        .collect::<Vec<_>>()
        .windows(2)
        .find_map(|lines| {
            if !lines[1].ends_with(text) {
                return None;
            }
            lines[0]
                .trim()
                .strip_prefix("<!-- block ")
                .and_then(|id| id.strip_suffix(" -->"))
                .map(String::from)
        })
        .unwrap_or_else(|| panic!("no block id for {text:?}: {read}"))
}

#[test]
fn read_ids_include_all_descendants_with_top_level_pagination() {
    let (_dir, v, page) = fixture();
    let read = run(
        &v,
        "read_page",
        json!({"page": page, "block_ids": true, "offset": 1, "limit": 1}),
    );
    for (text, id) in [
        ("Parent task", "parent"),
        ("Left sibling", "left"),
        ("Nested target", "target"),
        ("Old grandchild", "grandchild"),
        ("Right sibling", "right"),
        ("Keep grandchild", "cousin"),
    ] {
        assert_eq!(id_for_text(&read, text), id);
        assert_eq!(read.matches(&format!("<!-- block {id} -->")).count(), 1);
    }
    assert!(
        read.contains("  <!-- block target -->\n  - [ ] Nested target"),
        "{read}"
    );
    assert!(
        read.contains("    <!-- block grandchild -->\n    - [ ] Old grandchild"),
        "{read}"
    );
    assert!(
        !read.contains("Before tree") && !read.contains("After tree"),
        "{read}"
    );
    assert!(read.contains("Blocks 2–2 of 3. More: offset=2"), "{read}");
    let plain = run(&v, "read_page", json!({"page": page}));
    assert!(!plain.contains("<!-- block "), "{plain}");
    assert!(plain.contains("    - [ ] Old grandchild"), "{plain}");
}

#[test]
fn nested_ids_round_trip_through_replace_then_delete_preserving_neighbors() {
    let (_dir, v, page) = fixture();
    let before = v.load_page(page.clone()).unwrap();
    let read = run(&v, "read_page", json!({"page": page, "block_ids": true}));
    let target = id_for_text(&read, "Nested target");
    run(
        &v,
        "edit_page",
        json!({"page": page, "op": "replace", "block_ids": [target],
            "markdown": "- [x] Replacement task\n  - [ ] New grandchild"}),
    );
    let replaced = v.load_page(page.clone()).unwrap();
    assert_eq!(replaced.blocks[0], before.blocks[0]);
    assert_eq!(replaced.blocks[2], before.blocks[2]);
    assert_eq!(replaced.blocks[1].id, before.blocks[1].id);
    assert_eq!(replaced.blocks[1].text, before.blocks[1].text);
    let children = &replaced.blocks[1].children;
    assert_eq!(children.len(), 3);
    assert_eq!(children[0], before.blocks[1].children[0]);
    assert_eq!(children[2], before.blocks[1].children[2]);
    assert_eq!(children[1].kind, BlockKind::Todo);
    assert!(children[1].checked);
    assert_eq!(children[1].text, "Replacement task");
    assert_eq!(children[1].children[0].text, "New grandchild");

    let read = run(&v, "read_page", json!({"page": page, "block_ids": true}));
    assert!(!read.contains("Old grandchild"), "{read}");
    let replacement = id_for_text(&read, "Replacement task");
    run(
        &v,
        "edit_page",
        json!({"page": page, "op": "delete", "block_ids": [replacement]}),
    );
    let deleted = v.load_page(page).unwrap();
    assert_eq!(deleted.blocks[0], before.blocks[0]);
    assert_eq!(deleted.blocks[2], before.blocks[2]);
    assert_eq!(
        deleted.blocks[1].children,
        vec![children[0].clone(), children[2].clone()]
    );
}

#[test]
fn batch_edits_accept_nested_ids_and_keep_other_branches() {
    let (_dir, v, page) = fixture();
    let before = v.load_page(page.clone()).unwrap();
    let read = run(&v, "read_page", json!({"page": page, "block_ids": true}));
    let left = id_for_text(&read, "Left sibling");
    let grandchild = id_for_text(&read, "Old grandchild");
    let output = run(
        &v,
        "edit_page",
        json!({"page": page, "ops": [
            {"op": "delete", "block_ids": [left]},
            {"op": "replace", "block_ids": [grandchild], "markdown": "- [x] Finished grandchild"}
        ]}),
    );
    assert!(output.starts_with("1. Replaced"), "{output}");
    assert!(output.contains("\n2. Replaced"), "{output}");
    let after = v.load_page(page).unwrap();
    assert_eq!(after.blocks[0], before.blocks[0]);
    assert_eq!(after.blocks[2], before.blocks[2]);
    assert_eq!(after.blocks[1].children.len(), 2);
    assert_eq!(after.blocks[1].children[0].id, "target");
    assert_eq!(
        after.blocks[1].children[0].children[0].text,
        "Finished grandchild"
    );
    assert!(after.blocks[1].children[0].children[0].checked);
    assert_eq!(after.blocks[1].children[1], before.blocks[1].children[2]);
}

#[test]
fn nested_multi_block_replace_validates_all_ids_before_editing() {
    let (_dir, v, page) = fixture();
    let before = v.load_page(page.clone()).unwrap();
    let read = run(&v, "read_page", json!({"page": page, "block_ids": true}));
    let left = id_for_text(&read, "Left sibling");
    let target = id_for_text(&read, "Nested target");
    let result = call(
        &v,
        "edit_page",
        &json!({"page": page, "op": "replace", "block_ids": [left, "missing"], "markdown": "Changed"}),
    );
    assert!(result.is_err());
    assert_eq!(v.load_page(page.clone()).unwrap().blocks, before.blocks);
    run(
        &v,
        "edit_page",
        json!({"page": page, "op": "replace", "block_ids": [left, target], "markdown": "Merged"}),
    );
    let after = v.load_page(page).unwrap();
    assert_eq!(after.blocks[0], before.blocks[0]);
    assert_eq!(after.blocks[2], before.blocks[2]);
    assert_eq!(after.blocks[1].children.len(), 2);
    assert_eq!(after.blocks[1].children[0].text, "Merged");
    assert_eq!(after.blocks[1].children[1], before.blocks[1].children[2]);
}

#[test]
fn id_rendering_keeps_numbered_indentation_and_code_fences() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::open(dir.path().to_string_lossy().into_owned()).unwrap();
    let page = v.create_page(None, "Numbered code".into()).unwrap();
    let mut ops = Vec::new();
    for n in 1..=10 {
        let mut op = insert(
            &format!("item-{n}"),
            &format!("Item {n}\ncontinued"),
            None,
            (n > 1).then(|| format!("item-{}", n - 1)).as_deref(),
        );
        if let BlockOp::Insert { block, .. } = &mut op {
            block.kind = BlockKind::Numbered;
        }
        ops.push(op);
    }
    let mut code = insert("code", "let x = 1;", Some("item-10"), None);
    if let BlockOp::Insert { block, .. } = &mut code {
        block.kind = BlockKind::Code;
    }
    ops.push(code);
    v.apply(page.clone(), ops).unwrap();
    let read = run(&v, "read_page", json!({"page": page, "block_ids": true}));
    assert!(read.contains("10. Item 10\n    continued"), "{read}");
    assert!(
        read.contains("    <!-- block code -->\n    ```\n    let x = 1;\n    ```"),
        "{read}"
    );
    assert_eq!(read.matches("<!-- block code -->").count(), 1);
}
