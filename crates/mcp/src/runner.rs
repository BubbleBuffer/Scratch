//! Runs a command-line agent (Claude Code, Codex, or any command) for an
//! AI task or a merge proposal. Desktop only.
//!
//! - The command is a program and an argument list; nothing goes through a
//!   shell. Page content is never put on the command line: the prompt is
//!   written to the agent's stdin.
//! - Arguments may contain placeholders, replaced inside that argument
//!   only: `{mcp_config}` (a JSON file naming the vault's MCP endpoint, for
//!   `claude --mcp-config`), `{mcp_url}` (the endpoint URL) and
//!   `{output_file}` (a file the agent may write its final answer to; when
//!   it exists after the run it is the answer instead of stdout).
//! - The environment gets `NOTES_MCP_URL`, `NOTES_MCP_TOKEN` and
//!   `NOTES_MCP_CONFIG`. The config file holds the token; it is created in
//!   the work folder and deleted after the run.
//! - On Windows the child gets no console window.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{err, McpError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// The built-in commands: `claude` (Claude Code in print mode) and `codex`
/// (`codex exec`). `program` overrides the executable (a full path).
pub fn default_command(kind: &str, program: Option<&str>) -> Option<AgentCommand> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let (prog, args) = match kind {
        "claude" => (
            "claude",
            s(&[
                "-p",
                "--mcp-config",
                "{mcp_config}",
                "--strict-mcp-config",
                "--allowedTools",
                "mcp__notes",
                "--output-format",
                "text",
            ]),
        ),
        "codex" => (
            "codex",
            s(&[
                "exec",
                "--skip-git-repo-check",
                "--color",
                "never",
                "-c",
                "mcp_servers.notes.url=\"{mcp_url}\"",
                "-c",
                "mcp_servers.notes.bearer_token_env_var=\"NOTES_MCP_TOKEN\"",
                "--output-last-message",
                "{output_file}",
                "-",
            ]),
        ),
        _ => return None,
    };
    Some(AgentCommand {
        program: program
            .filter(|p| !p.trim().is_empty())
            .unwrap_or(prog)
            .to_string(),
        args,
    })
}

/// The model argument for `kind` from a synced default model
/// (`provider/model`, as the agent stores it): `codex/<m>` gives Codex
/// `-m <m>`, `claude-code/<m>` gives Claude Code `--model <m>`. A model of
/// another provider, or none, adds nothing (the program's own default).
pub fn model_args(kind: &str, model: Option<&str>) -> Vec<String> {
    let model = model.map(str::trim).unwrap_or_default();
    let (flag, prefix) = match kind {
        "codex" => ("-m", "codex/"),
        "claude" => ("--model", "claude-code/"),
        _ => return Vec::new(),
    };
    match model.strip_prefix(prefix).map(str::trim) {
        Some(m) if !m.is_empty() => vec![flag.to_string(), m.to_string()],
        _ => Vec::new(),
    }
}

/// [`default_command`] with the synced default `model` ([`model_args`]),
/// put before the arguments that end the command line.
pub fn default_command_with_model(
    kind: &str,
    program: Option<&str>,
    model: Option<&str>,
) -> Option<AgentCommand> {
    let mut cmd = default_command(kind, program)?;
    let extra = model_args(kind, model);
    if !extra.is_empty() {
        // Codex's prompt comes from stdin as the last argument `-`; Claude's
        // flags may go anywhere after `-p`.
        let at = match kind {
            "codex" => cmd.args.len().saturating_sub(1),
            _ => cmd.args.len(),
        };
        cmd.args.splice(at..at, extra);
    }
    Some(cmd)
}

#[derive(Debug, Clone)]
pub struct RunContext {
    /// The vault's MCP endpoint (`http://127.0.0.1:47101/mcp`) and token;
    /// empty when there is none (the agent then has no vault tools).
    pub mcp_url: String,
    pub mcp_token: String,
    /// The agent's working folder (created if missing).
    pub work_dir: PathBuf,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunOutput {
    /// None when killed (timeout) or ended by a signal.
    pub code: Option<i32>,
    /// The answer: `{output_file}` if the agent wrote it, else stdout.
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

impl RunOutput {
    pub fn ok(&self) -> bool {
        self.code == Some(0) && !self.timed_out
    }

    /// The answer, or why there is none.
    pub fn result(&self) -> std::result::Result<String, String> {
        if self.timed_out {
            return Err("the agent took too long and was stopped".into());
        }
        if self.ok() {
            return Ok(self.stdout.trim().to_string());
        }
        let tail: String = {
            let e = self.stderr.trim();
            let e = if e.is_empty() { self.stdout.trim() } else { e };
            let lines: Vec<&str> = e.lines().rev().take(6).collect();
            lines.into_iter().rev().collect::<Vec<_>>().join("\n")
        };
        Err(format!(
            "the agent exited with {}{}",
            self.code
                .map(|c| format!("code {c}"))
                .unwrap_or_else(|| "a signal".into()),
            if tail.is_empty() {
                String::new()
            } else {
                format!(": {tail}")
            }
        ))
    }
}

/// Finds `program` on PATH (with Windows' executable extensions) unless it
/// is a path.
pub fn resolve_program(program: &str) -> Option<PathBuf> {
    let p = Path::new(program);
    if p.components().count() > 1 || p.is_absolute() {
        return p.is_file().then(|| p.to_path_buf());
    }
    let exts: Vec<String> = if cfg!(windows) {
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into());
        std::iter::once(String::new())
            .chain(
                pathext
                    .split(';')
                    .filter(|e| !e.is_empty())
                    .map(|e| e.to_ascii_lowercase()),
            )
            .collect()
    } else {
        vec![String::new()]
    };
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for e in &exts {
            let c = dir.join(format!("{program}{e}"));
            if c.is_file() {
                return Some(c);
            }
        }
    }
    None
}

fn read_all(mut r: impl Read + Send + 'static) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = r.read_to_end(&mut b);
        String::from_utf8_lossy(&b).into_owned()
    })
}

/// Runs `cmd` with `prompt` on stdin; see the module docs.
pub fn run(cmd: &AgentCommand, prompt: &str, ctx: &RunContext) -> Result<RunOutput> {
    let program = resolve_program(&cmd.program).ok_or_else(|| {
        McpError(format!(
            "{} was not found (set its path in Settings > AI)",
            cmd.program
        ))
    })?;
    std::fs::create_dir_all(&ctx.work_dir)?;
    let tag = crate::random_hex()?[..16].to_string();
    let config = ctx.work_dir.join(format!("mcp-{tag}.json"));
    let output = ctx.work_dir.join(format!("answer-{tag}.md"));
    let has_mcp = !ctx.mcp_url.is_empty();
    if has_mcp {
        let cfg = json!({ "mcpServers": { "notes": {
            "type": "http", "url": ctx.mcp_url,
            "headers": { "Authorization": format!("Bearer {}", ctx.mcp_token) } } } });
        std::fs::write(&config, cfg.to_string())?;
    } else {
        std::fs::write(&config, json!({ "mcpServers": {} }).to_string())?;
    }
    let cleanup = || {
        let _ = std::fs::remove_file(&config);
        let _ = std::fs::remove_file(&output);
    };
    let args: Vec<String> = cmd
        .args
        .iter()
        .map(|a| {
            a.replace("{mcp_config}", &config.to_string_lossy())
                .replace("{mcp_url}", &ctx.mcp_url)
                .replace("{output_file}", &output.to_string_lossy())
        })
        .collect();
    let mut c = Command::new(&program);
    c.args(&args)
        .current_dir(&ctx.work_dir)
        .env("NOTES_MCP_URL", &ctx.mcp_url)
        .env("NOTES_MCP_TOKEN", &ctx.mcp_token)
        .env("NOTES_MCP_CONFIG", &config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = match c.spawn() {
        Ok(ch) => ch,
        Err(e) => {
            cleanup();
            return err(format!("could not start {}: {e}", program.display()));
        }
    };
    let out = read_all(child.stdout.take().expect("piped"));
    let errs = read_all(child.stderr.take().expect("piped"));
    {
        let mut stdin = child.stdin.take().expect("piped");
        let p = prompt.to_string();
        // A separate thread: a child that does not read its stdin must not
        // block us.
        std::thread::spawn(move || {
            let _ = stdin.write_all(p.as_bytes());
        });
    }
    let start = Instant::now();
    let (code, timed_out) = loop {
        match child.try_wait()? {
            Some(s) => break (s.code(), false),
            None if start.elapsed() > ctx.timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break (None, true);
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    let mut stdout = out.join().unwrap_or_default();
    let stderr = errs.join().unwrap_or_default();
    // With {output_file} the file is the answer (an empty one too) and
    // stdout is progress; if the file was never written, stdout.
    if let Ok(answer) = std::fs::read_to_string(&output) {
        stdout = answer;
    }
    cleanup();
    Ok(RunOutput {
        code,
        stdout,
        stderr,
        timed_out,
    })
}
