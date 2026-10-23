//! Exercise the CLI against an isolated, real HTTP hub. Never use the user's
//! configuration, Claude projects, or deployed vault in these tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, SystemTime};

use notes_hub::agent_tokens::{AgentScope, AgentTokens};
use notes_hub::{AgentSection, Config as HubConfig, Server};
use serde_json::{json, Value};
use vault_tools::notes_cli::{self, Client, Config};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("notes_cli_{}", vault_core::new_id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Env {
    server: Server,
    url: String,
    token: String,
    data: TempDir,
}

impl Env {
    async fn new() -> Self {
        let data = TempDir::new();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = HubConfig {
            listen: listener.local_addr().unwrap().to_string(),
            data_dir: data.0.join("hub"),
            trust_proxy: false,
            agent: AgentSection {
                enabled: false,
                ..AgentSection::default()
            },
            ..HubConfig::default()
        };
        let store = AgentTokens::open(config.vault_dir()).unwrap();
        let token = store.add("cli-test", AgentScope::Write).unwrap();
        let server = notes_hub::start_on(listener, config).await.unwrap();
        let url = format!("http://{}/mcp", server.addr());
        Self {
            server,
            url,
            token,
            data,
        }
    }

    async fn cli_at(
        &self,
        args: &[&str],
        input: Option<&str>,
        cwd: &Path,
    ) -> anyhow::Result<String> {
        let args = args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let input = input.map(str::to_owned);
        let cwd = cwd.to_path_buf();
        let config = Config {
            hub: self.url.clone(),
            token: self.token.clone(),
        };
        let result = tokio::task::spawn_blocking(move || {
            notes_cli::run_with_config(&args, input.as_deref(), &cwd, config)
        })
        .await
        .unwrap();
        match &result {
            Ok(output) => assert!(
                !output.contains(&self.token),
                "credential leaked in success output"
            ),
            Err(error) => assert!(
                !format!("{error:#}").contains(&self.token),
                "credential leaked in error"
            ),
        }
        result
    }

    async fn cli(&self, args: &[&str], input: Option<&str>) -> String {
        self.cli_at(args, input, &self.data.0).await.unwrap()
    }

    async fn import(&self, projects: &Path, map: &Path, dry_run: bool) -> String {
        let projects = projects.to_path_buf();
        let map = map.to_path_buf();
        let config = Config {
            hub: self.url.clone(),
            token: self.token.clone(),
        };
        let output = tokio::task::spawn_blocking(move || {
            let client = Client::new(config)?;
            notes_cli::import::run(Some(&client), &projects, &map, dry_run)
        })
        .await
        .unwrap()
        .unwrap();
        assert!(
            !output.contains(&self.token),
            "credential leaked in import output"
        );
        output
    }

    fn memory_page(&self, path: &str) -> vault_core::PageSummary {
        let vault = self.server.hub().vault();
        let mut parent = None;
        let mut found = None;
        for segment in std::iter::once("Memory").chain(path.split('/')) {
            let children = vault.list_children(parent, 0, 1000).unwrap();
            let matching = children
                .into_iter()
                .filter(|p| p.title == segment)
                .collect::<Vec<_>>();
            assert_eq!(
                matching.len(),
                1,
                "missing or duplicate path segment {segment}"
            );
            found = matching.into_iter().next();
            parent = found.as_ref().map(|p| p.id.clone());
        }
        found.unwrap()
    }

    fn assert_fixed_memory_schema(&self, project: &str) {
        let schema = self
            .server
            .hub()
            .vault()
            .get_schema(self.memory_page(project).id)
            .unwrap();
        let mut names = schema
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(
            names,
            ["description", "name", "source", "type"],
            "import must preserve the fixed memory schema"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn page_roundtrip_search_and_raw_json() {
    let env = Env::new().await;
    env.cli(
        &["write", "CLI roundtrip"],
        Some("Original cliuniqueneedle body."),
    )
    .await;
    let read = env.cli(&["read", "CLI roundtrip"], None).await;
    assert!(read.contains("Original cliuniqueneedle body."), "{read}");
    let found = env.cli(&["search", "cliuniqueneedle"], None).await;
    assert!(found.contains("CLI roundtrip"), "{found}");
    env.cli(&["append", "CLI roundtrip"], Some("Appended paragraph."))
        .await;
    let read = env.cli(&["read", "CLI roundtrip"], None).await;
    assert!(read.contains("Original cliuniqueneedle body."));
    assert!(read.contains("Appended paragraph."));
    env.cli(&["write", "CLI roundtrip"], Some("Replacement body."))
        .await;
    let read = env.cli(&["read", "CLI roundtrip"], None).await;
    assert!(read.contains("Replacement body."));
    assert!(!read.contains("Original"));
    assert!(!read.contains("Appended"));
    let matching = env
        .server
        .hub()
        .vault()
        .list_children(None, 0, 1000)
        .unwrap()
        .into_iter()
        .filter(|p| p.title == "CLI roundtrip")
        .count();
    assert_eq!(
        matching, 1,
        "writing an existing title must replace its body"
    );

    let raw = env.cli(&["--json", "read", "CLI roundtrip"], None).await;
    let raw: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(raw["isError"], false, "{raw}");
    assert_eq!(raw["content"][0]["type"], "text");
    assert!(raw["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Replacement body."));
    assert!(
        raw.get("jsonrpc").is_none(),
        "--json is the raw tool result"
    );
    let parent = env
        .server
        .hub()
        .vault()
        .create_page(None, "CLI parent".into())
        .unwrap();
    env.cli(
        &["write", "CLI child", "--parent", "CLI parent"],
        Some("Nested body."),
    )
    .await;
    let children = env
        .server
        .hub()
        .vault()
        .list_children(Some(parent), 0, 100)
        .unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].title, "CLI child");
    assert!(env
        .cli(&["read", "CLI child"], None)
        .await
        .contains("Nested body."));
    env.server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn database_query_applies_filter_and_sort() {
    let env = Env::new().await;
    let vault = env.server.hub().vault();
    let database = vault
        .create_page(None, "CLI query database".into())
        .unwrap();
    vault
        .set_schema(
            database.clone(),
            vec![vault_core::PropDef {
                key: "points".into(),
                name: "Points".into(),
                kind: vault_core::PropKind::Number,
                options: Vec::new(),
                config: String::new(),
            }],
        )
        .unwrap();
    for (title, points) in [
        ("Query low", 1.0),
        ("Query middle", 2.0),
        ("Query high", 3.0),
    ] {
        let row = vault
            .create_page(Some(database.clone()), title.into())
            .unwrap();
        vault
            .set_property(row, "points".into(), vault_core::PropValue::Number(points))
            .unwrap();
    }
    let result = env
        .cli(
            &[
                "query",
                "CLI query database",
                "--filter",
                r#"[{"property":"Points","op":">=","value":2}]"#,
                "--sort",
                r#"[{"property":"Points","descending":true}]"#,
            ],
            None,
        )
        .await;
    assert!(!result.contains("Query low"), "{result}");
    assert!(
        result.find("Query high").unwrap() < result.find("Query middle").unwrap(),
        "{result}"
    );
    env.server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_crud_preserves_unspecified_fields_and_scopes_search() {
    let env = Env::new().await;
    env.cli(
        &[
            "memory",
            "write",
            "test-project/one",
            "--description",
            "Original description",
            "--type",
            "project",
            "--name",
            "Visible name",
        ],
        Some("memorycliuniqueneedle first"),
    )
    .await;
    let read = env.cli(&["memory", "read", "test-project/one"], None).await;
    assert!(read.contains("Visible name"));
    assert!(read.contains("Original description"));
    assert!(read.contains("memorycliuniqueneedle first"));
    env.cli(
        &[
            "memory",
            "update",
            "test-project/one",
            "--description",
            "Revised description",
        ],
        None,
    )
    .await;
    env.cli(
        &["memory", "update", "test-project/one", "--append"],
        Some("Second paragraph."),
    )
    .await;
    let read = env.cli(&["memory", "read", "test-project/one"], None).await;
    assert!(read.contains("Revised description"));
    assert!(read.contains("Visible name"));
    assert!(read.contains("memorycliuniqueneedle first"));
    assert!(read.contains("Second paragraph."));
    let listing = env.cli(&["memory", "list", "test-project"], None).await;
    assert!(listing.contains("test-project/one"));
    let search = env
        .cli(
            &[
                "memory",
                "search",
                "memorycliuniqueneedle",
                "--scope",
                "test-project",
            ],
            None,
        )
        .await;
    assert!(search.contains("- **one** "), "{search}");
    assert!(search.contains("in Memory / test-project"), "{search}");
    env.cli(
        &["memory", "update", "test-project/one"],
        Some("Replaced memory."),
    )
    .await;
    let read = env.cli(&["memory", "read", "test-project/one"], None).await;
    assert!(read.contains("Replaced memory."));
    assert!(!read.contains("Second paragraph."));
    env.cli(&["memory", "delete", "test-project/one"], None)
        .await;
    let listing = env.cli(&["memory", "list", "test-project"], None).await;
    assert!(!listing.contains("test-project/one"));
    assert!(env
        .cli_at(&["memory", "read", "test-project/one"], None, &env.data.0)
        .await
        .is_err());
    env.server.stop().await;
}

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn moved_memory_retains_import_source_for_rescanning() {
    let env = Env::new().await;
    for path in ["original/item", "destination/anchor"] {
        env.cli(
            &[
                "memory",
                "write",
                path,
                "--description",
                "Move fixture",
                "--type",
                "project",
            ],
            Some("Move fixture body."),
        )
        .await;
    }
    let memory = env.memory_page("original/item");
    let original = env.memory_page("original");
    let vault = env.server.hub().vault();
    let source_key = vault
        .get_schema(original.id)
        .unwrap()
        .into_iter()
        .find(|field| field.name == "source")
        .unwrap()
        .key;
    vault
        .set_property(
            memory.id.clone(),
            source_key,
            vault_core::PropValue::Text("original/item".into()),
        )
        .unwrap();
    let url = env.url.clone();
    let token = env.token.clone();
    let home = env.data.0.join("move-subprocess-home");
    let output = tokio::task::spawn_blocking(move || {
        // isolated_bin deliberately supplies null stdin: run() must treat
        // that empty pipe as no body on this property-only update.
        isolated_bin(&home)
            .env("NOTES_HUB_URL", url)
            .env("NOTES_TOKEN", token)
            .args([
                "memory",
                "update",
                "original/item",
                "--new-path",
                "destination/item",
            ])
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    let output_text = combined(&output);
    assert!(
        !output_text.contains(&env.token),
        "move subprocess leaked its credential"
    );
    assert!(output.status.success(), "{output_text}");
    let read = env.cli(&["memory", "read", "destination/item"], None).await;
    assert_eq!(
        env.memory_page("destination/item").id,
        memory.id,
        "relocation must preserve the page identity"
    );
    let original_listing = env.cli(&["memory", "list", "original"], None).await;
    assert!(
        !original_listing.contains("original/item"),
        "relocation must not leave an original copy"
    );
    assert!(
        read.contains("- source: original/item"),
        "moving a memory must keep its import source visible for rescanning: {read}"
    );
    for field in [
        "- name: item",
        "- description: Move fixture",
        "- type: project",
        "Move fixture body.",
    ] {
        assert!(read.contains(field), "relocation lost {field}: {read}");
    }
    env.assert_fixed_memory_schema("original");
    env.assert_fixed_memory_schema("destination");
    env.server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_index_combines_global_with_main_repository_and_honors_budget() {
    let env = Env::new().await;
    let empty = env
        .cli(&["memory", "index", "--project", "missing-project"], None)
        .await;
    assert!(
        empty.contains("No memories"),
        "empty vault and missing optional project must be accepted: {empty}"
    );
    for (path, description) in [
        (
            "global/common",
            "Global preferences 🐾🐾🐾🐾🐾🐾🐾🐾🐾🐾🐾🐾🐾🐾🐾🐾🐾🐾🐾🐾🐾🐾🐾🐾🐾🐾🐾🐾🐾🐾",
        ),
        ("main-repository/local-a", "Main repository knowledge"),
        ("main-repository/local-b", "Second repository memory"),
        ("main-repository/local-c", "Third repository memory"),
        ("another-project/other", "Must stay out"),
    ] {
        env.cli(
            &[
                "memory",
                "write",
                path,
                "--description",
                description,
                "--type",
                "project",
            ],
            Some("Body omitted from index."),
        )
        .await;
    }
    let repo = env.data.0.join("main-repository");
    let worktree = env.data.0.join("different-worktree-name");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--quiet"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=CLI Tests",
            "-c",
            "user.email=cli-tests@example.invalid",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "fixture",
        ],
    );
    git(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "fixture-worktree",
            worktree.to_str().unwrap(),
        ],
    );
    let inferred = env
        .cli_at(&["memory", "index"], None, &worktree)
        .await
        .unwrap();
    assert!(inferred.contains("global/common"), "{inferred}");
    assert!(inferred.contains("main-repository/local-a"), "{inferred}");
    assert!(!inferred.contains("another-project/other"), "{inferred}");
    assert!(!inferred.contains("Body omitted"), "{inferred}");
    let explicit = env
        .cli(&["memory", "index", "--project", "main-repository"], None)
        .await;
    assert!(explicit.contains("global/common"));
    assert!(explicit.contains("main-repository/local-a"));
    let (global_section, project_section) = explicit.split_once("\n\n").unwrap();
    assert!(global_section.starts_with("## global\n"), "{explicit}");
    assert!(
        project_section.starts_with("## main-repository\n"),
        "{explicit}"
    );
    let project_lines = project_section.lines().skip(1).collect::<Vec<_>>();
    assert_eq!(project_lines.len(), 3);
    for keep in [1, 2] {
        let expected = format!(
            "{global_section}\n\n## main-repository\n{}\n… {} more: list them with `notes memory list main-repository`",
            project_lines[..keep].join("\n"),
            project_lines.len() - keep,
        );
        let budget = expected.chars().count().to_string();
        let limited = env
            .cli(
                &[
                    "memory",
                    "index",
                    "--project",
                    "main-repository",
                    "--max-chars",
                    &budget,
                ],
                None,
            )
            .await;
        assert_eq!(
            limited, expected,
            "keep all global entries and as many complete project lines as fit"
        );
    }
    let non_git = env.data.0.join("non-git").join("main-repository");
    fs::create_dir_all(&non_git).unwrap();
    let no_repo = env
        .cli_at(&["memory", "index"], None, &non_git)
        .await
        .unwrap();
    assert!(no_repo.contains("global/common"), "{no_repo}");
    assert!(
        !no_repo.contains("main-repository/local-a"),
        "ordinary folder name must not infer a project: {no_repo}"
    );
    let limited = env
        .cli(
            &[
                "memory",
                "index",
                "--project",
                "main-repository",
                "--max-chars",
                "48",
            ],
            None,
        )
        .await;
    assert!(
        limited.chars().count() <= 48,
        "budget exceeded: {limited:?}"
    );
    env.server.stop().await;
}

fn isolated_bin(home: &Path) -> Command {
    fs::create_dir_all(home).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_notes"));
    command
        .current_dir(home)
        .env_remove("NOTES_TOKEN")
        .env_remove("NOTES_HUB_URL")
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("APPDATA", home)
        .env("LOCALAPPDATA", home)
        .env("XDG_CONFIG_HOME", home)
        .stdin(Stdio::null());
    command
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn missing_hub_is_clear_and_has_no_built_in_default() {
    let temp = TempDir::new();
    let output = isolated_bin(&temp.0.join("home"))
        .env("NOTES_TOKEN", "synthetic-token")
        .args(["search", "something"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let text = combined(&output);
    assert!(text.contains("no hub configured"), "{text}");
    assert!(!text.contains("synthetic-token"), "token leaked: {text}");
}

#[test]
fn missing_token_is_clear_and_offline_dry_run_needs_no_credentials() {
    let temp = TempDir::new();
    let output = isolated_bin(&temp.0.join("home"))
        .args(["search", "something"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let text = combined(&output);
    assert!(
        text.contains("NOTES_TOKEN") || text.to_lowercase().contains("token"),
        "{text}"
    );
    assert!(
        !text.to_lowercase().contains("connection refused"),
        "missing token should fail before network: {text}"
    );

    let projects = temp.0.join("projects");
    let map = temp.0.join("map.json");
    fs::create_dir_all(projects.join("Projects-offline").join("memory")).unwrap();
    fs::write(
        projects.join("Projects-offline/memory/note.md"),
        memory("Offline", "reference", "Offline body.", ""),
    )
    .unwrap();
    fs::write(
        &map,
        json!({"prefixes":["Projects-"],"overrides":{},"fallback":"unsorted"}).to_string(),
    )
    .unwrap();
    let output = notes_cli::import::run(None, &projects, &map, true).unwrap();
    assert!(output.contains("offline: new=1"), "{output}");
    let output = isolated_bin(&temp.0.join("offline-home"))
        .args([
            "memory",
            "import",
            projects.to_str().unwrap(),
            "--map",
            map.to_str().unwrap(),
            "--dry-run",
            "--offline",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", combined(&output));
    assert!(String::from_utf8_lossy(&output.stdout).contains("offline: new=1"));
}

#[test]
fn memory_rules_print_the_rules_text_exactly_without_credentials() {
    let temp = TempDir::new();
    let output = isolated_bin(&temp.0.join("rules-home"))
        .args(["memory", "rules"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", combined(&output));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        vault_tools::help::MEMORY_RULES
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subprocess_configuration_and_auth_errors_never_print_token() {
    let env = Env::new().await;
    env.cli(&["write", "Subprocess test"], Some("subprocessneedle"))
        .await;
    env.cli(&["write", "Credential echo"], Some(&env.token))
        .await;
    let redacted = env.cli(&["read", "Credential echo"], None).await;
    assert!(
        redacted.contains("[redacted]"),
        "synthetic credential echoed from vault must be redacted"
    );
    let error = env
        .cli_at(&["memory", "read", &env.token], None, &env.data.0)
        .await
        .unwrap_err();
    assert!(!error.to_string().contains(&env.token));
    let url = env.url.clone();
    let token = env.token.clone();
    let home = env.data.0.join("subprocess-home");
    let output = tokio::task::spawn_blocking(move || {
        isolated_bin(&home)
            .env("NOTES_HUB_URL", url)
            .env("NOTES_TOKEN", token)
            .args(["--json", "read", "Subprocess test"])
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    let text = combined(&output);
    assert!(
        !text.contains(&env.token),
        "credential leaked in subprocess output"
    );
    assert!(output.status.success(), "{text}");
    let raw: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(raw["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("subprocessneedle"));

    let url = env.url.clone();
    let home = env.data.0.join("failure-home");
    let bad_token = "cli-test-secret-must-never-appear";
    let output = tokio::task::spawn_blocking(move || {
        isolated_bin(&home)
            .env("NOTES_HUB_URL", url)
            .env("NOTES_TOKEN", bad_token)
            .args(["search", "needle"])
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(!output.status.success());
    let text = combined(&output);
    assert!(
        !text.contains(bad_token),
        "credential leaked in auth failure"
    );
    assert!(!text.trim().is_empty());
    env.server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subprocess_config_file_environment_precedence_and_parse_error_redaction() {
    let env = Env::new().await;
    env.cli(&["write", "Config fixture"], Some("configfixturebody"))
        .await;
    let home = env.data.0.join("config-home");
    let url = env.url.clone();
    let token = env.token.clone();
    tokio::task::spawn_blocking(move || {
        let config_dir = home.join("notes");
        fs::create_dir_all(&config_dir).unwrap();
        let config = config_dir.join("config.toml");
        let check_success = |output: Output| {
            let text = combined(&output);
            assert!(!text.contains(&token), "config credential leaked");
            assert!(output.status.success(), "{text}");
            assert!(String::from_utf8_lossy(&output.stdout).contains("configfixturebody"));
        };
        fs::write(&config, format!("hub = {url:?}\ntoken = {token:?}\n")).unwrap();
        check_success(
            isolated_bin(&home)
                .args(["read", "Config fixture"])
                .output()
                .unwrap(),
        );

        fs::write(
            &config,
            format!("hub = \"http://127.0.0.1:1\"\ntoken = {token:?}\n"),
        )
        .unwrap();
        check_success(
            isolated_bin(&home)
                .env("NOTES_HUB_URL", &url)
                .args(["read", "Config fixture"])
                .output()
                .unwrap(),
        );
        fs::write(
            &config,
            format!("hub = {url:?}\ntoken = \"incorrect-file-token\"\n"),
        )
        .unwrap();
        check_success(
            isolated_bin(&home)
                .env("NOTES_TOKEN", &token)
                .args(["read", "Config fixture"])
                .output()
                .unwrap(),
        );

        let malformed_secret = "private-config-syntax-must-not-be-printed";
        fs::write(
            &config,
            format!("hub = {url:?}\ntoken = \"{malformed_secret}\n"),
        )
        .unwrap();
        let output = isolated_bin(&home)
            .args(["read", "Config fixture"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let text = combined(&output);
        assert!(
            !text.contains(malformed_secret),
            "malformed config credential leaked"
        );
        assert!(text.contains("invalid notes config"), "{text}");
        check_success(
            isolated_bin(&home)
                .env("NOTES_HUB_URL", &url)
                .env("NOTES_TOKEN", &token)
                .args(["read", "Config fixture"])
                .output()
                .unwrap(),
        );
    })
    .await
    .unwrap();
    env.server.stop().await;
}

fn memory(name: &str, kind: &str, body: &str, extras: &str) -> String {
    format!("---\nname: {name}\ndescription: Description of {name}\nmetadata:\n  type: {kind}\n{extras}---\n\n{body}\n")
}

fn imported_metadata(read: &str) -> serde_yaml::Value {
    let (_, section) = read
        .rsplit_once("\n## Imported metadata\n")
        .expect("missing trailing imported metadata section");
    let (fence, section) = section
        .trim_start()
        .split_once('\n')
        .expect("missing YAML fence");
    // The vault's current markdown round trip omits code-fence languages.
    assert!(
        matches!(fence, "```yaml" | "```"),
        "metadata must be fenced YAML"
    );
    let (yaml, trailing) = section
        .rsplit_once("\n```")
        .expect("missing closing YAML fence");
    assert!(
        trailing.trim().is_empty(),
        "imported metadata must be the trailing section"
    );
    serde_yaml::from_str(yaml).expect("imported metadata must preserve valid YAML")
}

fn fixture(projects: &Path, slug: &str, stem: &str, contents: &str, timestamp: u64) -> PathBuf {
    let dir = projects.join(slug).join("memory");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{stem}.md"));
    fs::write(&path, contents).unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(
            fs::FileTimes::new()
                .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(timestamp)),
        )
        .unwrap();
    path
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn importer_resolves_twins_preserves_extras_and_follows_moved_page_identity() {
    let env = Env::new().await;
    let projects = env.data.0.join("fixture-projects");
    let map = env.data.0.join("import-map.json");
    fs::write(
        &map,
        json!({
            "prefixes":["Projects-", "Projects-long-prefix-"],
            "overrides":{}, "fallback":"unsorted"
        })
        .to_string(),
    )
    .unwrap();
    let first = "Projects-demo";
    let twin = "Projects-long-prefix-demo";
    let identical = memory(
        "Identical",
        "reference",
        "# Rich fixture\n\nidenticaltwinsneedle richfixturebaseline _italic_ **bold** [external link](https://example.invalid/fixture).\n\n- First bullet\n- Second bullet\n\n```rust\nlet fixture = 42;\n```",
        "",
    );
    fixture(&projects, first, "identical", &identical, 1_700_000_000);
    fixture(&projects, twin, "identical", &identical, 1_700_000_001);
    fixture(
        &projects,
        first,
        "conflict",
        &memory("Conflict", "project", "old losing content", ""),
        1_700_000_000,
    );
    let winner = fixture(
        &projects,
        twin,
        "conflict",
        &memory(
            "Conflict",
            "project",
            "new winning content",
            "  modified: imported-original-timestamp\n  node_type: fixture-node\n  originSessionId: fixture-session\nsource: fixture-source\npriority: high\n",
        ),
        1_700_000_010,
    );
    fixture(
        &projects,
        "Projects-scratch-session",
        "scratch",
        &memory("Scratch", "feedback", "scratchbodyneedle", ""),
        1_700_000_000,
    );
    fixture(
        &projects,
        first,
        "MEMORY",
        "# Generated index must not become a memory\n",
        1_700_000_000,
    );
    let pages_before_dry_run = env
        .server
        .hub()
        .vault()
        .list_children(None, 0, 1000)
        .unwrap();
    assert!(pages_before_dry_run
        .iter()
        .all(|page| page.title != "Memory"));
    let dry_report = env.import(&projects, &map, true).await;
    assert!(
        dry_report.contains("demo: new=2 updated=0 unchanged=0 conflicts=1"),
        "{dry_report}"
    );
    assert!(
        dry_report.contains("unsorted: new=1 updated=0 unchanged=0 conflicts=0"),
        "{dry_report}"
    );
    assert_eq!(
        env.server
            .hub()
            .vault()
            .list_children(None, 0, 1000)
            .unwrap(),
        pages_before_dry_run,
        "connected dry run must not create Memory or any other page"
    );
    assert!(
        dry_report.contains(&format!(
            "Conflict demo/conflict: selected {}",
            winner.display()
        )),
        "{dry_report}"
    );
    assert!(
        !dry_report.contains("Conflict demo/identical:"),
        "identical twins must not be conflicts"
    );
    let first_report = env.import(&projects, &map, false).await;
    assert!(
        first_report.contains(&format!(
            "Conflict demo/conflict: selected {}",
            winner.display()
        )),
        "{first_report}"
    );
    assert!(!first_report.trim().is_empty());
    let listing = env.cli(&["memory", "list", "demo"], None).await;
    assert_eq!(listing.matches("demo/identical").count(), 1, "{listing}");
    assert_eq!(listing.matches("demo/conflict").count(), 1, "{listing}");
    assert!(!listing.contains("MEMORY"), "{listing}");
    let read = env.cli(&["memory", "read", "demo/conflict"], None).await;
    assert!(read.contains("new winning content"), "{read}");
    assert!(!read.contains("old losing content"), "{read}");
    let expected_metadata = serde_yaml::to_value(json!({
        "metadata": {
            "modified": "imported-original-timestamp",
            "node_type": "fixture-node",
            "originSessionId": "fixture-session"
        },
        "priority": "high"
    }))
    .unwrap();
    assert_eq!(
        imported_metadata(&read),
        expected_metadata,
        "preserve original metadata nesting in the body"
    );
    assert!(
        !read.contains("fixture-source"),
        "source-file source is excluded from imported metadata"
    );
    assert!(
        !read.contains("import_source_hash") && !read.contains("import_content_hash"),
        "import must not store fingerprint metadata"
    );
    assert!(
        read.contains("source: demo/conflict"),
        "source import key lost: {read}"
    );
    let unsorted = env.cli(&["memory", "list", "unsorted"], None).await;
    assert!(unsorted.contains("unsorted/scratch"), "{unsorted}");
    env.assert_fixed_memory_schema("demo");
    env.assert_fixed_memory_schema("unsorted");
    let rich_read = env.cli(&["memory", "read", "demo/identical"], None).await;
    assert!(
        !rich_read.contains("## Imported metadata"),
        "a file without extras needs no metadata section"
    );

    let original = env.memory_page("demo/conflict");
    let rich_original = env.memory_page("demo/identical");
    env.server.hub().vault().flush().unwrap();
    let rich_versions_before = env
        .server
        .hub()
        .vault()
        .list_versions(rich_original.id.clone())
        .unwrap()
        .len();
    let versions_before = env
        .server
        .hub()
        .vault()
        .list_versions(original.id.clone())
        .unwrap()
        .len();
    let second_report = env.import(&projects, &map, false).await;
    assert!(
        second_report.contains("demo: new=0 updated=0 unchanged=2 conflicts=1"),
        "{second_report}"
    );
    assert!(
        second_report.contains("unsorted: new=0 updated=0 unchanged=1 conflicts=0"),
        "{second_report}"
    );
    assert_eq!(
        env.server
            .hub()
            .vault()
            .list_versions(rich_original.id.clone())
            .unwrap()
            .len(),
        rich_versions_before,
        "rich markdown normalization must not cause history edits on unchanged reimport"
    );
    let current = env.memory_page("demo/conflict");
    assert_eq!(current.id, original.id);
    assert_eq!(
        env.server
            .hub()
            .vault()
            .list_versions(current.id.clone())
            .unwrap()
            .len(),
        versions_before,
        "unchanged reimport must not add history edits"
    );

    env.cli(
        &["memory", "update", "demo/identical"],
        Some("Vault-only user body edit."),
    )
    .await;
    let restore_report = env.import(&projects, &map, false).await;
    assert!(
        restore_report.contains("demo: new=0 updated=1 unchanged=1 conflicts=1"),
        "{restore_report}"
    );
    assert_eq!(
        env.memory_page("demo/identical").id,
        rich_original.id,
        "restore must preserve the imported page identity"
    );
    let restored = env.cli(&["memory", "read", "demo/identical"], None).await;
    assert!(restored.contains("richfixturebaseline"), "{restored}");
    assert!(
        !restored.contains("Vault-only user body edit."),
        "{restored}"
    );
    let restored_report = env.import(&projects, &map, false).await;
    assert!(
        restored_report.contains("demo: new=0 updated=0 unchanged=2 conflicts=1"),
        "{restored_report}"
    );

    env.cli(
        &[
            "memory",
            "write",
            "destination/anchor",
            "--description",
            "Destination container",
            "--type",
            "project",
        ],
        Some("anchor"),
    )
    .await;
    env.cli(
        &[
            "memory",
            "update",
            "demo/conflict",
            "--new-path",
            "destination/conflict",
        ],
        None,
    )
    .await;
    assert_eq!(
        env.memory_page("destination/conflict").id,
        current.id,
        "memory_update relocation must preserve the page identity"
    );
    let relocated = env
        .cli(&["memory", "read", "destination/conflict"], None)
        .await;
    assert_eq!(imported_metadata(&relocated), expected_metadata);
    for field in [
        "- name: Conflict",
        "- description: Description of Conflict",
        "- type: project",
        "- source: demo/conflict",
        "new winning content",
    ] {
        assert!(
            relocated.contains(field),
            "relocation lost {field}: {relocated}"
        );
    }
    fs::write(
        &winner,
        memory(
            "Conflict",
            "project",
            "changed after manual move",
            "  modified: imported-original-timestamp\n  node_type: fixture-node\n  originSessionId: fixture-session\nsource: fixture-source\npriority: high\n",
        ),
    )
    .unwrap();
    env.import(&projects, &map, false).await;
    let moved = env.memory_page("destination/conflict");
    assert_eq!(
        moved.id, original.id,
        "import must update the manually moved memory in place"
    );
    let moved_read = env
        .cli(&["memory", "read", "destination/conflict"], None)
        .await;
    assert!(
        moved_read.contains("changed after manual move"),
        "{moved_read}"
    );
    assert_eq!(
        imported_metadata(&moved_read),
        expected_metadata,
        "moving and reimporting preserves original metadata nesting"
    );
    assert!(
        moved_read.contains("- source: demo/conflict"),
        "moving and reimporting preserves the import source identity"
    );
    env.assert_fixed_memory_schema("destination");
    env.assert_fixed_memory_schema("demo");
    let original_scope = env.cli(&["memory", "list", "demo"], None).await;
    assert!(
        !original_scope.contains("demo/conflict"),
        "moved import was duplicated: {original_scope}"
    );
    env.server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jobs_commands_reach_the_hubs_jobs_tool() {
    // The test hub runs no agent, so create is refused by the hub itself.
    let env = Env::new().await;
    let off = env
        .cli_at(&["jobs", "create", "Summarise"], None, &env.data.0)
        .await
        .unwrap_err();
    assert!(format!("{off:#}").contains("agent is off"), "{off:#}");
    let listed = env.cli(&["jobs", "list", "--limit", "5"], None).await;
    let listed: serde_json::Value = serde_json::from_str(listed.trim()).unwrap_or_default();
    assert_eq!(listed["jobs"], serde_json::json!([]), "{listed}");
    let missing = env
        .cli_at(
            &["jobs", "wait", "nope", "--timeout-ms", "0"],
            None,
            &env.data.0,
        )
        .await
        .unwrap_err();
    assert!(
        format!("{missing:#}").contains("no such job"),
        "{missing:#}"
    );
    for bad in [
        vec!["jobs"],
        vec!["jobs", "get"],
        vec!["jobs", "create", "x", "--tools", "all"],
        vec!["jobs", "list", "--tools", "read"],
    ] {
        assert!(
            env.cli_at(&bad, None, &env.data.0).await.is_err(),
            "{bad:?}"
        );
    }
    env.server.stop().await;
}
