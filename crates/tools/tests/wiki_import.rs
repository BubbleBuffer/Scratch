use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde_json::{json, Value};
use vault_core::{PageId, PageSummary, PageVersion, PropValue, Property, Vault};
use vault_tools::wiki_import::{self, DEFAULT_SOURCE_LIMIT};
use vault_tools::Approval;

fn open(dir: &Path) -> Vault {
    Vault::open(dir.join("vault").to_string_lossy().into_owned()).unwrap()
}

fn set_name(header: &mut tar::Header, name: &str) {
    // Written raw so that paths the tar crate would refuse can be tested.
    let bytes = name.as_bytes();
    let field = &mut header.as_old_mut().name;
    field.fill(0);
    field[..bytes.len()].copy_from_slice(bytes);
}

fn tar_of(files: &[(&str, &str)]) -> Vec<u8> {
    let bytes: Vec<(&str, &[u8])> = files.iter().map(|(p, b)| (*p, b.as_bytes())).collect();
    tar_of_bytes(&bytes)
}

fn tar_of_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, body) in files {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(0o644);
        header.set_size(body.len() as u64);
        set_name(&mut header, path);
        header.set_cksum();
        builder.append(&header, *body).unwrap();
    }
    builder.into_inner().unwrap()
}

fn special_tar(name: &str, kind: tar::EntryType) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(kind);
    header.set_mode(0o777);
    header.set_size(0);
    header.set_link_name("../outside.md").unwrap();
    set_name(&mut header, name);
    header.set_cksum();
    builder.append(&header, io::empty()).unwrap();
    builder.into_inner().unwrap()
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn root(name: &str, archive: &[u8]) -> Value {
    json!({ "name": name, "archive_base64": b64(archive) })
}

fn project(name: &str, roots: Vec<Value>) -> Value {
    json!({
        "name": name,
        "repo_path": format!("/repos/{name}"),
        "git_remote": "git@example.com:me/notes.git",
        "last_activity": "2026-10-01",
        "roots": roots,
    })
}

fn run(v: &Vault, input: &Value) -> Value {
    let out = vault_tools::call(v, "import_markdown", input).unwrap();
    serde_json::from_str(&out).unwrap()
}

fn has_marker(props: &[Property], key: &str) -> bool {
    props
        .iter()
        .any(|p| p.key == key && p.value == PropValue::Checkbox(true))
}

fn children(v: &Vault, parent: Option<&str>) -> Vec<PageSummary> {
    v.list_children(parent.map(str::to_string), 0, 1000)
        .unwrap()
}

fn projects_db(v: &Vault) -> Option<PageId> {
    children(v, None)
        .into_iter()
        .find(|p| p.is_database() && has_marker(&p.properties, "ai.projects"))
        .map(|p| p.id)
}

fn containers(v: &Vault, row: &str) -> Vec<PageSummary> {
    children(v, Some(row))
        .into_iter()
        .filter(|c| has_marker(&c.properties, "ai.project_root"))
        .collect()
}

fn prop(v: &Vault, page: &str, key: &str) -> Option<PropValue> {
    v.load_page(page.to_string())
        .unwrap()
        .properties
        .into_iter()
        .find(|p| p.key == key)
        .map(|p| p.value)
}

/// Every page's id, title and update time, in a stable order.
fn pages(v: &Vault) -> Vec<(String, String, i64)> {
    let mut out = Vec::new();
    let mut pending: Vec<Option<PageId>> = vec![None];
    while let Some(parent) = pending.pop() {
        for page in v.list_children(parent, 0, 1000).unwrap() {
            pending.push(Some(page.id.clone()));
            out.push((page.id, page.title, page.updated_ms));
        }
    }
    out.sort();
    out
}

fn versions(v: &Vault) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = v
        .page_versions()
        .unwrap()
        .into_iter()
        .map(|PageVersion { page, version }| (page, version))
        .collect();
    out.sort();
    out
}

/// Every file under `dir` with its bytes, in a stable order.
fn files(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let relative = path.strip_prefix(dir).unwrap().to_path_buf();
                out.push((relative, fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn repo_import_is_a_hidden_write_tool_on_mcp_only() {
    // import_markdown is its own hidden MCP tool: never listed, not an alias.
    assert!(vault_tools::definitions()
        .iter()
        .all(|t| t.name != "import_markdown"));
    let tool = vault_tools::hidden()
        .iter()
        .find(|t| t.name == "import_markdown")
        .unwrap();
    assert!(!tool.read_only);
    assert!(!tool.destructive_hint);
    assert_eq!(tool.approval, Approval::None);
    assert!(tool.serves(vault_tools::Surface::Mcp));
    assert!(!tool.serves(vault_tools::Surface::InAppAgent));
    let found = vault_tools::lookup("import_markdown").unwrap();
    assert!(std::ptr::eq(found, tool));
    assert!(vault_tools::get("import_markdown").is_none());
    assert!(vault_tools::alias("import_markdown").is_none());
    // create_page no longer takes the import arguments.
    let create = vault_tools::get("create_page").unwrap();
    assert!(create.input_schema()["properties"].get("roots").is_none());
}

#[test]
fn two_projects_and_a_multi_root_project_reuse_rows_and_containers() {
    let dir = tempfile::tempdir().unwrap();
    let v = open(dir.path());
    let alpha = project(
        "Alpha",
        vec![root(
            "docs",
            &tar_of(&[("a.md", "# A\n\nalpha\n"), ("b.md", "# B\n")]),
        )],
    );
    let beta = project("Beta", vec![root("docs", &tar_of(&[("c.md", "# C\n")]))]);
    let gamma = project(
        "Gamma",
        vec![
            root("api", &tar_of(&[("api.md", "# API\n")])),
            root("web", &tar_of(&[("web.md", "# Web\n")])),
        ],
    );

    let first: Vec<Value> = [&alpha, &beta, &gamma]
        .iter()
        .map(|input| run(&v, input))
        .collect();
    assert!(first
        .iter()
        .all(|out| out["row_created"] == true && out["row_updated"] == false));
    assert!(first.iter().all(|out| out["dry_run"] == false));
    assert_eq!(first[0]["counts_relative_to"], "existing vault");
    assert_ne!(first[0]["project_page"], first[1]["project_page"]);
    assert_eq!(first[0]["roots"][0]["report"]["created"], 2);
    assert_eq!(
        first[0]["roots"][0]["report"]["top_pages"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let db = projects_db(&v).unwrap();
    assert_eq!(children(&v, Some(&db)).len(), 3);
    let gamma_row = first[2]["project_page"].as_str().unwrap().to_string();
    let first_containers = containers(&v, &gamma_row);
    let mut titles: Vec<String> = first_containers.iter().map(|c| c.title.clone()).collect();
    titles.sort();
    assert_eq!(titles, ["api", "web"]);
    for (i, name) in ["api", "web"].iter().enumerate() {
        let container = first_containers.iter().find(|c| c.title == *name).unwrap();
        let top = first[2]["roots"][i]["report"]["top_pages"][0]
            .as_str()
            .unwrap();
        assert_eq!(
            v.load_page(top.to_string()).unwrap().parent.as_deref(),
            Some(container.id.as_str())
        );
    }

    let second: Vec<Value> = [&alpha, &beta, &gamma]
        .iter()
        .map(|input| run(&v, input))
        .collect();
    for (before, after) in first.iter().zip(&second) {
        assert_eq!(before["project_page"], after["project_page"]);
        assert_eq!(after["row_created"], false);
        assert_eq!(after["row_updated"], false);
    }
    assert_eq!(children(&v, Some(&db)).len(), 3);
    assert_eq!(rows_named(&v, &db, "Gamma").len(), 1);
    let second_containers = containers(&v, &gamma_row);
    assert_eq!(second_containers.len(), 2);
    for container in &first_containers {
        assert!(second_containers.iter().any(|c| c.id == container.id));
    }
    for i in 0..2 {
        let report = &second[2]["roots"][i]["report"];
        assert_eq!(report["created"], 0);
        assert_eq!(report["updated"], 0);
        assert_eq!(report["orphaned"], 0);
        assert_eq!(
            report["top_pages"],
            first[2]["roots"][i]["report"]["top_pages"]
        );
    }
    let alpha_report = &second[0]["roots"][0]["report"];
    assert_eq!(alpha_report["created"], 0);
    assert_eq!(alpha_report["updated"], 0);
    assert_eq!(alpha_report["unchanged"], 2);
}

fn rows_named(v: &Vault, db: &str, name: &str) -> Vec<PageSummary> {
    children(v, Some(db))
        .into_iter()
        .filter(|row| row.title == name)
        .collect()
}

#[test]
fn settled_status_survives_and_missing_status_is_imported() {
    let dir = tempfile::tempdir().unwrap();
    let v = open(dir.path());
    let input = project("Alpha", vec![root("docs", &tar_of(&[("a.md", "# A\n")]))]);
    let row = run(&v, &input)["project_page"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        prop(&v, &row, "status"),
        Some(PropValue::Select("imported".into()))
    );
    v.set_property(
        row.clone(),
        "status".into(),
        PropValue::Select("cut over".into()),
    )
    .unwrap();

    let mut moved = input.clone();
    moved["last_activity"] = json!("2026-10-14T09:30");
    moved["repo_path"] = json!("/repos/moved");
    let out = run(&v, &moved);
    assert_eq!(out["project_page"], row.as_str());
    assert_eq!(out["row_created"], false);
    assert_eq!(out["row_updated"], true);
    assert_eq!(
        prop(&v, &row, "status"),
        Some(PropValue::Select("cut over".into()))
    );
    assert_eq!(
        prop(&v, &row, "repo_path"),
        Some(PropValue::Text("/repos/moved".into()))
    );
    assert!(matches!(
        prop(&v, &row, "last_activity"),
        Some(PropValue::Date { has_time: true, .. })
    ));

    v.remove_property(row.clone(), "status".into()).unwrap();
    let again = run(&v, &moved);
    assert_eq!(again["row_updated"], true);
    assert_eq!(
        prop(&v, &row, "status"),
        Some(PropValue::Select("imported".into()))
    );

    let same = run(&v, &moved);
    assert_eq!(same["row_updated"], false);
}

#[test]
fn last_activity_keeps_offsets_as_utc_milliseconds() {
    let dir = tempfile::tempdir().unwrap();
    let v = open(dir.path());
    let mut input = project("Offsets", vec![root("docs", &tar_of(&[("a.md", "# A\n")]))]);
    // 2026-10-14T07:30:00Z is 1791963000000 ms; the offset forms name the same
    // instant, as git's %cI does, and the fraction keeps its milliseconds.
    for (text, ms) in [
        ("2026-10-14T09:30:00+02:00", 1_791_963_000_000),
        ("2026-10-14T09:30:00.250+02:00", 1_791_963_000_250),
        ("2026-10-14T07:30:00Z", 1_791_963_000_000),
    ] {
        input["last_activity"] = json!(text);
        let out = run(&v, &input);
        let row = out["project_page"].as_str().unwrap().to_string();
        assert_eq!(
            prop(&v, &row, "last_activity"),
            Some(PropValue::Date {
                start_ms: ms,
                end_ms: None,
                has_time: true
            }),
            "{text}"
        );
    }
    // Date-only values keep whole days, as before.
    input["last_activity"] = json!("2026-10-14");
    let out = run(&v, &input);
    let row = out["project_page"].as_str().unwrap().to_string();
    assert_eq!(
        prop(&v, &row, "last_activity"),
        Some(PropValue::Date {
            start_ms: 1_791_936_000_000,
            end_ms: None,
            has_time: false
        })
    );
    input["last_activity"] = json!("not a date");
    let err = vault_tools::call(&v, "import_markdown", &input).unwrap_err();
    assert!(err.0.contains("last_activity"), "{}", err.0);
}

#[test]
fn missing_or_null_last_activity_is_empty_and_clears_a_value() {
    let dir = tempfile::tempdir().unwrap();
    let v = open(dir.path());
    let mut input = project("Quiet", vec![root("docs", &tar_of(&[("a.md", "# A\n")]))]);
    input["last_activity"] = Value::Null;
    let created = run(&v, &input);
    let row = created["project_page"].as_str().unwrap().to_string();
    assert_eq!(created["row_created"], true);
    assert_eq!(prop(&v, &row, "last_activity"), None);

    let mut missing = input.clone();
    missing.as_object_mut().unwrap().remove("last_activity");
    let repeat = run(&v, &missing);
    assert_eq!(repeat["row_updated"], false);
    assert_eq!(repeat["roots"][0]["report"]["unchanged"], 1);

    input["last_activity"] = json!("2026-10-14T07:30:00Z");
    assert_eq!(run(&v, &input)["row_updated"], true);
    assert!(prop(&v, &row, "last_activity").is_some());

    input["last_activity"] = json!("");
    let cleared = run(&v, &input);
    assert_eq!(cleared["row_updated"], true);
    assert_eq!(prop(&v, &row, "last_activity"), None);
    assert_eq!(run(&v, &input)["row_updated"], false);
}

#[test]
fn real_imports_are_attributed_to_the_caller_actor() {
    let dir = tempfile::tempdir().unwrap();
    let v = open(dir.path());
    let input = project(
        "Attributed",
        vec![root("docs", &tar_of(&[("a.md", "# A\n")]))],
    );
    let out = v.with_actor("agent:importer", || run(&v, &input));
    let row = out["project_page"].as_str().unwrap().to_string();
    let page = out["roots"][0]["report"]["top_pages"][0]
        .as_str()
        .unwrap()
        .to_string();
    for id in [row, page] {
        let versions = v.list_versions(id.clone()).unwrap();
        assert!(!versions.is_empty(), "{id}");
        assert!(
            versions.iter().all(|x| x.device == "agent:importer"),
            "{id}: {:?}",
            versions.iter().map(|x| &x.device).collect::<Vec<_>>()
        );
    }
}

#[test]
fn simultaneous_imports_share_the_database_row_and_source_pages() {
    let dir = tempfile::tempdir().unwrap();
    let vault = open(dir.path());
    let input = project(
        "Concurrent",
        vec![root("wiki", &tar_of(&[("intro.md", "Intro")]))],
    );
    let barrier = std::sync::Barrier::new(2);
    let reports = std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            barrier.wait();
            run(&vault, &input)
        });
        let second = scope.spawn(|| {
            barrier.wait();
            run(&vault, &input)
        });
        [first.join().unwrap(), second.join().unwrap()]
    });
    assert_eq!(reports[0]["project_page"], reports[1]["project_page"]);
    let top = children(&vault, None);
    assert_eq!(
        top.iter()
            .filter(|page| has_marker(&page.properties, "ai.projects"))
            .count(),
        1
    );
    let rows = children(&vault, Some(&top[0].id));
    assert_eq!(rows.len(), 1);
    assert_eq!(children(&vault, Some(&rows[0].id)).len(), 1);
    assert_eq!(
        reports
            .iter()
            .map(|report| report["roots"][0]["report"]["created"].as_u64().unwrap())
            .sum::<u64>(),
        1
    );
    assert_eq!(
        reports
            .iter()
            .map(|report| report["roots"][0]["report"]["unchanged"].as_u64().unwrap())
            .sum::<u64>(),
        1
    );
}

#[test]
fn real_repeat_changes_no_page_versions_or_timestamps() {
    let dir = tempfile::tempdir().unwrap();
    let v = open(dir.path());
    let input = project(
        "Alpha",
        vec![root(
            "docs",
            &tar_of(&[("a.md", "# A\n"), ("b.md", "# B\n")]),
        )],
    );
    run(&v, &input);
    v.flush().unwrap();
    let before_pages = pages(&v);
    let before_versions = versions(&v);

    let out = run(&v, &input);
    v.flush().unwrap();
    assert_eq!(out["row_created"], false);
    assert_eq!(out["row_updated"], false);
    assert_eq!(out["roots"][0]["report"]["created"], 0);
    assert_eq!(out["roots"][0]["report"]["updated"], 0);
    assert_eq!(out["roots"][0]["report"]["unchanged"], 2);
    assert_eq!(pages(&v), before_pages);
    assert_eq!(versions(&v), before_versions);
}

#[test]
fn folder_links_do_not_link_their_contents() {
    let dir = tempfile::tempdir().unwrap();
    let v = open(dir.path());
    let archive = tar_of(&[
        (
            "benchmarks/README.md",
            "# Benchmarks\n\n[runs](runs/)\n\n[ab](ab/)\n",
        ),
        ("benchmarks/runs/a.json", "{\"ms\": 1}\n"),
        ("benchmarks/runs/b.json", "{\"ms\": 2}\n"),
        ("benchmarks/ab/c.json", "{\"ms\": 3}\n"),
    ]);
    let input = project("Bench", vec![root("wiki", &archive)]);
    let first = run(&v, &input);
    v.flush().unwrap();
    let report = &first["roots"][0]["report"];
    assert_eq!(report["attachments"], 0);
    assert_eq!(report["not_imported"], 3);
    let expected = json!([
        "benchmarks/ab/c.json",
        "benchmarks/runs/a.json",
        "benchmarks/runs/b.json"
    ]);
    assert_eq!(report["unlinked_files"], expected);

    let before_pages = pages(&v);
    let before_versions = versions(&v);
    let second = run(&v, &input);
    v.flush().unwrap();
    let again = &second["roots"][0]["report"];
    assert_eq!(again["created"], 0);
    assert_eq!(again["updated"], 0);
    assert_eq!(again["not_imported"], 3);
    assert_eq!(again["unlinked_files"], expected);
    assert_eq!(pages(&v), before_pages);
    assert_eq!(versions(&v), before_versions);
}

const IMAGE: &[u8] = b"\x89PNG fake image bytes";

#[test]
fn unlinked_source_files_are_reported_and_secrets_are_not() {
    let dir = tempfile::tempdir().unwrap();
    let v = open(dir.path());
    let archive = tar_of_bytes(&[
        ("a.md", b"# A\n\n![pic](images/pic.png)\n".as_slice()),
        ("images/pic.png", IMAGE),
        ("benchmarks/data.json", b"{\"ms\": 3}\n".as_slice()),
        (".env", b"TOKEN=secret\n".as_slice()),
    ]);
    let input = project("Alpha", vec![root("docs", &archive)]);

    let first = run(&v, &input);
    v.flush().unwrap();
    let report = &first["roots"][0]["report"];
    assert_eq!(report["attachments"], 1);
    assert_eq!(report["not_imported"], 1);
    assert_eq!(report["unlinked_files"], json!(["benchmarks/data.json"]));
    assert_eq!(
        report["skipped_files"],
        json!([{
            "source_file": ".env", "reason": "secret file pattern"
        }])
    );
    let parsed: wiki_import::WikiImportReport = serde_json::from_value(report.clone()).unwrap();
    assert_eq!(parsed.import.attachments, 1);
    assert_eq!(parsed.unlinked_files, ["benchmarks/data.json"]);

    // A reply from an older hub has neither new field and still parses.
    let mut old = report.clone();
    old.as_object_mut().unwrap().remove("not_imported");
    old.as_object_mut().unwrap().remove("unlinked_files");
    let old: wiki_import::WikiImportReport = serde_json::from_value(old).unwrap();
    assert_eq!(old.not_imported, 0);
    assert!(old.unlinked_files.is_empty());

    let before_pages = pages(&v);
    let before_versions = versions(&v);
    let second = run(&v, &input);
    v.flush().unwrap();
    let again = &second["roots"][0]["report"];
    assert_eq!(again["created"], 0);
    assert_eq!(again["updated"], 0);
    assert_eq!(again["attachments"], 1);
    assert_eq!(again["unlinked_files"], json!(["benchmarks/data.json"]));
    assert_eq!(again["not_imported"], 1);
    assert_eq!(again["skipped_files"], report["skipped_files"]);
    assert_eq!(pages(&v), before_pages);
    assert_eq!(versions(&v), before_versions);
}

#[test]
fn identical_bytes_at_another_path_are_still_reported_unlinked() {
    let dir = tempfile::tempdir().unwrap();
    let v = open(dir.path());
    // Same bytes as the linked image, under a nested path whose case is kept.
    let archive = tar_of_bytes(&[
        ("a.md", b"# A\n\n![pic](images/pic.png)\n".as_slice()),
        ("images/pic.png", IMAGE),
        ("Notes/Raw/Shot.JSON", IMAGE),
    ]);
    let out = run(&v, &project("Alpha", vec![root("docs", &archive)]));
    assert_eq!(
        out["roots"][0]["report"]["unlinked_files"],
        json!(["Notes/Raw/Shot.JSON"])
    );
    assert_eq!(out["roots"][0]["report"]["not_imported"], 1);
}

#[test]
fn dry_run_leaves_the_real_vault_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let v = open(dir.path());
    let alpha = project("Alpha", vec![root("docs", &tar_of(&[("a.md", "# A\n")]))]);
    run(&v, &alpha);
    v.flush().unwrap();
    let vault_dir = dir.path().join("vault");
    let before = (
        v.page_count().unwrap(),
        pages(&v),
        versions(&v),
        files(&vault_dir),
    );

    let mut new_project = project("Delta", vec![root("docs", &tar_of(&[("d.md", "# D\n")]))]);
    new_project["dry_run"] = json!(true);
    let mut changed_alpha = alpha.clone();
    changed_alpha["dry_run"] = json!(true);
    changed_alpha["last_activity"] = json!("2026-12-01");
    let out_new = run(&v, &new_project);
    let out_alpha = run(&v, &changed_alpha);
    v.flush().unwrap();

    assert_eq!(out_new["dry_run"], true);
    assert_eq!(out_new["counts_relative_to"], "empty vault");
    assert_eq!(out_new["row_created"], true);
    assert_eq!(out_new["roots"][0]["report"]["created"], 1);
    assert_eq!(out_alpha["row_created"], true);
    assert!(v
        .load_page(out_new["project_page"].as_str().unwrap().to_string())
        .is_err());
    assert_eq!(rows_named(&v, &projects_db(&v).unwrap(), "Delta").len(), 0);

    let after = (
        v.page_count().unwrap(),
        pages(&v),
        versions(&v),
        files(&vault_dir),
    );
    assert_eq!(after, before);
}

#[test]
fn oversized_source_is_refused_with_its_name_and_limit() {
    let dir = tempfile::tempdir().unwrap();
    let v = open(dir.path());
    let input = project("Big", vec![root("docs", &tar_of(&[("a.md", "# A\n")]))]);
    let err = wiki_import::call(&v, &input, 64).unwrap_err();
    assert!(
        err.0.contains("\"docs\"") && err.0.contains("64 byte source limit"),
        "{}",
        err.0
    );
    assert!(projects_db(&v).is_none());
    assert_eq!(v.page_count().unwrap(), 0);
    assert!(wiki_import::call(&v, &input, DEFAULT_SOURCE_LIMIT).is_ok());
}

#[test]
fn unsafe_archives_are_refused_before_any_write() {
    let dir = tempfile::tempdir().unwrap();
    let v = open(dir.path());
    let cases: Vec<(&str, Vec<u8>, &str)> = vec![
        ("..", tar_of(&[("../escape.md", "x")]), ".."),
        ("absolute", tar_of(&[("/abs.md", "x")]), "absolute"),
        ("backslash", tar_of(&[("dir\\x.md", "x")]), "backslash"),
        ("drive", tar_of(&[("C:/drive.md", "x")]), "drive letter"),
        (
            "nested drive",
            tar_of(&[("a/C:/escape.md", "x")]),
            "drive letter",
        ),
        (
            "symlink",
            special_tar("link.md", tar::EntryType::Symlink),
            "link",
        ),
        (
            "hard link",
            special_tar("link.md", tar::EntryType::Link),
            "link",
        ),
    ];
    for (label, archive, needle) in &cases {
        let input = project("Bad", vec![root("docs", archive)]);
        let err = wiki_import::call(&v, &input, DEFAULT_SOURCE_LIMIT).unwrap_err();
        assert!(err.0.contains(*needle), "{label}: {}", err.0);
    }

    let good = tar_of(&[("ok.md", "# ok\n")]);
    let mixed = project("Mixed", vec![root("good", &good), root("bad", &cases[0].1)]);
    assert!(wiki_import::call(&v, &mixed, DEFAULT_SOURCE_LIMIT).is_err());
    assert!(projects_db(&v).is_none());
    assert_eq!(v.page_count().unwrap(), 0);
}
