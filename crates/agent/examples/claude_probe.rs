//! Local discovery / missing-login probe; never sends a signed-in model request.
use std::sync::Arc;
use std::time::{Duration, Instant};
use vault_agent::{Agent, AgentConfig};
use vault_core::Vault;

fn main() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let missing_login = std::env::args().any(|a| a == "--missing-login");
    if missing_login {
        // This example is a separate process. No global login files are changed.
        std::env::set_var("CLAUDE_CONFIG_DIR", temp.path().join("no-login"));
        for name in [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "CLAUDE_CODE_OAUTH_TOKEN",
        ] {
            std::env::remove_var(name);
        }
    }
    let vault = Arc::new(Vault::open(
        temp.path().join("vault").to_string_lossy().into_owned(),
    )?);
    let agent = Agent::open(
        vault,
        AgentConfig {
            data_dir: temp.path().join("data"),
            ..AgentConfig::default()
        },
    )?;
    let found = agent.claude_detection();
    println!(
        "bun: {}",
        found
            .bun_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "not found".into())
    );
    println!(
        "extension: {}",
        found
            .claude_code_dir
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "not found".into())
    );
    println!(
        "node_modules: {}",
        found
            .claude_code_dir
            .as_ref()
            .is_some_and(|p| p.join("node_modules").is_dir())
    );
    let models = agent.models();
    println!(
        "claude models: {:?}",
        models
            .iter()
            .filter(|m| m.id.starts_with("claude-code/"))
            .map(|m| &m.id)
            .collect::<Vec<_>>()
    );
    if found.bun_path.is_some() && found.claude_code_dir.is_some() {
        anyhow::ensure!(
            models.iter().any(|m| m.id.starts_with("claude-code/")),
            "Detected Claude extension did not report any models"
        );
    }
    if missing_login {
        anyhow::ensure!(
            agent
                .host_info()
                .providers
                .iter()
                .any(|p| p == "claude-code"),
            "Claude was not detected"
        );
        let session = agent.create_session("Missing login probe")?;
        agent.send(&session.id, "Say hello", Some("claude-code/haiku".into()))?;
        let start = Instant::now();
        loop {
            let state = agent.session_state(&session.id)?;
            if let Some(run) = state.runs.last() {
                if run.status != "running" && run.status != "pending" {
                    println!(
                        "run: {} error: {}",
                        run.status,
                        run.error.as_deref().unwrap_or("none")
                    );
                    anyhow::ensure!(
                        run.status == "failed"
                            && run
                                .error
                                .as_deref()
                                .is_some_and(|e| e.to_lowercase().contains("/login")),
                        "missing login did not surface /login"
                    );
                    break;
                }
            }
            if start.elapsed() > Duration::from_secs(40) {
                agent.cancel(&session.id);
                anyhow::bail!("missing login did not settle within 40s");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    Ok(())
}
