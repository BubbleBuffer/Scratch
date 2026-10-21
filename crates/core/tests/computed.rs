//! Files, formula and rollup properties.

use std::fs;
use std::path::PathBuf;

use vault_core::*;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("vault_core_{tag}_{}", new_id()));
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).expect("open")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn def(key: &str, name: &str, kind: PropKind, config: &str) -> PropDef {
    PropDef {
        key: key.into(),
        name: name.into(),
        kind,
        options: vec![],
        config: config.into(),
    }
}

fn formula(key: &str, name: &str, src: &str) -> PropDef {
    def(
        key,
        name,
        PropKind::Formula,
        &serde_json::json!({ "formula": src }).to_string(),
    )
}

fn rollup(key: &str, name: &str, relation: &str, prop: &str, func: &str) -> PropDef {
    def(
        key,
        name,
        PropKind::Rollup,
        &serde_json::json!({
            "rollupRelation": relation,
            "rollupProperty": prop,
            "rollupFunction": func,
        })
        .to_string(),
    )
}

fn value(v: &Vault, page: &str, key: &str) -> Option<PropValue> {
    v.load_page(page.to_string())
        .unwrap()
        .properties
        .into_iter()
        .find(|p| p.key == key)
        .map(|p| p.value)
}

fn titles(r: &QueryResult) -> Vec<String> {
    r.rows.iter().map(|p| p.title.clone()).collect()
}

fn cond(prop: &str, op: FilterOp, value: PropValue) -> Filter {
    Filter::Cond(Condition {
        prop: prop.into(),
        op,
        value,
    })
}

#[test]
fn formulas_evaluate_filter_sort_and_report_errors() {
    let t = TempDir::new("formula");
    let v = t.open();
    let c = v.create_page(None, "Orders".into()).unwrap();
    v.set_schema(
        c.clone(),
        vec![
            def("price", "Price", PropKind::Number, ""),
            def("qty", "Qty", PropKind::Number, ""),
            formula("total", "Total", r#"prop("Price") * prop("Qty")"#),
            formula(
                "label",
                "Label",
                r#"prop("Name") + ": " + if(prop("Total") > 20, "big", "small")"#,
            ),
            formula("bad", "Bad", r#"prop("Price") / 0"#),
            formula("syntax", "Syntax", r#"1 +"#),
            formula("loop1", "Loop1", r#"prop("Loop2") + 1"#),
            formula("loop2", "Loop2", r#"prop("Loop1") + 1"#),
        ],
    )
    .unwrap();
    let rows = [
        ("Apples", 2.0, 3.0),
        ("Pears", 5.0, 6.0),
        ("Plums", 1.5, 10.0),
    ];
    let mut ids = Vec::new();
    for (name, price, qty) in rows {
        let r = v.create_page(Some(c.clone()), name.into()).unwrap();
        v.set_property(r.clone(), "price".into(), PropValue::Number(price))
            .unwrap();
        v.set_property(r.clone(), "qty".into(), PropValue::Number(qty))
            .unwrap();
        ids.push(r);
    }
    assert_eq!(value(&v, &ids[0], "total"), Some(PropValue::Number(6.0)));
    assert_eq!(
        value(&v, &ids[1], "label"),
        Some(PropValue::Text("Pears: big".into()))
    );
    assert_eq!(
        value(&v, &ids[0], "bad"),
        Some(PropValue::Error("Division by zero".into()))
    );
    assert!(matches!(
        value(&v, &ids[0], "syntax"),
        Some(PropValue::Error(_))
    ));
    assert!(
        matches!(value(&v, &ids[0], "loop1"), Some(PropValue::Error(e)) if e.contains("Circular"))
    );
    // Computed values cannot be set.
    assert!(v
        .set_property(ids[0].clone(), "total".into(), PropValue::Number(1.0))
        .is_err());
    // Recomputed when an input changes.
    v.set_property(ids[0].clone(), "qty".into(), PropValue::Number(20.0))
        .unwrap();
    assert_eq!(value(&v, &ids[0], "total"), Some(PropValue::Number(40.0)));
    // Renames are seen by formulas using the title.
    v.rename_page(ids[2].clone(), "Damsons".into()).unwrap();
    assert_eq!(
        value(&v, &ids[2], "label"),
        Some(PropValue::Text("Damsons: small".into()))
    );

    // Filters and sorts.
    let r = v
        .query(
            Some(c.clone()),
            Some(cond("total", FilterOp::Greater, PropValue::Number(20.0))),
            vec![Sort {
                prop: "total".into(),
                descending: true,
            }],
            0,
            10,
        )
        .unwrap();
    assert_eq!(titles(&r), ["Apples", "Pears"]);
    let r = v
        .query(
            Some(c.clone()),
            Some(cond(
                "label",
                FilterOp::Contains,
                PropValue::Text("SMALL".into()),
            )),
            vec![],
            0,
            10,
        )
        .unwrap();
    assert_eq!(titles(&r), ["Damsons"]);
    // Rule values typed as text still compare as numbers.
    let q = |value: PropValue| {
        let r = v
            .query(
                Some(c.clone()),
                Some(cond("total", FilterOp::Greater, value)),
                vec![],
                0,
                10,
            )
            .unwrap();
        titles(&r)
    };
    assert_eq!(q(PropValue::Text("20".into())), ["Apples", "Pears"]);
    assert!(q(PropValue::Text("abc".into())).is_empty());
    let r = v
        .query(
            Some(c.clone()),
            None,
            vec![Sort {
                prop: "total".into(),
                descending: false,
            }],
            0,
            10,
        )
        .unwrap();
    assert_eq!(titles(&r), ["Damsons", "Pears", "Apples"]);
    // Row summaries carry the values too.
    let total = r.rows[0]
        .properties
        .iter()
        .find(|p| p.key == "total")
        .map(|p| p.value.clone());
    assert_eq!(total, Some(PropValue::Number(15.0)));

    // Changing the formula recomputes every row; computed values are not in
    // the markdown files or the documents.
    let mut schema = v.get_schema(c.clone()).unwrap();
    schema.iter_mut().find(|d| d.key == "total").unwrap().config =
        serde_json::json!({ "formula": "prop(\"Qty\") + 1" }).to_string();
    v.set_schema(c.clone(), schema.clone()).unwrap();
    assert_eq!(value(&v, &ids[1], "total"), Some(PropValue::Number(7.0)));
    v.flush().unwrap();
    v.reindex().unwrap();
    assert_eq!(value(&v, &ids[1], "total"), Some(PropValue::Number(7.0)));
    // A formula turned into text loses its computed values.
    schema.iter_mut().find(|d| d.key == "total").unwrap().kind = PropKind::Text;
    v.set_schema(c.clone(), schema).unwrap();
    assert_eq!(value(&v, &ids[1], "total"), None);
}

#[test]
fn rollups_aggregate_over_relations() {
    let t = TempDir::new("rollup");
    let v = t.open();
    let tasks = v.create_page(None, "Tasks".into()).unwrap();
    let projects = v.create_page(None, "Projects".into()).unwrap();
    v.set_schema(
        tasks.clone(),
        vec![
            def("pts", "Points", PropKind::Number, ""),
            def("done", "Done", PropKind::Checkbox, ""),
            formula("double", "Double", r#"prop("Points") * 2"#),
        ],
    )
    .unwrap();
    let rel_config = serde_json::json!({ "relation": tasks }).to_string();
    v.set_schema(
        projects.clone(),
        vec![
            def("tasks", "Tasks", PropKind::Relation, &rel_config),
            rollup("count", "Count", "tasks", "$title", "count"),
            rollup("sum", "Sum", "tasks", "pts", "sum"),
            rollup("avg", "Avg", "tasks", "pts", "average"),
            rollup("min", "Min", "tasks", "pts", "min"),
            rollup("max", "Max", "tasks", "pts", "max"),
            rollup("names", "Names", "tasks", "$title", "show_original"),
            rollup("pct", "Done %", "tasks", "done", "percent_checked"),
            rollup("dbl", "Doubles", "tasks", "double", "sum"),
            rollup("broken", "Broken", "gone", "pts", "sum"),
            rollup("unset", "Unset", "", "", ""),
            formula("summary", "Summary", r#"format(prop("Sum")) + " pts""#),
        ],
    )
    .unwrap();
    let mut task_ids = Vec::new();
    for (name, pts, done) in [("A", 3.0, true), ("B", 5.0, false), ("C", 10.0, true)] {
        let r = v.create_page(Some(tasks.clone()), name.into()).unwrap();
        v.set_property(r.clone(), "pts".into(), PropValue::Number(pts))
            .unwrap();
        v.set_property(r.clone(), "done".into(), PropValue::Checkbox(done))
            .unwrap();
        task_ids.push(r);
    }
    let p = v
        .create_page(Some(projects.clone()), "Launch".into())
        .unwrap();
    v.set_property(
        p.clone(),
        "tasks".into(),
        PropValue::Relation(task_ids[..2].to_vec()),
    )
    .unwrap();
    assert_eq!(value(&v, &p, "count"), Some(PropValue::Number(2.0)));
    assert_eq!(value(&v, &p, "sum"), Some(PropValue::Number(8.0)));
    assert_eq!(value(&v, &p, "avg"), Some(PropValue::Number(4.0)));
    assert_eq!(value(&v, &p, "min"), Some(PropValue::Number(3.0)));
    assert_eq!(value(&v, &p, "max"), Some(PropValue::Number(5.0)));
    assert_eq!(value(&v, &p, "names"), Some(PropValue::Text("A, B".into())));
    assert_eq!(value(&v, &p, "pct"), Some(PropValue::Number(50.0)));
    assert_eq!(value(&v, &p, "dbl"), Some(PropValue::Number(16.0)));
    assert!(matches!(value(&v, &p, "broken"), Some(PropValue::Error(_))));
    assert_eq!(value(&v, &p, "unset"), None);
    assert_eq!(
        value(&v, &p, "summary"),
        Some(PropValue::Text("8 pts".into()))
    );

    // A change on the other side updates the rollup.
    v.set_property(task_ids[1].clone(), "pts".into(), PropValue::Number(7.0))
        .unwrap();
    assert_eq!(value(&v, &p, "sum"), Some(PropValue::Number(10.0)));
    assert_eq!(value(&v, &p, "dbl"), Some(PropValue::Number(20.0)));
    v.rename_page(task_ids[0].clone(), "Alpha".into()).unwrap();
    assert_eq!(
        value(&v, &p, "names"),
        Some(PropValue::Text("Alpha, B".into()))
    );
    // Adding a relation and trashing a related page.
    v.set_property(
        p.clone(),
        "tasks".into(),
        PropValue::Relation(task_ids.clone()),
    )
    .unwrap();
    assert_eq!(value(&v, &p, "sum"), Some(PropValue::Number(20.0)));
    v.delete_page(task_ids[2].clone()).unwrap();
    assert_eq!(value(&v, &p, "sum"), Some(PropValue::Number(10.0)));
    // Filters on rollups.
    let r = v
        .query(
            Some(projects.clone()),
            Some(cond(
                "sum",
                FilterOp::GreaterOrEqual,
                PropValue::Number(10.0),
            )),
            vec![],
            0,
            10,
        )
        .unwrap();
    assert_eq!(titles(&r), ["Launch"]);
    // A rebuilt index computes them again.
    v.reindex().unwrap();
    assert_eq!(value(&v, &p, "sum"), Some(PropValue::Number(10.0)));
    assert_eq!(value(&v, &p, "dbl"), Some(PropValue::Number(20.0)));
}

#[test]
fn files_property_references_attachments() {
    let t = TempDir::new("filesprop");
    let v = t.open();
    let c = v.create_page(None, "Docs".into()).unwrap();
    v.set_schema(c.clone(), vec![def("files", "Files", PropKind::Files, "")])
        .unwrap();
    let a = v
        .add_attachment_bytes("report.pdf".into(), b"%PDF report".to_vec())
        .unwrap();
    let b = v
        .add_attachment_bytes("photo.png".into(), b"PNG photo".to_vec())
        .unwrap();
    let unused = v
        .add_attachment_bytes("junk.txt".into(), b"junk".to_vec())
        .unwrap();
    let r = v.create_page(Some(c.clone()), "Row".into()).unwrap();
    v.set_property(
        r.clone(),
        "files".into(),
        PropValue::Files(vec![a.clone(), b.clone(), a.clone()]),
    )
    .unwrap();
    assert_eq!(
        value(&v, &r, "files"),
        Some(PropValue::Files(vec![a.clone(), b.clone()]))
    );
    // Attachments in a files property are kept by the garbage collector.
    assert_eq!(v.gc_attachments().unwrap(), 1);
    assert!(v.attachment_info(unused).is_err());
    assert!(v.attachment_info(a.clone()).is_ok());
    // Written to the markdown as blob links, read back by rescan.
    v.flush().unwrap();
    let md = fs::read_dir(t.0.join("pages"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(&format!("-{r}.md")))
        .unwrap();
    let text = fs::read_to_string(&md).unwrap();
    assert!(
        text.contains(&format!(".vault/blobs/{}/{a}", &a[..2])),
        "{text}"
    );
    fs::write(&md, format!("{text}\nEdited outside.\n")).unwrap();
    v.rescan().unwrap();
    let page = v.load_page(r.clone()).unwrap();
    assert!(page.blocks.iter().any(|b| b.text == "Edited outside."));
    assert_eq!(
        value(&v, &r, "files"),
        Some(PropValue::Files(vec![a.clone(), b.clone()]))
    );
    // Contains filters.
    let q = v
        .query(
            Some(c.clone()),
            Some(cond("files", FilterOp::Contains, PropValue::Files(vec![b]))),
            vec![],
            0,
            10,
        )
        .unwrap();
    assert_eq!(titles(&q), ["Row"]);
}

#[test]
fn rollup_rows_are_reported_changed_with_their_targets() {
    let t = TempDir::new("rollupfeed");
    let v = t.open();
    let tasks = v.create_page(None, "Tasks".into()).unwrap();
    let projects = v.create_page(None, "Projects".into()).unwrap();
    v.set_schema(
        tasks.clone(),
        vec![def("pts", "Points", PropKind::Number, "")],
    )
    .unwrap();
    let rel = serde_json::json!({ "relation": tasks }).to_string();
    v.set_schema(
        projects.clone(),
        vec![
            def("tasks", "Tasks", PropKind::Relation, &rel),
            rollup("sum", "Sum", "tasks", "pts", "sum"),
        ],
    )
    .unwrap();
    let task = v.create_page(Some(tasks.clone()), "T".into()).unwrap();
    let p = v.create_page(Some(projects.clone()), "P".into()).unwrap();
    v.set_property(
        p.clone(),
        "tasks".into(),
        PropValue::Relation(vec![task.clone()]),
    )
    .unwrap();
    let seq = v.poll_changes(0).seq;
    v.set_property(task.clone(), "pts".into(), PropValue::Number(4.0))
        .unwrap();
    let b = v.poll_changes(seq);
    assert!(b.changes.contains(&Change::PageChanged(p.clone())), "{b:?}");
    assert_eq!(value(&v, &p, "sum"), Some(PropValue::Number(4.0)));
}
