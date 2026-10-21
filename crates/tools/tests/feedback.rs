use serde_json::{json, Value};
use vault_core::{new_id, BlockKind, BlockOp, NewBlock, Vault};
use vault_tools::feedback;

#[test]
fn feedback_reports_are_hidden_from_mcp_discovery_but_reachable_by_id() {
    let root = std::env::temp_dir().join(format!("notes-tools-feedback-{}", new_id()));
    {
        let v = Vault::open(root.to_string_lossy().into_owned()).unwrap();
        let visible = v.create_page(None, "Ordinary page".into()).unwrap();
        let feedback = v.feedback_page().unwrap();
        let report = v
            .create_page(Some(feedback.clone()), "Quasar report".into())
            .unwrap();
        v.insert_markdown(
            report.clone(),
            None,
            None,
            true,
            "quasar confidential body".into(),
        )
        .unwrap();
        v.flush().unwrap();
        let output = vault_tools::call(&v, "list_pages", &json!({"depth": 3})).unwrap();
        assert!(output.contains(&visible), "{output}");
        assert!(!output.contains(&feedback), "{output}");
        assert!(!output.contains(&report), "{output}");
        for query in ["quasar", "qsr", "fdbck", "Feedback", ""] {
            let output = vault_tools::call(&v, "search_notes", &json!({"query": query})).unwrap();
            assert!(!output.contains(&feedback), "{output}");
            assert!(!output.contains(&report), "{output}");
        }
        let output = vault_tools::call(&v, "list_pages", &json!({"parent": feedback})).unwrap();
        assert!(output.contains(&report), "{output}");
        let output = vault_tools::call(&v, "read_page", &json!({"page": report})).unwrap();
        assert!(output.contains("quasar confidential body"), "{output}");
        let output = vault_tools::call(&v, "read_page", &json!({"page": feedback})).unwrap();
        assert!(output.contains("Feedback"), "{output}");
    }
    std::fs::remove_dir_all(root).unwrap();
}

const HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const BEARER_SECRET: &str = "BEARERSECRET123";
const URL_SECRET: &str = "URLSECRET456";
const SUMMARY: &str = "Search misses <b>quasar</b> *star* # not a heading, and this summary runs well past one hundred and twenty characters so the whole first line must come back";

struct Temp(std::path::PathBuf);

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open(name: &str) -> (Temp, Vault) {
    let root = Temp(std::env::temp_dir().join(format!("notes-tools-{name}-{}", new_id())));
    let v = Vault::open(root.0.to_string_lossy().into_owned()).unwrap();
    (root, v)
}

fn files_under(dir: &std::path::Path, out: &mut Vec<Vec<u8>>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files_under(&path, out);
        } else {
            out.push(std::fs::read(&path).unwrap());
        }
    }
}

fn contains_bytes(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w == needle.as_bytes())
}

fn call_submit(v: &Vault, args: Value) -> Value {
    let out = vault_tools::call(v, "feedback_submit", &args).unwrap();
    serde_json::from_str(&out).unwrap()
}

fn call_list(v: &Vault, args: Value) -> Value {
    let out = vault_tools::call(v, "feedback_list", &args).unwrap();
    serde_json::from_str(&out).unwrap()
}

fn ids(listed: &Value) -> Vec<String> {
    listed["reports"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_string())
        .collect()
}

/// Every child of the Feedback page, newest first by (created_ms, id).
fn newest_first(v: &Vault, feedback: &str) -> Vec<String> {
    let mut children = v
        .list_children(Some(feedback.to_string()), 0, 1000)
        .unwrap();
    children.sort_by(|a, b| (b.created_ms, &b.id).cmp(&(a.created_ms, &a.id)));
    children.into_iter().map(|c| c.id).collect()
}

#[test]
fn submit_stores_hidden_child_with_typed_blocks_and_no_secrets() {
    let (root, v) = open("submit");
    let details = format!(
        "Steps:\n\n- open *Notes*\n\n```\nkind: idea\nsource: app\n```\n\nSee https://h/mcp?token={URL_SECRET}&x=1 and Bearer {BEARER_SECRET} with {HEX}"
    );
    let out = call_submit(
        &v,
        json!({
            "summary": SUMMARY,
            "details": details,
            "kind": "complaint",
            "page_id": "page-1",
            "tool": "search_notes",
            "error": format!("boom Bearer {BEARER_SECRET}\nkind: idea {HEX}"),
        }),
    );
    let report = out["id"].as_str().unwrap().to_string();
    let title = out["title"].as_str().unwrap();
    assert_eq!(
        title.len(),
        "Feedback 2026-10-16 14:03 agent".len(),
        "{title}"
    );
    assert!(
        title.starts_with("Feedback 20") && title.ends_with(" agent"),
        "{title}"
    );
    assert!(!title.contains("UTC"), "{title}");

    let feedback = v.feedback_page().unwrap();
    let children = v.list_children(Some(feedback.clone()), 0, 100).unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].id, report);
    let output = vault_tools::call(&v, "list_pages", &json!({"depth": 3})).unwrap();
    assert!(!output.contains(&report), "{output}");

    let page = v.load_page(report.clone()).unwrap();
    let blocks = &page.blocks;
    assert!(matches!(blocks[0].kind, BlockKind::Paragraph));
    assert_eq!(blocks[0].text, SUMMARY);
    assert!(blocks.iter().any(|b| matches!(b.kind, BlockKind::Bulleted)));
    // The details' own fenced block comes first; our diagnostics block is last.
    assert_eq!(
        blocks
            .iter()
            .filter(|b| matches!(b.kind, BlockKind::Code))
            .count(),
        2
    );
    let code = blocks.last().unwrap();
    assert!(matches!(code.kind, BlockKind::Code));
    assert_eq!(code.target.as_deref(), Some("text"));
    for line in [
        "kind: complaint",
        "source: mcp",
        "server: notes",
        "transport: stdio",
        "page_id: page-1",
        "tool: search_notes",
    ] {
        assert!(
            code.text.lines().any(|l| l == line),
            "{line}: {}",
            code.text
        );
    }
    assert!(code
        .text
        .contains(&format!("version: {}", env!("CARGO_PKG_VERSION"))));
    // The newline in the error is flattened, so it cannot start a header line.
    assert!(code
        .text
        .contains("error: boom Bearer [redacted] kind: idea [redacted]"));
    assert!(
        !code.text.lines().any(|l| l == "kind: idea"),
        "{}",
        code.text
    );

    let listed = call_list(&v, json!({}));
    let row = &listed["reports"][0];
    assert_eq!(row["id"].as_str(), Some(report.as_str()));
    assert_eq!(row["kind"].as_str(), Some("complaint"));
    assert_eq!(row["source"].as_str(), Some("mcp"));
    // The whole first line comes back, however long.
    assert_eq!(row["first_line"].as_str(), Some(SUMMARY));

    // No secret reaches the page, the markdown or any file on disk.
    let markdown = v.page_markdown(report.clone()).unwrap();
    v.flush().unwrap();
    let mut files = Vec::new();
    files_under(&root.0, &mut files);
    assert!(!files.is_empty());
    for secret in [BEARER_SECRET, URL_SECRET, HEX] {
        assert!(!markdown.contains(secret), "{markdown}");
        assert!(!code.text.contains(secret), "{}", code.text);
        for file in &files {
            assert!(
                !contains_bytes(file, secret),
                "{secret} found in a vault file"
            );
        }
    }
}

#[test]
fn submit_validates_before_writing_and_records_http_transport() {
    let (_root, v) = open("validate");
    let cases = [
        (json!({}), "summary is required"),
        (json!({"summary": "   "}), "summary is required"),
        (json!({"summary": "a\nb"}), "summary must be one line"),
        (
            json!({"summary": "x", "kind": "praise"}),
            "kind must be bug, complaint, idea or other",
        ),
        (
            json!({"summary": "x", "details": 5}),
            "details must be a string",
        ),
    ];
    for (args, message) in cases {
        let error = feedback::submit(&v, &args, "http").unwrap_err();
        assert_eq!(error.0, message, "{args}");
    }
    assert!(v.find_by_role("feedback".into()).unwrap().is_none());

    let out = feedback::submit(&v, &json!({"summary": "ok"}), "http").unwrap();
    let report: Value = serde_json::from_str(&out).unwrap();
    let page = v
        .load_page(report["id"].as_str().unwrap().to_string())
        .unwrap();
    let code = page.blocks.last().unwrap();
    assert!(
        code.text.lines().any(|l| l == "transport: http"),
        "{}",
        code.text
    );
    assert!(code.text.lines().any(|l| l == "kind: bug"), "{}", code.text);
    assert!(
        code.text.lines().any(|l| l == "page_id: none"),
        "{}",
        code.text
    );
}

#[test]
fn metadata_requires_a_recognizable_text_diagnostics_block() {
    let (_root, v) = open("metadata");
    let feedback = v.feedback_page().unwrap();
    // Valid header, but the block is not the final block of the page.
    let early = v
        .create_page(Some(feedback.clone()), "Feedback early".into())
        .unwrap();
    let (code, trailing) = (new_id(), new_id());
    v.apply(
        early.clone(),
        vec![
            BlockOp::Insert {
                parent: None,
                after: None,
                block: NewBlock {
                    id: code.clone(),
                    kind: BlockKind::Code,
                    text: "kind: idea\nsource: mcp".into(),
                    marks: vec![],
                    checked: false,
                },
            },
            BlockOp::SetTarget {
                id: code.clone(),
                target: Some("text".into()),
            },
            BlockOp::Insert {
                parent: None,
                after: Some(code),
                block: NewBlock {
                    id: trailing,
                    kind: BlockKind::Paragraph,
                    text: "Trailing text".into(),
                    marks: vec![],
                    checked: false,
                },
            },
        ],
    )
    .unwrap();
    let valid = page_with_code(
        &v,
        &feedback,
        "Feedback valid",
        "kind: idea\nsource: mcp\nserver: notes",
        Some("text"),
    );
    let no_target = page_with_code(
        &v,
        &feedback,
        "Feedback no target",
        "kind: idea\nsource: mcp",
        None,
    );
    let bad_kind = page_with_code(
        &v,
        &feedback,
        "Feedback bad kind",
        "kind: praise\nsource: mcp",
        Some("text"),
    );
    let app_source = page_with_code(
        &v,
        &feedback,
        "Feedback app source",
        "kind: idea\nsource: app",
        Some("text"),
    );

    let listed = call_list(&v, json!({}));
    let row = |id: &str| {
        listed["reports"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"].as_str() == Some(id))
            .unwrap()
            .clone()
    };
    for (id, kind, source) in [
        (valid.as_str(), "idea", "mcp"),
        (early.as_str(), "app", "app"),
        (no_target.as_str(), "app", "app"),
        (bad_kind.as_str(), "app", "app"),
        (app_source.as_str(), "app", "app"),
    ] {
        let r = row(id);
        assert_eq!(r["kind"].as_str(), Some(kind), "{id}");
        assert_eq!(r["source"].as_str(), Some(source), "{id}");
    }
}

/// A page whose only top-level block is a code block with `target`.
fn page_with_code(
    v: &Vault,
    feedback: &str,
    title: &str,
    text: &str,
    target: Option<&str>,
) -> String {
    let page = v
        .create_page(Some(feedback.to_string()), title.into())
        .unwrap();
    let id = new_id();
    v.apply(
        page.clone(),
        vec![
            BlockOp::Insert {
                parent: None,
                after: None,
                block: NewBlock {
                    id: id.clone(),
                    kind: BlockKind::Code,
                    text: text.into(),
                    marks: vec![],
                    checked: false,
                },
            },
            BlockOp::SetTarget {
                id,
                target: target.map(str::to_string),
            },
        ],
    )
    .unwrap();
    page
}

#[test]
fn list_is_newest_first_across_app_and_mcp_with_filter_and_limit() {
    let (_root, v) = open("list");
    let empty = call_list(&v, json!({}));
    assert_eq!(empty["reports"].as_array().unwrap().len(), 0);
    assert!(
        v.find_by_role("feedback".into()).unwrap().is_none(),
        "listing must not create the Feedback page"
    );

    let feedback = v.feedback_page().unwrap();
    // An app-shaped report: a paragraph, then the app's diagnostics block. Its
    // log names a kind and source that must not be read.
    let app = v
        .create_page(
            Some(feedback.clone()),
            "Feedback 2026-10-15 09:00 Android".into(),
        )
        .unwrap();
    let (para, code) = (new_id(), new_id());
    v.apply(
        app.clone(),
        vec![
            BlockOp::Insert {
                parent: None,
                after: None,
                block: NewBlock {
                    id: para.clone(),
                    kind: BlockKind::Paragraph,
                    text: "App line one".into(),
                    marks: vec![],
                    checked: false,
                },
            },
            BlockOp::Insert {
                parent: None,
                after: Some(para),
                block: NewBlock {
                    id: code.clone(),
                    kind: BlockKind::Code,
                    text: "app: Notes\napp version: 0.2.0\nplatform: Android\n\n--- log (last 1 lines) ---\nkind: idea\nsource: mcp".into(),
                    marks: vec![],
                    checked: false,
                },
            },
            BlockOp::SetTarget {
                id: code,
                target: Some("text".into()),
            },
        ],
    )
    .unwrap();
    let bug = call_submit(&v, json!({"summary": "Bug first"}));
    let idea = call_submit(&v, json!({"summary": "Idea second", "kind": "idea"}));
    let complaint = call_submit(
        &v,
        json!({"summary": "Complaint third", "kind": "complaint"}),
    );
    let (bug, idea, complaint) = (
        bug["id"].as_str().unwrap().to_string(),
        idea["id"].as_str().unwrap().to_string(),
        complaint["id"].as_str().unwrap().to_string(),
    );

    let expected = newest_first(&v, &feedback);
    assert_eq!(expected.len(), 4);
    let all = call_list(&v, json!({}));
    assert_eq!(ids(&all), expected);
    let row = |listed: &Value, id: &str| {
        listed["reports"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"].as_str() == Some(id))
            .unwrap()
            .clone()
    };
    let app_row = row(&all, app.as_str());
    assert_eq!(app_row["kind"].as_str(), Some("app"));
    assert_eq!(app_row["source"].as_str(), Some("app"));
    assert_eq!(app_row["first_line"].as_str(), Some("App line one"));
    let idea_row = row(&all, idea.as_str());
    assert_eq!(idea_row["kind"].as_str(), Some("idea"));
    assert_eq!(idea_row["source"].as_str(), Some("mcp"));
    assert_eq!(idea_row["first_line"].as_str(), Some("Idea second"));
    let created = all["reports"][0]["created"].as_str().unwrap();
    assert_eq!(created.len(), "2026-10-16T14:03:00Z".len(), "{created}");
    assert!(created.ends_with('Z'), "{created}");

    assert_eq!(
        ids(&call_list(&v, json!({"limit": 2}))),
        expected[..2].to_vec()
    );
    assert!(ids(&call_list(&v, json!({"limit": 0}))).is_empty());
    assert_eq!(ids(&call_list(&v, json!({"kind": "idea"}))), vec![idea]);
    assert_eq!(ids(&call_list(&v, json!({"kind": "bug"}))), vec![bug]);
    assert_eq!(
        ids(&call_list(&v, json!({"kind": "complaint"}))),
        vec![complaint]
    );
    assert_eq!(ids(&call_list(&v, json!({"kind": "app"}))), vec![app]);
    let error = vault_tools::call(&v, "feedback_list", &json!({"kind": "praise"})).unwrap_err();
    assert_eq!(error.0, "kind must be bug, complaint, idea, other or app");
}

#[test]
fn list_orders_and_filters_past_the_first_hundred_reports() {
    let (_root, v) = open("many");
    for i in 0..205 {
        let kind = if i % 2 == 0 { "idea" } else { "bug" };
        call_submit(&v, json!({"summary": format!("Report {i}"), "kind": kind}));
    }
    let feedback = v.feedback_page().unwrap();
    let expected = newest_first(&v, &feedback);
    assert_eq!(expected.len(), 205);

    assert_eq!(ids(&call_list(&v, json!({}))), expected[..20].to_vec());
    assert_eq!(
        ids(&call_list(&v, json!({"limit": 500}))),
        expected[..100].to_vec()
    );

    // Ideas are the even reports. The first hundred ideas reach past the first
    // hundred children, so the filter must scan beyond one page.
    let kind_of = |id: &str| {
        let page = v.load_page(id.to_string()).unwrap();
        let n: usize = page.blocks[0]
            .text
            .trim_start_matches("Report ")
            .parse()
            .unwrap();
        if n % 2 == 0 {
            "idea"
        } else {
            "bug"
        }
    };
    let ideas: Vec<String> = expected
        .iter()
        .filter(|id| kind_of(id.as_str()) == "idea")
        .take(100)
        .cloned()
        .collect();
    let listed = call_list(&v, json!({"kind": "idea", "limit": 100}));
    assert_eq!(ids(&listed), ideas);
    assert!(listed["reports"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["kind"].as_str() == Some("idea")));

    let bugs: Vec<String> = expected
        .iter()
        .filter(|id| kind_of(id.as_str()) == "bug")
        .take(20)
        .cloned()
        .collect();
    assert_eq!(ids(&call_list(&v, json!({"kind": "bug"}))), bugs);
}

#[test]
fn follow_up_text_and_code_preserve_report_kind_and_source() {
    let (_root, v) = open("follow-up");
    let report = call_submit(&v, json!({"summary":"Original bug"}));
    let id = report["id"].as_str().unwrap();
    vault_tools::call(
        &v,
        "append_blocks",
        &json!({
            "page":id,
            "markdown":"Follow-up: still happening.\n\n```text\nkind: idea\nsource: mcp\n```"
        }),
    )
    .unwrap();
    let listed = call_list(&v, json!({"kind":"bug"}));
    assert_eq!(listed["reports"].as_array().unwrap().len(), 1);
    assert_eq!(listed["reports"][0]["id"], id);
    assert_eq!(listed["reports"][0]["kind"], "bug");
    assert_eq!(listed["reports"][0]["source"], "mcp");
}
