//! `notes-hub [--config <file>]`: serves the vault replica with sync and
//! OIDC login. `--devices`, `--unpair <label>` and `--pair <label>` (an
//! emergency path that skips login) manage device tokens. `--agent-token`
//! manages agent tokens; it opens only their file, so it works while the
//! server holds the vault.

use std::path::PathBuf;
use std::slice::Iter;

use notes_hub::agent_tokens::{AgentScope, AgentTokens};
use notes_hub::{open_hub, Config};

const USAGE: &str = "usage:
  notes-hub [--config <file>]                    serve (keys can be overridden by NOTES_HUB_* variables)
  notes-hub [--config <file>] --devices          list devices
  notes-hub [--config <file>] --unpair <label>   revoke a device's tokens
  notes-hub [--config <file>] --pair <label>     emergency: create a token without login
  notes-hub [--config <file>] --agent-token add <name> --scope read|write
  notes-hub [--config <file>] --agent-token list     (name, scope, created in Unix ms)
  notes-hub [--config <file>] --agent-token revoke <name>";

enum Cmd {
    Serve,
    Devices,
    Unpair(String),
    Pair(String),
    AgentToken(AgentTokenCmd),
}

enum AgentTokenCmd {
    Add { name: String, scope: AgentScope },
    List,
    Revoke(String),
}

/// The next argument, or the usage error.
fn take(it: &mut Iter<'_, String>) -> Result<String, String> {
    it.next().cloned().ok_or_else(|| USAGE.to_string())
}

fn parse(args: &[String]) -> Result<(Option<PathBuf>, Cmd), String> {
    let mut config = None;
    let mut cmd = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let next = match a.as_str() {
            "--config" => {
                config = Some(PathBuf::from(take(&mut it)?));
                continue;
            }
            "--devices" => Cmd::Devices,
            "--unpair" => Cmd::Unpair(take(&mut it)?),
            "--pair" => Cmd::Pair(take(&mut it)?),
            "--agent-token" => Cmd::AgentToken(parse_agent_token(&mut it)?),
            _ => return Err(USAGE.into()),
        };
        if cmd.replace(next).is_some() {
            return Err(USAGE.into());
        }
    }
    Ok((config, cmd.unwrap_or(Cmd::Serve)))
}

/// `add <name> --scope read|write`, `list` or `revoke <name>`.
fn parse_agent_token(it: &mut Iter<'_, String>) -> Result<AgentTokenCmd, String> {
    match take(it)?.as_str() {
        "add" => {
            let name = take(it)?;
            if take(it)? != "--scope" {
                return Err(USAGE.into());
            }
            let scope = take(it)?
                .parse::<AgentScope>()
                .map_err(|e| format!("{e}\n{USAGE}"))?;
            Ok(AgentTokenCmd::Add { name, scope })
        }
        "list" => Ok(AgentTokenCmd::List),
        "revoke" => Ok(AgentTokenCmd::Revoke(take(it)?)),
        _ => Err(USAGE.into()),
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("notes-hub: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (path, cmd) = parse(&args)?;
    let config = Config::load(path.as_deref())?;
    match cmd {
        Cmd::Serve => serve(config),
        Cmd::Devices => {
            for d in open_hub(&config)?.devices() {
                println!(
                    "{}\t{}",
                    d.label,
                    d.device.as_deref().unwrap_or("(not used yet)")
                );
            }
            Ok(())
        }
        Cmd::Unpair(label) => {
            let n = open_hub(&config)?
                .unpair(&label)
                .map_err(|e| e.to_string())?;
            println!("revoked {n} token(s)");
            Ok(())
        }
        Cmd::Pair(label) => {
            let token = open_hub(&config)?.pair(&label).map_err(|e| e.to_string())?;
            eprintln!(
                "notes-hub: warning: this token skips the login. Use it only when login is \
                 broken, and revoke it (--unpair {label:?}) once it is not needed."
            );
            println!("{token}");
            Ok(())
        }
        Cmd::AgentToken(cmd) => agent_token(&config, cmd),
    }
}

/// Manages agent tokens. Opens only the token file, never the vault, which
/// the running server holds. The new token goes to stdout alone, so a
/// script can capture it; the notice goes to stderr.
fn agent_token(config: &Config, cmd: AgentTokenCmd) -> Result<(), String> {
    let store = AgentTokens::open(config.vault_dir()).map_err(|e| e.to_string())?;
    match cmd {
        AgentTokenCmd::Add { name, scope } => {
            let token = store.add(&name, scope).map_err(|e| e.to_string())?;
            eprintln!(
                "notes-hub: agent token {name:?} ({scope}) added. It is shown once and not \
                 stored, so copy it now."
            );
            println!("{token}");
        }
        AgentTokenCmd::List => {
            for t in store.list().map_err(|e| e.to_string())? {
                println!("{}\t{}\t{}", t.name, t.scope, t.created);
            }
        }
        AgentTokenCmd::Revoke(name) => {
            if !store.revoke(&name).map_err(|e| e.to_string())? {
                return Err(format!("no agent token named {name:?}"));
            }
            println!("revoked {name}");
        }
    }
    Ok(())
}

fn serve(config: Config) -> Result<(), String> {
    notes_hub::init_logging();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async move {
        let mut server = notes_hub::start(config).await?;
        tokio::select! {
            r = server.wait() => return r.map_err(|e| e.to_string()),
            _ = shutdown_signal() => {}
        }
        tracing::info!("notes-hub stopping");
        server.stop().await;
        Ok(())
    })
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = term.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
