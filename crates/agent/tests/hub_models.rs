//! Model policy tests use scripted runs and synthetic credentials, never the network.
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use vault_agent::{tasks::JobOutcome, Agent, AgentConfig, HostKind, TestHooks};
use vault_core::Vault;

const MODELS: [&str; 4] = [
    "claude-code/claude-opus-5-5",
    "claude-code/claude-sonnet-5-5",
    "claude-code/claude-haiku-5-5",
    "opencode/deepseek-v4.1-flash",
];

fn open(root: &std::path::Path, host_kind: HostKind, model: &str) -> Agent {
    let vault = Arc::new(Vault::open(root.join("vault").to_string_lossy().into_owned()).unwrap());
    Agent::open(
        vault,
        AgentConfig {
            host_kind,
            data_dir: root.join("data"),
            default_model: model.into(),
            bun_path: Some(root.join("missing-bun")),
            ..AgentConfig::default()
        },
    )
    .unwrap()
}

#[test]
fn provider_environment() {
    // Separate processes prevent environment races with other tests or real credentials.
    let case = std::env::var("NOTES_TEST_PROVIDER_CASE").ok();
    let Some(case) = case else {
        for case in ["missing", "present", "empty"] {
            let root = tempfile::tempdir().unwrap();
            let mut child = Command::new(std::env::current_exe().unwrap());
            child
                .args(["--exact", "provider_environment", "--nocapture"])
                .env("NOTES_TEST_PROVIDER_CASE", case)
                .env("CLAUDE_CONFIG_DIR", root.path())
                .env_remove("CLAUDE_CODE_OAUTH_TOKEN")
                .env_remove("OPENCODE_API_KEY");
            if case != "missing" {
                child.env(
                    "OPENCODE_API_KEY",
                    if case == "present" {
                        "synthetic-test-key"
                    } else {
                        ""
                    },
                );
            }
            assert!(child.status().unwrap().success(), "credential case {case}");
        }
        return;
    };

    let root = tempfile::tempdir().unwrap();
    let hub = open(root.path(), HostKind::Hub, MODELS[1])
        .with_model_allowlist(MODELS.map(str::to_owned).into())
        .unwrap();
    let providers = hub.host_info().providers;
    assert_eq!(providers.iter().any(|p| p == "opencode"), case == "present");
    assert!(!providers.iter().any(|p| p == "codex"));
    let models = hub.models();
    assert_eq!(models.len(), 4);
    assert_eq!(models[0].id, MODELS[1]);
    assert!(models
        .iter()
        .all(|m| MODELS.contains(&m.id.as_str()) && !m.id.starts_with("codex/")));

    if case != "present" {
        let s = hub.create_session("Missing OpenCode key").unwrap();
        let error = hub
            .send(&s.id, "hello", Some(MODELS[3].into()))
            .unwrap_err()
            .to_string();
        assert!(error.contains("OPENCODE_API_KEY"), "{error}");
        hub.vault()
            .set_agent_default_model("hub".into(), Some(MODELS[3].into()))
            .unwrap();
        let error = hub
            .run_job("missing-key", &s.id, "hello", Duration::from_secs(5))
            .unwrap_err()
            .to_string();
        assert!(error.contains("OPENCODE_API_KEY"), "{error}");
        hub.vault()
            .set_agent_default_model("hub".into(), None)
            .unwrap();
    }
    let s = hub.create_session("Missing Claude login").unwrap();
    let result = hub
        .run_job("missing-login", &s.id, "hello", Duration::from_secs(5))
        .unwrap();
    let JobOutcome::Failed(error) = result else {
        panic!("{result:?}");
    };
    assert!(
        error.contains("CLAUDE_CONFIG_DIR") && error.contains("CLAUDE_CODE_OAUTH_TOKEN"),
        "{error}"
    );
    drop(hub);

    let desktop = open(root.path(), HostKind::Desktop, "");
    assert!(desktop.host_info().providers.iter().any(|p| p == "codex"));
    assert_eq!(desktop.default_model(), vault_agent::DEFAULT_MODEL);
    assert_eq!(
        desktop
            .host_info()
            .providers
            .iter()
            .any(|p| p == "opencode"),
        case == "present"
    );
    assert_eq!(
        desktop.models().iter().any(|model| model.id == MODELS[3]),
        case == "present"
    );
}

#[test]
fn hub_selection_rejects_overrides_and_ignores_disallowed_synced_defaults() {
    let root = tempfile::tempdir().unwrap();
    let vault =
        Arc::new(Vault::open(root.path().join("vault").to_string_lossy().into_owned()).unwrap());
    let agent = Agent::open(
        vault.clone(),
        AgentConfig {
            host_kind: HostKind::Hub,
            data_dir: root.path().join("data"),
            default_model: "scripted/configured".into(),
            bun_path: Some(root.path().join("missing-bun")),
            test: TestHooks {
                scripted: Some((vec![], "ok".into())),
                ..TestHooks::default()
            },
            ..AgentConfig::default()
        },
    )
    .unwrap()
    .with_model_allowlist(vec![
        "scripted/configured".into(),
        "scripted/allowed".into(),
    ])
    .unwrap();

    for (vault_model, expected) in [
        ("codex/old-vault-choice", "scripted/configured"),
        ("scripted/allowed", "scripted/allowed"),
    ] {
        vault
            .set_agent_default_model("hub".into(), Some(vault_model.into()))
            .unwrap();
        assert_eq!(agent.default_model(), expected);
        let s = agent.create_session("Job selection").unwrap();
        assert_eq!(
            agent
                .run_job("selection", &s.id, "hello", Duration::from_secs(5))
                .unwrap(),
            JobOutcome::Completed("ok".into())
        );
        assert_eq!(
            agent
                .session_state(&s.id)
                .unwrap()
                .runs
                .last()
                .unwrap()
                .model,
            expected
        );
        let s = agent.create_session("Chat selection").unwrap();
        let sent = agent.send(&s.id, "hello", None).unwrap();
        let vault_agent::Sent::Started(run) = sent else {
            panic!("expected a new run");
        };
        assert_eq!(run.model, expected);
    }
    let s = agent.create_session("Disallowed explicit model").unwrap();
    let error = agent
        .send(&s.id, "hello", Some("codex/disallowed".into()))
        .unwrap_err()
        .to_string();
    assert!(error.contains("agent.models"), "{error}");
    assert!(agent.session_state(&s.id).unwrap().runs.is_empty());
}
