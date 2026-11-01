//! `notes-hub` configuration: a TOML file (`--config`), each key overridable
//! by an environment variable `NOTES_HUB_<KEY>` (`NOTES_HUB_OIDC_<KEY>` for
//! the `[oidc]` table, `NOTES_HUB_MCP_OAUTH_<KEY>` for `[mcp_oauth]`,
//! `NOTES_HUB_AGENT_<KEY>` for `[agent]`, and
//! `NOTES_HUB_SCHEDULES_<KEY>` for `[schedules]`). List values in variables are separated by commas
//! or spaces. Reference: `wiki/sync/hub.md`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Address to listen on (plain HTTP; Caddy terminates TLS).
    pub listen: String,
    /// Holds the vault replica (`<data_dir>/vault`).
    pub data_dir: PathBuf,
    /// The URL clients and the browser reach the hub at, for example
    /// `https://notes.example.com`. The OIDC redirect URI is
    /// `<public_url>/auth/callback`.
    pub public_url: String,
    /// The Android package that opens this hub's `/open/` links as an App
    /// Link (`de.example.app`). Set with `android_sha256_certs`; empty serves
    /// no `/.well-known/assetlinks.json`.
    pub android_package: String,
    /// SHA-256 fingerprints of the package's signing certificates, as
    /// `AA:BB:...` (32 bytes each, either case). Several are allowed, for key
    /// rotation.
    pub android_sha256_certs: Vec<String>,
    /// Whether to trust `X-Forwarded-For` for the client address in logs.
    /// True by default, since the hub runs behind Caddy; set it to false
    /// when the hub is reachable directly.
    pub trust_proxy: bool,
    /// Refuse device tokens that are not bound to a device key yet
    /// (`wiki/sync/device-keys.md`). Off by default: an unbound
    /// token works, and its client upgrades it on the next sync.
    pub require_device_keys: bool,
    /// Maximum tar archive bytes per markdown source, before base64.
    pub import_source_limit_bytes: usize,
    pub oidc: OidcConfig,
    pub mcp_oauth: McpOAuthConfig,
    /// Opt-in BubblePaw MCP gateway. Requires the existing MCP OAuth config.
    pub mcp_agent_enabled: bool,
    /// Deprecated and ignored since 2026-10-23: `/mcp/agent` without
    /// `?host=` is any-computer mode, with no default host. [`Config::load`]
    /// prints a deprecation line when it is set to anything but `hub`.
    pub mcp_agent_default_host: String,
    pub agent: AgentSection,
    pub schedules: SchedulesSection,
    /// The jobs API's limits; fixed (not read from the file or environment),
    /// tests shorten them.
    #[doc(hidden)]
    #[serde(skip)]
    pub job_limits: crate::jobs::JobLimits,
}

/// `[schedules]`: provider-independent automation on this hub.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SchedulesSection {
    pub runner: bool,
}

impl Default for SchedulesSection {
    fn default() -> Self {
        Self { runner: true }
    }
}

/// `[agent]`: the agent the hub runs over its vault replica, served at
/// `GET /agent/ws`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentSection {
    /// Run the agent (default true).
    pub enabled: bool,
    /// The hub advertises native tools by default; desktop agents retain JS.
    pub execution_mode: vault_agent::bubblepaw_sdk::harness::ExecutionMode,
    /// The models the hub offers, `provider/model`, in the order shown.
    pub models: Vec<String>,
    /// `provider/model`, one of `models`: the model for runs that name none.
    /// [`Config::validate`] refuses a default that is not listed.
    pub default_model: String,
    /// The `bun` executable for the Claude Code extension (empty: none).
    pub bun: String,
    /// The Claude Code extension folder, holding `src/main.ts` (empty: none).
    pub claude_code_extension: String,
    /// Set when a config still has the removed `approvals` key (or
    /// `NOTES_HUB_AGENT_APPROVALS`). It is ignored: hub agent runs use
    /// Bypass. [`Config::load`] prints a deprecation line.
    #[doc(hidden)]
    #[serde(rename = "approvals", deserialize_with = "present")]
    pub legacy_approvals: bool,
    /// Set when a config still has the removed `enable_js` key (or
    /// `NOTES_HUB_AGENT_ENABLE_JS`). It is ignored: JavaScript is offered by
    /// `execution_mode = "js"`. [`Config::load`] prints a deprecation line.
    #[doc(hidden)]
    #[serde(rename = "enable_js", deserialize_with = "present")]
    pub legacy_enable_js: bool,
}

/// Accepts any value: the key is there.
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    serde::de::IgnoredAny::deserialize(d).map(|_| true)
}

const LEGACY_APPROVALS: &str = "notes-hub: [agent] approvals and NOTES_HUB_AGENT_APPROVALS \
     are deprecated and ignored (hub agent runs use Bypass); remove them.";

const LEGACY_ENABLE_JS: &str = "notes-hub: [agent] enable_js and NOTES_HUB_AGENT_ENABLE_JS \
     are deprecated and ignored; set execution_mode = \"js\" for JavaScript, or remove them.";

const LEGACY_DEFAULT_HOST: &str = "notes-hub: mcp_agent_default_host and      NOTES_HUB_MCP_AGENT_DEFAULT_HOST are deprecated and ignored (/mcp/agent names the      computer in each call; /mcp/agent?host=<name> pins one); remove them.";

/// The built-in models: three Claude Code models and DeepSeek through OpenCode.
const DEFAULT_MODELS: [&str; 4] = [
    "claude-code/claude-opus-5-5",
    "claude-code/claude-sonnet-5-5",
    "claude-code/claude-haiku-5-5",
    "opencode/deepseek-v4.1-flash",
];
const DEFAULT_MODEL: &str = "claude-code/claude-sonnet-5-5";

impl Default for AgentSection {
    fn default() -> AgentSection {
        AgentSection {
            enabled: true,
            execution_mode: vault_agent::bubblepaw_sdk::harness::ExecutionMode::Native,
            models: DEFAULT_MODELS.map(String::from).into(),
            default_model: DEFAULT_MODEL.into(),
            bun: String::new(),
            claude_code_extension: String::new(),
            legacy_approvals: false,
            legacy_enable_js: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OidcConfig {
    /// The issuer URL, for example
    /// `https://auth.example.com/application/o/notes/`. Empty: no login.
    pub issuer: String,
    pub client_id: String,
    /// The client secret, optional: leave it empty (or unset) for a public
    /// client, which signs in with PKCE only. Environment:
    /// `NOTES_HUB_OIDC_CLIENT_SECRET` (empty clears it).
    pub client_secret: String,
    /// Users allowed to sign in, by email or `sub`.
    pub allowed_users: Vec<String>,
    /// Groups (the ID token's `groups` claim) whose members may sign in.
    pub allowed_groups: Vec<String>,
    /// Scopes to ask for. Default: `openid email profile`, plus `groups`
    /// if `allowed_groups` is set.
    pub scopes: Option<Vec<String>>,
}

/// `[mcp_oauth]`: the public OAuth client whose access tokens `/mcp` accepts.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct McpOAuthConfig {
    pub issuer: String,
    pub client_id: String,
}

impl McpOAuthConfig {
    pub fn enabled(&self) -> bool {
        !self.issuer.is_empty() && !self.client_id.is_empty()
    }
}

impl Default for Config {
    fn default() -> Config {
        Config {
            listen: "0.0.0.0:8080".into(),
            data_dir: PathBuf::new(),
            public_url: String::new(),
            android_package: String::new(),
            android_sha256_certs: Vec::new(),
            trust_proxy: true,
            require_device_keys: false,
            import_source_limit_bytes: 64 << 20,
            oidc: OidcConfig::default(),
            mcp_oauth: McpOAuthConfig::default(),
            mcp_agent_enabled: false,
            mcp_agent_default_host: "hub".into(),
            agent: AgentSection::default(),
            schedules: SchedulesSection::default(),
            job_limits: crate::jobs::JobLimits::default(),
        }
    }
}

impl OidcConfig {
    /// The scopes sent to the issuer.
    pub fn scopes(&self) -> Vec<String> {
        match &self.scopes {
            Some(s) => s.clone(),
            None => {
                let mut s: Vec<String> = ["openid", "email", "profile"].map(String::from).into();
                if !self.allowed_groups.is_empty() {
                    s.push("groups".into());
                }
                s
            }
        }
    }

    pub fn enabled(&self) -> bool {
        !self.issuer.is_empty()
    }
}

fn list(v: &str) -> Vec<String> {
    v.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

fn boolean(key: &str, v: &str) -> Result<bool, String> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(format!("{key}: not a boolean: {v:?}")),
    }
}

/// `de.example.app`: at least two dot-separated segments, each starting with
/// a letter and then letters, digits or underscores.
fn valid_android_package(package: &str) -> bool {
    let parts: Vec<&str> = package.split('.').collect();
    parts.len() >= 2
        && parts.iter().all(|part| {
            let mut chars = part.chars();
            matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

/// The uppercase `AA:BB:...` form of a SHA-256 fingerprint (32 hex bytes,
/// either case, colon-separated), or None.
fn sha256_fingerprint(text: &str) -> Option<String> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    let well_formed = parts.len() == 32
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()));
    well_formed.then(|| parts.join(":").to_ascii_uppercase())
}

impl Config {
    /// Reads `path` (if given), then applies the process environment.
    pub fn load(path: Option<&Path>) -> Result<Config, String> {
        let mut c = match path {
            Some(p) => {
                let text =
                    std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
                Config::from_toml(&text).map_err(|e| format!("{}: {e}", p.display()))?
            }
            None => Config::default(),
        };
        c.apply_env(|k| std::env::var(k).ok())?;
        c.validate()?;
        if c.agent.legacy_approvals {
            eprintln!("{LEGACY_APPROVALS}");
        }
        if c.agent.legacy_enable_js {
            eprintln!("{LEGACY_ENABLE_JS}");
        }
        if c.mcp_agent_default_host != "hub" {
            eprintln!("{LEGACY_DEFAULT_HOST}");
        }
        Ok(c)
    }

    pub fn from_toml(text: &str) -> Result<Config, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// Applies `NOTES_HUB_*` overrides read through `get`.
    pub fn apply_env(&mut self, get: impl Fn(&str) -> Option<String>) -> Result<(), String> {
        let var = |k: &str| get(&format!("NOTES_HUB_{k}"));
        if let Some(v) = var("SCHEDULES_RUNNER") {
            self.schedules.runner = boolean("NOTES_HUB_SCHEDULES_RUNNER", &v)?;
        }
        if let Some(v) = var("LISTEN") {
            self.listen = v;
        }
        if let Some(v) = var("DATA_DIR") {
            self.data_dir = v.into();
        }
        if let Some(v) = var("PUBLIC_URL") {
            self.public_url = v;
        }
        if let Some(v) = var("ANDROID_PACKAGE") {
            self.android_package = v;
        }
        if let Some(v) = var("ANDROID_SHA256_CERTS") {
            self.android_sha256_certs = list(&v);
        }
        if let Some(v) = var("TRUST_PROXY") {
            self.trust_proxy = boolean("NOTES_HUB_TRUST_PROXY", &v)?;
        }
        if let Some(v) = var("REQUIRE_DEVICE_KEYS") {
            self.require_device_keys = boolean("NOTES_HUB_REQUIRE_DEVICE_KEYS", &v)?;
        }
        if let Some(v) = var("IMPORT_SOURCE_LIMIT_BYTES") {
            self.import_source_limit_bytes = v.parse().map_err(|_| {
                "NOTES_HUB_IMPORT_SOURCE_LIMIT_BYTES must be a positive integer".to_string()
            })?;
        }
        let o = &mut self.oidc;
        if let Some(v) = var("OIDC_ISSUER") {
            o.issuer = v;
        }
        if let Some(v) = var("OIDC_CLIENT_ID") {
            o.client_id = v;
        }
        if let Some(v) = var("OIDC_CLIENT_SECRET") {
            o.client_secret = v;
        }
        if let Some(v) = var("OIDC_ALLOWED_USERS") {
            o.allowed_users = list(&v);
        }
        if let Some(v) = var("OIDC_ALLOWED_GROUPS") {
            o.allowed_groups = list(&v);
        }
        if let Some(v) = var("OIDC_SCOPES") {
            o.scopes = Some(list(&v));
        }
        if let Some(v) = var("MCP_OAUTH_ISSUER") {
            self.mcp_oauth.issuer = v;
        }
        if let Some(v) = var("MCP_OAUTH_CLIENT_ID") {
            self.mcp_oauth.client_id = v;
        }
        if let Some(v) = var("MCP_AGENT_ENABLED") {
            self.mcp_agent_enabled = boolean("NOTES_HUB_MCP_AGENT_ENABLED", &v)?;
        }
        if let Some(v) = var("MCP_AGENT_DEFAULT_HOST") {
            self.mcp_agent_default_host = v;
        }
        let a = &mut self.agent;
        if let Some(v) = var("AGENT_ENABLED") {
            a.enabled = boolean("NOTES_HUB_AGENT_ENABLED", &v)?;
        }
        if let Some(v) = var("AGENT_EXECUTION_MODE") {
            a.execution_mode = match v.trim().to_ascii_lowercase().as_str() {
                "native" => vault_agent::bubblepaw_sdk::harness::ExecutionMode::Native,
                "js" => vault_agent::bubblepaw_sdk::harness::ExecutionMode::Js,
                _ => {
                    return Err(format!(
                        "NOTES_HUB_AGENT_EXECUTION_MODE: native or js, not {v:?}"
                    ))
                }
            };
        }
        if var("AGENT_ENABLE_JS").is_some() {
            a.legacy_enable_js = true;
        }
        if let Some(v) = var("AGENT_MODELS") {
            a.models = list(&v);
        }
        if let Some(v) = var("AGENT_DEFAULT_MODEL") {
            a.default_model = v;
        }
        if let Some(v) = var("AGENT_BUN") {
            a.bun = v;
        }
        if let Some(v) = var("AGENT_CLAUDE_CODE_EXTENSION") {
            a.claude_code_extension = v;
        }
        if var("AGENT_APPROVALS").is_some() {
            a.legacy_approvals = true;
        }
        Ok(())
    }

    /// Checks the keys a server needs.
    pub fn validate(&self) -> Result<(), String> {
        if self.import_source_limit_bytes == 0
            || self
                .import_source_limit_bytes
                .checked_add(2)
                .and_then(|n| (n / 3).checked_mul(24))
                .is_none()
        {
            return Err(
                "import_source_limit_bytes must be a positive, representable byte limit".into(),
            );
        }
        if self.data_dir.as_os_str().is_empty() {
            return Err("data_dir is not set".into());
        }
        if self.mcp_oauth.issuer.is_empty() != self.mcp_oauth.client_id.is_empty() {
            return Err("mcp_oauth.issuer and mcp_oauth.client_id are required together".into());
        }
        if self.mcp_agent_enabled && !self.mcp_oauth.enabled() {
            return Err(
                "mcp_agent_enabled requires mcp_oauth.issuer and mcp_oauth.client_id".into(),
            );
        }
        if self.oidc.enabled() || self.mcp_oauth.enabled() {
            let u = url::Url::parse(&self.public_url)
                .map_err(|e| format!("public_url {:?}: {e}", self.public_url))?;
            if u.scheme() != "https" && u.scheme() != "http" {
                return Err("public_url must be an http(s) URL".into());
            }
        }
        if self.oidc.enabled() && self.oidc.client_id.is_empty() {
            return Err("oidc.client_id is required".into());
        }
        if !self.agent.models.contains(&self.agent.default_model) {
            return Err(format!(
                "agent.default_model {:?} is not one of agent.models {:?}",
                self.agent.default_model, self.agent.models
            ));
        }
        if self.android_package.is_empty() != self.android_sha256_certs.is_empty() {
            return Err("android_package and android_sha256_certs are required together".into());
        }
        if !self.android_package.is_empty() {
            if !valid_android_package(&self.android_package) {
                return Err(format!(
                    "android_package {:?} is not a package name like de.example.app",
                    self.android_package
                ));
            }
            for cert in &self.android_sha256_certs {
                if sha256_fingerprint(cert).is_none() {
                    return Err(format!(
                        "android_sha256_certs: {cert:?} is not a SHA-256 fingerprint (AA:BB:...)"
                    ));
                }
            }
        }
        Ok(())
    }

    /// The Digital Asset Links statement for `android_package` and its
    /// certificates, served at `/.well-known/assetlinks.json`; None when no
    /// Android app is configured.
    pub fn assetlinks_json(&self) -> Option<String> {
        if self.android_package.is_empty() {
            return None;
        }
        let certs: Vec<String> = self
            .android_sha256_certs
            .iter()
            .filter_map(|c| sha256_fingerprint(c))
            .collect();
        Some(
            serde_json::json!([{
                "relation": ["delegate_permission/common.handle_all_urls"],
                "target": {
                    "namespace": "android_app",
                    "package_name": self.android_package,
                    "sha256_cert_fingerprints": certs,
                }
            }])
            .to_string(),
        )
    }

    /// `<data_dir>/vault`.
    pub fn vault_dir(&self) -> PathBuf {
        self.data_dir.join("vault")
    }

    /// The agent's store and workspace live in `<data_dir>/agent/`.
    pub fn agent_dir(&self) -> PathBuf {
        self.data_dir.join("agent")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_agent_opt_in_and_default_host() {
        let mut config = Config::from_toml("data_dir = '/tmp/test'").unwrap();
        assert!(!config.mcp_agent_enabled);
        assert_eq!(config.mcp_agent_default_host, "hub");
        config
            .apply_env(|key| match key {
                "NOTES_HUB_MCP_AGENT_ENABLED" => Some("true".into()),
                "NOTES_HUB_MCP_AGENT_DEFAULT_HOST" => Some("desktop".into()),
                _ => None,
            })
            .unwrap();
        assert!(config.mcp_agent_enabled);
        assert_eq!(config.mcp_agent_default_host, "desktop");
        assert!(config
            .validate()
            .unwrap_err()
            .contains("requires mcp_oauth"));
        config.public_url = "https://notes.example.com".into();
        config.mcp_oauth = McpOAuthConfig {
            issuer: "https://auth.example.com".into(),
            client_id: "notes-mcp".into(),
        };
        config.validate().unwrap();
        assert!(config
            .apply_env(|key| (key == "NOTES_HUB_MCP_AGENT_ENABLED").then(|| "invalid".into()))
            .is_err());
        let parsed =
            Config::from_toml("mcp_agent_enabled = true\nmcp_agent_default_host = 'hub'").unwrap();
        assert!(parsed.mcp_agent_enabled);
    }

    #[test]
    fn mcp_oauth_defaults_toml_environment_and_validation() {
        let mut config = Config::from_toml("data_dir = '/tmp/test'").unwrap();
        assert_eq!(config.mcp_oauth, McpOAuthConfig::default());
        assert!(!config.mcp_oauth.enabled());
        config.validate().unwrap();
        assert!(Config::from_toml("[mcp_oauth]\nclient_secret = 'no'").is_err());
        config = Config::from_toml(
            "data_dir = '/tmp/test'\npublic_url = 'https://notes.example.com/'\n\
             [mcp_oauth]\nissuer = 'https://issuer.example/'\nclient_id = 'mcp'",
        )
        .unwrap();
        assert!(config.mcp_oauth.enabled());
        config.validate().unwrap();
        config
            .apply_env(|key| match key {
                "NOTES_HUB_MCP_OAUTH_ISSUER" => Some("https://auth.example/".into()),
                "NOTES_HUB_MCP_OAUTH_CLIENT_ID" => Some("connector".into()),
                _ => None,
            })
            .unwrap();
        assert_eq!(config.mcp_oauth.issuer, "https://auth.example/");
        assert_eq!(config.mcp_oauth.client_id, "connector");
        config.validate().unwrap();
        for public_url in ["", "not a URL", "ftp://notes.example"] {
            config.public_url = public_url.into();
            assert!(config.validate().unwrap_err().contains("public_url"));
        }
        config.public_url = "http://localhost:8000".into();
        config.validate().unwrap();
        for (issuer, client_id) in [("https://auth.example/", ""), ("", "connector")] {
            config.mcp_oauth = McpOAuthConfig {
                issuer: issuer.into(),
                client_id: client_id.into(),
            };
            assert!(!config.mcp_oauth.enabled());
            assert!(config.validate().unwrap_err().contains("required together"));
        }
        config
            .apply_env(|key| key.starts_with("NOTES_HUB_MCP_OAUTH_").then(String::new))
            .unwrap();
        assert!(!config.mcp_oauth.enabled());
        config.validate().unwrap();
    }

    #[test]
    fn schedules_default_toml_and_environment() {
        assert!(Config::default().schedules.runner);
        let mut config =
            Config::from_toml("data_dir = '/tmp/test'\n[schedules]\nrunner = false").unwrap();
        assert!(!config.schedules.runner);
        config
            .apply_env(|key| (key == "NOTES_HUB_SCHEDULES_RUNNER").then(|| "true".into()))
            .unwrap();
        assert!(config.schedules.runner);
        assert!(config
            .apply_env(|key| (key == "NOTES_HUB_SCHEDULES_RUNNER").then(|| "invalid".into()))
            .is_err());
    }

    #[test]
    fn android_app_links_env_validation_and_statement() {
        const FP: &str = "91:B6:FA:29:8D:52:AF:E8:23:FE:D0:D7:74:53:56:E7:6A:A6:88:8E:72:CE:9B:16:04:5D:94:E3:92:E5:86:59";
        let mut config = Config {
            data_dir: "/tmp/test".into(),
            ..Config::default()
        };
        assert_eq!(config.assetlinks_json(), None);
        config
            .apply_env(|key| match key {
                "NOTES_HUB_ANDROID_PACKAGE" => Some("de.bubblebuffer.notes".into()),
                "NOTES_HUB_ANDROID_SHA256_CERTS" => {
                    Some(format!("{FP}, {}", FP.to_ascii_lowercase()))
                }
                _ => None,
            })
            .unwrap();
        config.validate().unwrap();
        let statement: serde_json::Value =
            serde_json::from_str(&config.assetlinks_json().unwrap()).unwrap();
        assert_eq!(
            statement,
            serde_json::json!([{
                "relation": ["delegate_permission/common.handle_all_urls"],
                "target": {
                    "namespace": "android_app",
                    "package_name": "de.bubblebuffer.notes",
                    "sha256_cert_fingerprints": [FP, FP],
                }
            }])
        );

        let mut bad = config.clone();
        bad.android_package.clear();
        assert!(bad.validate().unwrap_err().contains("required together"));
        bad.android_sha256_certs.clear();
        bad.android_package = "de.bubblebuffer.notes".into();
        assert!(bad.validate().unwrap_err().contains("required together"));
        bad.android_sha256_certs = vec![FP.into()];
        bad.validate().unwrap();
        for package in [
            "notes",
            "de..notes",
            "1de.notes",
            "de.bubble-buffer",
            "de.notes.",
        ] {
            bad.android_package = package.into();
            assert!(
                bad.validate().unwrap_err().contains("android_package"),
                "{package}"
            );
        }
        bad.android_package = "de.bubblebuffer.notes".into();
        for cert in [
            "91:B6",
            "91B6FA298D52AFE823FED0D7745356E76AA6888E72CE9B16045D94E392E58659",
        ] {
            bad.android_sha256_certs = vec![cert.into()];
            assert!(bad.validate().unwrap_err().contains("SHA-256"), "{cert}");
        }
        assert!(Config::default().assetlinks_json().is_none());
    }

    #[test]
    fn import_source_limit_toml_env_and_validation() {
        assert_eq!(Config::default().import_source_limit_bytes, 64 << 20);
        let mut config =
            Config::from_toml("data_dir = '/tmp/test'\nimport_source_limit_bytes = 2048").unwrap();
        assert_eq!(config.import_source_limit_bytes, 2048);
        config
            .apply_env(|key| (key == "NOTES_HUB_IMPORT_SOURCE_LIMIT_BYTES").then(|| "4096".into()))
            .unwrap();
        assert_eq!(config.import_source_limit_bytes, 4096);
        config.validate().unwrap();
        config.import_source_limit_bytes = 0;
        assert!(config
            .validate()
            .unwrap_err()
            .contains("import_source_limit_bytes"));
        assert!(config
            .apply_env(|key| (key == "NOTES_HUB_IMPORT_SOURCE_LIMIT_BYTES").then(|| "bad".into()))
            .is_err());
    }

    #[test]
    fn oidc_client_secret_is_optional() {
        let with_oidc = |secret: &str| {
            Config::from_toml(&format!(
                r#"
                data_dir = "/data"
                public_url = "https://notes.example.com"
                [oidc]
                issuer = "https://auth.example.com/application/o/notes/"
                client_id = "notes"
                {secret}
                "#
            ))
            .unwrap()
        };
        // Unset or empty: a public client, valid.
        with_oidc("").validate().unwrap();
        with_oidc("client_secret = \"\"").validate().unwrap();
        // A secret still validates.
        with_oidc("client_secret = \"s\"").validate().unwrap();
        // An empty environment value clears a configured secret.
        let mut c = with_oidc("client_secret = \"s\"");
        c.apply_env(|k| (k == "NOTES_HUB_OIDC_CLIENT_SECRET").then(String::new))
            .unwrap();
        assert!(c.oidc.client_secret.is_empty());
        c.validate().unwrap();
        // The client ID is still required when OIDC is on.
        let mut c = with_oidc("");
        c.oidc.client_id.clear();
        assert!(c.validate().unwrap_err().contains("oidc.client_id"));
    }

    #[test]
    fn toml_then_env() {
        let mut c = Config::from_toml(
            r#"
            data_dir = "/data"
            public_url = "https://notes.example.com"
            [oidc]
            issuer = "https://auth.example.com/application/o/notes/"
            client_id = "notes"
            client_secret = "s"
            allowed_groups = ["notes-users"]
            "#,
        )
        .unwrap();
        assert_eq!(c.listen, "0.0.0.0:8080");
        assert!(c.trust_proxy);
        assert_eq!(c.oidc.scopes(), ["openid", "email", "profile", "groups"]);
        c.apply_env(|k| match k {
            "NOTES_HUB_LISTEN" => Some("127.0.0.1:9000".into()),
            "NOTES_HUB_TRUST_PROXY" => Some("false".into()),
            "NOTES_HUB_OIDC_ALLOWED_USERS" => Some("a@example.com, b".into()),
            "NOTES_HUB_OIDC_SCOPES" => Some("openid email".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(c.listen, "127.0.0.1:9000");
        assert!(!c.trust_proxy);
        // Off by default; the env var turns it on and rejects non-booleans.
        assert!(!c.require_device_keys);
        c.apply_env(|k| (k == "NOTES_HUB_REQUIRE_DEVICE_KEYS").then(|| "true".into()))
            .unwrap();
        assert!(c.require_device_keys);
        assert!(c
            .apply_env(|k| (k == "NOTES_HUB_REQUIRE_DEVICE_KEYS").then(|| "maybe".into()))
            .is_err());
        assert_eq!(c.oidc.allowed_users, ["a@example.com", "b"]);
        assert_eq!(c.oidc.scopes(), ["openid", "email"]);
        c.validate().unwrap();
        assert!(Config::from_toml("nope = 1").is_err());

        let mut c = Config::from_toml(
            r#"
            data_dir = "/data"
            [agent]
            default_model = "claude-code/claude-haiku-5-5"
            "#,
        )
        .unwrap();
        assert!(c.agent.enabled);
        assert_eq!(
            c.agent.execution_mode,
            vault_agent::bubblepaw_sdk::harness::ExecutionMode::Native
        );
        assert!(!c.agent.legacy_enable_js);
        assert!(!c.agent.legacy_approvals);
        c.apply_env(|k| match k {
            "NOTES_HUB_AGENT_ENABLED" => Some("off".into()),
            "NOTES_HUB_AGENT_BUN" => Some("/usr/local/bin/bun".into()),
            "NOTES_HUB_AGENT_EXECUTION_MODE" => Some("js".into()),
            "NOTES_HUB_AGENT_ENABLE_JS" => Some("true".into()),
            _ => None,
        })
        .unwrap();
        assert!(!c.agent.enabled);
        assert_eq!(c.agent.bun, "/usr/local/bin/bun");
        assert_eq!(c.agent.default_model, "claude-code/claude-haiku-5-5");
        assert_eq!(
            c.agent.execution_mode,
            vault_agent::bubblepaw_sdk::harness::ExecutionMode::Js
        );
        assert!(c.agent.legacy_enable_js);
        assert!(Config::from_toml("[agent]\nexecution_mode = \"invalid\"").is_err());
        assert!(c
            .apply_env(|k| (k == "NOTES_HUB_AGENT_EXECUTION_MODE").then(|| "invalid".into()))
            .is_err());
        assert!(!c.agent.legacy_approvals);
    }

    #[test]
    fn the_removed_enable_js_setting_is_accepted_and_ignored() {
        let c = Config::from_toml("[agent]\nenable_js = true").unwrap();
        assert!(c.agent.legacy_enable_js);
        assert!(c.agent.enabled);
        let mut c = Config::default();
        c.apply_env(|k| (k == "NOTES_HUB_AGENT_ENABLE_JS").then(|| "invalid".into()))
            .unwrap();
        assert!(c.agent.legacy_enable_js);
    }

    #[test]
    fn the_removed_approvals_setting_is_accepted_and_ignored() {
        for value in ["\"ask\"", "\"allow\"", "\"maybe\"", "1"] {
            let c = Config::from_toml(&format!("[agent]\napprovals = {value}")).unwrap();
            assert!(c.agent.legacy_approvals, "{value}");
            assert!(c.agent.enabled);
        }
        let mut c = Config::default();
        c.apply_env(|k| (k == "NOTES_HUB_AGENT_APPROVALS").then(|| "x".into()))
            .unwrap();
        assert!(c.agent.legacy_approvals);
        assert!(Config::from_toml("[agent]\napprovalz = \"ask\"").is_err());
    }

    #[test]
    fn agent_models_defaults_toml_environment_and_validation() {
        let mut config = Config::from_toml("data_dir = '/tmp/test'").unwrap();
        assert_eq!(
            config.agent.models,
            [
                "claude-code/claude-opus-5-5",
                "claude-code/claude-sonnet-5-5",
                "claude-code/claude-haiku-5-5",
                "opencode/deepseek-v4.1-flash",
            ]
        );
        assert_eq!(config.agent.default_model, "claude-code/claude-sonnet-5-5");
        config.validate().unwrap();

        config = Config::from_toml(
            "data_dir = '/tmp/test'\n[agent]\n\
             models = ['claude-code/claude-haiku-5-5', 'opencode/deepseek-v4.1-flash']\n\
             default_model = 'opencode/deepseek-v4.1-flash'",
        )
        .unwrap();
        assert_eq!(
            config.agent.models,
            [
                "claude-code/claude-haiku-5-5",
                "opencode/deepseek-v4.1-flash"
            ]
        );
        assert_eq!(config.agent.default_model, "opencode/deepseek-v4.1-flash");
        config.validate().unwrap();

        config
            .apply_env(|key| match key {
                "NOTES_HUB_AGENT_MODELS" => Some(
                    "claude-code/claude-opus-5-5, claude-code/claude-sonnet-5-5 \
                     opencode/deepseek-v4.1-flash"
                        .into(),
                ),
                "NOTES_HUB_AGENT_DEFAULT_MODEL" => Some("claude-code/claude-opus-5-5".into()),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            config.agent.models,
            [
                "claude-code/claude-opus-5-5",
                "claude-code/claude-sonnet-5-5",
                "opencode/deepseek-v4.1-flash",
            ]
        );
        assert_eq!(config.agent.default_model, "claude-code/claude-opus-5-5");
        config.validate().unwrap();

        // A default that is not listed fails, from TOML, the environment, or an empty default.
        let unlisted = Config::from_toml(
            "data_dir = '/tmp/test'\n[agent]\ndefault_model = 'claude-code/claude-fable-5-1'",
        )
        .unwrap();
        assert!(unlisted
            .validate()
            .unwrap_err()
            .contains("agent.default_model"));
        let mut bad = config.clone();
        bad.apply_env(|key| {
            (key == "NOTES_HUB_AGENT_DEFAULT_MODEL").then(|| "claude-code/claude-haiku-5-5".into())
        })
        .unwrap();
        assert!(bad.validate().unwrap_err().contains("agent.default_model"));
        let mut bad = config.clone();
        bad.apply_env(|key| (key == "NOTES_HUB_AGENT_DEFAULT_MODEL").then(String::new))
            .unwrap();
        assert!(bad.validate().unwrap_err().contains("agent.default_model"));
        // An empty list fails too: the default cannot be on it.
        let mut bad = config.clone();
        bad.apply_env(|key| (key == "NOTES_HUB_AGENT_MODELS").then(String::new))
            .unwrap();
        assert!(bad.agent.models.is_empty());
        assert!(bad.validate().unwrap_err().contains("agent.models"));
    }
}
