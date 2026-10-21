//! Integration tests for the MVP data features (Core: MVP scope, Part A, internal wiki).

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
    fn root(&self) -> String {
        self.0.to_string_lossy().into_owned()
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

fn nb(id: &str, kind: BlockKind, text: &str) -> NewBlock {
    NewBlock {
        id: id.into(),
        kind,
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

fn md_path(t: &TempDir, id: &str) -> Option<PathBuf> {
    fs::read_dir(t.0.join("pages"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(&format!("-{id}.md")))
}

fn md_file(t: &TempDir, id: &str) -> String {
    fs::read_to_string(md_path(t, id).expect("markdown file")).unwrap()
}

/// Milliseconds of midnight UTC of a civil date.
fn day(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146_097 + doe - 719_468) * 86_400_000
}

fn date(start_ms: i64, end_ms: Option<i64>) -> PropValue {
    PropValue::Date {
        start_ms,
        end_ms,
        has_time: false,
    }
}

fn cond(prop: &str, op: FilterOp, value: PropValue) -> Filter {
    Filter::Cond(Condition {
        prop: prop.into(),
        op,
        value,
    })
}

fn titles(r: &QueryResult) -> Vec<String> {
    r.rows.iter().map(|p| p.title.clone()).collect()
}

fn opt(name: &str) -> SelectOption {
    SelectOption {
        id: String::new(),
        name: name.into(),
        color: String::new(),
    }
}

fn def(key: &str, name: &str, kind: PropKind, options: Vec<SelectOption>) -> PropDef {
    PropDef {
        key: key.into(),
        name: name.into(),
        kind,
        options,
        config: String::new(),
    }
}

fn prop<'a>(props: &'a [Property], key: &str) -> Option<&'a PropValue> {
    props.iter().find(|p| p.key == key).map(|p| &p.value)
}

/// A collection "Tasks" with status (To do, Doing, Done), due, tags (a, b,
/// c), points and done. Returns (collection, schema).
fn tasks(v: &Vault) -> (PageId, Vec<PropDef>) {
    let c = v.create_page(None, "Tasks".into()).unwrap();
    let schema = v
        .set_schema(
            c.clone(),
            vec![
                def(
                    "status",
                    "Status",
                    PropKind::Status,
                    vec![opt("To do"), opt("Doing"), opt("Done")],
                ),
                def("due", "Due", PropKind::Date, vec![]),
                def(
                    "tags",
                    "Tags",
                    PropKind::MultiSelect,
                    vec![opt("a"), opt("b"), opt("c")],
                ),
                def("points", "Points", PropKind::Number, vec![]),
                def("done", "Done", PropKind::Checkbox, vec![]),
                def("note", "Note", PropKind::Text, vec![]),
            ],
        )
        .unwrap();
    (c, schema)
}

fn option_id(schema: &[PropDef], key: &str, name: &str) -> String {
    schema
        .iter()
        .find(|d| d.key == key)
        .unwrap()
        .options
        .iter()
        .find(|o| o.name == name)
        .unwrap()
        .id
        .clone()
}

// ------------------------------------------------------------------ properties

#[test]
fn properties_are_stored_indexed_and_written() {
    let t = TempDir::new("props");
    let p;
    {
        let v = t.open();
        p = v.create_page(None, "Trip".into()).unwrap();
        v.set_property(p.clone(), "city".into(), PropValue::Text("Zürich".into()))
            .unwrap();
        v.set_property(p.clone(), "budget".into(), PropValue::Number(1200.5))
            .unwrap();
        v.set_property(
            p.clone(),
            "when".into(),
            date(day(2026, 10, 1), Some(day(2026, 10, 3))),
        )
        .unwrap();
        v.set_property(p.clone(), "booked".into(), PropValue::Checkbox(true))
            .unwrap();
        v.set_property(
            p.clone(),
            "tags".into(),
            PropValue::MultiSelect(vec!["x".into(), "y".into(), "x".into()]),
        )
        .unwrap();
        v.set_property(
            p.clone(),
            "site".into(),
            PropValue::Url("https://a.b".into()),
        )
        .unwrap();
        // Invalid values and reserved keys.
        assert!(matches!(
            v.set_property(p.clone(), "n".into(), PropValue::Number(f64::NAN)),
            Err(VaultError::Invalid(_))
        ));
        assert!(matches!(
            v.set_property(p.clone(), "$title".into(), PropValue::Text("x".into())),
            Err(VaultError::Invalid(_))
        ));
        assert!(matches!(
            v.set_property(p.clone(), "d".into(), date(10, Some(5))),
            Err(VaultError::Invalid(_))
        ));
        // Empty removes.
        v.set_property(p.clone(), "site".into(), PropValue::Text(String::new()))
            .unwrap();
        v.set_property(p.clone(), "booked".into(), PropValue::Empty)
            .unwrap();
        v.remove_property(p.clone(), "budget".into()).unwrap();

        let page = v.load_page(p.clone()).unwrap();
        let keys: Vec<&str> = page.properties.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, ["city", "tags", "when"]);
        assert_eq!(
            prop(&page.properties, "tags"),
            Some(&PropValue::MultiSelect(vec!["x".into(), "y".into()]))
        );
        // Summaries carry them too; queries see them at once.
        let s = v.list_pages(None, 0, 10).unwrap();
        assert_eq!(s[0].properties, page.properties);
        let r = v
            .query(
                None,
                Some(cond(
                    "city",
                    FilterOp::Contains,
                    PropValue::Text("zür".into()),
                )),
                vec![],
                0,
                10,
            )
            .unwrap();
        assert_eq!(r.total, 1);
        v.flush().unwrap();
        let md = md_file(&t, &p);
        assert!(
            md.contains("city: \"Zürich\"") || md.contains("city: Zürich"),
            "{md}"
        );
        assert!(md.contains("when:"), "{md}");
    }
    // Persisted in the document and rebuilt into the index.
    let v = t.open();
    assert_eq!(v.load_page(p.clone()).unwrap().properties.len(), 3);
    v.reindex().unwrap();
    let r = v
        .query(
            None,
            Some(cond(
                "tags",
                FilterOp::Contains,
                PropValue::MultiSelect(vec!["y".into()]),
            )),
            vec![],
            0,
            10,
        )
        .unwrap();
    assert_eq!(titles(&r), ["Trip"]);
}

// ----------------------------------------------------------------- collections

#[test]
fn schema_rename_delete_and_kind_change_update_rows() {
    let t = TempDir::new("schema");
    let v = t.open();
    let (c, schema) = tasks(&v);
    assert!(schema.iter().all(|d| !d.key.is_empty()));
    assert!(schema[0]
        .options
        .iter()
        .all(|o| !o.id.is_empty() && o.color == "default"));
    assert!(v.load_page(c.clone()).unwrap().is_database());
    assert_eq!(v.list_views(c.clone()).unwrap().len(), 1);
    let done = option_id(&schema, "status", "Done");
    let tag_a = option_id(&schema, "tags", "a");
    let tag_b = option_id(&schema, "tags", "b");

    let r1 = v.create_page(Some(c.clone()), "One".into()).unwrap();
    v.set_property(r1.clone(), "status".into(), PropValue::Status(done.clone()))
        .unwrap();
    v.set_property(
        r1.clone(),
        "tags".into(),
        PropValue::MultiSelect(vec![tag_a.clone(), tag_b.clone()]),
    )
    .unwrap();
    v.set_property(r1.clone(), "note".into(), PropValue::Text("42".into()))
        .unwrap();
    v.set_property(r1.clone(), "points".into(), PropValue::Number(3.0))
        .unwrap();
    // Rows take only schema keys, values of the right kind and known options.
    for (k, val) in [
        ("nope", PropValue::Text("x".into())),
        ("status", PropValue::Status("unknown".into())),
        ("status", PropValue::Text("Done".into())),
        ("points", PropValue::Text("3".into())),
    ] {
        assert!(
            matches!(
                v.set_property(r1.clone(), k.into(), val),
                Err(VaultError::Invalid(_))
            ),
            "{k}"
        );
    }

    // A view that uses "points" and "tags".
    let view = v
        .add_view(
            c.clone(),
            ViewDef {
                id: String::new(),
                name: "Big".into(),
                kind: ViewKind::Board,
                filter: Some(Filter::And(vec![
                    cond("points", FilterOp::Greater, PropValue::Number(1.0)),
                    cond("status", FilterOp::IsNotEmpty, PropValue::Empty),
                ])),
                sorts: vec![Sort {
                    prop: "points".into(),
                    descending: true,
                }],
                group_by: Some("tags".into()),
                date_prop: None,
                visible_props: vec!["points".into(), "status".into()],
                card_cover: None,
                config: String::new(),
            },
        )
        .unwrap();

    // Rename: only the name changes; values stay, markdown uses the new name.
    let mut s2 = schema.clone();
    s2.iter_mut().find(|d| d.key == "status").unwrap().name = "State".into();
    // Delete "points", drop option "b", and turn "note" into a number.
    s2.retain(|d| d.key != "points");
    s2.iter_mut()
        .find(|d| d.key == "tags")
        .unwrap()
        .options
        .retain(|o| o.name != "b");
    s2.iter_mut().find(|d| d.key == "note").unwrap().kind = PropKind::Number;
    let s2 = v.set_schema(c.clone(), s2).unwrap();
    assert_eq!(s2.iter().find(|d| d.key == "status").unwrap().name, "State");

    let row = v.load_page(r1.clone()).unwrap();
    assert_eq!(
        prop(&row.properties, "status"),
        Some(&PropValue::Status(done))
    );
    assert_eq!(prop(&row.properties, "points"), None);
    assert_eq!(
        prop(&row.properties, "tags"),
        Some(&PropValue::MultiSelect(vec![tag_a]))
    );
    assert_eq!(
        prop(&row.properties, "note"),
        Some(&PropValue::Number(42.0))
    );
    // The view lost its references to "points".
    let vd = v
        .list_views(c.clone())
        .unwrap()
        .into_iter()
        .find(|x| x.id == view)
        .unwrap();
    assert!(vd.sorts.is_empty());
    assert_eq!(vd.visible_props, ["status"]);
    assert_eq!(
        vd.filter,
        Some(Filter::And(vec![cond(
            "status",
            FilterOp::IsNotEmpty,
            PropValue::Empty
        )]))
    );
    assert_eq!(vd.group_by.as_deref(), Some("tags"));
    v.flush().unwrap();
    let md = md_file(&t, &r1);
    assert!(
        md.contains("State: Done") || md.contains("State: \"Done\""),
        "{md}"
    );
    assert!(!md.contains("Points"), "{md}");

    // Duplicate names are rejected.
    let mut bad = s2.clone();
    bad.push(def("", "state", PropKind::Text, vec![]));
    assert!(matches!(
        v.set_schema(c.clone(), bad),
        Err(VaultError::Invalid(_))
    ));
}

#[test]
fn views_are_stored_in_the_collection_document() {
    let t = TempDir::new("views");
    let (c, ids) = {
        let v = t.open();
        let (c, _) = tasks(&v);
        let mk = |name: &str, kind| ViewDef {
            id: String::new(),
            name: name.into(),
            kind,
            filter: None,
            sorts: vec![],
            group_by: None,
            date_prop: Some("due".into()),
            visible_props: vec![],
            card_cover: None,
            config: String::new(),
        };
        let a = v
            .add_view(c.clone(), mk("Cal", ViewKind::Calendar))
            .unwrap();
        let b = v
            .add_view(c.clone(), mk("Line", ViewKind::Timeline))
            .unwrap();
        v.move_view(c.clone(), b.clone(), 0).unwrap();
        let mut upd = v.list_views(c.clone()).unwrap()[2].clone();
        upd.name = "Calendar".into();
        v.update_view(c.clone(), upd).unwrap();
        // Unknown properties are rejected.
        let mut bad = mk("Bad", ViewKind::Table);
        bad.group_by = Some("nope".into());
        assert!(matches!(
            v.add_view(c.clone(), bad),
            Err(VaultError::Invalid(_))
        ));
        assert!(matches!(
            v.delete_view(c.clone(), "nope".into()),
            Err(VaultError::NotFound(_))
        ));
        (c, (a, b))
    };
    let v = t.open();
    let views = v.list_views(c.clone()).unwrap();
    let names: Vec<&str> = views.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(names, ["Line", "Table", "Calendar"]);
    assert_eq!(views[0].id, ids.1);
    v.delete_view(c.clone(), ids.0).unwrap();
    assert_eq!(v.list_views(c).unwrap().len(), 2);
}

#[test]
fn default_template_is_copied_into_new_rows() {
    let t = TempDir::new("template");
    let v = t.open();
    let (c, schema) = tasks(&v);
    let todo = option_id(&schema, "status", "To do");
    let tpl = v.create_page(Some(c.clone()), "Template".into()).unwrap();
    v.apply(
        tpl.clone(),
        vec![
            ins(None, nb("h", BlockKind::Heading2, "Steps")),
            ins(Some("h"), nb("t", BlockKind::Todo, "First step")),
        ],
    )
    .unwrap();
    v.set_property(
        tpl.clone(),
        "status".into(),
        PropValue::Status(todo.clone()),
    )
    .unwrap();
    v.set_icon(tpl.clone(), Some("📌".into())).unwrap();
    v.set_default_template(c.clone(), Some(tpl.clone()))
        .unwrap();
    assert_eq!(v.default_template(c.clone()).unwrap(), Some(tpl.clone()));
    let other = v.create_page(None, "Elsewhere".into()).unwrap();
    assert!(v.set_default_template(c.clone(), Some(other)).is_err());

    let row = v.create_page(Some(c.clone()), "Real".into()).unwrap();
    let p = v.load_page(row.clone()).unwrap();
    assert_eq!(p.title, "Real");
    assert_eq!(p.icon.as_deref(), Some("📌"));
    assert_eq!(
        prop(&p.properties, "status"),
        Some(&PropValue::Status(todo))
    );
    let texts: Vec<&str> = p.blocks.iter().map(|b| b.text.as_str()).collect();
    assert_eq!(texts, ["Steps", "First step"]);
    assert!(p.blocks.iter().all(|b| b.id != "h" && b.id != "t"));
    // The template is no row of queries.
    let r = v.query(Some(c.clone()), None, vec![], 0, 10).unwrap();
    assert_eq!(titles(&r), ["Real"]);
    v.set_default_template(c.clone(), None).unwrap();
    let plain = v.create_page(Some(c.clone()), "Plain".into()).unwrap();
    assert!(v.load_page(plain).unwrap().blocks.is_empty());
}

// -------------------------------------------------------------------- queries

/// Rows: (title, status, due, tags, points, done).
fn fill(v: &Vault, c: &str, schema: &[PropDef]) {
    let st = |n: &str| PropValue::Status(option_id(schema, "status", n));
    let tg = |ns: &[&str]| {
        PropValue::MultiSelect(ns.iter().map(|n| option_id(schema, "tags", n)).collect())
    };
    let rows: Vec<(&str, PropValue, PropValue, PropValue, PropValue, bool)> = vec![
        (
            "Alpha",
            st("To do"),
            date(day(2026, 1, 30), Some(day(2026, 2, 2))),
            tg(&["a"]),
            PropValue::Number(5.0),
            false,
        ),
        (
            "Beta",
            st("Doing"),
            date(day(2026, 2, 10), None),
            tg(&["a", "b"]),
            PropValue::Number(1.0),
            true,
        ),
        (
            "Gamma",
            st("Done"),
            date(day(2026, 3, 1), None),
            tg(&["c"]),
            PropValue::Number(8.0),
            true,
        ),
        (
            "Delta",
            st("To do"),
            PropValue::Empty,
            PropValue::Empty,
            PropValue::Empty,
            false,
        ),
        (
            "alphabet",
            PropValue::Empty,
            date(day(2026, 1, 31), None),
            tg(&["b"]),
            PropValue::Number(3.0),
            false,
        ),
    ];
    for (title, s, d, tags, pts, done) in rows {
        let id = v.create_page(Some(c.to_string()), title.into()).unwrap();
        for (k, val) in [("status", s), ("due", d), ("tags", tags), ("points", pts)] {
            v.set_property(id.clone(), k.into(), val).unwrap();
        }
        if done {
            v.set_property(id.clone(), "done".into(), PropValue::Checkbox(true))
                .unwrap();
        }
    }
}

#[test]
fn query_filter_combinations_and_sorts() {
    let t = TempDir::new("query");
    let v = t.open();
    let (c, schema) = tasks(&v);
    fill(&v, &c, &schema);
    let q =
        |f: Option<Filter>, sorts: Vec<Sort>| v.query(Some(c.clone()), f, sorts, 0, 100).unwrap();
    let asc = |p: &str| Sort {
        prop: p.into(),
        descending: false,
    };
    let desc = |p: &str| Sort {
        prop: p.into(),
        descending: true,
    };

    // No filter: sibling (creation) order.
    assert_eq!(
        titles(&q(None, vec![])),
        ["Alpha", "Beta", "Gamma", "Delta", "alphabet"]
    );
    // Title sorts case-insensitively.
    assert_eq!(
        titles(&q(None, vec![asc("$title")])),
        ["Alpha", "alphabet", "Beta", "Delta", "Gamma"]
    );
    // Numbers, empties last in both directions.
    assert_eq!(
        titles(&q(None, vec![asc("points")])),
        ["Beta", "alphabet", "Alpha", "Gamma", "Delta"]
    );
    assert_eq!(
        titles(&q(None, vec![desc("points")])),
        ["Gamma", "Alpha", "alphabet", "Beta", "Delta"]
    );
    // Status sorts in option order.
    assert_eq!(
        titles(&q(None, vec![desc("status"), asc("$title")])),
        ["Gamma", "Beta", "Alpha", "Delta", "alphabet"]
    );

    let done = PropValue::Status(option_id(&schema, "status", "Done"));
    let todo = PropValue::Status(option_id(&schema, "status", "To do"));
    let a = option_id(&schema, "tags", "a");
    let b = option_id(&schema, "tags", "b");
    let cases: Vec<(Filter, Vec<&str>)> = vec![
        (
            cond("status", FilterOp::Is, todo.clone()),
            vec!["Alpha", "Delta"],
        ),
        (
            cond("status", FilterOp::IsNot, done.clone()),
            vec!["Alpha", "Beta", "Delta", "alphabet"],
        ),
        (
            cond("status", FilterOp::IsEmpty, PropValue::Empty),
            vec!["alphabet"],
        ),
        (
            cond(
                "tags",
                FilterOp::Contains,
                PropValue::MultiSelect(vec![a.clone()]),
            ),
            vec!["Alpha", "Beta"],
        ),
        (
            cond(
                "tags",
                FilterOp::Contains,
                PropValue::MultiSelect(vec![a.clone(), b.clone()]),
            ),
            vec!["Beta"],
        ),
        (
            cond(
                "tags",
                FilterOp::NotContains,
                PropValue::MultiSelect(vec![b]),
            ),
            vec!["Alpha", "Gamma", "Delta"],
        ),
        (
            cond("points", FilterOp::GreaterOrEqual, PropValue::Number(3.0)),
            vec!["Alpha", "Gamma", "alphabet"],
        ),
        (
            cond("points", FilterOp::Less, PropValue::Number(3.0)),
            vec!["Beta"],
        ),
        (
            cond("done", FilterOp::Checked, PropValue::Empty),
            vec!["Beta", "Gamma"],
        ),
        (
            cond("done", FilterOp::Unchecked, PropValue::Empty),
            vec!["Alpha", "Delta", "alphabet"],
        ),
        (
            cond("due", FilterOp::Before, date(day(2026, 2, 1), None)),
            vec!["Alpha", "alphabet"],
        ),
        (
            cond("due", FilterOp::OnOrAfter, date(day(2026, 2, 10), None)),
            vec!["Beta", "Gamma"],
        ),
        // Alpha's range (Jan 30 - Feb 2) overlaps February.
        (
            cond(
                "due",
                FilterOp::WithinRange,
                date(day(2026, 2, 1), Some(day(2026, 3, 1))),
            ),
            vec!["Alpha", "Beta"],
        ),
        (
            cond(
                "$title",
                FilterOp::StartsWith,
                PropValue::Text("ALPHA".into()),
            ),
            vec!["Alpha", "alphabet"],
        ),
        (
            cond("$title", FilterOp::Contains, PropValue::Text("et".into())),
            vec!["Beta", "alphabet"],
        ),
        (
            cond("$title", FilterOp::EndsWith, PropValue::Text("a".into())),
            vec!["Alpha", "Beta", "Gamma", "Delta"],
        ),
        (
            cond("$title", FilterOp::Is, PropValue::Text("beta".into())),
            vec!["Beta"],
        ),
        (
            Filter::And(vec![
                cond("status", FilterOp::IsNot, done.clone()),
                Filter::Or(vec![
                    cond("points", FilterOp::Greater, PropValue::Number(4.0)),
                    cond("due", FilterOp::IsEmpty, PropValue::Empty),
                ]),
            ]),
            vec!["Alpha", "Delta"],
        ),
        (
            Filter::Or(vec![
                cond("status", FilterOp::Is, done),
                Filter::And(vec![
                    cond("tags", FilterOp::Contains, PropValue::MultiSelect(vec![a])),
                    cond("done", FilterOp::Checked, PropValue::Empty),
                ]),
            ]),
            vec!["Beta", "Gamma"],
        ),
        (
            Filter::And(vec![]),
            vec!["Alpha", "Beta", "Gamma", "Delta", "alphabet"],
        ),
        (Filter::Or(vec![]), vec![]),
    ];
    for (f, want) in cases {
        let got = q(Some(f.clone()), vec![]);
        assert_eq!(titles(&got), want, "{f:?}");
        assert_eq!(got.total as usize, want.len());
    }
    // Ops that do not fit the value.
    assert!(v
        .query(
            Some(c.clone()),
            Some(cond("points", FilterOp::Before, PropValue::Number(1.0))),
            vec![],
            0,
            10
        )
        .is_err());
    // Paging keeps the total.
    let page = v
        .query(Some(c.clone()), None, vec![asc("$title")], 1, 2)
        .unwrap();
    assert_eq!(
        (titles(&page), page.total),
        (vec!["alphabet".to_string(), "Beta".into()], 5)
    );
    let past = v.query(Some(c.clone()), None, vec![], 10, 2).unwrap();
    assert_eq!((past.rows.len(), past.total), (0, 5));

    // Vault scope: every page, `*` means any property.
    let other = v.create_page(None, "Loose".into()).unwrap();
    v.set_property(other, "when".into(), date(day(2026, 2, 3), None))
        .unwrap();
    let r = v
        .query(
            None,
            Some(cond(
                "*",
                FilterOp::WithinRange,
                date(day(2026, 2, 1), Some(day(2026, 2, 8))),
            )),
            vec![asc("$title")],
            0,
            10,
        )
        .unwrap();
    assert_eq!(titles(&r), ["Alpha", "Loose"]);
    let all = v.query(None, None, vec![], 0, 100).unwrap();
    assert_eq!(all.total, 7); // collection, five rows, one loose page
}

#[test]
fn board_groups_count_rows_per_column() {
    let t = TempDir::new("groups");
    let v = t.open();
    let (c, schema) = tasks(&v);
    fill(&v, &c, &schema);
    let g = v
        .query_groups(Some(c.clone()), None, "status".into())
        .unwrap();
    let got: Vec<(&str, u32)> = g.iter().map(|x| (x.label.as_str(), x.count)).collect();
    assert_eq!(got, [("", 1), ("To do", 2), ("Doing", 1), ("Done", 1)]);
    assert_eq!(g[0].value, PropValue::Empty);
    assert_eq!(g[1].color.as_deref(), Some("default"));
    // A group's value queries its rows.
    let r = v
        .query(
            Some(c.clone()),
            Some(cond("status", FilterOp::Is, g[1].value.clone())),
            vec![],
            0,
            10,
        )
        .unwrap();
    assert_eq!(r.total, 2);
    // Multi-select rows count in each of their options.
    let g = v
        .query_groups(Some(c.clone()), None, "tags".into())
        .unwrap();
    let got: Vec<(&str, u32)> = g.iter().map(|x| (x.label.as_str(), x.count)).collect();
    assert_eq!(got, [("", 1), ("a", 2), ("b", 2), ("c", 1)]);
    // With a filter; and a checkbox.
    let g = v
        .query_groups(
            Some(c.clone()),
            Some(cond("points", FilterOp::Greater, PropValue::Number(2.0))),
            "done".into(),
        )
        .unwrap();
    let got: Vec<(&str, u32)> = g.iter().map(|x| (x.label.as_str(), x.count)).collect();
    assert_eq!(got, [("false", 2), ("true", 1)]);
    // Numbers: one column per value, sorted, empty first.
    let g = v
        .query_groups(Some(c.clone()), None, "points".into())
        .unwrap();
    let got: Vec<(&str, u32)> = g.iter().map(|x| (x.label.as_str(), x.count)).collect();
    assert_eq!(got, [("", 1), ("1", 1), ("3", 1), ("5", 1), ("8", 1)]);
    assert!(v.query_groups(Some(c), None, "$title".into()).is_err());
}

#[test]
fn calendar_ranges_cross_month_boundaries() {
    let t = TempDir::new("range");
    let v = t.open();
    let (c, schema) = tasks(&v);
    fill(&v, &c, &schema);
    let feb = v
        .query_range(
            Some(c.clone()),
            "due".into(),
            day(2026, 2, 1),
            day(2026, 3, 1),
            None,
        )
        .unwrap();
    let got: Vec<(&str, i64)> = feb
        .iter()
        .map(|e| (e.page.title.as_str(), e.start_ms))
        .collect();
    // Alpha starts in January and ends in February; Gamma starts on March 1
    // (the end is exclusive).
    assert_eq!(
        got,
        [("Alpha", day(2026, 1, 30)), ("Beta", day(2026, 2, 10))]
    );
    assert_eq!(feb[0].end_ms, Some(day(2026, 2, 2)));
    assert!(!feb[0].has_time);
    assert_eq!(feb[0].prop, "due");
    let jan = v
        .query_range(
            Some(c.clone()),
            "due".into(),
            day(2026, 1, 1),
            day(2026, 2, 1),
            None,
        )
        .unwrap();
    let got: Vec<&str> = jan.iter().map(|e| e.page.title.as_str()).collect();
    assert_eq!(got, ["Alpha", "alphabet"]);
    // Year boundary and filters.
    let dec = v
        .query_range(
            Some(c.clone()),
            "due".into(),
            day(2025, 12, 1),
            day(2026, 1, 31),
            Some(cond("points", FilterOp::Greater, PropValue::Number(4.0))),
        )
        .unwrap();
    assert_eq!(
        dec.iter()
            .map(|e| e.page.title.as_str())
            .collect::<Vec<_>>(),
        ["Alpha"]
    );
    // Created time as the date.
    let now = v.load_page(c.clone()).unwrap().created_ms;
    let made = v
        .query_range(
            Some(c.clone()),
            "$created".into(),
            now - 60_000,
            now + 3_600_000,
            None,
        )
        .unwrap();
    assert_eq!(made.len(), 5);
    // The vault scope with any date property.
    let any = v
        .query_range(None, "*".into(), day(2026, 3, 1), day(2026, 4, 1), None)
        .unwrap();
    assert_eq!(
        any.iter()
            .map(|e| e.page.title.as_str())
            .collect::<Vec<_>>(),
        ["Gamma"]
    );
}

// ------------------------------------------------------------------- the tree

#[test]
fn move_page_keeps_a_stable_sibling_order() {
    let t = TempDir::new("move");
    let (a, b, c, x) = {
        let v = t.open();
        let a = v.create_page(None, "A".into()).unwrap();
        let b = v.create_page(None, "B".into()).unwrap();
        let c = v.create_page(None, "C".into()).unwrap();
        let x = v.create_page(Some(a.clone()), "X".into()).unwrap();
        let order = |v: &Vault, p: Option<PageId>| -> Vec<String> {
            v.list_children(p, 0, 100)
                .unwrap()
                .into_iter()
                .map(|s| s.title)
                .collect()
        };
        assert_eq!(order(&v, None), ["A", "B", "C"]);
        v.move_page(c.clone(), None, None).unwrap();
        assert_eq!(order(&v, None), ["C", "A", "B"]);
        v.move_page(c.clone(), None, Some(a.clone())).unwrap();
        assert_eq!(order(&v, None), ["A", "C", "B"]);
        // Into another page, after its child.
        v.move_page(b.clone(), Some(a.clone()), Some(x.clone()))
            .unwrap();
        assert_eq!(order(&v, None), ["A", "C"]);
        assert_eq!(order(&v, Some(a.clone())), ["X", "B"]);
        assert_eq!(
            v.load_page(b.clone()).unwrap().parent.as_deref(),
            Some(a.as_str())
        );
        // Not into itself or its subtree; `after` must be a sibling.
        assert!(matches!(
            v.move_page(a.clone(), Some(x.clone()), None),
            Err(VaultError::Invalid(_))
        ));
        assert!(matches!(
            v.move_page(a.clone(), Some(a.clone()), None),
            Err(VaultError::Invalid(_))
        ));
        assert!(matches!(
            v.move_page(c.clone(), None, Some(x.clone())),
            Err(VaultError::Invalid(_))
        ));
        // Many inserts at the front stay ordered.
        for _ in 0..30 {
            v.move_page(c.clone(), None, None).unwrap();
            v.move_page(c.clone(), None, Some(a.clone())).unwrap();
        }
        assert_eq!(order(&v, None), ["A", "C"]);
        (a, b, c, x)
    };
    let v = t.open();
    let top: Vec<String> = v
        .list_children(None, 0, 10)
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(top, [a.clone(), c]);
    let kids: Vec<String> = v
        .list_children(Some(a), 0, 10)
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(kids, [x, b]);
}

#[test]
fn duplicate_copies_the_subtree_after_the_original() {
    let t = TempDir::new("dup");
    let v = t.open();
    let a = v.create_page(None, "Plan".into()).unwrap();
    let z = v.create_page(None, "Zed".into()).unwrap();
    v.apply(
        a.clone(),
        vec![ins(None, nb("p", BlockKind::Paragraph, "Body"))],
    )
    .unwrap();
    v.set_property(a.clone(), "k".into(), PropValue::Text("v".into()))
        .unwrap();
    v.set_favorite(a.clone(), true).unwrap();
    let child = v.create_page(Some(a.clone()), "Step".into()).unwrap();
    v.apply(
        child.clone(),
        vec![ins(None, nb("q", BlockKind::Todo, "Do it"))],
    )
    .unwrap();

    let copy = v.duplicate_page(a.clone()).unwrap();
    let top: Vec<String> = v
        .list_children(None, 0, 10)
        .unwrap()
        .into_iter()
        .map(|s| s.title)
        .collect();
    assert_eq!(top, ["Plan", "Plan (copy)", "Zed"]);
    let p = v.load_page(copy.clone()).unwrap();
    assert_eq!(p.blocks[0].text, "Body");
    assert_ne!(p.blocks[0].id, "p");
    assert_eq!(prop(&p.properties, "k"), Some(&PropValue::Text("v".into())));
    assert!(!p.favorite);
    let kids = v.list_children(Some(copy.clone()), 0, 10).unwrap();
    assert_eq!(kids.len(), 1);
    assert_eq!(kids[0].title, "Step");
    assert_ne!(kids[0].id, child);
    assert_eq!(
        v.load_page(kids[0].id.clone()).unwrap().blocks[0].text,
        "Do it"
    );
    // The original is untouched.
    assert_eq!(v.list_children(Some(a), 0, 10).unwrap()[0].id, child);
    let _ = z;
}

#[test]
fn trash_restores_nested_pages() {
    let t = TempDir::new("trash");
    let v = t.open();
    let a = v.create_page(None, "Garden".into()).unwrap();
    let b = v.create_page(Some(a.clone()), "Beds".into()).unwrap();
    let c = v.create_page(Some(b.clone()), "Carrots".into()).unwrap();
    let d = v.create_page(Some(a.clone()), "Dahlias".into()).unwrap();
    v.flush().unwrap();
    assert!(md_path(&t, &c).is_some());

    // Trash a grandchild on its own, then the root.
    v.delete_page(c.clone()).unwrap();
    v.delete_page(a.clone()).unwrap();
    assert_eq!(v.page_count().unwrap(), 0);
    assert!(v.search("dahlias".into(), 10).unwrap().is_empty());
    assert!(v.query(None, None, vec![], 0, 10).unwrap().rows.is_empty());
    let trash: Vec<String> = v
        .list_trash()
        .unwrap()
        .into_iter()
        .map(|x| x.page.title)
        .collect();
    assert_eq!(trash, ["Garden", "Carrots"]);
    v.flush().unwrap();
    for id in [&a, &b, &c, &d] {
        assert!(
            md_path(&t, id).is_none(),
            "markdown of trashed pages is removed"
        );
        assert!(v.load_page(id.clone()).unwrap().in_trash);
    }
    // Pages in the trash can't take children or move.
    assert!(v.create_page(Some(b.clone()), "x".into()).is_err());
    assert!(v.move_page(b.clone(), None, None).is_err());

    // Restoring the root brings back its subtree, but not Carrots.
    v.restore_page(a.clone()).unwrap();
    assert_eq!(v.page_count().unwrap(), 3);
    assert!(v.load_page(c.clone()).unwrap().in_trash);
    assert!(!v.load_page(b.clone()).unwrap().in_trash);
    assert_eq!(v.search("dahlias".into(), 10).unwrap().len(), 1);
    v.flush().unwrap();
    assert!(md_path(&t, &d).is_some());
    assert!(md_path(&t, &c).is_none());
    // Restoring Carrots puts it back under Beds.
    v.restore_page(c.clone()).unwrap();
    assert_eq!(
        v.load_page(c.clone()).unwrap().parent.as_deref(),
        Some(b.as_str())
    );

    // A page whose parent is still in the trash comes back at the top level.
    v.delete_page(d.clone()).unwrap();
    v.delete_page(a.clone()).unwrap();
    v.restore_page(d.clone()).unwrap();
    let dp = v.load_page(d.clone()).unwrap();
    assert!(!dp.in_trash);
    assert_eq!(dp.parent, None);

    // Purge and empty.
    assert!(v.purge_page(d.clone()).is_err(), "not in the trash");
    assert_eq!(v.empty_trash().unwrap(), 3);
    for id in [&a, &b, &c] {
        assert!(matches!(
            v.load_page(id.clone()),
            Err(VaultError::NotFound(_))
        ));
    }
    assert!(v.list_trash().unwrap().is_empty());
    drop(v);
    let v = t.open();
    assert_eq!(v.page_count().unwrap(), 1);
    v.reindex().unwrap();
    assert_eq!(v.page_count().unwrap(), 1);
}

#[test]
fn favorites_icon_and_cover() {
    let t = TempDir::new("fav");
    let v = t.open();
    let a = v.create_page(None, "A".into()).unwrap();
    let b = v.create_page(None, "B".into()).unwrap();
    v.set_favorite(b.clone(), true).unwrap();
    v.set_favorite(a.clone(), true).unwrap();
    v.set_favorite(a.clone(), true).unwrap();
    let favs: Vec<String> = v
        .list_favorites()
        .unwrap()
        .into_iter()
        .map(|p| p.title)
        .collect();
    assert_eq!(favs, ["B", "A"]);
    v.set_favorite(b.clone(), false).unwrap();
    assert_eq!(v.list_favorites().unwrap().len(), 1);
    v.delete_page(a.clone()).unwrap();
    assert!(v.list_favorites().unwrap().is_empty());
    v.restore_page(a.clone()).unwrap();
    assert!(v.load_page(a.clone()).unwrap().favorite);

    v.set_icon(a.clone(), Some("🌱".into())).unwrap();
    assert_eq!(
        v.list_pages(None, 0, 10)
            .unwrap()
            .iter()
            .find(|p| p.id == a)
            .unwrap()
            .icon
            .as_deref(),
        Some("🌱")
    );
    v.set_icon(a.clone(), Some("  ".into())).unwrap();
    assert_eq!(v.load_page(a.clone()).unwrap().icon, None);

    let img = v
        .add_attachment_bytes("cover.png".into(), vec![1, 2, 3])
        .unwrap();
    v.set_cover(a.clone(), Some(img.clone())).unwrap();
    assert_eq!(v.load_page(a.clone()).unwrap().cover, Some(img.clone()));
    assert!(matches!(
        v.set_cover(a.clone(), Some("f".repeat(64))),
        Err(VaultError::NotFound(_))
    ));
    // The cover keeps its blob alive.
    assert_eq!(v.gc_attachments().unwrap(), 0);
    v.set_cover(a.clone(), None).unwrap();
    assert_eq!(v.gc_attachments().unwrap(), 1);
}

// ------------------------------------------------------------ links and files

#[test]
fn backlinks_follow_edits_renames_and_deletes() {
    let t = TempDir::new("links");
    let v = t.open();
    let target = v.create_page(None, "Target".into()).unwrap();
    let src = v.create_page(None, "Source".into()).unwrap();
    let blk = v.create_page(None, "Blocky".into()).unwrap();
    v.apply(
        src.clone(),
        vec![BlockOp::Insert {
            parent: None,
            after: None,
            block: NewBlock {
                id: "p".into(),
                kind: BlockKind::Paragraph,
                text: "see Target now".into(),
                marks: vec![Mark {
                    start: 4,
                    end: 10,
                    kind: MarkKind::PageLink(target.clone()),
                }],
                checked: false,
            },
        }],
    )
    .unwrap();
    v.apply(
        blk.clone(),
        vec![
            ins(None, nb("l", BlockKind::LinkToPage, "")),
            BlockOp::SetTarget {
                id: "l".into(),
                target: Some(target.clone()),
            },
        ],
    )
    .unwrap();
    assert_eq!(
        v.load_page(blk.clone()).unwrap().blocks[0]
            .target
            .as_deref(),
        Some(target.as_str())
    );
    v.flush().unwrap();
    let back = |v: &Vault| -> Vec<String> {
        let mut x: Vec<String> = v
            .backlinks(target.clone())
            .unwrap()
            .into_iter()
            .map(|p| p.title)
            .collect();
        x.sort();
        x
    };
    assert_eq!(back(&v), ["Blocky", "Source"]);
    let md = md_file(&t, &src);
    assert!(
        md.contains("[[Target|Target]]") || md.contains("[[Target]]"),
        "{md}"
    );
    assert!(
        md.contains(&format!("\"Target\": {target}")) || md.contains(&format!("Target: {target}")),
        "{md}"
    );

    // Renaming the target rewrites the link text in the source's markdown.
    v.rename_page(target.clone(), "Goal".into()).unwrap();
    v.flush().unwrap();
    let md = md_file(&t, &src);
    assert!(md.contains("[[Goal|Target]]"), "{md}");

    // Removing the mark removes the backlink.
    v.apply(
        src.clone(),
        vec![BlockOp::SetText {
            id: "p".into(),
            text: "see Target now".into(),
            marks: vec![],
        }],
    )
    .unwrap();
    v.flush().unwrap();
    assert_eq!(back(&v), ["Blocky"]);
    // A source in the trash does not count; restored, it does again.
    v.delete_page(blk.clone()).unwrap();
    assert!(back(&v).is_empty());
    v.restore_page(blk.clone()).unwrap();
    assert_eq!(back(&v), ["Blocky"]);
    // Survives a rebuild of the index.
    v.reindex().unwrap();
    assert_eq!(back(&v), ["Blocky"]);
}

#[test]
fn attachments_are_deduplicated_and_collected() {
    let t = TempDir::new("files");
    let src = TempDir::new("files_src");
    let v = t.open();
    let a1 = v
        .add_attachment_bytes("photo.png".into(), b"PNGDATA".to_vec())
        .unwrap();
    let a2 = v
        .add_attachment_bytes("again.png".into(), b"PNGDATA".to_vec())
        .unwrap();
    assert_eq!(a1, a2);
    assert_eq!(a1.len(), 64);
    let path = src.0.join("report.pdf");
    fs::write(&path, b"%PDF-1.4 hello").unwrap();
    let f = v
        .add_attachment_from_path(path.to_string_lossy().into_owned())
        .unwrap();
    let info = v.attachment_info(f.clone()).unwrap();
    assert_eq!(
        (info.name.as_str(), info.mime.as_str(), info.size),
        ("report.pdf", "application/pdf", 14)
    );
    let info = v.attachment_info(a1.clone()).unwrap();
    assert_eq!(
        (info.name.as_str(), info.mime.as_str()),
        ("photo.png", "image/png")
    );
    let p = v.attachment_path(a1.clone()).unwrap();
    assert_eq!(fs::read(&p).unwrap(), b"PNGDATA");
    assert!(p
        .replace('\\', "/")
        .contains(&format!(".vault/blobs/{}/{a1}", &a1[..2])));
    let blobs = |t: &TempDir| {
        fs::read_dir(t.0.join(".vault/blobs"))
            .unwrap()
            .map(|d| fs::read_dir(d.unwrap().path()).unwrap().count())
            .sum::<usize>()
    };
    assert_eq!(blobs(&t), 2);
    let unused = v
        .add_attachment_bytes("junk.txt".into(), b"junk".to_vec())
        .unwrap();

    let page = v.create_page(None, "Media".into()).unwrap();
    v.apply(
        page.clone(),
        vec![
            ins(None, nb("i", BlockKind::Image, "A photo")),
            BlockOp::SetTarget {
                id: "i".into(),
                target: Some(a1.clone()),
            },
            ins(Some("i"), nb("f", BlockKind::File, "report.pdf")),
            BlockOp::SetTarget {
                id: "f".into(),
                target: Some(f.clone()),
            },
            ins(Some("f"), nb("w", BlockKind::Bookmark, "Example")),
            BlockOp::SetTarget {
                id: "w".into(),
                target: Some("https://example.com".into()),
            },
        ],
    )
    .unwrap();
    // Unused blobs go; used ones stay.
    assert_eq!(v.gc_attachments().unwrap(), 1);
    assert!(matches!(
        v.attachment_path(unused),
        Err(VaultError::NotFound(_))
    ));
    assert_eq!(blobs(&t), 2);
    v.flush().unwrap();
    let md = md_file(&t, &page);
    assert!(
        md.contains(&format!("![A photo](../.vault/blobs/{}/{a1})", &a1[..2])),
        "{md}"
    );
    assert!(md.contains("<https://example.com> Example"), "{md}");
    // Pages in the trash still hold their attachments; purged ones do not.
    v.delete_page(page.clone()).unwrap();
    assert_eq!(v.gc_attachments().unwrap(), 0);
    v.purge_page(page).unwrap();
    assert_eq!(v.gc_attachments().unwrap(), 2);
    assert_eq!(blobs(&t), 0);
}

// ------------------------------------------------------------- history, feed

#[test]
fn history_versions_load_and_restore() {
    let t = TempDir::new("history");
    let p;
    {
        let v = t.open();
        p = v.create_page(None, "Diary".into()).unwrap();
        v.apply(
            p.clone(),
            vec![ins(None, nb("a", BlockKind::Paragraph, "first draft"))],
        )
        .unwrap();
        v.set_property(p.clone(), "mood".into(), PropValue::Text("calm".into()))
            .unwrap();
    }
    // Another device continues: a new version.
    fs::write(t.0.join(".vault/device"), "other-device").unwrap();
    let v = t.open();
    assert_eq!(v.device_id(), "other-device");
    v.apply(
        p.clone(),
        vec![
            BlockOp::SetText {
                id: "a".into(),
                text: "second draft".into(),
                marks: vec![],
            },
            ins(Some("a"), nb("b", BlockKind::Heading1, "Added")),
        ],
    )
    .unwrap();
    v.rename_page(p.clone(), "Journal".into()).unwrap();
    v.remove_property(p.clone(), "mood".into()).unwrap();
    let versions = v.list_versions(p.clone()).unwrap();
    assert_eq!(versions.len(), 2, "{versions:?}");
    assert_eq!(versions[0].device, "other-device");
    assert_ne!(versions[1].device, "other-device");
    assert!(
        versions[0].summary.contains("content"),
        "{}",
        versions[0].summary
    );
    assert!(versions[0].time_ms >= versions[1].time_ms);

    let old = v.load_page_at(p.clone(), versions[1].id.clone()).unwrap();
    assert_eq!(old.title, "Diary");
    assert_eq!(old.blocks.len(), 1);
    assert_eq!(old.blocks[0].text, "first draft");
    assert_eq!(
        prop(&old.properties, "mood"),
        Some(&PropValue::Text("calm".into()))
    );
    // The current page is unchanged by loading an old version.
    assert_eq!(v.load_page(p.clone()).unwrap().title, "Journal");
    assert!(v.load_page_at(p.clone(), "garbage".into()).is_err());

    v.restore_version(p.clone(), versions[1].id.clone())
        .unwrap();
    let now = v.load_page(p.clone()).unwrap();
    assert_eq!(now.title, "Diary");
    assert_eq!(
        now.blocks
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>(),
        ["first draft"]
    );
    assert_eq!(now.blocks[0].id, "a", "unchanged blocks keep their ids");
    assert_eq!(
        prop(&now.properties, "mood"),
        Some(&PropValue::Text("calm".into()))
    );
    // Restoring added to the history; the newer version is still there.
    let after = v.list_versions(p.clone()).unwrap();
    assert_eq!(after.len(), 2);
    assert_ne!(after[0].id, versions[0].id);
    let newer = v.load_page_at(p.clone(), versions[0].id.clone()).unwrap();
    assert_eq!(newer.title, "Journal");
    assert_eq!(v.search("diary".into(), 5).unwrap().len(), 1);
}

#[test]
fn change_feed_reports_changes_in_sequence() {
    let document_changes = |batch: &ChangeBatch| batch.changes.iter()
        .filter(|change| !matches!(change, Change::IndexCommitted { .. }))
        .cloned().collect::<Vec<_>>();
    let committed = |batch: &ChangeBatch, page: &str| {
        let commits = batch.changes.iter().filter_map(|change| match change {
            Change::IndexCommitted { generation, pages } => Some((*generation, pages)),
            _ => None,
        }).collect::<Vec<_>>();
        assert!(!commits.is_empty(), "successful index writes publish their commit");
        assert!(commits.iter().all(|(generation, pages)| *generation > 0 && pages.len() == 1 && pages[0] == page));
        assert!(commits.windows(2).all(|pair| pair[0].0 < pair[1].0));
    };
    let t = TempDir::new("feed");
    let v = t.open();
    let start = v.poll_changes(0);
    assert_eq!((start.seq, start.changes.len(), start.reset), (0, 0, false));
    let a = v.create_page(None, "A".into()).unwrap();
    let b1 = v.poll_changes(0);
    assert!(b1.seq > 0);
    assert_eq!(
        document_changes(&b1),
        [Change::PageCreated(a.clone()), Change::TreeChanged]
    );
    committed(&b1, &a);
    // Nothing new.
    let same = v.poll_changes(b1.seq);
    assert!(same.changes.is_empty() && !same.reset && same.seq == b1.seq);

    v.apply(
        a.clone(),
        vec![ins(None, nb("x", BlockKind::Paragraph, "hi"))],
    )
    .unwrap();
    v.apply(
        a.clone(),
        vec![BlockOp::SetText {
            id: "x".into(),
            text: "hey".into(),
            marks: vec![],
        }],
    )
    .unwrap();
    v.set_property(a.clone(), "k".into(), PropValue::Number(1.0))
        .unwrap();
    let b2 = v.poll_changes(b1.seq);
    assert!(b2.seq > b1.seq);
    assert_eq!(
        document_changes(&b2),
        [Change::PageChanged(a.clone())],
        "repeats are reported once"
    );
    committed(&b2, &a);

    let c = v.create_page(Some(a.clone()), "C".into()).unwrap();
    v.move_page(c.clone(), None, None).unwrap();
    v.delete_page(c.clone()).unwrap();
    let b3 = v.poll_changes(b2.seq);
    assert!(b3.changes.contains(&Change::PageCreated(c.clone())));
    assert!(b3.changes.contains(&Change::PageDeleted(c.clone())));
    assert_eq!(document_changes(&b3).last(), Some(&Change::TreeChanged));
    // Everything since the start, in order of latest occurrence.
    let all = v.poll_changes(0);
    assert_eq!(all.seq, b3.seq);
    assert!(all.changes.contains(&Change::PageChanged(a.clone())));
    // A sequence from the future (another session) asks for a reload.
    assert!(v.poll_changes(b3.seq + 100).reset);
    drop(v);
    let v = t.open();
    assert!(
        v.poll_changes(b3.seq).reset,
        "sequences restart with the vault"
    );
}

// --------------------------------------------------------- import and rescan

#[test]
fn import_an_obsidian_style_folder() {
    let src = TempDir::new("import_src");
    let d = &src.0;
    fs::create_dir_all(d.join("Projects/assets")).unwrap();
    fs::create_dir_all(d.join(".obsidian")).unwrap();
    fs::write(d.join(".obsidian/app.json"), "{}").unwrap();
    fs::write(
        d.join("Home.md"),
        "---\ntags: [start, \"#pinned\"]\nrating: 4\ndue: 2026-10-01\nsource: https://example.com\n---\n# Welcome\n\nSee [[Plan|the plan]] and [[Missing note]].\n\n![[diagram.png]]\n\n- [ ] call [[Projects/Plan]]\n- [x] done\n",
    )
    .unwrap();
    fs::write(d.join("Projects.md"), "Folder note for projects.\n").unwrap();
    fs::write(
        d.join("Projects/Plan.md"),
        "Back to [[Home]].\n\n![[Home]]\n\n[spec](assets/spec.pdf)\n",
    )
    .unwrap();
    fs::write(d.join("Projects/assets/diagram.png"), b"\x89PNG fake").unwrap();
    fs::write(d.join("Projects/assets/spec.pdf"), b"%PDF fake").unwrap();
    fs::create_dir_all(d.join("Empty")).unwrap();

    let t = TempDir::new("import");
    let v = t.open();
    let report = v.import_markdown(src.root(), None).unwrap();
    assert_eq!(report.pages, 3, "{report:?}");
    assert_eq!(report.attachments, 2);
    assert_eq!(report.unresolved_links, 1);
    let top: Vec<String> = v
        .list_children(None, 0, 10)
        .unwrap()
        .into_iter()
        .map(|p| p.title)
        .collect();
    assert_eq!(top, ["Home", "Projects"]);
    let home_id = report.top_pages[0].clone();
    let projects = v.load_page(report.top_pages[1].clone()).unwrap();
    assert_eq!(projects.blocks[0].text, "Folder note for projects.");
    let plan = v.list_children(Some(projects.id.clone()), 0, 10).unwrap();
    assert_eq!(plan.len(), 1);
    let plan_id = plan[0].id.clone();

    let home = v.load_page(home_id.clone()).unwrap();
    assert_eq!(home.blocks[0].kind, BlockKind::Heading1);
    let para = &home.blocks[1];
    assert_eq!(para.text, "See the plan and [[Missing note]].");
    assert_eq!(
        para.marks,
        [
            Mark {
                start: 4,
                end: 12,
                kind: MarkKind::Link("Plan".into())
            },
            Mark {
                start: 4,
                end: 12,
                kind: MarkKind::PageLink(plan_id.clone())
            },
        ]
    );
    let img = &home.blocks[2];
    assert_eq!(img.kind, BlockKind::Image);
    let blob = img.target.clone().unwrap();
    assert_eq!(
        fs::read(v.attachment_path(blob.clone()).unwrap()).unwrap(),
        b"\x89PNG fake"
    );
    assert_eq!(v.attachment_info(blob).unwrap().name, "diagram.png");
    assert_eq!(home.blocks[3].kind, BlockKind::Todo);
    assert!(home.blocks[3]
        .marks
        .iter()
        .any(|m| m.kind == MarkKind::PageLink(plan_id.clone())));
    assert!(home.blocks[3]
        .marks
        .iter()
        .any(|m| m.kind == MarkKind::Link("Projects/Plan".into())));
    assert!(home.blocks[4].checked);
    assert_eq!(
        prop(&home.properties, "tags"),
        Some(&PropValue::MultiSelect(vec![
            "start".into(),
            "pinned".into()
        ]))
    );
    assert_eq!(
        prop(&home.properties, "rating"),
        Some(&PropValue::Number(4.0))
    );
    assert_eq!(
        prop(&home.properties, "due"),
        Some(&date(day(2026, 10, 1), None))
    );
    assert_eq!(
        prop(&home.properties, "source"),
        Some(&PropValue::Url("https://example.com".into()))
    );

    let p = v.load_page(plan_id.clone()).unwrap();
    assert!(p.blocks[0]
        .marks
        .iter()
        .any(|m| m.kind == MarkKind::PageLink(home_id.clone())));
    assert!(p.blocks[0]
        .marks
        .iter()
        .any(|m| m.kind == MarkKind::Link("Home".into())));
    assert_eq!(p.blocks[1].kind, BlockKind::LinkToPage);
    assert_eq!(p.blocks[1].target.as_deref(), Some(home_id.as_str()));
    assert_eq!(p.blocks[2].kind, BlockKind::File);
    assert_eq!(p.blocks[2].text, "spec");

    v.flush().unwrap();
    let back: Vec<String> = v
        .backlinks(home_id)
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(back, vec![plan_id.clone()]);
    assert_eq!(v.search("welcome".into(), 5).unwrap().len(), 1);
    let feed = v.poll_changes(0);
    assert!(feed.changes.contains(&Change::PageCreated(plan_id)));
    // Importing under a page.
    let holder = v.create_page(None, "Imported".into()).unwrap();
    let r2 = v.import_markdown(src.root(), Some(holder.clone())).unwrap();
    assert_eq!(r2.pages, 3);
    assert_eq!(v.list_children(Some(holder), 0, 10).unwrap().len(), 2);
}

#[test]
fn rescan_applies_external_markdown_edits() {
    let t = TempDir::new("rescan");
    let v = t.open();
    let p = v.create_page(None, "Groceries".into()).unwrap();
    let other = v.create_page(None, "Recipes".into()).unwrap();
    v.apply(
        p.clone(),
        vec![
            ins(None, nb("h", BlockKind::Heading1, "List")),
            ins(Some("h"), nb("a", BlockKind::Bulleted, "apples")),
            ins(Some("a"), nb("b", BlockKind::Bulleted, "bread")),
            ins(Some("b"), nb("c", BlockKind::Paragraph, "the end")),
        ],
    )
    .unwrap();
    v.set_property(p.clone(), "shop".into(), PropValue::Text("market".into()))
        .unwrap();
    v.flush().unwrap();
    // Nothing changed outside: nothing to do.
    assert_eq!(v.rescan().unwrap(), 0);
    let path = md_path(&t, &p).unwrap();
    let md = fs::read_to_string(&path).unwrap();
    let edited = md
        .replace("title: \"Groceries\"", "title: \"Shopping\"")
        .replace("- bread", "- **fresh** bread\n- milk, see [[Recipes]]")
        .replace("shop: \"market\"", "shop: \"corner store\"\npriority: 2");
    assert_ne!(edited, md);
    fs::write(&path, &edited).unwrap();
    // A new file dropped into pages/.
    fs::write(t.0.join("pages/new note.md"), "Written elsewhere.\n").unwrap();

    let before = v.poll_changes(0).seq;
    assert_eq!(v.rescan().unwrap(), 2);
    let page = v.load_page(p.clone()).unwrap();
    assert_eq!(page.title, "Shopping");
    let texts: Vec<&str> = page.blocks.iter().map(|b| b.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "List",
            "apples",
            "fresh bread",
            "milk, see Recipes",
            "the end"
        ]
    );
    let ids: Vec<&str> = page.blocks.iter().map(|b| b.id.as_str()).collect();
    assert_eq!(
        &ids[..3],
        ["h", "a", "b"],
        "unchanged and edited blocks keep their ids"
    );
    assert_eq!(ids[4], "c");
    assert_eq!(
        page.blocks[2].marks,
        [Mark {
            start: 0,
            end: 5,
            kind: MarkKind::Bold
        }]
    );
    assert_eq!(
        page.blocks[3].marks[0].kind,
        MarkKind::PageLink(other.clone())
    );
    assert_eq!(
        prop(&page.properties, "shop"),
        Some(&PropValue::Text("corner store".into()))
    );
    assert_eq!(
        prop(&page.properties, "priority"),
        Some(&PropValue::Number(2.0))
    );
    let feed = v.poll_changes(before);
    assert!(feed.changes.contains(&Change::PageChanged(p.clone())));

    let new = v.search("elsewhere".into(), 5).unwrap();
    assert_eq!(new.len(), 1);
    assert_eq!(new[0].title, "new note");
    v.flush().unwrap();
    // The writer replaced the edited file with the app's rendering (renamed
    // after the new title) and took over the new file.
    assert!(!path.exists());
    let md2 = md_file(&t, &p);
    assert!(md2.contains("title: \"Shopping\""), "{md2}");
    assert!(md2.contains("- **fresh** bread"), "{md2}");
    assert!(!t.0.join("pages/new note.md").exists());
    assert!(md_path(&t, &new[0].id).is_some());
    let back: Vec<String> = v
        .backlinks(other)
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(back, vec![p.clone()]);
    // Rescanning again finds nothing; a deleted file is written again.
    assert_eq!(v.rescan().unwrap(), 0);
    fs::remove_file(md_path(&t, &p).unwrap()).unwrap();
    assert_eq!(v.rescan().unwrap(), 0);
    v.flush().unwrap();
    assert!(md_path(&t, &p).is_some());
    assert_eq!(v.load_page(p).unwrap().blocks.len(), 5);
}

#[test]
fn edited_markdown_is_not_overwritten_before_rescan() {
    let t = TempDir::new("keep");
    let v = t.open();
    let p = v.create_page(None, "Note".into()).unwrap();
    v.apply(
        p.clone(),
        vec![ins(None, nb("a", BlockKind::Paragraph, "one"))],
    )
    .unwrap();
    v.flush().unwrap();
    let path = md_path(&t, &p).unwrap();
    let edited = fs::read_to_string(&path)
        .unwrap()
        .replace("one", "one (outside)");
    fs::write(&path, &edited).unwrap();
    // An app edit meanwhile does not clobber the outside edit ...
    v.apply(
        p.clone(),
        vec![ins(Some("a"), nb("b", BlockKind::Paragraph, "two"))],
    )
    .unwrap();
    v.flush().unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), edited);
    // ... and rescan merges both.
    assert_eq!(v.rescan().unwrap(), 1);
    let texts: Vec<String> = v
        .load_page(p.clone())
        .unwrap()
        .blocks
        .into_iter()
        .map(|b| b.text)
        .collect();
    assert_eq!(texts, ["one (outside)", "two"]);
}

// ----------------------------------------------------------------------- seed

#[test]
fn seeded_tasks_collection_answers_its_views() {
    let t = TempDir::new("tasks");
    let c = seed::generate_tasks(t.root(), 300).unwrap();
    assert!(seed::generate_tasks(t.root(), 10).is_err());
    let v = t.open();
    let tasks = v.load_page(c.clone()).unwrap();
    assert_eq!(tasks.title, seed::TASKS_TITLE);
    assert!(tasks.is_database());
    let views = v.list_views(c.clone()).unwrap();
    assert_eq!(views.len(), 5);
    let all = v.query(Some(c.clone()), None, vec![], 0, 10).unwrap();
    assert_eq!(all.total, 300);
    let open = &views[1];
    let r = v
        .query(
            Some(c.clone()),
            open.filter.clone(),
            open.sorts.clone(),
            0,
            50,
        )
        .unwrap();
    assert!(r.total > 50 && r.total < 300);
    let dues: Vec<i64> = r
        .rows
        .iter()
        .map(|p| match prop(&p.properties, "due") {
            Some(PropValue::Date { start_ms, .. }) => *start_ms,
            other => panic!("{other:?}"),
        })
        .collect();
    assert!(dues.windows(2).all(|w| w[0] <= w[1]));
    let groups = v
        .query_groups(Some(c.clone()), None, "status".into())
        .unwrap();
    assert_eq!(groups.iter().map(|g| g.count).sum::<u32>(), 300);
    let now = tasks.created_ms;
    let month = v
        .query_range(
            Some(c.clone()),
            "due".into(),
            now - 15 * 86_400_000,
            now + 15 * 86_400_000,
            None,
        )
        .unwrap();
    assert!(!month.is_empty());
    // Rebuilding the index gives the same answers.
    v.reindex().unwrap();
    assert_eq!(
        v.query(Some(c.clone()), open.filter.clone(), vec![], 0, 1)
            .unwrap()
            .total,
        r.total
    );
    // Row markdown uses the schema's names.
    let row = &all.rows[0];
    assert!(md_file(&t, &row.id).contains("Status: "));
}

// ------------------------------------------------ interface additions (app)

#[test]
fn email_phone_config_and_database_blocks() {
    let t = TempDir::new("dbadd");
    let v = t.open();
    let c = v.create_page(None, "People".into()).unwrap();
    let mut schema = v
        .set_schema(
            c.clone(),
            vec![
                def("mail", "Mail", PropKind::Email, vec![]),
                def("tel", "Tel", PropKind::Phone, vec![]),
                PropDef {
                    config: r#"{"format":"percent"}"#.into(),
                    ..def("n", "Score", PropKind::Number, vec![])
                },
                def("tag", "Tag", PropKind::Select, vec![opt("x")]),
            ],
        )
        .unwrap();
    assert_eq!(schema[2].config, r#"{"format":"percent"}"#);
    let r = v.create_page(Some(c.clone()), "Ann".into()).unwrap();
    v.set_property(
        r.clone(),
        "mail".into(),
        PropValue::Text("ann@example.com".into()),
    )
    .unwrap();
    v.set_property(r.clone(), "tel".into(), PropValue::Text("+1 555".into()))
        .unwrap();
    assert!(v
        .set_property(r.clone(), "mail".into(), PropValue::Number(1.0))
        .is_err());
    let q = v
        .query(
            Some(c.clone()),
            Some(cond(
                "mail",
                FilterOp::Contains,
                PropValue::Text("EXAMPLE".into()),
            )),
            vec![Sort {
                prop: "tel".into(),
                descending: false,
            }],
            0,
            10,
        )
        .unwrap();
    assert_eq!(titles(&q), vec!["Ann"]);

    // A colour, a new option and a config change keep the rows as they are.
    schema[3].options[0].color = "red".into();
    schema[3].options.push(opt("y"));
    schema[3].config = "{}".into();
    let schema = v.set_schema(c.clone(), schema).unwrap();
    assert_eq!(schema[3].options.len(), 2);
    assert_eq!(schema[3].config, "{}");
    let p = v.load_page(r.clone()).unwrap();
    assert_eq!(
        prop(&p.properties, "mail"),
        Some(&PropValue::Text("ann@example.com".into()))
    );

    // Views keep their config.
    let mut views = v.list_views(c.clone()).unwrap();
    views[0].config = r#"{"widths":{"mail":120}}"#.into();
    v.update_view(c.clone(), views[0].clone()).unwrap();
    assert_eq!(v.list_views(c.clone()).unwrap()[0].config, views[0].config);

    // An inline database block: kept, linked and written to markdown.
    let host = v.create_page(None, "Host".into()).unwrap();
    v.move_page(c.clone(), Some(host.clone()), None).unwrap();
    v.apply(
        host.clone(),
        vec![
            ins(None, nb("db1", BlockKind::Database, "")),
            BlockOp::SetTarget {
                id: "db1".into(),
                target: Some(c.clone()),
            },
        ],
    )
    .unwrap();
    let h = v.load_page(host.clone()).unwrap();
    assert_eq!(h.blocks[0].kind, BlockKind::Database);
    assert_eq!(h.blocks[0].target.as_deref(), Some(c.as_str()));
    v.flush().unwrap();
    let md = md_file(&t, &host);
    assert!(md.contains("![[People]] <!-- database -->"), "{md}");
    assert!(v.backlinks(c.clone()).unwrap().iter().any(|p| p.id == host));
    // Reading the markdown back keeps the block.
    assert_eq!(v.rescan().unwrap(), 0);
    let h = v.load_page(host).unwrap();
    assert_eq!(h.blocks[0].kind, BlockKind::Database);
}
