//! `notes setup claude|codex`: connects Claude Code or Codex to this
//! computer's notes hub with a new agent token. The sign-in is the desktop
//! app's (its `settings.json` and device id), so nothing is typed or stored
//! by hand. Re-running changes nothing, and no token is ever revoked here.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Codex,
}

impl Agent {
    fn name(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
        }
    }
}

/// The environment variable Codex reads the agent token from.
pub const CODEX_TOKEN_VAR: &str = "NOTES_CODEX_TOKEN";

const MAX_NAME: usize = 40;
const MAX_ATTEMPTS: u32 = 9;
const NOT_SIGNED_IN: &str = "not signed in: sign in to a hub in the Notes desktop app on this computer first, then run this again. Nothing was changed.";

/// The desktop app's hub sign-in. Holds the device token, so it has no Debug.
struct Signin {
    /// Hub base address without a trailing slash.
    url: String,
    token: String,
    device: String,
}

pub fn setup(agent: Agent) -> Result<String> {
    match agent {
        Agent::Claude => setup_claude(),
        Agent::Codex => setup_codex(),
    }
}

// ---------------------------------------------------------------- Claude Code

fn setup_claude() -> Result<String> {
    let bin = claude_bin();
    // Everything that can fail without side effects runs before any token is
    // created: the binary, then whether the server already exists.
    let version = run_claude(&bin, &["--version"]).map_err(|_| {
        anyhow!("Claude Code was not found (looked for `{bin}`). Install Claude Code or set NOTES_CLAUDE_BIN, then run this again. No token was created.")
    })?;
    if !version.status.success() {
        bail!("`{bin} --version` failed. No token was created.");
    }
    let get = run_claude(&bin, &["mcp", "get", "notes"])
        .map_err(|_| anyhow!("cannot run `{bin} mcp get`. Nothing was changed."))?;
    if get.status.success() {
        return Ok(
            "Claude Code already has a notes MCP server, so nothing changed.\n\
             To set it up again: claude mcp remove notes -s user, then run notes setup claude.\n"
                .into(),
        );
    }

    let signin = read_signin()?;
    let host = host_name();
    let (name, secret) = create_token(&signin, Agent::Claude, &host)?;
    let mcp_url = format!("{}/mcp", signin.url);
    let header = format!("Authorization: Bearer {secret}");
    let add = run_claude(
        &bin,
        &[
            "mcp",
            "add",
            "--transport",
            "http",
            "--scope",
            "user",
            "notes",
            mcp_url.as_str(),
            "--header",
            header.as_str(),
        ],
    )
    .map_err(|_| {
        anyhow!("cannot run `{bin} mcp add`. The token {name} was created and has not been revoked; revoke it in Settings > AI > Agent tokens.")
    })?;
    if !add.status.success() {
        let detail = redact(&String::from_utf8_lossy(&add.stderr), &secret);
        let detail = detail.trim();
        let detail = if detail.is_empty() {
            "no details".to_string()
        } else {
            detail.chars().take(300).collect()
        };
        bail!("`claude mcp add` failed ({detail}). The token {name} was created and has not been revoked; revoke it in Settings > AI > Agent tokens.");
    }
    Ok(format!(
        "Connected Claude Code to notes: added the notes MCP server (user scope) at {mcp_url}.\n\
         Token: {name}. Revoke it in Settings > AI > Agent tokens if you stop using it.\n\
         Restart Claude Code if it is running.\n"
    ))
}

fn claude_bin() -> String {
    env_nonempty("NOTES_CLAUDE_BIN")
        .map(|v| v.to_string_lossy().into_owned())
        .unwrap_or_else(|| "claude".into())
}

fn run_claude(bin: &str, args: &[&str]) -> std::io::Result<Output> {
    Command::new(bin).args(args).stdin(Stdio::null()).output()
}

// ---------------------------------------------------------------------- Codex

fn setup_codex() -> Result<String> {
    let path = codex_config_path()?;
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => bail!("cannot read {}. Nothing was changed.", path.display()),
    };
    let doc: toml::Value = toml::from_str(&existing).map_err(|_| {
        anyhow!(
            "{} is not valid TOML. Fix it or move it aside, then run this again. Nothing was changed.",
            path.display()
        )
    })?;
    if has_notes_server(&doc) {
        return Ok(format!(
            "Codex already has the notes MCP server in {}, so nothing changed.\n\
             To set it up again: delete the [mcp_servers.notes] table from that file, then run notes setup codex.\n",
            path.display()
        ));
    }

    let signin = read_signin()?;
    let mcp_url = format!("{}/mcp", signin.url);
    // The file gets the address and the variable name only, never the secret.
    let section = format!(
        "[mcp_servers.notes]\nurl = {}\nbearer_token_env_var = {}\n",
        toml_string(&mcp_url),
        toml_string(CODEX_TOKEN_VAR)
    );
    let mut content = existing.clone();
    if !content.is_empty() {
        if !content.ends_with('\n') {
            content.push('\n');
        }
        content.push('\n');
    }
    content.push_str(&section);
    // Check the result before any token exists: the file must still parse and
    // the new table must hold the address.
    let verified: toml::Value = toml::from_str(&content).map_err(|_| {
        anyhow!(
            "could not add the notes server to {} without breaking the file (check for an inline mcp_servers value). Nothing was changed.",
            path.display()
        )
    })?;
    let url_ok = verified
        .get("mcp_servers")
        .and_then(|s| s.get("notes"))
        .and_then(|n| n.get("url"))
        .and_then(toml::Value::as_str)
        == Some(mcp_url.as_str());
    if !url_ok {
        bail!("could not verify the notes server table. Nothing was changed.");
    }

    let host = host_name();
    let (name, secret) = create_token(&signin, Agent::Codex, &host)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|_| {
            anyhow!("cannot create {}. The token {name} was created and has not been revoked; revoke it in Settings > AI > Agent tokens.", dir.display())
        })?;
    }
    std::fs::write(&path, content).map_err(|_| {
        anyhow!("cannot write {}. The token {name} was created and has not been revoked; revoke it in Settings > AI > Agent tokens.", path.display())
    })?;

    // The only place the secret is printed. It cannot be shown again.
    Ok(format!(
        "Connected Codex to notes: added [mcp_servers.notes] to {path}.\n\
         Token: {name}. Revoke it in Settings > AI > Agent tokens if you stop using it.\n\
         \n\
         Codex reads the token from the {var} environment variable, which must be set where Codex runs.\n\
         Windows (PowerShell, then open a new terminal):\n\
         \x20 setx {var} \"{secret}\"\n\
         Shell profile (bash or zsh):\n\
         \x20 export {var}='{secret}'\n\
         \n\
         This token is shown only once and cannot be shown again. The config file does not contain it.\n",
        path = path.display(),
        var = CODEX_TOKEN_VAR,
    ))
}

fn codex_config_path() -> Result<PathBuf> {
    let dir = match env_nonempty("CODEX_HOME") {
        Some(dir) => PathBuf::from(dir),
        None => home_dir()?.join(".codex"),
    };
    Ok(dir.join("config.toml"))
}

fn has_notes_server(doc: &toml::Value) -> bool {
    doc.get("mcp_servers")
        .and_then(|s| s.get("notes"))
        .is_some()
}

/// A TOML basic string. JSON string escapes are valid TOML escapes too.
fn toml_string(s: &str) -> String {
    serde_json::to_string(s).expect("a string always serializes")
}

// ------------------------------------------------------------ hub and tokens

/// Creates `<agent>-<host>` (or `-2` … `-9` on a duplicate name). Returns the
/// name and the secret, which the caller must not print except where stated.
fn create_token(signin: &Signin, agent: Agent, host: &str) -> Result<(String, String)> {
    for n in 1..=MAX_ATTEMPTS {
        let name = token_name(agent, host, n);
        let (status, body) = post_token(signin, &name)?;
        match status {
            200 | 201 => {
                let secret = body
                    .get("token")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| anyhow!("the notes hub returned no token. Nothing was changed."))?
                    .to_string();
                return Ok((name, secret));
            }
            409 => continue,
            400 => bail!("the notes hub rejected the request (HTTP 400). Nothing was changed."),
            401 | 403 => bail!("the notes hub refused this computer's sign-in. Sign in to the hub in the Notes desktop app on this computer again, then run this again. Nothing was changed."),
            503 => bail!("the notes hub could not store the new token. Nothing was changed."),
            other => bail!("the notes hub returned HTTP {other} for the new token. Nothing was changed."),
        }
    }
    bail!(
        "there are already {MAX_ATTEMPTS} tokens named {}. Revoke old ones in Settings > AI > Agent tokens, then run this again.",
        token_name(agent, host, 1)
    )
}

/// `POST <hub>/agent-tokens`, with the device token and `X-Device`. The
/// response body is returned as parsed JSON; errors never echo it.
fn post_token(signin: &Signin, name: &str) -> Result<(u16, Value)> {
    let agent = ureq::config::Config::builder()
        .http_status_as_error(false)
        // Never replay the device credential to a redirect destination.
        .max_redirects(0)
        .timeout_connect(Some(Duration::from_secs(5)))
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .new_agent();
    // The Write tool set: every vault tool (integration spec phase 5).
    let body = json!({"name": name, "scope": "write"}).to_string();
    let mut response = agent
        .post(&format!("{}/agent-tokens", signin.url))
        .header("Authorization", &format!("Bearer {}", signin.token))
        .header("X-Device", &signin.device)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .send(body.as_bytes())
        .map_err(|_| anyhow!("cannot reach the notes hub. Check that the Notes app is running and signed in. Nothing was changed."))?;
    let status = response.status().as_u16();
    let bytes = response
        .body_mut()
        .with_config()
        .limit(1 << 20)
        .read_to_vec()
        .map_err(|_| anyhow!("cannot read the notes hub response. Nothing was changed."))?;
    let value = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
    Ok((status, value))
}

/// `<agent>-<host>`: the host lowercased, any character outside `[a-z0-9-]`
/// turned into `-`, and the whole name cut to the hub's 40-character limit.
/// Suffix `n > 1` is appended after cutting the stem to make room.
fn token_name(agent: Agent, host: &str, n: u32) -> String {
    let host: String = host
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let host = host.trim_matches('-');
    let host = if host.is_empty() { "computer" } else { host };
    let base = truncate_name(&format!("{}-{host}", agent.name()), MAX_NAME);
    if n <= 1 {
        return base;
    }
    let suffix = format!("-{n}");
    let stem = truncate_name(&base, MAX_NAME - suffix.len());
    format!("{stem}{suffix}")
}

/// Cuts an ASCII name to `max` bytes and drops trailing hyphens.
fn truncate_name(s: &str, max: usize) -> String {
    let end = s.len().min(max);
    s[..end].trim_end_matches('-').to_string()
}

fn host_name() -> String {
    if cfg!(windows) {
        if let Some(name) = env_nonempty("COMPUTERNAME") {
            return name.to_string_lossy().into_owned();
        }
    }
    if let Some(name) = env_nonempty("HOSTNAME") {
        return name.to_string_lossy().into_owned();
    }
    if let Ok(text) = std::fs::read_to_string("/etc/hostname") {
        let name = text.trim();
        if !name.is_empty() {
            return name.to_string();
        }
    }
    Command::new("hostname")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "computer".into())
}

/// Reads the desktop app's hub sign-in and this computer's device id. Only
/// https, or http on 127.0.0.1 or localhost, is accepted.
fn read_signin() -> Result<Signin> {
    let data = data_dir()?;
    let settings_path = data.join("settings.json");
    let bytes = match std::fs::read(&settings_path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => bail!(NOT_SIGNED_IN),
        Err(_) => bail!(
            "cannot read the Notes app settings at {}. Nothing was changed.",
            settings_path.display()
        ),
    };
    let settings: Value = serde_json::from_slice(&bytes).map_err(|_| {
        anyhow!(
            "the Notes app settings at {} are not valid JSON. Nothing was changed.",
            settings_path.display()
        )
    })?;
    let field = |key: &str| -> Option<String> {
        settings
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let (Some(raw_url), Some(token)) = (field("clientUrl"), field("clientToken")) else {
        bail!(NOT_SIGNED_IN);
    };
    let vault = field("vaultPath")
        .map(PathBuf::from)
        .unwrap_or_else(|| data.join("vault"));
    let url = hub_base(&raw_url)?;
    let device = read_device(&vault)?;
    Ok(Signin { url, token, device })
}

fn hub_base(raw: &str) -> Result<String> {
    let url = url::Url::parse(raw).map_err(|_| {
        anyhow!("the hub address in the Notes app is not a valid URL. Nothing was changed.")
    })?;
    let local = matches!(url.host_str(), Some("127.0.0.1") | Some("localhost"));
    let scheme_ok = url.scheme() == "https" || (url.scheme() == "http" && local);
    if !scheme_ok
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("the hub address in the Notes app must use https (http is allowed only for 127.0.0.1 and localhost). Nothing was changed.");
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn read_device(vault: &Path) -> Result<String> {
    let device = std::fs::read_to_string(vault.join(".vault").join("device"))
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if device.is_empty() || !device.bytes().all(|b| b.is_ascii_graphic()) {
        bail!(
            "the Notes vault at {} has no device id. Open the Notes app once, then run this again. Nothing was changed.",
            vault.display()
        );
    }
    Ok(device)
}

// ------------------------------------------------------------- paths and env

/// The app's data folder, as `paths.dart` and `notes-mcp` find it:
/// `NOTES_DATA_DIR`, else `%LOCALAPPDATA%\Notes` on Windows, else
/// `~/.local/share/notes_app` (macOS included, as in `paths.dart`).
fn data_dir() -> Result<PathBuf> {
    if let Some(dir) = env_nonempty("NOTES_DATA_DIR") {
        return Ok(dir.into());
    }
    if cfg!(windows) {
        let base = env_nonempty("LOCALAPPDATA")
            .ok_or_else(|| anyhow!("cannot find %LOCALAPPDATA%. Nothing was changed."))?;
        return Ok(PathBuf::from(base).join("Notes"));
    }
    Ok(home_dir()?.join(".local").join("share").join("notes_app"))
}

fn home_dir() -> Result<PathBuf> {
    let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    env_nonempty(name)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("cannot find your home folder ({name} is not set)"))
}

fn env_nonempty(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name).filter(|v| !v.is_empty())
}

/// Removes every occurrence of `secret` from `text`.
fn redact(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        text.to_string()
    } else {
        text.replace(secret, "[redacted]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_names_follow_the_agent_host_rule() {
        assert_eq!(token_name(Agent::Claude, "MyBox", 1), "claude-mybox");
        assert_eq!(
            token_name(Agent::Codex, "my box.local", 1),
            "codex-my-box-local"
        );
        assert_eq!(token_name(Agent::Codex, "", 1), "codex-computer");
        assert_eq!(token_name(Agent::Codex, "--odd--", 1), "codex-odd");
        assert_eq!(token_name(Agent::Claude, "mybox", 2), "claude-mybox-2");
        assert_eq!(token_name(Agent::Claude, "mybox", 9), "claude-mybox-9");
    }

    #[test]
    fn token_names_fit_the_hub_limit() {
        let long = "a".repeat(80);
        let first = token_name(Agent::Codex, &long, 1);
        assert_eq!(first.len(), MAX_NAME);
        let second = token_name(Agent::Codex, &long, 2);
        assert!(second.len() <= MAX_NAME);
        assert!(second.ends_with("-2"));
        assert!(second
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'));
    }

    #[test]
    fn hub_address_rules() {
        assert_eq!(
            hub_base("https://hub.example/").unwrap(),
            "https://hub.example"
        );
        assert_eq!(
            hub_base("http://127.0.0.1:4100").unwrap(),
            "http://127.0.0.1:4100"
        );
        assert!(hub_base("http://hub.example").is_err());
        assert!(hub_base("https://user:pw@hub.example").is_err());
        assert!(hub_base("https://hub.example/?x=1").is_err());
    }
}
