//! `notes wiki import` preparation, refusal and report tests. These use temp
//! git repositories and never call a live hub: the only network use is a
//! closed local port, which must fail fast.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use base64::Engine;
use serde_json::json;
use vault_tools::notes_cli::wiki::{self, Options, Prepared, Resolved, Skipped};
use vault_tools::notes_cli::{self, Config};

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("wiki_cli_{}", vault_core::new_id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn git(dir: &Path, args: &[&str]) {
    git_at(dir, args, None);
}

/// Runs git with an optional author and committer date, for dated commits.
fn git_at(dir: &Path, args: &[&str], date: Option<&str>) {
    let mut command = Command::new("git");
    command
        .args([
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir);
    if let Some(date) = date {
        command
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn make_repo(parent: &Path, name: &str) -> PathBuf {
    let dir = parent.join(name);
    fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    git(&dir, &["commit", "-q", "--allow-empty", "-m", "init"]);
    dir
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// Archive paths in archive order, each with `true` when it is a folder.
fn archive_entries(archive: &[u8]) -> Vec<(String, bool)> {
    let mut reader = tar::Archive::new(archive);
    let mut entries = Vec::new();
    for entry in reader.entries().unwrap() {
        let entry = entry.unwrap();
        let is_dir = entry.header().entry_type().is_dir();
        let path = entry.path().unwrap().to_string_lossy().replace('\\', "/");
        entries.push((path.trim_end_matches('/').to_string(), is_dir));
    }
    entries
}

fn options() -> Options {
    Options::default()
}

fn no_config() -> anyhow::Result<Config> {
    Err(anyhow!("config must not load for this run"))
}

fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
}

#[test]
fn repo_and_worktree_share_the_sanitized_main_name() {
    let base = Temp::new();
    let main = make_repo(&base.0, "my proj!");
    write(&main.join("wiki/index.md"), "# Home\n");
    let worktree = base.0.join("other-wt");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            worktree.to_str().unwrap(),
            "-b",
            "wt-branch",
        ],
    );
    write(&worktree.join("wiki/index.md"), "# Home\n");

    for cwd in [&main, &worktree] {
        let Resolved::Ready(source) = wiki::resolve(&options(), cwd).unwrap() else {
            panic!("expected a wiki to import from {}", cwd.display());
        };
        assert_eq!(source.project, "my_proj_");
        assert_eq!(source.root_name, "wiki");
        assert!(source.in_git);
        // The repo path is the main repository, never the linked worktree.
        assert_eq!(
            fs::canonicalize(&source.repo_root).unwrap(),
            fs::canonicalize(&main).unwrap()
        );
    }

    let explicit = Options {
        project: Some("custom.name_1".into()),
        ..options()
    };
    let Resolved::Ready(source) = wiki::resolve(&explicit, &main).unwrap() else {
        panic!("explicit project should resolve");
    };
    assert_eq!(source.project, "custom.name_1");

    let invalid = Options {
        project: Some("bad name".into()),
        ..options()
    };
    assert!(wiki::resolve(&invalid, &main).is_err());
    assert_eq!(wiki::sanitize("café x/y"), "caf___x_y");
}

#[test]
fn non_ascii_repo_name_becomes_one_underscore_per_byte() {
    // Matches `tr -c` in session-start.sh: ï is two UTF-8 bytes, so two `_`.
    assert_eq!(wiki::sanitize("naïve notes"), "na__ve_notes");

    let base = Temp::new();
    let repo = make_repo(&base.0, "naïve notes");
    write(&repo.join("wiki/index.md"), "# Home\n");
    let Resolved::Ready(source) = wiki::resolve(&options(), &repo).unwrap() else {
        panic!("wiki should resolve in a non-ASCII repository");
    };
    assert_eq!(source.project, "na__ve_notes");
}

#[test]
fn wiki_is_preferred_over_docs_and_nothing_is_an_empty_success() {
    let base = Temp::new();
    let repo = make_repo(&base.0, "prefer");
    assert_eq!(
        wiki::resolve(&options(), &repo).unwrap(),
        Resolved::NothingToImport
    );
    write(&repo.join("docs/guide.md"), "# Guide\n");
    let Resolved::Ready(source) = wiki::resolve(&options(), &repo).unwrap() else {
        panic!("docs should be used when there is no wiki");
    };
    assert_eq!(source.root_name, "docs");
    write(&repo.join("wiki/index.md"), "# Home\n");
    let Resolved::Ready(source) = wiki::resolve(&options(), &repo).unwrap() else {
        panic!("wiki should win over docs");
    };
    assert_eq!(source.root_name, "wiki");

    // Outside Git with no DIR there is nothing to import, and no config loads.
    let outside = Temp::new();
    let line = notes_cli::run_wiki(&args(&["wiki", "import"]), &outside.0, no_config).unwrap();
    assert_eq!(line, wiki::NOTHING_LINE);
}

#[test]
fn explicit_directory_outside_git_uses_its_basename() {
    let base = Temp::new();
    let dir = base.0.join("field notes");
    write(&dir.join("page.md"), "# Page\n");
    let Resolved::Ready(source) = wiki::resolve(
        &Options {
            dir: Some(dir.clone()),
            ..options()
        },
        &base.0,
    )
    .unwrap() else {
        panic!("explicit directory should resolve");
    };
    assert_eq!(source.project, "field_notes");
    assert!(!source.in_git);
    assert_eq!(
        fs::canonicalize(&source.repo_root).unwrap(),
        fs::canonicalize(&dir).unwrap()
    );
    let prepared = wiki::prepare(&source, &options()).unwrap();
    assert_eq!(prepared.git_remote, None);
    assert!(prepared.last_activity.is_some(), "mtime fallback");
}

#[test]
fn notes_app_is_refused_without_loading_config() {
    let base = Temp::new();
    let app = make_repo(&base.0, "notes-app");
    write(&app.join("wiki/index.md"), "# Home\n");
    let refused = wiki::REFUSED_LINE;

    assert_eq!(
        notes_cli::run_wiki(&args(&["wiki", "import"]), &app, no_config).unwrap(),
        refused
    );
    let other = make_repo(&base.0, "elsewhere");
    write(&other.join("wiki/index.md"), "# Home\n");
    assert_eq!(
        notes_cli::run_wiki(
            &args(&["wiki", "import", "--project", "notes-app"]),
            &other,
            no_config
        )
        .unwrap(),
        refused
    );
    let dir = app.join("wiki");
    assert_eq!(
        notes_cli::run_wiki(
            &args(&["wiki", "import", dir.to_str().unwrap()]),
            &base.0,
            no_config
        )
        .unwrap(),
        refused
    );
}

#[test]
fn secrets_and_excludes_are_never_packed() {
    let base = Temp::new();
    let root = base.0.join("wiki");
    write(&root.join("index.md"), "# Home\n");
    write(&root.join("sub/page.md"), "# Page\n");
    write(&root.join(".env"), "SECRET_VALUE_ENV=1\n");
    write(&root.join("keys/server.pem"), "SECRET_VALUE_PEM\n");
    write(&root.join("credentials.txt"), "SECRET_VALUE_CRED\n");
    write(&root.join(".obsidian/app.json"), "{}\n");
    write(&root.join("drafts/todo.md"), "# Draft\n");
    write(&root.join("private/plan.md"), "# Plan\n");
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join("index.md"), root.join("link.md")).unwrap();

    let excludes = vec!["drafts/**".to_string(), "private".to_string()];
    let packed = wiki::pack(&root, &excludes).unwrap();

    // Folders survive even when their files are left out; `private` is an
    // excluded subtree, so it is gone entirely.
    let entries = archive_entries(&packed.archive);
    let expected = vec![
        ("drafts".to_string(), true),
        ("index.md".to_string(), false),
        ("keys".to_string(), true),
        ("sub".to_string(), true),
        ("sub/page.md".to_string(), false),
    ];
    assert_eq!(entries, expected);
    assert!(!packed
        .archive
        .windows(b"SECRET_VALUE".len())
        .any(|w| w == b"SECRET_VALUE"));

    let skipped = |path: &str, reason: &'static str| {
        packed.skipped.contains(&Skipped {
            path: path.into(),
            reason,
        })
    };
    assert!(skipped(".env", "secret file pattern"));
    assert!(skipped("keys/server.pem", "secret file pattern"));
    assert!(skipped("credentials.txt", "secret file pattern"));
    assert!(skipped(".obsidian/", "dot directory or file"));
    assert!(skipped("drafts/todo.md", "excluded by caller"));
    assert!(skipped("private/", "excluded by caller"));
    #[cfg(unix)]
    assert!(skipped("link.md", "symbolic link"));
    assert_eq!(packed.files, 2);
    assert!(!entries
        .iter()
        .any(|(path, _)| path.starts_with('.') || path.starts_with("private")));

    // The payload carries the same archive, base64 encoded, and the excludes.
    let prepared = Prepared {
        project: "p".into(),
        repo_path: root.to_string_lossy().into_owned(),
        git_remote: None,
        last_activity: None,
        root_name: "wiki".into(),
        excludes: excludes.clone(),
        archive: packed.archive.clone(),
        local_skipped: packed.skipped.clone(),
        packed_files: packed.files,
        dry_run: true,
    };
    let payload = prepared.payload();
    let root_entry = &payload["roots"][0];
    assert_eq!(root_entry["name"], "wiki");
    assert_eq!(payload["name"], prepared.project);
    assert!(payload.get("title").is_none(), "{payload}");
    assert_eq!(root_entry["exclude"], json!(excludes));
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(root_entry["archive_base64"].as_str().unwrap())
        .unwrap();
    assert_eq!(decoded, packed.archive);
    assert_eq!(payload["dry_run"], true);
}

#[test]
fn cli_rejects_missing_unlinked_fields_instead_of_reporting_zero() {
    let complete = json!({
        "project_page": "page", "row_created": false, "row_updated": false,
        "dry_run": false, "counts_relative_to": "existing vault",
        "roots": [{"name": "wiki", "report": {
            "pages": 0, "attachments": 0, "unresolved_links": 0, "top_pages": [],
            "not_imported": 0, "unlinked_files": []
        }}]
    });
    assert_eq!(
        wiki::parse_reply(&complete.to_string()).unwrap().roots[0]
            .report
            .not_imported,
        0
    );
    for fields in [
        vec!["not_imported"],
        vec!["unlinked_files"],
        vec!["not_imported", "unlinked_files"],
    ] {
        let mut incomplete = complete.clone();
        for field in fields {
            incomplete["roots"][0]["report"]
                .as_object_mut()
                .unwrap()
                .remove(field);
        }
        let error = wiki::parse_reply(&incomplete.to_string()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "notes hub wiki import response lacks unlinked-file reporting; update the hub"
        );
    }
    let mut inconsistent = complete;
    inconsistent["roots"][0]["report"]["unlinked_files"] = json!(["runs/result.json"]);
    let error = wiki::parse_reply(&inconsistent.to_string()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid wiki import response: unlinked-file count does not match its list"
    );
}

#[test]
fn older_hub_report_defaults_unlinked_file_fields() {
    let reply: wiki::Reply = serde_json::from_value(json!({
        "project_page": "page",
        "row_created": false,
        "row_updated": false,
        "dry_run": false,
        "counts_relative_to": "existing vault",
        "roots": [{"name": "wiki", "report": {
            "pages": 0, "attachments": 1, "unresolved_links": 0, "top_pages": []
        }}]
    }))
    .unwrap();
    assert_eq!(reply.roots[0].report.import.attachments, 1);
    assert_eq!(reply.roots[0].report.not_imported, 0);
    assert!(reply.roots[0].report.unlinked_files.is_empty());
}

#[test]
fn report_lists_counts_targets_orphans_unlinked_and_skips_with_redaction() {
    let reply: wiki::Reply = serde_json::from_value(json!({
        "project_page": "Notes page",
        "row_created": true,
        "row_updated": false,
        "dry_run": true,
        "counts_relative_to": "previous import",
        "roots": [{
            "name": "wiki",
            "report": {
                "pages": 4,
                "attachments": 1,
                "not_imported": 1,
                "unlinked_files": ["benchmarks/data.json"],
                "unresolved_links": 1,
                "top_pages": [],
                "created": 2,
                "updated": 1,
                "unchanged": 1,
                "orphaned": 1,
                "tables": 1,
                "images": 1,
                "resolved_links": 3,
                "unresolved_targets": [{"source_file": "a.md", "target": "missing tok-123"}],
                "orphaned_sources": ["old.md"],
                "skipped_files": [{"source_file": "x.pem", "reason": "secret file pattern"}]
            }
        }]
    }))
    .unwrap();
    let prepared = Prepared {
        project: "my-proj".into(),
        repo_path: "/repos/my-proj".into(),
        git_remote: Some("https://u:pw@host/o/r.git".into()),
        last_activity: Some("2026-10-14T10:00:00+00:00".into()),
        root_name: "wiki".into(),
        excludes: vec!["drafts/**".into()],
        archive: Vec::new(),
        local_skipped: vec![Skipped {
            path: ".env".into(),
            reason: "secret file pattern",
        }],
        packed_files: 3,
        dry_run: true,
    };

    let summary = wiki::summary(&reply, &prepared);
    assert!(
        summary.contains("project my-proj (dry run, nothing written)"),
        "{summary}"
    );
    assert!(
        summary.contains("project page Notes page: row created"),
        "{summary}"
    );
    assert!(
        summary.contains(
            "wiki: pages 4, created 2, updated 1, unchanged 1, orphaned 1, attachments 1, images 1, tables 1, links 3 resolved / 1 unresolved, skipped 2 (1 local, 1 hub), not imported 1 (not linked from any page)"
        ),
        "{summary}"
    );
    assert!(summary.contains("counts relative to: previous import"));

    let report = wiki::report(&reply, &prepared, "tok-123");
    for expected in [
        "# Wiki import: my-proj",
        "- Dry run: yes",
        "- Counts relative to: previous import",
        "- Repo path: /repos/my-proj",
        "- Last activity: 2026-10-14T10:00:00+00:00",
        "- Files packed: 3",
        "- Excludes: `drafts/**`",
        "| Pages | 4 |",
        "| Unresolved links | 1 |",
        "| Not imported (not linked from any page) | 1 |",
        "### Not imported: not linked from any page",
        "- `benchmarks/data.json`",
        "`a.md` → `missing [redacted]`",
        "`old.md`",
        "`x.pem`: secret file pattern (hub)",
        "`.env`: secret file pattern (local, not transmitted)",
    ] {
        assert!(
            report.contains(expected),
            "missing {expected:?} in:\n{report}"
        );
    }
    // Credentials are removed from the report, not just the terminal.
    assert!(!report.contains("tok-123"));
    assert!(!report.contains("u:pw"));
    assert!(report.contains("[redacted]@host/o/r.git"));
}

#[test]
fn empty_folders_survive_packing_but_dot_folders_do_not() {
    let base = Temp::new();
    let root = base.0.join("wiki");
    write(&root.join("index.md"), "# Home\n");
    fs::create_dir_all(root.join("empty")).unwrap();
    fs::create_dir_all(root.join("nested/deeper")).unwrap();
    fs::create_dir_all(root.join(".cache/inner")).unwrap();

    let packed = wiki::pack(&root, &[]).unwrap();
    assert_eq!(
        archive_entries(&packed.archive),
        vec![
            ("empty".to_string(), true),
            ("index.md".to_string(), false),
            ("nested".to_string(), true),
            ("nested/deeper".to_string(), true),
        ]
    );
    assert_eq!(packed.files, 1);
    assert!(packed.skipped.contains(&Skipped {
        path: ".cache/".into(),
        reason: "dot directory or file",
    }));
}

#[test]
fn remote_query_and_fragment_are_removed() {
    assert_eq!(
        wiki::strip_credentials("https://user:tok@host.example/o/r.git?token=abc#frag"),
        "https://host.example/o/r.git"
    );
    assert_eq!(
        wiki::strip_credentials("https://host/o/r.git?x=1"),
        "https://host/o/r.git"
    );
    assert_eq!(
        wiki::strip_credentials("ssh://git@host:2222/o/r.git"),
        "ssh://host:2222/o/r.git"
    );
}

#[test]
fn prepare_sends_origin_without_credentials_query_or_fragment() {
    let base = Temp::new();
    let repo = make_repo(&base.0, "remote-check");
    write(&repo.join("wiki/index.md"), "# Home\n");
    git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            "https://u:tok@example.invalid/o/r.git?token=abc#frag",
        ],
    );
    let Resolved::Ready(source) = wiki::resolve(&options(), &repo).unwrap() else {
        panic!("wiki should resolve");
    };
    let prepared = wiki::prepare(&source, &options()).unwrap();
    assert_eq!(
        prepared.git_remote.as_deref(),
        Some("https://example.invalid/o/r.git")
    );
    let payload = prepared.payload().to_string();
    assert!(!payload.contains("tok"), "{payload}");
    assert!(!payload.contains("token=abc"), "{payload}");
}

#[test]
fn last_activity_comes_from_the_selected_folder_commits() {
    let base = Temp::new();
    let repo = make_repo(&base.0, "activity");
    write(&repo.join("wiki/index.md"), "# Home\n");
    git(&repo, &["add", "wiki/index.md"]);
    git_at(
        &repo,
        &["commit", "-q", "-m", "wiki page"],
        Some("2020-01-02T03:04:05+00:00"),
    );
    // A later commit outside the wiki folder must not move last_activity.
    write(&repo.join("other.txt"), "unrelated\n");
    git(&repo, &["add", "other.txt"]);
    git_at(
        &repo,
        &["commit", "-q", "-m", "other file"],
        Some("2024-05-06T07:08:09+00:00"),
    );

    let Resolved::Ready(source) = wiki::resolve(&options(), &repo).unwrap() else {
        panic!("wiki should resolve");
    };
    let prepared = wiki::prepare(&source, &options()).unwrap();
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(prepared.last_activity.as_deref().unwrap()).unwrap(),
        chrono::DateTime::parse_from_rfc3339("2020-01-02T03:04:05+00:00").unwrap()
    );
}

#[test]
fn explicit_notes_app_is_refused_even_without_wiki_or_docs() {
    let base = Temp::new();
    let repo = make_repo(&base.0, "notes-app");
    assert_eq!(
        notes_cli::run_wiki(&args(&["wiki", "import"]), &repo, no_config).unwrap(),
        wiki::REFUSED_LINE
    );
    assert_eq!(
        notes_cli::run_wiki(
            &args(&["wiki", "import", "--project", "notes-app"]),
            &repo,
            no_config
        )
        .unwrap(),
        wiki::REFUSED_LINE
    );
    assert_eq!(
        notes_cli::run_wiki(
            &args(&[
                "wiki",
                "import",
                "--project",
                "notes-app",
                "/definitely/not/here"
            ]),
            &repo,
            no_config
        )
        .unwrap(),
        wiki::REFUSED_LINE
    );
}

#[test]
fn remote_credentials_are_removed() {
    assert_eq!(
        wiki::strip_credentials("https://user:tok@host.example/o/r.git"),
        "https://host.example/o/r.git"
    );
    assert_eq!(
        wiki::strip_credentials("git@github.com:o/r.git"),
        "github.com:o/r.git"
    );
    assert_eq!(wiki::strip_credentials("V:/repos/r"), "V:/repos/r");
    assert_eq!(
        wiki::redact("see https://u:p@h/x and tok", "tok"),
        "see https://[redacted]@h/x and [redacted]"
    );
}

#[test]
fn exclude_is_repeatable_only_for_wiki_import() {
    let base = Temp::new();
    let app = make_repo(&base.0, "notes-app");
    write(&app.join("wiki/index.md"), "# Home\n");
    let no_network = || -> anyhow::Result<Config> {
        Ok(Config {
            hub: "http://127.0.0.1:1".into(),
            token: "unused".into(),
        })
    };

    // Refused before config, even with repeated excludes and an inline value.
    let line = notes_cli::run_wiki(
        &args(&[
            "wiki",
            "import",
            "--exclude",
            "a",
            "--exclude=b/**",
            "--report",
            "r.md",
        ]),
        &app,
        no_config,
    )
    .unwrap();
    assert_eq!(line, wiki::REFUSED_LINE);

    // Other commands reject these options before any hub call.
    let other = make_repo(&base.0, "other");
    write(&other.join("wiki/index.md"), "# Home\n");
    for words in [
        vec!["memory", "index", "--exclude", "x"],
        vec!["memory", "index", "--report", "r.md"],
        vec!["search", "x", "--exclude=y"],
    ] {
        let error = notes_cli::run_with_config(
            &args(&words),
            None,
            &other,
            Config {
                hub: "http://127.0.0.1:1".into(),
                token: "unused".into(),
            },
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "unknown option; use notes --help",
            "{words:?}"
        );
    }
    assert!(
        notes_cli::run_wiki(&args(&["wiki", "import", "--exclude"]), &other, no_network).is_err()
    );
    assert!(notes_cli::run_wiki(
        &args(&["wiki", "import", "--exclude", ""]),
        &other,
        no_network
    )
    .is_err());
    assert!(notes_cli::run_wiki(&args(&["wiki", "import", "a", "b"]), &other, no_network).is_err());
    assert!(notes_cli::run_wiki(&args(&["wiki", "list"]), &other, no_network).is_err());
}

#[test]
fn closed_local_port_fails_within_ten_seconds() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let base = Temp::new();
    let repo = make_repo(&base.0, "closed-port");
    write(&repo.join("wiki/index.md"), "# Home\n");
    let token = "closed-port-token-must-not-print";
    let started = Instant::now();
    let error = notes_cli::run_wiki(&args(&["wiki", "import", "--dry-run"]), &repo, || {
        Ok(Config {
            hub: format!("http://127.0.0.1:{port}"),
            token: token.into(),
        })
    })
    .unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(error.to_string(), "cannot reach notes hub");
    assert!(!error.to_string().contains(token));
}

#[test]
fn cli_accounts_for_json_files_beside_markdown_reports_on_repeat() {
    let base = Temp::new();
    let repo = make_repo(&base.0, "benchmark-project");
    write(&repo.join("wiki/index.md"), "# Home\n");
    write(
        &repo.join("wiki/benchmarks/README.md"),
        "# Benchmarks\n\n[runs](runs/) [ab](ab/)\n",
    );
    let mut expected = Vec::new();
    for i in 0..30 {
        let folder = if i < 14 { "runs" } else { "ab" };
        let stem = format!("benchmarks/{folder}/{i:02}");
        write(
            &repo.join(format!("wiki/{stem}.md")),
            "# Benchmark report\n",
        );
        write(
            &repo.join(format!("wiki/{stem}.json")),
            "{\"elapsed\": 1}\n",
        );
        expected.push(format!("{stem}.json"));
    }
    for i in 0..57 {
        write(&repo.join(format!("wiki/guides/{i:02}.md")), "# Guide\n");
    }
    expected.sort();
    let opts = Options::default();
    let Resolved::Ready(source) = wiki::resolve(&opts, &repo).unwrap() else {
        panic!()
    };
    let prepared = wiki::prepare(&source, &opts).unwrap();
    assert_eq!(prepared.packed_files, 119);
    let vault =
        vault_core::Vault::open(base.0.join("vault").to_string_lossy().into_owned()).unwrap();
    for run in 0..2 {
        let response = vault_tools::call(&vault, "import_markdown", &prepared.payload()).unwrap();
        let reply: wiki::Reply = serde_json::from_str(&response).unwrap();
        let report = &reply.roots[0].report;
        assert_eq!(report.import.attachments, 0);
        assert_eq!(report.not_imported, 30);
        assert_eq!(report.unlinked_files, expected);
        if run == 1 {
            assert_eq!(report.import.created + report.import.updated, 0);
        }
        assert!(wiki::summary(&reply, &prepared).contains("not imported 30"));
        let full = wiki::report(&reply, &prepared, "");
        for path in &expected {
            assert!(full.contains(&format!("- `{path}`")));
        }
    }
}

#[test]
fn prepared_cli_payload_reimports_without_any_new_ops() {
    let base = Temp::new();
    let repo = make_repo(&base.0, "prepared-project");
    write(
        &repo.join("wiki/index.md"),
        "# Intro\n\nSee [missing](absent.md).\n\n![image](image.png)\n",
    );
    write(&repo.join("wiki/image.png"), "synthetic image bytes");
    write(&repo.join("wiki/benchmarks/data.json"), "{\"value\": 1}\n");
    write(&repo.join("wiki/.env"), "NEVER_TRANSMIT=secret\n");
    fs::create_dir_all(repo.join("wiki/empty")).unwrap();
    let opts = Options::default();
    let Resolved::Ready(source) = wiki::resolve(&opts, &repo).unwrap() else {
        panic!()
    };
    let prepared = wiki::prepare(&source, &opts).unwrap();
    let vault =
        vault_core::Vault::open(base.0.join("vault").to_string_lossy().into_owned()).unwrap();
    let first = vault_tools::call(&vault, "import_markdown", &prepared.payload()).unwrap();
    let first: serde_json::Value = serde_json::from_str(&first).unwrap();
    assert_eq!(first["roots"][0]["report"]["created"], 1);
    assert_eq!(first["roots"][0]["report"]["attachments"], 1);
    assert_eq!(first["roots"][0]["report"]["not_imported"], 1);
    assert_eq!(
        first["roots"][0]["report"]["unlinked_files"],
        json!(["benchmarks/data.json"])
    );
    let before = vault.page_versions().unwrap();
    let second = vault_tools::call(&vault, "import_markdown", &prepared.payload()).unwrap();
    let reply: wiki::Reply = serde_json::from_str(&second).unwrap();
    assert!(!reply.row_created && !reply.row_updated);
    assert_eq!(reply.roots[0].report.import.unchanged, 1);
    assert_eq!(reply.roots[0].report.import.attachments, 1);
    assert_eq!(reply.roots[0].report.not_imported, 1);
    assert_eq!(
        reply.roots[0].report.unlinked_files,
        vec!["benchmarks/data.json"]
    );
    assert_eq!(prepared.local_skipped.len(), 1);
    assert_eq!(prepared.local_skipped[0].path, ".env");
    assert_eq!(vault.page_versions().unwrap(), before);
    let report = wiki::report(&reply, &prepared, "synthetic-token");
    assert!(wiki::summary(&reply, &prepared).contains("pages 1, created 0, updated 0, unchanged 1"));
    assert!(report.contains("| Pages | 1 |"));
    assert!(
        report.contains("### Not imported: not linked from any page\n\n- `benchmarks/data.json`")
    );
    assert!(
        report.contains("absent.md")
            && report.contains(".env")
            && report.contains("secret file pattern"),
        "{report}"
    );
}

#[test]
fn binary_closed_port_exits_nonzero_fast_with_one_stderr_line() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let base = Temp::new();
    let repo = make_repo(&base.0, "binary-closed-port");
    write(&repo.join("wiki/index.md"), "# Home\n");
    let token = "synthetic-binary-token";
    let start = Instant::now();
    let result = Command::new(env!("CARGO_BIN_EXE_notes"))
        .args(["wiki", "import"])
        .current_dir(&repo)
        .env("NOTES_TOKEN", token)
        .env("NOTES_HUB_URL", format!("http://127.0.0.1:{port}"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(start.elapsed() < Duration::from_secs(10));
    assert!(result.stdout.is_empty());
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(
        stderr.contains("cannot reach notes hub") && !stderr.contains(token),
        "{stderr}"
    );
}
