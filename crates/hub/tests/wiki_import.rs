//! Only a disposable local hub is used. The CLI is never invoked.
use base64::{engine::general_purpose::STANDARD, Engine};
use notes_hub::agent_tokens::{AgentScope, AgentTokens};
use notes_hub::{AgentSection, Config};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

fn archive(files: &[(&str, &[u8])]) -> String {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, bytes) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, path, *bytes).unwrap();
    }
    STANDARD.encode(tar.into_inner().unwrap())
}

/// A repo import through the hidden `import_markdown` tool (name = the project).
fn request(archive: String, dry: bool) -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
        "name":"import_markdown","arguments":{
            "name":"sample","repo_path":"/repo/sample","git_remote":"https://example.org/sample.git",
            "last_activity":"2026-10-14T00:00:00Z","dry_run":dry,
            "roots":[{"name":"wiki","archive_base64":archive}]
        }
    }})
}

/// The old import shape sent to create_page: refused, with the pointer to import_markdown.
fn create_page_with_roots(archive: String) -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
        "name":"create_page","arguments":{
            "title":"sample","repo_path":"/repo/sample",
            "roots":[{"name":"wiki","archive_base64":archive}]
        }
    }})
}

fn files(root: &Path) -> BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)> {
    fn visit(
        root: &Path,
        path: &Path,
        out: &mut BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)>,
    ) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    (
                        fs::read(&path).unwrap(),
                        fs::metadata(&path).unwrap().modified().unwrap(),
                    ),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}

#[tokio::test]
async fn import_scope_actor_dry_run_and_repeat_through_hub() {
    let data = std::env::temp_dir().join(format!("wiki_hub_{}", vault_core::new_id()));
    let config = Config {
        data_dir: data.clone(),
        agent: AgentSection {
            enabled: false,
            ..Default::default()
        },
        ..Default::default()
    };
    let store = AgentTokens::open(config.vault_dir()).unwrap();
    let read = store.add("reader", AgentScope::Read).unwrap();
    let write = store.add("wiki-writer", AgentScope::Write).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = notes_hub::start_on(listener, config).await.unwrap();
    let client = reqwest::Client::new();
    let url = format!("http://{}/mcp", server.addr());
    // The hub's MCP endpoint serves the tool; the in-app agent surface does not.
    let listed: Value = client
        .post(&url)
        .bearer_auth(&write)
        .json(&json!({"jsonrpc":"2.0","id":0,"method":"tools/list"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(names.contains(&"create_page"), "{listed}");
    assert!(
        !names.contains(&"import_markdown"),
        "import_markdown is hidden: {listed}"
    );
    let vault = server.hub().vault();
    vault.create_page(None, "Handmade".into()).unwrap();
    vault.flush().unwrap();
    let before = vault.page_versions().unwrap();
    let before_files = files(Path::new(&vault.root()));
    let input = archive(&[(
        "intro.md",
        b"# Intro\n\n| A | B |\n|---|---|\n| one | two |\n",
    )]);
    let refused: Value = client
        .post(&url)
        .bearer_auth(&read)
        .json(&request(input.clone(), false))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(refused["error"].is_object(), "{refused}");
    // create_page no longer imports: roots fail on the hub too, and write nothing.
    let refused_create: Value = client
        .post(&url)
        .bearer_auth(&write)
        .json(&create_page_with_roots(input.clone()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        refused_create["result"]["isError"], true,
        "{refused_create}"
    );
    let text = refused_create["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(
        text.contains(vault_tools::CREATE_PAGE_ROOTS_ERROR),
        "{text}"
    );
    let dry: Value = client
        .post(&url)
        .bearer_auth(&write)
        .json(&request(input.clone(), true))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(dry["result"]["isError"], false, "{dry}");
    let dry: Value =
        serde_json::from_str(dry["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(dry["counts_relative_to"], "empty vault");
    assert_eq!(vault.page_versions().unwrap(), before);
    assert_eq!(files(Path::new(&vault.root())), before_files);
    let first: Value = client
        .post(&url)
        .bearer_auth(&write)
        .json(&request(input.clone(), false))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(first["result"]["isError"], false, "{first}");
    let first: Value =
        serde_json::from_str(first["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    let page = first["project_page"].as_str().unwrap().to_owned();
    let history = vault.list_versions(page).unwrap();
    assert!(!history.is_empty());
    assert!(
        history.iter().all(|v| v.device == "agent:wiki-writer"),
        "{history:?}"
    );
    let child = vault
        .list_children(Some(first["project_page"].as_str().unwrap().into()), 0, 100)
        .unwrap()
        .remove(0);
    let history = vault.list_versions(child.id.clone()).unwrap();
    assert!(
        history.iter().all(|v| v.device == "agent:wiki-writer"),
        "{history:?}"
    );
    vault.flush().unwrap();
    let versions = vault.page_versions().unwrap();
    let second: Value = client
        .post(&url)
        .bearer_auth(&write)
        .json(&request(input, false))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(second["result"]["isError"], false, "{second}");
    let second: Value =
        serde_json::from_str(second["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(second["row_created"], false);
    assert_eq!(second["row_updated"], false);
    assert_eq!(second["roots"][0]["report"]["created"], 0);
    assert_eq!(second["roots"][0]["report"]["updated"], 0);
    assert_eq!(second["roots"][0]["report"]["unchanged"], 1);
    assert_eq!(vault.page_versions().unwrap(), versions);
    vault
        .rename_page(child.id.clone(), "ordinary edit".into())
        .unwrap();
    let history = vault.list_versions(child.id).unwrap();
    assert!(
        history.iter().any(|v| v.device == "agent:wiki-writer"),
        "{history:?}"
    );
    assert!(
        history.iter().any(|v| v.device != "agent:wiki-writer"),
        "{history:?}"
    );
    server.stop().await;
    fs::remove_dir_all(data).unwrap();
}

#[tokio::test]
async fn source_cap_and_import_only_large_body_allowance() {
    let data = std::env::temp_dir().join(format!("wiki_cap_{}", vault_core::new_id()));
    let mut config = Config {
        data_dir: data.clone(),
        agent: AgentSection {
            enabled: false,
            ..Default::default()
        },
        ..Default::default()
    };
    config.import_source_limit_bytes = 12 << 20;
    let store = AgentTokens::open(config.vault_dir()).unwrap();
    let write = store.add("writer", AgentScope::Write).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = notes_hub::start_on(listener, config).await.unwrap();
    let client = reqwest::Client::new();
    let url = format!("http://{}/mcp", server.addr());
    let blob = vec![b'x'; 7 << 20];
    let input = archive(&[("intro.md", b"intro"), ("large.txt", &blob)]);
    let accepted = client
        .post(&url)
        .bearer_auth(&write)
        .json(&request(input.clone(), true))
        .send()
        .await
        .unwrap();
    assert_eq!(accepted.status(), reqwest::StatusCode::OK);
    let accepted: Value = accepted.json().await.unwrap();
    assert_eq!(accepted["result"]["isError"], false, "{accepted}");
    let mut two_roots = request(input.clone(), true);
    two_roots["params"]["arguments"]["roots"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"docs","archive_base64":input.clone()}));
    let accepted: Value = client
        .post(&url)
        .bearer_auth(&write)
        .json(&two_roots)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(accepted["result"]["isError"], false, "{accepted}");
    // An ordinary create_page is not an import, so it gets the ordinary body
    // limit even though the import source allows more.
    let ordinary = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
        "name":"create_page","arguments":{"title":"Ordinary","markdown":"x".repeat(9 << 20)}
    }});
    let refused = client
        .post(&url)
        .bearer_auth(&write)
        .json(&ordinary)
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), reqwest::StatusCode::PAYLOAD_TOO_LARGE);
    // create_page with roots is not an import either: the large body is refused
    // before the roots error is reached.
    let mut sneaky = create_page_with_roots(archive(&[("intro.md", b"intro")]));
    sneaky["params"]["arguments"]["markdown"] = json!("x".repeat(9 << 20));
    let refused = client
        .post(&url)
        .bearer_auth(&write)
        .json(&sneaky)
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), reqwest::StatusCode::PAYLOAD_TOO_LARGE);
    let oversized = archive(&[("too-large.txt", &vec![b'x'; 13 << 20])]);
    let error: Value = client
        .post(&url)
        .bearer_auth(&write)
        .json(&request(oversized, true))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(error["result"]["isError"], true, "{error}");
    let text = error["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("wiki") && text.contains("12582912"), "{text}");
    server.stop().await;
    fs::remove_dir_all(data).unwrap();
}

/// Folder links are not file links: the md-less folders under `benchmarks/`
/// are reported as unlinked, on the first run and on an unchanged repeat.
#[tokio::test]
async fn folder_links_do_not_link_their_contents_through_hub() {
    let data = std::env::temp_dir().join(format!("wiki_hub_{}", vault_core::new_id()));
    let config = Config {
        data_dir: data.clone(),
        agent: AgentSection {
            enabled: false,
            ..Default::default()
        },
        ..Default::default()
    };
    let store = AgentTokens::open(config.vault_dir()).unwrap();
    let write = store.add("wiki-writer", AgentScope::Write).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = notes_hub::start_on(listener, config).await.unwrap();
    let url = format!("http://{}/mcp", server.addr());
    let vault = server.hub().vault();
    let source = data.join("source");
    for (path, body) in [
        (
            "benchmarks/README.md",
            "# Benchmarks\n\n[runs](runs/)\n\n[ab](ab/)\n",
        ),
        ("benchmarks/runs/a.json", "{\"ms\": 1}\n"),
        ("benchmarks/runs/b.json", "{\"ms\": 2}\n"),
        ("benchmarks/ab/c.json", "{\"ms\": 3}\n"),
    ] {
        let target = source.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, body).unwrap();
    }
    let packed = vault_tools::notes_cli::wiki::pack(&source, &[]).unwrap();
    assert_eq!(packed.files, 4);
    let prepared = std::sync::Arc::new(vault_tools::notes_cli::wiki::Prepared {
        project: "sample".into(),
        repo_path: "/repo/sample".into(),
        git_remote: Some("https://example.org/sample.git".into()),
        last_activity: Some("2026-10-14T00:00:00Z".into()),
        root_name: "wiki".into(),
        excludes: vec![],
        archive: packed.archive,
        local_skipped: packed.skipped,
        packed_files: packed.files,
        dry_run: false,
    });
    let expected = [
        "benchmarks/ab/c.json",
        "benchmarks/runs/a.json",
        "benchmarks/runs/b.json",
    ];
    let import = || {
        let url = url.clone();
        let write = write.clone();
        let prepared = prepared.clone();
        async move {
            tokio::task::spawn_blocking(move || {
                let client = vault_tools::notes_cli::Client::new(vault_tools::notes_cli::Config {
                    hub: url,
                    token: write,
                })
                .unwrap();
                vault_tools::notes_cli::wiki::send(&client, &prepared).unwrap()
            })
            .await
            .unwrap()
        }
    };
    let first = import().await;
    vault.flush().unwrap();
    let before_versions = vault.page_versions().unwrap();
    let again = import().await;
    vault.flush().unwrap();

    let first = &first.roots[0].report;
    assert_eq!(first.import.attachments, 0);
    assert_eq!(first.not_imported, 3);
    assert_eq!(first.unlinked_files, expected);
    let again_report = &again.roots[0].report;
    assert_eq!(again_report.import.created + again_report.import.updated, 0);
    assert_eq!(again_report.not_imported, 3);
    assert_eq!(again_report.unlinked_files, expected);
    assert!(vault_tools::notes_cli::wiki::summary(&again, &prepared).contains("not imported 3"));
    assert!(vault_tools::notes_cli::wiki::report(&again, &prepared, "")
        .contains("- `benchmarks/runs/a.json`"));
    assert_eq!(vault.page_versions().unwrap(), before_versions);
    let _ = fs::remove_dir_all(&data);
}
