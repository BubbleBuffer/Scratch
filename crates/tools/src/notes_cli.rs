//! Thin, synchronous CLI over the hub's authenticated MCP endpoint.
//! Credentials come only from the environment or the user's config file.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use serde::Deserialize;
use serde_json::{json, Map, Value};

#[path = "notes_cli/import.rs"]
pub mod import;
#[path = "notes_cli/wiki.rs"]
pub mod wiki;

pub const DEFAULT_HUB: &str = "https://notes.breadcat.cc";

/// Deliberately has no Debug implementation: the token is a secret.
pub struct Config {
    pub hub: String,
    pub token: String,
}

#[derive(Deserialize, Default)]
struct FileConfig {
    hub: Option<String>,
    token: Option<String>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let hub = env_value("NOTES_HUB_URL")?;
        let token = env_value("NOTES_TOKEN")?;
        // A complete environment override must also work with an unreadable
        // config file. Missing values alone fall back to disk.
        let file = if hub.is_none() || token.is_none() {
            match config_path() {
                Some(path) => match std::fs::read_to_string(path) {
                    Ok(text) => toml::from_str::<FileConfig>(&text).map_err(|_| {
                        anyhow!("invalid notes config; expected hub and token strings")
                    })?,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => FileConfig::default(),
                    Err(_) => bail!("cannot read notes config"),
                },
                None => FileConfig::default(),
            }
        } else {
            FileConfig::default()
        };
        let token = token
            .or(file.token)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                anyhow!("missing token; set NOTES_TOKEN or token in notes/config.toml")
            })?;
        Ok(Self {
            hub: hub.or(file.hub).unwrap_or_else(|| DEFAULT_HUB.into()),
            token,
        })
    }
}

fn env_value(name: &str) -> Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) => Ok((!value.trim().is_empty()).then_some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(_) => bail!("invalid notes environment setting"),
    }
}

fn config_path() -> Option<PathBuf> {
    #[cfg(windows)]
    let root = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")));
    root.map(|p| p.join("notes").join("config.toml"))
}

pub struct Client {
    endpoint: String,
    token: String,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(config: Config) -> Result<Self> {
        if config.token.trim().is_empty() {
            bail!("missing token; set NOTES_TOKEN or token in notes/config.toml");
        }
        if config.token.chars().any(|c| c.is_control()) {
            bail!("invalid notes token");
        }
        let mut url = url::Url::parse(&config.hub).map_err(|_| anyhow!("invalid notes hub URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("invalid notes hub URL");
        }
        let path = url.path().trim_end_matches('/');
        if !path.ends_with("/mcp") {
            url.set_path(&format!("{path}/mcp"));
        }
        let agent = ureq::config::Config::builder()
            .http_status_as_error(false)
            // Never replay the bearer credential to a redirect destination.
            .max_redirects(0)
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .new_agent();
        Ok(Self {
            endpoint: url.to_string(),
            token: config.token,
            agent,
        })
    }

    /// Returns the raw MCP tool result. Tool failures remain isError results;
    /// [`tool_text`] turns them into errors for ordinary callers.
    pub fn call(&self, name: &str, arguments: &Value) -> Result<Value> {
        let request = json!({"jsonrpc":"2.0", "id":1, "method":"tools/call",
            "params":{"name":name,"arguments":arguments}});
        let mut response = self
            .agent
            .post(&self.endpoint)
            .header("Authorization", &format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .send(request.to_string().as_bytes())
            .map_err(|_| anyhow!("cannot reach notes hub"))?;
        match response.status().as_u16() {
            200 => {}
            401 | 403 => bail!("notes hub refused access; check token and scope"),
            300..=399 => bail!("notes hub redirected the request; check NOTES_HUB_URL"),
            status => bail!("notes hub returned HTTP {status}"),
        }
        let bytes = response
            .body_mut()
            .with_config()
            .limit(16 << 20)
            .read_to_vec()
            .map_err(|_| anyhow!("cannot read notes hub response"))?;
        let mut reply: Value =
            serde_json::from_slice(&bytes).map_err(|_| anyhow!("invalid notes hub response"))?;
        redact_value(&mut reply, &self.token);
        if reply.get("error").is_some() {
            bail!("notes hub rejected the tool request");
        }
        reply
            .get("result")
            .cloned()
            .ok_or_else(|| anyhow!("invalid notes hub response"))
    }
}

fn redact_value(value: &mut Value, token: &str) {
    match value {
        Value::String(s) => *s = s.replace(token, "[redacted]"),
        Value::Array(items) => items.iter_mut().for_each(|v| redact_value(v, token)),
        Value::Object(fields) => {
            let old = std::mem::take(fields);
            for (key, mut value) in old {
                redact_value(&mut value, token);
                fields.insert(key.replace(token, "[redacted]"), value);
            }
        }
        _ => {}
    }
}

pub fn tool_text(result: &Value) -> Result<String> {
    let content = result
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("invalid notes tool result"))?;
    let text = content
        .iter()
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        // The hub's MCP adapter prefixes tool errors for display. Remove
        // exactly that wrapper so callers can distinguish a missing page
        // from other failures without changing successful/raw results.
        let text = text.strip_prefix("Error: ").unwrap_or(&text);
        bail!(
            "{}",
            if text.is_empty() {
                "notes tool failed"
            } else {
                text
            }
        );
    }
    Ok(text)
}

const HELP: &str = "notes [--json] COMMAND\n\
search QUERY | read TITLE_OR_ID | query DATABASE [--filter JSON] [--sort JSON]\n\
write TITLE [--parent TITLE_OR_ID] < markdown | append TITLE_OR_ID < markdown\n\
memory list [PATH] | search QUERY [--scope PATH] | read PATH | delete PATH | rules\n\
memory write PATH --description TEXT --type TYPE [--name TEXT] < markdown\n\
memory update PATH [--new-path PATH] [--description TEXT] [--type TYPE] [--name TEXT] [--append] [< markdown]\n\
memory index [--project PROJECT] [--max-chars N]\n\
memory import PROJECTS_DIR --map MAP [--dry-run] [--offline]\n\
wiki import [--project NAME] [--dry-run] [--exclude GLOB]... [--report FILE] [DIR]\n\
Credentials: NOTES_TOKEN or token in notes/config.toml; hub: NOTES_HUB_URL or hub in config.";

struct Parsed {
    words: Vec<String>,
    options: BTreeMap<String, String>,
    /// Repeatable `--exclude` values, kept apart from the single-value map.
    excludes: Vec<String>,
    json: bool,
}

fn parse(args: &[String]) -> Result<Parsed> {
    reject_token_args(args)?;
    let mut parsed = Parsed {
        words: Vec::new(),
        options: BTreeMap::new(),
        excludes: Vec::new(),
        json: false,
    };
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        // Parser errors deliberately never echo command-line values.
        if arg == "--json" {
            parsed.json = true;
        } else if matches!(arg.as_str(), "--append" | "--dry-run" | "--offline") {
            if parsed
                .options
                .insert(arg[2..].into(), String::new())
                .is_some()
            {
                bail!("duplicate option");
            }
        } else if let Some(option) = arg.strip_prefix("--") {
            let (key, inline) = option
                .split_once('=')
                .map_or((option, None), |(k, v)| (k, Some(v)));
            if !matches!(
                key,
                "parent"
                    | "filter"
                    | "sort"
                    | "match"
                    | "limit"
                    | "offset"
                    | "scope"
                    | "description"
                    | "type"
                    | "name"
                    | "new-path"
                    | "project"
                    | "max-chars"
                    | "map"
                    | "report"
                    | "exclude"
            ) {
                bail!("unknown option; use notes --help");
            }
            let value = match inline {
                Some(v) => v.to_string(),
                None => {
                    i += 1;
                    args.get(i)
                        .filter(|v| !v.starts_with("--"))
                        .cloned()
                        .ok_or_else(|| anyhow!("option needs a value"))?
                }
            };
            if key == "exclude" {
                if value.trim().is_empty() {
                    bail!("empty --exclude pattern");
                }
                parsed.excludes.push(value);
            } else if parsed.options.insert(key.into(), value).is_some() {
                bail!("duplicate option");
            }
        } else if arg.starts_with('-') {
            bail!("unknown option; use notes --help");
        } else {
            parsed.words.push(arg.clone());
        }
        i += 1;
    }
    // Only wiki import takes these; other commands reject them as unknown.
    if !is_wiki(&parsed) && (!parsed.excludes.is_empty() || parsed.options.contains_key("report")) {
        bail!("unknown option; use notes --help");
    }
    Ok(parsed)
}

fn is_wiki(parsed: &Parsed) -> bool {
    parsed.words.first().map(String::as_str) == Some("wiki")
}

fn reject_token_args(args: &[String]) -> Result<()> {
    if args.iter().any(|a| {
        a.starts_with("--token") || a.starts_with("--agent-token") || a.starts_with("--notes-token")
    }) {
        bail!("token arguments are forbidden; use NOTES_TOKEN or notes/config.toml");
    }
    Ok(())
}

fn require_words(parsed: &Parsed, count: usize) -> Result<()> {
    if parsed.words.len() != count {
        bail!("invalid command arguments; use notes --help");
    }
    Ok(())
}

fn allowed(parsed: &Parsed, keys: &[&str]) -> Result<()> {
    if parsed.options.keys().any(|k| !keys.contains(&k.as_str())) {
        bail!("option is not valid for this command");
    }
    Ok(())
}

fn required_option<'a>(p: &'a Parsed, key: &str) -> Result<&'a str> {
    p.options
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| anyhow!("missing required --{key} option"))
}

fn stdin_required(stdin: Option<&str>) -> Result<&str> {
    stdin.ok_or_else(|| anyhow!("provide markdown on standard input"))
}

/// Production entry point; arguments exclude the binary name.
pub fn run(args: &[String]) -> Result<String> {
    reject_token_args(args)?;
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        return Ok(HELP.into());
    }
    let parsed = parse(args)?;
    if is_rules(&parsed) {
        return Ok(rules_text());
    }
    let cwd = std::env::current_dir().map_err(|_| anyhow!("cannot determine current directory"))?;
    if is_wiki(&parsed) {
        return wiki_command(parsed, &cwd, Config::load);
    }
    if is_import(&parsed) && parsed.options.contains_key("offline") {
        return execute_import(&parsed, None);
    }
    // Do not wait for stdin on read-only commands or ordinary terminal use.
    let wants_body = matches!(
        parsed.words.first().map(String::as_str),
        Some("write" | "append")
    ) || parsed.words.first().map(String::as_str) == Some("memory")
        && matches!(
            parsed.words.get(1).map(String::as_str),
            Some("write" | "update")
        );
    let mut input = String::new();
    let stdin = if wants_body && !std::io::stdin().is_terminal() {
        std::io::stdin()
            .read_to_string(&mut input)
            .map_err(|_| anyhow!("cannot read standard input"))?;
        if input.is_empty()
            && parsed.words.first().map(String::as_str) == Some("memory")
            && parsed.words.get(1).map(String::as_str) == Some("update")
            && ["description", "type", "name", "new-path"]
                .iter()
                .any(|key| parsed.options.contains_key(*key))
        {
            None
        } else {
            Some(input.as_str())
        }
    } else {
        None
    };
    let config = Config::load()?;
    run_parsed(parsed, stdin, &cwd, config)
}

/// Explicit configuration and input make local integration tests independent
/// of user credentials, shell state and the live hub.
pub fn run_with_config(
    args: &[String],
    stdin: Option<&str>,
    cwd: &Path,
    config: Config,
) -> Result<String> {
    reject_token_args(args)?;
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        return Ok(HELP.into());
    }
    let parsed = parse(args)?;
    if is_rules(&parsed) {
        return Ok(rules_text());
    }
    if is_wiki(&parsed) {
        return wiki_command(parsed, cwd, || Ok(config));
    }
    run_parsed(parsed, stdin, cwd, config)
}

/// Runs `wiki import` with `config` loaded only if a hub call is needed.
/// Tests use this to exercise the refusal and empty paths without credentials.
pub fn run_wiki(
    args: &[String],
    cwd: &Path,
    config: impl FnOnce() -> Result<Config>,
) -> Result<String> {
    reject_token_args(args)?;
    let parsed = parse(args)?;
    if !is_wiki(&parsed) {
        bail!("invalid command arguments; use notes --help");
    }
    wiki_command(parsed, cwd, config)
}

fn wiki_command(p: Parsed, cwd: &Path, config: impl FnOnce() -> Result<Config>) -> Result<String> {
    if p.words.len() > 3 || p.words.get(1).map(String::as_str) != Some("import") {
        bail!("invalid command arguments; use notes --help");
    }
    allowed(&p, &["project", "dry-run", "report"])?;
    let opts = wiki::Options {
        project: p.options.get("project").cloned(),
        dry_run: p.options.contains_key("dry-run"),
        excludes: p.excludes.clone(),
        report: p.options.get("report").map(PathBuf::from),
        dir: p.words.get(2).map(PathBuf::from),
    };
    wiki::run(&opts, cwd, config)
}

/// `notes memory rules`: answered locally, before any config or hub access.
fn is_rules(parsed: &Parsed) -> bool {
    parsed.words.first().map(String::as_str) == Some("memory")
        && parsed.words.get(1).map(String::as_str) == Some("rules")
}

/// The session-start memory rules, without the file's final newline (the
/// binary adds one, so the output is exactly the rules text).
fn rules_text() -> String {
    crate::help::MEMORY_RULES.trim_end_matches('\n').to_string()
}

fn is_import(parsed: &Parsed) -> bool {
    parsed.words.first().map(String::as_str) == Some("memory")
        && parsed.words.get(1).map(String::as_str) == Some("import")
}

fn execute_import(p: &Parsed, client: Option<&Client>) -> Result<String> {
    require_words(p, 3)?;
    allowed(p, &["map", "dry-run", "offline"])?;
    if p.options.contains_key("offline") && !p.options.contains_key("dry-run") {
        bail!("--offline requires --dry-run");
    }
    import::run(
        client,
        Path::new(&p.words[2]),
        Path::new(required_option(p, "map")?),
        p.options.contains_key("dry-run"),
    )
}

fn run_parsed(p: Parsed, stdin: Option<&str>, cwd: &Path, config: Config) -> Result<String> {
    if is_import(&p) && p.options.contains_key("offline") {
        return execute_import(&p, None);
    }
    let token = config.token.clone();
    // Catch all local output/error paths too, including a token accidentally
    // pasted into another argument or a server error containing the token.
    let output = execute(&p, stdin, cwd, config);
    if token.is_empty() {
        return output;
    }
    match output {
        Ok(text) => Ok(text.replace(&token, "[redacted]")),
        Err(error) => Err(anyhow!(error.to_string().replace(&token, "[redacted]"))),
    }
}

fn execute(p: &Parsed, stdin: Option<&str>, cwd: &Path, config: Config) -> Result<String> {
    let command = p.words.first().map(String::as_str).unwrap_or_default();
    let client = Client::new(config)?;
    if is_import(p) {
        return execute_import(p, Some(&client));
    }
    if command == "memory" && p.words.get(1).map(String::as_str) == Some("index") {
        require_words(p, 2)?;
        allowed(p, &["project", "max-chars"])?;
        let project = p
            .options
            .get("project")
            .cloned()
            .or_else(|| infer_project(cwd));
        let max = match p.options.get("max-chars") {
            Some(n) => n
                .parse::<usize>()
                .map_err(|_| anyhow!("--max-chars must be a positive integer"))?,
            None => 9500,
        };
        if max == 0 {
            bail!("--max-chars must be a positive integer");
        }
        if p.json {
            // Index composes two calls: retain each actual raw tool result.
            let mut results = vec![client.call("memory_list", &json!({"path":"global"}))?];
            if let Some(project) = project.as_deref().filter(|p| *p != "global") {
                results.push(client.call("memory_list", &json!({"path":project}))?);
            }
            for result in &results {
                if let Err(error) = tool_text(result) {
                    if !error.to_string().starts_with("no memory project ") {
                        return Err(error);
                    }
                }
            }
            return Ok(serde_json::to_string(&results)?);
        }
        return memory_index(&client, project.as_deref(), max);
    }
    let (name, arguments) = match command {
        "search" => {
            require_words(p, 2)?;
            allowed(p, &["limit", "offset"])?;
            let mut a = json!({"query":p.words[1]});
            numeric_options(p, &mut a)?;
            ("search_notes", a)
        }
        "read" => {
            require_words(p, 2)?;
            allowed(p, &["limit", "offset"])?;
            let mut a = json!({"page":p.words[1]});
            numeric_options(p, &mut a)?;
            ("read_page", a)
        }
        "query" => {
            require_words(p, 2)?;
            allowed(p, &["filter", "sort", "match", "limit", "offset"])?;
            let mut a = json!({"database":p.words[1]});
            numeric_options(p, &mut a)?;
            for key in ["filter", "sort"] {
                if let Some(text) = p.options.get(key) {
                    let value: Value = serde_json::from_str(text)
                        .map_err(|_| anyhow!("--{key} requires a JSON array"))?;
                    if !value.is_array() {
                        bail!("--{key} requires a JSON array");
                    }
                    a[key] = value;
                }
            }
            if let Some(m) = p.options.get("match") {
                if !matches!(m.as_str(), "all" | "any") {
                    bail!("--match must be all or any");
                }
                a["match"] = json!(m);
            }
            ("query_database", a)
        }
        "write" => {
            require_words(p, 2)?;
            allowed(p, &["parent"])?;
            let body = stdin_required(stdin)?;
            let existing = client.call("read_page", &json!({"page":p.words[1], "limit":1}))?;
            match tool_text(&existing) {
                Ok(_) => (
                    "edit_page",
                    json!({"page":p.words[1], "op":"set_body", "markdown":body}),
                ),
                Err(e)
                    if e.to_string().starts_with("no page ")
                        || e.to_string()
                            .starts_with("No page has the id or exact title") =>
                {
                    let mut a = json!({"title":p.words[1], "markdown":body});
                    if let Some(parent) = p.options.get("parent") {
                        a["parent"] = json!(parent);
                    }
                    ("create_page", a)
                }
                Err(e) => return Err(e),
            }
        }
        "append" => {
            require_words(p, 2)?;
            allowed(p, &[])?;
            (
                "edit_page",
                json!({"page":p.words[1], "op":"append", "markdown":stdin_required(stdin)?}),
            )
        }
        "memory" => memory_command(p, stdin)?,
        _ => bail!("unknown command; use notes --help"),
    };
    let result = client.call(name, &arguments)?;
    // --json changes rendering, never success/failure semantics.
    tool_text(&result)?;
    if p.json {
        Ok(result.to_string())
    } else {
        tool_text(&result)
    }
}

fn numeric_options(p: &Parsed, a: &mut Value) -> Result<()> {
    for key in ["limit", "offset"] {
        if let Some(value) = p.options.get(key) {
            let value = value
                .parse::<u32>()
                .map_err(|_| anyhow!("--{key} must be a nonnegative integer"))?;
            a[key] = json!(value);
        }
    }
    Ok(())
}

fn memory_command(p: &Parsed, stdin: Option<&str>) -> Result<(&'static str, Value)> {
    let command = p.words.get(1).map(String::as_str).unwrap_or_default();
    let mut a = Map::new();
    let name = match command {
        "list" => {
            if !(2..=3).contains(&p.words.len()) {
                bail!("memory list accepts an optional path");
            }
            allowed(p, &[])?;
            if let Some(path) = p.words.get(2) {
                a.insert("path".into(), json!(path));
            }
            "memory_list"
        }
        "search" => {
            require_words(p, 3)?;
            allowed(p, &["scope"])?;
            a.insert("query".into(), json!(p.words[2]));
            if let Some(scope) = p.options.get("scope") {
                a.insert("scope".into(), json!(scope));
            }
            "memory_search"
        }
        "read" | "delete" => {
            require_words(p, 3)?;
            allowed(p, &[])?;
            a.insert("path".into(), json!(p.words[2]));
            if command == "read" {
                "memory_read"
            } else {
                "memory_delete"
            }
        }
        "write" | "update" => {
            require_words(p, 3)?;
            allowed(
                p,
                if command == "write" {
                    &["description", "type", "name"]
                } else {
                    &["description", "type", "name", "append", "new-path"]
                },
            )?;
            a.insert("path".into(), json!(p.words[2]));
            if command == "write" {
                required_option(p, "description")?;
                required_option(p, "type")?;
                a.insert("body".into(), json!(stdin_required(stdin)?));
            } else if let Some(body) = stdin {
                a.insert(
                    if p.options.contains_key("append") {
                        "append"
                    } else {
                        "body"
                    }
                    .into(),
                    json!(body),
                );
            } else if p.options.contains_key("append") {
                bail!("--append requires markdown on standard input");
            }
            for key in ["description", "type", "name"] {
                if let Some(value) = p.options.get(key) {
                    if key == "type"
                        && !matches!(
                            value.as_str(),
                            "user" | "feedback" | "project" | "reference"
                        )
                    {
                        bail!("type must be user, feedback, project or reference");
                    }
                    a.insert(key.into(), json!(value));
                }
            }
            if let Some(path) = p.options.get("new-path") {
                a.insert("new_path".into(), json!(path));
            }
            if command == "update" && a.len() == 1 {
                bail!("memory update needs fields or markdown on standard input");
            }
            if command == "write" {
                "memory_write"
            } else {
                "memory_update"
            }
        }
        _ => bail!("unknown memory command; use notes --help"),
    };
    Ok((name, Value::Object(a)))
}

/// Uses the common git directory so linked worktrees inherit the main repo's
/// memory namespace. Outside a repository, only global memories are shown.
pub fn infer_project(cwd: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(cwd)
        .output()
        .ok();
    if let Some(output) = output.filter(|o| o.status.success()) {
        if let Ok(path) = String::from_utf8(output.stdout) {
            let path = PathBuf::from(path.trim());
            if let Some(repo) = path
                .parent()
                .and_then(Path::file_name)
                .and_then(|s| s.to_str())
            {
                return Some(repo.to_owned());
            }
        }
    }
    None
}

fn optional_listing(client: &Client, project: &str) -> Result<Vec<String>> {
    let result = client.call("memory_list", &json!({"path":project}))?;
    match tool_text(&result) {
        Ok(text) if text.starts_with("No memories in ") => Ok(Vec::new()),
        Ok(text) => Ok(text.lines().map(str::to_string).collect()),
        Err(e) if e.to_string().starts_with("no memory project ") => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

pub fn memory_index(client: &Client, project: Option<&str>, max_chars: usize) -> Result<String> {
    let global = optional_listing(client, "global")?;
    let project = project.filter(|p| *p != "global");
    let local = match project {
        Some(p) => optional_listing(client, p)?,
        None => Vec::new(),
    };
    if local.is_empty() {
        if let Some(project) = project {
            return Ok(empty_project_index(&global, project, max_chars));
        }
    }
    if global.is_empty() && local.is_empty() {
        return Ok(fit_chars("No memories.", max_chars));
    }
    let mut sections = Vec::new();
    if !global.is_empty() {
        sections.push(("global", global));
    }
    if let Some(p) = project {
        if !local.is_empty() {
            sections.push((p, local));
        }
    }
    let full = sections
        .iter()
        .map(|(p, lines)| format!("## {p}\n{}", lines.join("\n")))
        .collect::<Vec<_>>()
        .join("\n\n");
    if full.chars().count() <= max_chars {
        return Ok(full);
    }
    if sections.len() == 1 {
        return Ok(capped_section(sections[0].0, &sections[0].1, max_chars));
    }
    // Keep global whole whenever it fits. The remainder admits whole project
    // entries and their omission notice; a tiny cap may fit global alone.
    let global_full = format!("## {}\n{}", sections[0].0, sections[0].1.join("\n"));
    let global = capped_section(sections[0].0, &sections[0].1, max_chars);
    if global_full.chars().count() + 2 >= max_chars {
        return Ok(global);
    }
    let remaining = max_chars - global.chars().count() - 2;
    let local_marker = format!(
        "… {} more: list them with `notes memory list {}`",
        sections[1].1.len(),
        sections[1].0
    );
    if local_marker.chars().count() > remaining {
        return Ok(global);
    }
    let local = capped_section(sections[1].0, &sections[1].1, remaining);
    Ok(format!("{global}\n\n{local}"))
}

fn empty_project_index(global: &[String], project: &str, max_chars: usize) -> String {
    let message = format!("No memories for {project}.");
    if global.is_empty() {
        return fit_chars(&message, max_chars);
    }
    let global = capped_section("global", global, max_chars);
    let notice = format!("\n\n## {project}\n{message}");
    if global.chars().count() + notice.chars().count() <= max_chars {
        format!("{global}{notice}")
    } else {
        global
    }
}

fn capped_section(project: &str, lines: &[String], budget: usize) -> String {
    let heading = format!("## {project}\n");
    let full = format!("{heading}{}", lines.join("\n"));
    if full.chars().count() <= budget {
        return full;
    }
    let mut kept = 0;
    let mut body = String::new();
    while kept < lines.len() {
        let next = format!("{body}{}{}", if kept == 0 { "" } else { "\n" }, lines[kept]);
        let remaining = lines.len() - kept - 1;
        let marker = if remaining > 0 {
            format!("\n… {remaining} more: list them with `notes memory list {project}`")
        } else {
            String::new()
        };
        if heading.chars().count() + next.chars().count() + marker.chars().count() > budget {
            break;
        }
        body = next;
        kept += 1;
    }
    let marker = format!(
        "… {} more: list them with `notes memory list {project}`",
        lines.len() - kept
    );
    if heading.chars().count() + marker.chars().count() > budget {
        // Only the notice may be shortened when the complete minimum cannot
        // fit. Never emit a partial memory entry.
        return fit_chars(&marker, budget);
    }
    format!(
        "{heading}{body}{}{marker}",
        if kept > 0 { "\n" } else { "" }
    )
}

fn fit_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_project_keeps_global_and_adds_a_plain_notice_when_it_fits() {
        let global = vec!["- global/preference — Preference (user)".to_string()];
        let output = empty_project_index(&global, "new-project", 9500);
        assert_eq!(output, "## global\n- global/preference — Preference (user)\n\n## new-project\nNo memories for new-project.");
        assert!(!output.contains("more:"));
        assert_eq!(
            empty_project_index(&[], "new-project", 9500),
            "No memories for new-project."
        );
        let global_budget = "## global\n".chars().count() + global[0].chars().count();
        assert_eq!(
            empty_project_index(&global, "new-project", global_budget),
            "## global\n- global/preference — Preference (user)"
        );
    }

    #[test]
    fn tool_text_unwraps_the_hub_error_prefix_once() {
        let result = json!({"isError":true,"content":[{"type":"text",
            "text":"Error: no page \"New page\"; use search_notes to find its id"}]});
        assert_eq!(
            tool_text(&result).unwrap_err().to_string(),
            "no page \"New page\"; use search_notes to find its id"
        );
        let result = json!({"isError":true,"content":[{"type":"text",
            "text":"Error: no memory project \"global\""}]});
        assert_eq!(
            tool_text(&result).unwrap_err().to_string(),
            "no memory project \"global\""
        );
        let result = json!({"isError":true,"content":[{"type":"text",
            "text":"Error: Error: original message"}]});
        assert_eq!(
            tool_text(&result).unwrap_err().to_string(),
            "Error: original message"
        );
        let result = json!({"isError":false,"content":[{"type":"text", "text":"Error: ordinary page content"}]});
        assert_eq!(tool_text(&result).unwrap(), "Error: ordinary page content");
    }
}
