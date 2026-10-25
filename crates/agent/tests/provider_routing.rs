//! A model whose provider prefix is missing or unavailable fails before any
//! request, instead of reaching the default (Codex) provider. No network.
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use vault_agent::{Agent, AgentConfig, HostKind};
use vault_core::Vault;

const CLAUDE_MODEL: &str = "claude-code/claude-haiku-5-5";

#[test]
fn unavailable_provider_prefix_never_reaches_codex() {
    // A child process keeps the environment (CODEX_HOME, keys) test-local.
    if std::env::var_os("NOTES_TEST_ROUTING_CHILD").is_none() {
        let codex_home = tempfile::tempdir().unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "unavailable_provider_prefix_never_reaches_codex",
                "--nocapture",
            ])
            .env("NOTES_TEST_ROUTING_CHILD", "1")
            // Codex is registered, without a sign-in: a run that reached it
            // would start and fail with "Codex sign-in unavailable".
            .env("CODEX_HOME", codex_home.path())
            .env_remove("NOTES_CLAUDE_CODE_DIR")
            .env_remove("OPENCODE_API_KEY")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let vault =
        Arc::new(Vault::open(root.path().join("vault").to_string_lossy().into_owned()).unwrap());
    let agent = Agent::open(
        vault,
        AgentConfig {
            host_kind: HostKind::Desktop,
            data_dir: root.path().join("data"),
            default_model: CLAUDE_MODEL.into(),
            bun_path: Some(root.path().join("missing-bun")),
            claude_code_dir: Some(root.path().join("missing-extension")),
            ..AgentConfig::default()
        },
    )
    .unwrap();
    let providers = agent.host_info().providers;
    assert!(providers.iter().any(|p| p == "codex"), "{providers:?}");
    assert!(!providers.iter().any(|p| p == "claude-code"), "{providers:?}");

    let expect_claude_error = |error: String| {
        assert!(error.contains("claude-code is unavailable"), "{error}");
        assert!(error.contains("NOTES_CLAUDE_CODE_DIR"), "{error}");
        assert!(error.contains("Bun"), "{error}");
        assert!(!error.contains("Codex"), "{error}");
    };

    // Chat with an explicit model, and with the configured default.
    for model in [Some(CLAUDE_MODEL.to_owned()), None] {
        let s = agent.create_session("Claude Code missing").unwrap();
        let error = agent.send(&s.id, "hello", model).unwrap_err().to_string();
        expect_claude_error(error);
        assert!(agent.session_state(&s.id).unwrap().runs.is_empty());
    }

    // Delegated jobs (the CLI's and hub's task path) check the same way.
    let s = agent.create_session("Claude Code job").unwrap();
    let error = agent
        .run_job("claude-missing", &s.id, "hello", Duration::from_secs(5))
        .unwrap_err()
        .to_string();
    expect_claude_error(error);
    assert!(agent.session_state(&s.id).unwrap().runs.is_empty());

    // Missing key, and a prefix no provider has.
    for (model, needle) in [
        ("opencode/deepseek-v4.1-flash", "OPENCODE_API_KEY"),
        ("nonesuch/model", "no provider named \"nonesuch\""),
    ] {
        let s = agent.create_session("Other prefix").unwrap();
        let error = agent
            .send(&s.id, "hello", Some(model.into()))
            .unwrap_err()
            .to_string();
        assert!(error.contains(needle), "{model}: {error}");
        assert!(agent.session_state(&s.id).unwrap().runs.is_empty());
    }
}
