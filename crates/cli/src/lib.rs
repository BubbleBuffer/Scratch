//! `scratch-cli`: runs a desktop vault's AI task runner without the app's UI.
//!
//! It makes the same Rust calls as the app's claim loop (`AiStore.pump` in
//! the Flutter app): `tasks::recover` once, then `tasks::claim_next` for this
//! vault copy's device, `Agent::run_task`, and `tasks::finish` with the error
//! when the run could not start. The agent is opened with the config the app
//! builds for a desktop host. Task logic stays in `vault_mcp` and
//! `vault_agent`.
//!
//! Credentials come from the environment, as for the app's agent (the Claude
//! Code login, `OPENCODE_API_KEY`, the Codex login).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use vault_agent::{Agent, AgentConfig, HostKind, PermissionMode, TestHooks, ToolPack};
use vault_core::Vault;
use vault_mcp::tasks;

/// How long `run-tasks` waits between polls when no task is queued.
const POLL: Duration = Duration::from_secs(5);

const USAGE: &str = "usage:
  scratch-cli run-tasks --vault <DIR> [--device <ID>] [--model <provider/model>] [--once]
  scratch-cli task create --vault <DIR> --prompt <TEXT> [--page <ID>] [--project <NAME-or-ID>]
  scratch-cli task show --vault <DIR> <ID>
  scratch-cli project add --vault <DIR> --name <NAME> --path <REPO>
  scratch-cli page create --vault <DIR> --title <TITLE>";

/// Makes the agent's test hooks for each open (scripted provider, extra
/// tools). Tests only.
#[doc(hidden)]
pub type Hooks<'a> = &'a dyn Fn() -> TestHooks;

/// Runs one command line (without the program name), writing its output to
/// `out`.
pub fn run(args: &[String], out: &mut dyn Write) -> Result<()> {
    run_with(args, out, None)
}

/// [`run`] with test hooks for the agent. Tests only.
#[doc(hidden)]
pub fn run_with(args: &[String], out: &mut dyn Write, hooks: Option<Hooks>) -> Result<()> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["run-tasks", rest @ ..] => run_tasks(&Flags::parse(rest)?, out, hooks),
        ["task", "create", rest @ ..] => task_create(&Flags::parse(rest)?, out, hooks),
        ["task", "show", rest @ ..] => task_show(&Flags::parse(rest)?, out),
        ["project", "add", rest @ ..] => project_add(&Flags::parse(rest)?, out, hooks),
        ["page", "create", rest @ ..] => page_create(&Flags::parse(rest)?, out),
        [] | ["help" | "--help" | "-h", ..] => {
            writeln!(out, "{USAGE}")?;
            Ok(())
        }
        _ => bail!("unknown command\n{USAGE}"),
    }
}

/// `--name value` pairs, the `--once` switch and positional words.
#[derive(Default)]
struct Flags {
    values: Vec<(String, String)>,
    once: bool,
    positional: Vec<String>,
}

impl Flags {
    fn parse(words: &[&str]) -> Result<Flags> {
        const VALUED: [&str; 9] = [
            "--vault", "--device", "--model", "--prompt", "--page", "--project", "--name",
            "--path", "--title",
        ];
        let mut flags = Flags::default();
        let mut words = words.iter();
        while let Some(word) = words.next() {
            if *word == "--once" {
                flags.once = true;
            } else if VALUED.contains(word) {
                let value = words
                    .next()
                    .ok_or_else(|| anyhow!("{word} needs a value"))?;
                flags.values.push((word.to_string(), value.to_string()));
            } else if word.starts_with("--") {
                bail!("unknown option {word}\n{USAGE}");
            } else {
                flags.positional.push(word.to_string());
            }
        }
        Ok(flags)
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.trim().is_empty())
    }

    fn need(&self, name: &str) -> Result<&str> {
        self.get(name).ok_or_else(|| anyhow!("{name} is required"))
    }

    fn vault(&self) -> Result<PathBuf> {
        let dir = PathBuf::from(self.need("--vault")?);
        std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        std::path::absolute(&dir).with_context(|| format!("resolve {}", dir.display()))
    }
}

/// Opens the vault at `dir`. A vault another process holds (the app) is
/// refused: this process would not be its only writer.
fn open_vault(dir: &Path) -> Result<Arc<Vault>> {
    let vault = Vault::open(dir.to_string_lossy().into_owned())
        .map_err(|e| anyhow!("open vault {}: {e}", dir.display()))?;
    if !vault.is_owner() {
        bail!(
            "the vault {} is open in another process: close Scratch or use another vault",
            dir.display()
        );
    }
    Ok(Arc::new(vault))
}

/// The app's data folder for a vault: the vault's parent folder, as the app
/// uses when its settings name no other (the default vault is
/// `<data dir>/vault`). The agent keeps its sessions and projects in
/// `<data dir>/agent/`.
fn data_dir(vault_dir: &Path) -> PathBuf {
    vault_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| vault_dir.to_path_buf())
}

/// The app's data folder, as `paths.dart` finds it: `NOTES_DATA_DIR`, else
/// `%LOCALAPPDATA%\Notes` on Windows, else `~/.local/share/notes_app`.
fn app_data_dir() -> Option<PathBuf> {
    let env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    if let Some(dir) = env("NOTES_DATA_DIR") {
        return Some(dir.into());
    }
    if cfg!(windows) {
        return env("LOCALAPPDATA").map(|base| PathBuf::from(base).join("Notes"));
    }
    env("HOME").map(|home| PathBuf::from(home).join(".local/share/notes_app"))
}

/// The model for runs that name none: `--model`, else the vault's default
/// for `this_device`, else empty (the agent's built-in desktop default).
fn default_model(vault: &Vault, flag: Option<&str>) -> String {
    if let Some(model) = flag {
        return model.trim().to_string();
    }
    match vault.agent_defaults() {
        Ok(defaults) => defaults
            .model_for(vault_core::AGENT_RUNNER_THIS_DEVICE)
            .map(str::trim)
            .unwrap_or_default()
            .to_string(),
        Err(error) => {
            eprintln!("agent defaults unreadable, using the built-in default: {error}");
            String::new()
        }
    }
}

/// Opens the agent with the config the app builds for a desktop host: the
/// desktop tools (Home project at the user's home folder on first use),
/// Bypass permissions, and the runner's tool set from the vault. The app's
/// MCP servers are not applied.
fn open_agent(
    vault: &Arc<Vault>,
    vault_dir: &Path,
    model: String,
    hooks: Option<Hooks>,
) -> Result<Agent> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("cannot find the user's home directory"))?;
    // Claude Code as the app finds it: NOTES_CLAUDE_CODE_DIR, else the
    // app's saved Bun and extension paths (beside the vault, then in the
    // app's data folder), else Bun on PATH and the bundled extension.
    let mut settings_dirs = vec![data_dir(vault_dir)];
    settings_dirs.extend(app_data_dir());
    let (bun_path, claude_code_dir) = vault_agent::claude::app_overrides(&settings_dirs);
    let config = AgentConfig {
        host_kind: HostKind::Desktop,
        packs: vec![ToolPack::Desktop { root: home }],
        data_dir: data_dir(vault_dir),
        default_model: model,
        bun_path,
        claude_code_dir,
        permission_mode: PermissionMode::Bypass,
        test: hooks.map(|make| make()).unwrap_or_default(),
        tool_set: None,
        mcp: Default::default(),
    };
    Agent::open(vault.clone(), config).context("open the agent")
}

fn mcp(error: impl std::fmt::Display) -> anyhow::Error {
    anyhow!("{error}")
}

fn run_tasks(flags: &Flags, out: &mut dyn Write, hooks: Option<Hooks>) -> Result<()> {
    let dir = flags.vault()?;
    let vault = open_vault(&dir)?;
    let device = vault.device_id();
    if let Some(asked) = flags.get("--device") {
        if asked != device {
            bail!("this vault copy runs as device {device}; --device {asked} does not match");
        }
    }
    let model = default_model(&vault, flags.get("--model"));
    let agent = open_agent(&vault, &dir, model, hooks)?;
    tasks::recover(&vault, &device).map_err(mcp)?;
    loop {
        let Some(task) = tasks::claim_next(&vault, &device).map_err(mcp)? else {
            if flags.once {
                return Ok(());
            }
            std::thread::sleep(POLL);
            continue;
        };
        match agent.run_task(&vault, &task.id) {
            Ok(run) if run.ok => writeln!(out, "{} done", task.id)?,
            Ok(run) => writeln!(out, "{} failed: {}", task.id, one_line(&run.text))?,
            Err(error) => {
                // The run did not start: fail the row, as the app does.
                let text = format!("{error:#}");
                tasks::finish(&vault, &task.id, Err(text.clone())).map_err(mcp)?;
                writeln!(out, "{} failed: {}", task.id, one_line(&text))?;
            }
        }
        out.flush()?;
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn task_create(flags: &Flags, out: &mut dyn Write, hooks: Option<Hooks>) -> Result<()> {
    let dir = flags.vault()?;
    let prompt = flags.need("--prompt")?;
    let vault = open_vault(&dir)?;
    let device = vault.device_id();
    let page = match flags.get("--page") {
        Some(page) => page.to_string(),
        // No page named: the task gets a page of its own.
        None => vault
            .create_page(None, page_title(prompt))
            .map_err(|e| anyhow!("create page: {e}"))?,
    };
    let project = match flags.get("--project") {
        Some(name) => {
            // Opening the agent publishes this device's projects, which
            // `create_for_project` checks the project against.
            let model = default_model(&vault, None);
            let agent = open_agent(&vault, &dir, model, hooks)?;
            let projects = agent.projects_list()?;
            let project = projects
                .iter()
                .find(|p| p.id == name)
                .or_else(|| projects.iter().find(|p| p.name == name))
                .ok_or_else(|| anyhow!("unknown project {name:?}"))?;
            Some(project.id.clone())
        }
        None => None,
    };
    let task = tasks::create_for_project(
        &vault,
        prompt,
        &page,
        None,
        vault_agent::tasks::RUNNER,
        &device,
        project.as_deref(),
    )
    .map_err(mcp)?;
    writeln!(out, "{}", task.id)?;
    Ok(())
}

/// A page title from a prompt: its first line, at most 60 characters.
fn page_title(prompt: &str) -> String {
    let line = prompt.lines().find(|l| !l.trim().is_empty()).unwrap_or("AI task");
    line.trim().chars().take(60).collect()
}

fn task_show(flags: &Flags, out: &mut dyn Write) -> Result<()> {
    let dir = flags.vault()?;
    let [id] = flags.positional.as_slice() else {
        bail!("task show needs one task id");
    };
    let vault = open_vault(&dir)?;
    let task = tasks::get(&vault, id).map_err(mcp)?;
    writeln!(out, "id: {}", task.id)?;
    writeln!(out, "status: {}", task.status)?;
    writeln!(out, "error: {}", one_line(&task.error))?;
    writeln!(out, "progress: {}", one_line(&task.progress))?;
    writeln!(out, "project: {}", task.project)?;
    writeln!(out, "worktree_branch: {}", task.worktree_branch)?;
    writeln!(out, "worktree_path: {}", task.worktree_path)?;
    if task.status == tasks::DONE {
        // The result is the row's body (see `tasks::finish`).
        let result = vault
            .page_markdown(task.id.clone())
            .map_err(|e| anyhow!("read result: {e}"))?;
        writeln!(out, "result:\n{result}")?;
    }
    Ok(())
}

fn project_add(flags: &Flags, out: &mut dyn Write, hooks: Option<Hooks>) -> Result<()> {
    let dir = flags.vault()?;
    let name = flags.need("--name")?;
    let path = flags.need("--path")?;
    let vault = open_vault(&dir)?;
    let model = default_model(&vault, None);
    let agent = open_agent(&vault, &dir, model, hooks)?;
    let project = agent.project_add(name, path)?;
    writeln!(out, "{}", project.id)?;
    Ok(())
}

fn page_create(flags: &Flags, out: &mut dyn Write) -> Result<()> {
    let dir = flags.vault()?;
    let title = flags.need("--title")?;
    let vault = open_vault(&dir)?;
    let id = vault
        .create_page(None, title.to_string())
        .map_err(|e| anyhow!("create page: {e}"))?;
    writeln!(out, "{id}")?;
    Ok(())
}
