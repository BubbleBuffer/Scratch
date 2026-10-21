//! `notes setup claude|codex` against an isolated, real HTTP hub. The desktop
//! app's sign-in is a fake `settings.json` and device id inside a temp home,
//! so the user's configuration, Claude or Codex install and real tokens are
//! never touched. Tokens and secrets here are synthetic test values.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use notes_hub::agent_tokens::{AgentScope, AgentTokens};
use notes_hub::{AgentSection, Config as HubConfig, Server};
use serde_json::json;
use vault_core::new_id;
use vault_tools::notes_cli::{Client, Config};

/// The host name the tests give the command, so token names are known.
const HOST: &str = "setup-host";
const DEVICE: &str = "setup-test-device";
const CODEX_EXPORT: &str = "export NOTES_CODEX_TOKEN='";

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("notes_setup_{}", new_id()));
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
    _server: Server,
    /// `http://127.0.0.1:<port>`, the hub address the app would store.
    url: String,
    /// A device token the hub accepts with `DEVICE`.
    device_token: String,
    root: TempDir,
}

impl Env {
    /// `existing` are token names already on the hub before the run.
    async fn new(existing: &[&str]) -> Env {
        let root = TempDir::new();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = HubConfig {
            listen: listener.local_addr().unwrap().to_string(),
            data_dir: root.0.join("hub"),
            trust_proxy: false,
            agent: AgentSection {
                enabled: false,
                ..AgentSection::default()
            },
            ..HubConfig::default()
        };
        let store = AgentTokens::open(config.vault_dir()).unwrap();
        for name in existing {
            store.add(name, AgentScope::Write).unwrap();
        }
        let server = notes_hub::start_on(listener, config).await.unwrap();
        let hub = server.hub().clone();
        let device_token = tokio::task::spawn_blocking(move || hub.pair("setup-test"))
            .await
            .unwrap()
            .unwrap();
        let url = format!("http://{}", server.addr());
        Env {
            _server: server,
            url,
            device_token,
            root,
        }
    }

    fn home(&self) -> PathBuf {
        self.root.0.join("home")
    }

    fn vault_dir(&self) -> PathBuf {
        self.root.0.join("hub").join("vault")
    }

    /// Names of every token on the hub, read from its token file.
    fn token_names(&self) -> Vec<String> {
        AgentTokens::open(self.vault_dir())
            .unwrap()
            .list()
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect()
    }

    /// The tool set of every token on the hub, in token order.
    fn token_scopes(&self) -> Vec<AgentScope> {
        AgentTokens::open(self.vault_dir())
            .unwrap()
            .list()
            .unwrap()
            .into_iter()
            .map(|t| t.scope)
            .collect()
    }

    /// Writes the fake desktop-app sign-in into the temp home.
    fn sign_in(&self, client_url: &str, token: &str) {
        let data = app_data(&self.home());
        let vault = data.join("vault");
        fs::create_dir_all(vault.join(".vault")).unwrap();
        fs::write(vault.join(".vault").join("device"), format!("{DEVICE}\n")).unwrap();
        let settings = json!({
            "clientUrl": client_url,
            "clientToken": token,
            "clientDeviceLabel": "setup-test",
        });
        fs::write(data.join("settings.json"), settings.to_string()).unwrap();
    }

    fn sign_in_here(&self) {
        self.sign_in(&self.url, &self.device_token);
    }
}

/// The app's data folder for a temp home, as the app resolves it here.
fn app_data(home: &Path) -> PathBuf {
    if cfg!(windows) {
        home.join("Notes")
    } else {
        home.join(".local").join("share").join("notes_app")
    }
}

/// Runs `notes` with `args` in a fresh temp home. Credentials and the Claude
/// and Codex locations are cleared, so only what a test sets is visible.
async fn setup(home: &Path, args: &[&str], extra: Vec<(&'static str, String)>) -> Output {
    let home = home.to_path_buf();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    tokio::task::spawn_blocking(move || {
        fs::create_dir_all(&home).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_notes"));
        command
            .current_dir(&home)
            .args(&args)
            .env_remove("NOTES_TOKEN")
            .env_remove("NOTES_HUB_URL")
            .env_remove("NOTES_DATA_DIR")
            .env_remove("NOTES_CLAUDE_BIN")
            .env_remove("CODEX_HOME")
            .env_remove("COMPUTERNAME")
            .env("HOSTNAME", HOST)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("APPDATA", &home)
            .env("LOCALAPPDATA", &home)
            .env("XDG_CONFIG_HOME", &home)
            .stdin(Stdio::null());
        for (key, value) in extra {
            command.env(key, value);
        }
        command.output().unwrap()
    })
    .await
    .unwrap()
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// The secret Codex setup prints once, read back from its export line.
fn codex_secret(stdout: &str) -> String {
    let line = stdout
        .lines()
        .find(|l| l.contains(CODEX_EXPORT))
        .expect("the export line with the token");
    let rest = &line[line.find(CODEX_EXPORT).unwrap() + CODEX_EXPORT.len()..];
    rest[..rest.find('\'').unwrap()].to_string()
}

/// The hub must accept `token` on `/mcp`. A refused token is an error.
async fn hub_accepts(url: &str, token: &str) -> bool {
    let config = Config {
        hub: url.to_string(),
        token: token.to_string(),
    };
    tokio::task::spawn_blocking(move || {
        Client::new(config)
            .and_then(|client| client.call("search_notes", &json!({"query": "nothing"})))
            .is_ok()
    })
    .await
    .unwrap()
}

const CODEX_SECTION_SUFFIX: &str = "bearer_token_env_var = \"NOTES_CODEX_TOKEN\"\n";

fn codex_section(url: &str) -> String {
    format!("[mcp_servers.notes]\nurl = \"{url}/mcp\"\n{CODEX_SECTION_SUFFIX}")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_setup_writes_the_table_and_registers_one_token() {
    let env = Env::new(&[]).await;
    env.sign_in_here();
    let out = setup(&env.home(), &["setup", "codex"], vec![]).await;
    let text = combined(&out);
    assert!(out.status.success(), "{text}");
    assert!(!text.contains(&env.device_token), "device token printed");

    let config_path = env.home().join(".codex").join("config.toml");
    let config = fs::read_to_string(&config_path).unwrap();
    assert_eq!(config, codex_section(&env.url));
    assert!(!config.contains("Bearer"), "secret written to config");

    assert_eq!(env.token_names(), vec![format!("codex-{HOST}")]);
    // Setup asks for the Write tool set (integration spec phase 5).
    assert_eq!(env.token_scopes(), vec![AgentScope::Write]);

    let secret = codex_secret(&String::from_utf8_lossy(&out.stdout));
    assert!(!secret.is_empty());
    assert!(!config.contains(&secret), "secret written to config");
    assert!(
        hub_accepts(&env.url, &secret).await,
        "printed token refused"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_rerun_is_a_noop() {
    let env = Env::new(&[]).await;
    env.sign_in_here();
    let first = setup(&env.home(), &["setup", "codex"], vec![]).await;
    assert!(first.status.success(), "{}", combined(&first));
    let config_path = env.home().join(".codex").join("config.toml");
    let before = fs::read(&config_path).unwrap();

    let second = setup(&env.home(), &["setup", "codex"], vec![]).await;
    let text = combined(&second);
    assert!(second.status.success(), "{text}");
    assert!(text.contains("already has the notes MCP server"), "{text}");
    assert!(!text.contains(&env.device_token), "device token printed");
    assert!(!text.contains(CODEX_EXPORT), "a second token was printed");
    assert_eq!(fs::read(&config_path).unwrap(), before, "config changed");
    assert_eq!(env.token_names(), vec![format!("codex-{HOST}")]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_keeps_existing_config_byte_for_byte() {
    let env = Env::new(&[]).await;
    env.sign_in_here();
    let codex_dir = env.home().join(".codex");
    fs::create_dir_all(&codex_dir).unwrap();
    // No trailing newline: the setup must add the separator itself.
    let original = "model = \"gpt-test\"\n\n[profiles.fast]\nmodel = \"gpt-test-mini\"";
    fs::write(codex_dir.join("config.toml"), original).unwrap();

    let out = setup(&env.home(), &["setup", "codex"], vec![]).await;
    assert!(out.status.success(), "{}", combined(&out));
    let config = fs::read_to_string(codex_dir.join("config.toml")).unwrap();
    assert_eq!(config, format!("{original}\n\n{}", codex_section(&env.url)));
    assert_eq!(env.token_names(), vec![format!("codex-{HOST}")]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_name_takes_the_next_suffix() {
    let existing = format!("codex-{HOST}");
    let env = Env::new(&[existing.as_str()]).await;
    env.sign_in_here();
    let out = setup(&env.home(), &["setup", "codex"], vec![]).await;
    assert!(out.status.success(), "{}", combined(&out));
    let mut names = env.token_names();
    names.sort();
    assert_eq!(
        names,
        vec![format!("codex-{HOST}"), format!("codex-{HOST}-2")]
    );
    assert!(combined(&out).contains(&format!("Token: codex-{HOST}-2")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn not_signed_in_fails_and_writes_nothing() {
    let env = Env::new(&[]).await;
    // No settings.json: the app was never signed in on this computer.
    let out = setup(&env.home(), &["setup", "codex"], vec![]).await;
    let text = combined(&out);
    assert!(!out.status.success());
    assert!(
        text.contains("sign in to a hub in the Notes desktop app"),
        "{text}"
    );
    assert!(!env.home().join(".codex").join("config.toml").exists());
    assert!(env.token_names().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plain_http_to_a_remote_host_is_refused() {
    let env = Env::new(&[]).await;
    env.sign_in("http://hub.example", &env.device_token);
    let out = setup(&env.home(), &["setup", "codex"], vec![]).await;
    let text = combined(&out);
    assert!(!out.status.success());
    assert!(text.contains("must use https"), "{text}");
    assert!(!env.home().join(".codex").join("config.toml").exists());
    assert!(env.token_names().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_claude_creates_no_token() {
    let env = Env::new(&[]).await;
    env.sign_in_here();
    let out = setup(
        &env.home(),
        &["setup", "claude"],
        vec![(
            "NOTES_CLAUDE_BIN",
            env.root.0.join("no-such-claude").display().to_string(),
        )],
    )
    .await;
    let text = combined(&out);
    assert!(!out.status.success());
    assert!(text.contains("Claude Code was not found"), "{text}");
    assert!(env.token_names().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn usage_errors_run_nothing() {
    let temp = TempDir::new();
    let home = temp.0.join("home");
    for args in [
        vec!["setup"],
        vec!["setup", "cursor"],
        vec!["setup", "codex", "--json"],
        vec!["setup", "codex", "extra"],
    ] {
        let out = setup(&home, &args, vec![]).await;
        let text = combined(&out);
        assert!(!out.status.success(), "{args:?} succeeded");
        assert!(text.contains("usage: notes setup claude|codex"), "{text}");
    }
    assert!(!home.join(".codex").exists());
}

#[cfg(unix)]
mod claude {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in `claude`. It logs each invocation as one line per argument
    /// followed by a `--` line. `mcp get` fails until `mcp add` has run.
    fn fake_claude(dir: &Path) -> PathBuf {
        let path = dir.join("fake-claude.sh");
        let script = r#"#!/bin/sh
{ printf '%s\n' "$@"; echo '--'; } >> "$FAKE_CLAUDE_LOG"
case "$1" in
  --version) exit 0 ;;
  mcp)
    case "$2" in
      get) if [ -f "$FAKE_CLAUDE_STATE" ]; then exit 0; else exit 1; fi ;;
      add) : > "$FAKE_CLAUDE_STATE"; exit 0 ;;
    esac ;;
esac
exit 2
"#;
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// The invocations logged so far, each as its argument list.
    fn invocations(log: &Path) -> Vec<Vec<String>> {
        let text = fs::read_to_string(log).unwrap_or_default();
        let mut calls = vec![Vec::new()];
        for line in text.lines() {
            if line == "--" {
                calls.push(Vec::new());
            } else {
                calls.last_mut().unwrap().push(line.to_string());
            }
        }
        calls.retain(|c| !c.is_empty());
        calls
    }

    fn claude_env(dir: &Path, bin: &Path) -> Vec<(&'static str, String)> {
        vec![
            ("NOTES_CLAUDE_BIN", bin.display().to_string()),
            ("FAKE_CLAUDE_LOG", dir.join("log.txt").display().to_string()),
            ("FAKE_CLAUDE_STATE", dir.join("state").display().to_string()),
        ]
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn claude_setup_adds_the_server_once() {
        let env = Env::new(&[]).await;
        env.sign_in_here();
        let fake_dir = env.root.0.join("fake");
        fs::create_dir_all(&fake_dir).unwrap();
        let bin = fake_claude(&fake_dir);
        let log = fake_dir.join("log.txt");

        let first = setup(
            &env.home(),
            &["setup", "claude"],
            claude_env(&fake_dir, &bin),
        )
        .await;
        let text = combined(&first);
        assert!(first.status.success(), "{text}");
        assert!(!text.contains(&env.device_token), "device token printed");

        let adds: Vec<Vec<String>> = invocations(&log)
            .into_iter()
            .filter(|c| {
                c.first().map(String::as_str) == Some("mcp")
                    && c.get(1).map(String::as_str) == Some("add")
            })
            .collect();
        assert_eq!(adds.len(), 1, "{adds:?}");
        let add = &adds[0];
        assert!(
            add.windows(2)
                .any(|w| w[0] == "--transport" && w[1] == "http"),
            "{add:?}"
        );
        assert!(
            add.windows(2).any(|w| w[0] == "--scope" && w[1] == "user"),
            "{add:?}"
        );
        let url = format!("{}/mcp", env.url);
        assert!(add.contains(&url), "{add:?}");
        let header = add
            .windows(2)
            .find(|w| w[0] == "--header")
            .map(|w| w[1].clone())
            .expect("an --header argument");
        let secret = header
            .strip_prefix("Authorization: Bearer ")
            .expect("bearer header")
            .to_string();
        assert!(!secret.is_empty());
        assert!(hub_accepts(&env.url, &secret).await, "added token refused");
        assert_eq!(env.token_names(), vec![format!("claude-{HOST}")]);

        let second = setup(
            &env.home(),
            &["setup", "claude"],
            claude_env(&fake_dir, &bin),
        )
        .await;
        let text = combined(&second);
        assert!(second.status.success(), "{text}");
        assert!(text.contains("already has a notes MCP server"), "{text}");
        assert!(!text.contains(&secret), "secret printed on re-run");
        let adds_after = invocations(&log)
            .into_iter()
            .filter(|c| {
                c.first().map(String::as_str) == Some("mcp")
                    && c.get(1).map(String::as_str) == Some("add")
            })
            .count();
        assert_eq!(adds_after, 1, "re-run added the server again");
        assert_eq!(env.token_names(), vec![format!("claude-{HOST}")]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn claude_not_signed_in_adds_nothing() {
        let env = Env::new(&[]).await;
        let fake_dir = env.root.0.join("fake");
        fs::create_dir_all(&fake_dir).unwrap();
        let bin = fake_claude(&fake_dir);
        let out = setup(
            &env.home(),
            &["setup", "claude"],
            claude_env(&fake_dir, &bin),
        )
        .await;
        let text = combined(&out);
        assert!(!out.status.success());
        assert!(
            text.contains("sign in to a hub in the Notes desktop app"),
            "{text}"
        );
        let adds = invocations(&fake_dir.join("log.txt"))
            .into_iter()
            .filter(|c| c.get(1).map(String::as_str) == Some("add"))
            .count();
        assert_eq!(adds, 0);
        assert!(env.token_names().is_empty());
    }
}
