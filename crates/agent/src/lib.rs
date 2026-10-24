//! A BubblePaw agent over the open vault, shared by the desktop and hub hosts.
//!
//! - Sessions live in a per-device BubblePaw store, `<data_dir>/agent/agent.data/`,
//!   outside the vault (`agent/agent.sqlite` is the legacy store file).
//! - The default model surface is BubblePaw's `js` tool. Hosts can opt into
//!   native entrypoints; vault and history tools remain composable inside JS.
//! - Providers: `codex` (Codex sign-in), `claude-code` (the Bun extension, when
//!   configured) and, for tests only, `scripted`.
//! - The agent owns its own Tokio runtime, so callers can stay on plain threads.
//!
//! See wiki/integrations/agent.md, build step 3.

pub mod claude;
mod context;
mod gateway;
pub mod mcp;
mod outline_titles;
/// Explicitly renamed sessions retain their complete display name.
pub fn session_user_named_ids_at(data_dir: &std::path::Path) -> anyhow::Result<Vec<String>> {
    outline_titles::user_named_ids(&data_dir.join("agent/chat_outline.sqlite"))
}
pub mod projects;
mod records;
pub mod replay;
pub mod rpc;
pub mod schedules;
pub mod tasks;
pub mod testing;
pub mod tools;
mod ui;
pub mod wire;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use anyhow::{ensure, Context, Result};
use async_trait::async_trait;
use bubblepaw_core::{
    ApprovalMode, ApprovalPolicy, ApprovalRecord, CancellationToken, ClientInfo, Decision, Harness,
    HarnessConfig, HarnessEvent, LogPosition, Message, ModelInfo, ModelRequest, ModelResponse,
    Provider, ProviderError, ProviderErrorKind, QueuedInput, Run, RunOptions, Session, SessionBusy,
    Store, TextCallback, Tool, ToolRegistry, EVENTS_AFTER_LIMIT,
};
use bubblepaw_runner::artifacts::ArtifactStore;
use bubblepaw_runner::codemode::CodeMode;
use bubblepaw_runner::provider::{
    ExtensionConfig, ExtensionProvider, ProviderRegistry, ResponsesProvider, ScriptedProvider,
    ScriptedStep,
};
use bubblepaw_runner::runtime_note::RuntimeNotes;
use bubblepaw_runner::summarization::{cached_context_window, SummarizingContext, SummaryConfig};
use bubblepaw_runner::tools::register_history_tools;
use vault_core::Vault;

pub use bubblepaw_core;
pub use bubblepaw_runner;
pub use notes_agent_wire::{HostInfo, PermissionMode, Project};
pub use vault_tools::outline;

/// The default model when the device setting is empty: Codex with the
/// BubblePaw runner's default model.
pub const DEFAULT_MODEL: &str = "codex/gpt-6-luna";
const CODEX_URL: &str = "https://chatgpt.com/backend-api/codex";

/// Which application hosts the agent; clients discover this through host_info.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostKind {
    #[default]
    Desktop,
    Hub,
}

/// Optional capabilities in addition to the always-present vault and history tools.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPack {
    Desktop { root: PathBuf },
}

/// For tests only: the scripted provider and extra host tools.
#[derive(Default)]
pub struct TestHooks {
    /// Registers the `scripted` provider with these steps and final text.
    pub scripted: Option<(Vec<ScriptedStep>, String)>,
    /// Extra host tools (for example one declaring `Confirm`).
    pub extra_tools: Vec<Arc<dyn Tool>>,
    /// Registers [`testing::ConfirmTool`] (`test_confirm`), which asks for
    /// approval.
    pub confirm_tool: bool,
    /// Registers [`testing::SleepTool`] (`test_sleep`), which waits `ms`.
    pub sleep_tool: bool,
}

/// Host capabilities and per-device agent settings.
#[derive(Default)]
pub struct AgentConfig {
    /// The application hosting this agent, reported by [`Agent::host_info`].
    pub host_kind: HostKind,
    /// Additional tools; vault, history and environment capabilities are registered.
    pub packs: Vec<ToolPack>,
    /// The app's data directory; the agent keeps `agent/` inside it.
    pub data_dir: PathBuf,
    /// `provider/model`; empty means [`DEFAULT_MODEL`].
    pub default_model: String,
    /// The `bun` executable for the Claude Code extension.
    pub bun_path: Option<PathBuf>,
    /// The Claude Code extension folder (holding `src/main.ts`).
    pub claude_code_dir: Option<PathBuf>,
    pub permission_mode: PermissionMode,
    /// The tool set this engine registers, in place of its runner's set from
    /// the vault. The hub's job engines set it (each job gets an engine of
    /// its set); None reads the runner's set.
    pub tool_set: Option<vault_tools::ToolSet>,
    /// MCP servers whose tools this engine offers (desktop hosts with the
    /// Everything set only), at open; [`Agent::set_mcp`] replaces them.
    /// Empty: none.
    pub mcp: mcp::McpSettings,
    pub test: TestHooks,
}

/// The vault runner an engine of `host_kind` is: `hub` or `this_device`.
pub fn runner_of(host_kind: HostKind) -> &'static str {
    match host_kind {
        HostKind::Hub => vault_core::AGENT_RUNNER_HUB,
        HostKind::Desktop => vault_core::AGENT_RUNNER_THIS_DEVICE,
    }
}

/// The tool set this runner uses, as the vault stores it (Everything when
/// unset). An unreadable value falls back to Read, the narrowest set.
pub fn runner_tool_set(vault: &Vault, host_kind: HostKind) -> vault_tools::ToolSet {
    match vault
        .agent_tool_set(runner_of(host_kind))
        .map_err(|e| e.to_string())
        .and_then(|s| s.parse::<vault_tools::ToolSet>())
    {
        Ok(set) => set,
        Err(error) => {
            eprintln!("agent tool set unreadable, using read: {error}");
            vault_tools::ToolSet::Read
        }
    }
}

/// The client every command from the desktop app names.
pub fn desktop_client() -> ClientInfo {
    ClientInfo {
        id: "desktop".into(),
        name: Some("Desktop".into()),
    }
}

/// What `send` did.
#[derive(Debug, Clone)]
pub enum Sent {
    /// A new run started.
    Started(Run),
    /// The session was busy; the text was queued as a follow-up.
    FollowUp(QueuedInput),
}

/// A session snapshot and the event-log position it reflects.
#[derive(Debug, Clone)]
pub struct SessionState {
    pub session: Session,
    pub messages: Vec<Message>,
    pub runs: Vec<Run>,
    pub pending_approvals: Vec<ApprovalRecord>,
    pub position: LogPosition,
    pub project: Option<Project>,
    pub activity: Vec<notes_agent_wire::ToolActivity>,
    pub edits: Vec<notes_agent_wire::AgentEdit>,
}

/// A page of durable events.
#[derive(Debug, Clone)]
pub struct EventsPage {
    pub events: Vec<HarnessEvent>,
    pub last_seq: u64,
    /// Events after `after` were pruned: reload the session state instead.
    pub resync: bool,
}

/// What a subscriber receives.
#[derive(Debug, Clone)]
pub enum Forwarded {
    UiAction(notes_agent_wire::UiAction),
    Event(Box<HarnessEvent>),
    /// The subscriber fell behind and `skipped` events were dropped; catch up
    /// with `events_after(after)`.
    Gap {
        skipped: u64,
        after: u64,
    },
}

/// The outline of `session` in `store`, with its notes-owned file at `outline_path`.
/// The notes-owned outline wins whenever one was saved for the session, even an
/// empty one. Otherwise BubblePaw's context chapters are returned without sections,
/// each ending at its last message. Empty when neither exists.
pub fn read_session_outline(
    store: &Store,
    outline_path: &Path,
    session: &str,
) -> Result<vault_tools::outline::ChatOutline> {
    store.session(session)?;
    if let Some(chapters) = vault_tools::outline::OutlineStore::read_session(outline_path, session)?
    {
        return Ok(vault_tools::outline::ChatOutline { chapters });
    }
    let chapters = store.context_chapters(session)?;
    if chapters.is_empty() {
        return Ok(vault_tools::outline::ChatOutline::default());
    }
    let created: std::collections::HashMap<String, String> = store
        .messages(session)?
        .into_iter()
        .map(|m| (m.id, m.created_at))
        .collect();
    let time = |id: &str| -> Result<String> {
        created
            .get(id)
            .cloned()
            .with_context(|| format!("chapter anchor {id} is not in the session"))
    };
    let chapters = chapters
        .into_iter()
        .map(|c| {
            Ok(vault_tools::outline::OutlineChapter {
                started_at: time(&c.first_message_id)?,
                ended_at: Some(time(&c.last_message_id)?),
                id: c.id,
                title: c.title,
                summary: c.summary,
                decisions: Vec::new(),
                ongoing: false,
                first_message_id: c.first_message_id,
                last_message_id: c.last_message_id,
                sections: Vec::new(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(vault_tools::outline::ChatOutline { chapters })
}

/// The outline of `session` in the agent data directory `data_dir`, for a notes app
/// that holds no [`Agent`] of its own. The agent database is opened with
/// [`Store::open`], which hands back the live store when an open agent already has
/// that database, so no second handle is made while one is alive. Errors, instead of
/// creating a database, when the directory has no agent store.
pub fn session_outline_at(
    data_dir: &Path,
    session: &str,
) -> Result<vault_tools::outline::ChatOutline> {
    let (store, outline_path) = agent_store_at(data_dir)?;
    outline_titles::refresh(&store, &outline_path, session)?;
    read_session_outline(&store, &outline_path, session)
}

/// The agent store in `data_dir` and the outline file beside it. Errors, instead of
/// creating a database, when the directory has no agent store.
fn agent_store_at(data_dir: &Path) -> Result<(Store, PathBuf)> {
    let agent_dir = data_dir.join("agent");
    let db = agent_dir.join("agent.sqlite");
    // BubblePaw keeps the store in `agent.data/`; `agent.sqlite` is only a legacy file.
    ensure!(
        db.exists() || Store::legacy_data_dir(&db).is_dir(),
        "no agent store at {}",
        db.display()
    );
    let store = Store::open(&db).with_context(|| format!("open {}", db.display()))?;
    Ok((store, agent_dir.join(vault_tools::outline::FILE_NAME)))
}

/// The revision of `session`'s outline in the agent data directory `data_dir`: it
/// rises by one with every change and is 0 when none was made. Read-only: it never
/// creates a store or the outline file.
pub fn session_outline_revision_at(data_dir: &Path, session: &str) -> Result<u64> {
    let outline_path = data_dir.join("agent").join(vault_tools::outline::FILE_NAME);
    Ok(vault_tools::outline::OutlineStore::read_revision(
        &outline_path,
        session,
    )?)
}

/// The session's message ids in session order, for the outline's anchor checks.
/// Errors for an unknown session.
fn session_message_ids(store: &Store, session: &str) -> Result<Vec<String>> {
    store.session(session)?;
    Ok(store.messages(session)?.into_iter().map(|m| m.id).collect())
}

/// Adds a chapter to `session`'s outline, or replaces the chapter with the same
/// `first_message_id`, in the agent data directory `data_dir`. One transaction; every
/// anchor must be a message of the session, in order. Returns the new revision.
/// See [`vault_tools::outline::OutlineStore::upsert_chapter`].
pub fn upsert_outline_chapter_at(
    data_dir: &Path,
    session: &str,
    chapter: vault_tools::outline::OutlineChapter,
) -> Result<u64> {
    let (store, outline_path) = agent_store_at(data_dir)?;
    let order = session_message_ids(&store, session)?;
    let mut outline = vault_tools::outline::OutlineStore::open(&outline_path)?;
    let revision = outline.upsert_chapter(session, &order, chapter)?;
    outline_titles::refresh(&store, &outline_path, session)?;
    Ok(revision)
}

/// Adds a section to the chapter starting at `chapter_first_message_id`, or replaces
/// the section with the same `first_message_id`, in `data_dir`. Returns the new
/// revision. See [`vault_tools::outline::OutlineStore::upsert_section`].
pub fn upsert_outline_section_at(
    data_dir: &Path,
    session: &str,
    chapter_first_message_id: &str,
    section: vault_tools::outline::OutlineSection,
) -> Result<u64> {
    let (store, outline_path) = agent_store_at(data_dir)?;
    let order = session_message_ids(&store, session)?;
    let mut outline = vault_tools::outline::OutlineStore::open(&outline_path)?;
    let revision = outline.upsert_section(session, &order, chapter_first_message_id, section)?;
    outline_titles::refresh(&store, &outline_path, session)?;
    Ok(revision)
}

/// Closes the chapter starting at `first_message_id` in `data_dir`: its last anchor
/// becomes `last_message_id` and it ends at `ended_at` (RFC 3339). Returns the new
/// revision. See [`vault_tools::outline::OutlineStore::close_chapter`].
pub fn close_outline_chapter_at(
    data_dir: &Path,
    session: &str,
    first_message_id: &str,
    last_message_id: &str,
    ended_at: &str,
) -> Result<u64> {
    let (store, outline_path) = agent_store_at(data_dir)?;
    let order = session_message_ids(&store, session)?;
    let mut outline = vault_tools::outline::OutlineStore::open(&outline_path)?;
    let revision =
        outline.close_chapter(session, &order, first_message_id, last_message_id, ended_at)?;
    outline_titles::refresh(&store, &outline_path, session)?;
    Ok(revision)
}

/// The MCP servers of a desktop engine with the Everything set.
struct McpState {
    /// Their client; dropping it stops the servers.
    host: Arc<mcp::McpHost>,
    /// Held through [`Agent::set_mcp`], so applies do not interleave.
    apply: Mutex<()>,
    /// The settings last applied and the host's warnings from that reload.
    applied: Mutex<(mcp::McpSettings, Vec<String>)>,
}

/// The entrypoint surfaces over this engine's tools and the MCP tools as of
/// the last [`Agent::set_mcp`].
struct Surfaces {
    gateway: gateway::GatewayRuntime,
    /// The host relay's surface: the same tools, always native and without JS.
    relay: gateway::GatewayRuntime,
}

impl Surfaces {
    fn new(
        tools: &ToolRegistry,
        execution_mode: bubblepaw_core::ExecutionMode,
        enable_js: bool,
    ) -> Result<Surfaces> {
        Ok(Surfaces {
            gateway: gateway::GatewayRuntime::new(tools, execution_mode, enable_js)?,
            relay: gateway::GatewayRuntime::new(
                tools,
                bubblepaw_core::ExecutionMode::Native,
                false,
            )?,
        })
    }
}

/// The agent: harness, store, code mode and providers, on its own runtime.
pub struct Agent {
    /// None on a hub host or without the Everything set.
    mcp: Option<McpState>,
    /// The tools without MCP's, which [`Agent::set_mcp`] rebuilds the
    /// surfaces over.
    base_tools: ToolRegistry,
    execution_mode: bubblepaw_core::ExecutionMode,
    enable_js: bool,
    /// Swapped whole by [`Agent::set_mcp`]; a call keeps the surfaces it
    /// started with.
    surfaces: RwLock<Arc<Surfaces>>,
    scheduler: Mutex<std::result::Result<Option<Arc<schedules::Scheduler>>, String>>,
    records: Arc<records::Records>,
    ui: Arc<ui::Ui>,
    host_info: HostInfo,
    desktop: Option<Arc<tools::DesktopRuntime>>,
    projects: Option<Arc<projects::Projects>>,
    permission_mode: Mutex<PermissionMode>,
    permission_file: PathBuf,
    harness: Arc<Harness>,
    /// Template for delegated runs, whose registry adds a task-bound tool.
    task_harness_config: HarnessConfig,
    store: Store,
    /// `<data_dir>/agent/chat_outline.sqlite`, the notes-owned chapters and sections.
    outline_path: PathBuf,
    repl: Arc<CodeMode>,
    providers: Arc<ProviderRegistry>,
    extension: Option<Arc<ExtensionProvider>>,
    claude_detection: claude::ClaudeDetection,
    vault: Arc<Vault>,
    /// The configured default (`AgentConfig::default_model`, else
    /// [`DEFAULT_MODEL`]); a hub host prefers the vault's default for `hub`.
    default_model: String,
    /// The runner whose vault default model applies to runs without one:
    /// `hub` on a hub host. Desktop hosts get the model from the app, which
    /// resolves the device override and the vault value itself.
    vault_default_runner: Option<&'static str>,
    system_prompt: String,
    /// Stops each running [`Agent::forward_events`] task.
    forwarders: Mutex<Vec<CancellationToken>>,
    runtime: tokio::runtime::Runtime,
}

impl Agent {
    /// Opens the store and assembles the harness over `vault`.
    pub fn open(vault: Arc<Vault>, config: AgentConfig) -> Result<Agent> {
        Self::open_with_execution(vault, config, bubblepaw_core::ExecutionMode::Js, false)
    }

    /// Opens with an explicit BubblePaw entrypoint surface. Ordinary app callers
    /// keep using `open`, whose default remains JS.
    pub fn open_with_execution(
        vault: Arc<Vault>,
        config: AgentConfig,
        execution_mode: bubblepaw_core::ExecutionMode,
        enable_js: bool,
    ) -> Result<Agent> {
        let counter = AtomicUsize::new(0);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name_fn(move || format!("agent-{}", counter.fetch_add(1, Ordering::Relaxed)))
            .enable_all()
            .build()?;
        let _context = runtime.enter();

        let dir = config.data_dir.join("agent");
        let workspace = dir.join("workspace");
        std::fs::create_dir_all(&workspace)
            .with_context(|| format!("create {}", workspace.display()))?;
        let store = Store::open(dir.join("agent.sqlite"))?;
        let records = Arc::new(records::Records::open(
            &dir.join("agent.sqlite"),
            store.clone(),
        )?);
        let ui = Arc::new(ui::Ui::new());
        store.bind_workspace(&workspace)?;

        let artifacts = Arc::new(ArtifactStore::new(&workspace, dir.join("artifacts"))?);
        let repl = Arc::new(CodeMode::new(workspace.clone(), artifacts)?);

        // This runner's tool set, from the vault, read once per engine start.
        // The conversation tools (js, history_*, chat_read, ui_*) are in every
        // set; desktop and project tools come only with Everything.
        let tool_set = config
            .tool_set
            .unwrap_or_else(|| runner_tool_set(&vault, config.host_kind));
        let mut tools = ToolRegistry::new();
        tools::register_vault_tools(&mut tools, vault.clone(), records.clone(), tool_set)?;
        context::register_tools(&mut tools, store.clone(), records.clone(), vault.clone())?;
        ui::register(&mut tools, ui.clone(), vault.clone(), records.clone())?;
        let permission_file = dir.join("permission-mode.json");
        // Startup configuration is authoritative. Older saved permission files
        // are preserved but cannot restore approvals in the Bypass-only hosts.
        let permission_mode = config.permission_mode;
        let mut desktop = None;
        let mut projects = None;

        let mut pack_names = Vec::new();
        let mut system_prompt = execution_prompt(execution_mode, enable_js);
        for pack in &config.packs {
            match pack {
                ToolPack::Desktop { .. } if tool_set != vault_tools::ToolSet::Everything => {}
                ToolPack::Desktop { root } => {
                    let project_store =
                        Arc::new(projects::Projects::open(dir.join("projects.json"), root)?);

                    let runtime = tools::register_desktop_tools_recorded(
                        &mut tools,
                        project_store.clone(),
                        &workspace,
                        Some((records.clone(), vault.clone())),
                    )?;
                    desktop = Some(runtime);

                    projects = Some(project_store);
                    pack_names.push("desktop".into());
                    system_prompt.push_str(" Desktop tools work in named projects, each with a folder used as cwd. The current project is per session; project_current shows it and project_list lists projects. read, write, edit, search and list stay inside the current project. project_switch changes project; project_open adds a new folder as a project and switches to it. exec defaults to the current project's folder and is not a sandbox.");
                }
            }
        }
        for tool in config.test.extra_tools {
            tools.register_host(Arc::new(tools::RecordedTool::new(
                tool,
                records.clone(),
                vault.clone(),
                false,
            )))?;
        }
        if config.test.sleep_tool {
            tools.register_host(Arc::new(tools::RecordedTool::new(
                Arc::new(testing::SleepTool),
                records.clone(),
                vault.clone(),
                false,
            )))?;
        }
        if config.test.confirm_tool {
            tools.register_host(Arc::new(tools::RecordedTool::new(
                Arc::new(testing::ConfirmTool),
                records.clone(),
                vault.clone(),
                false,
            )))?;
        }
        let mut support_tools = ToolRegistry::new();
        register_history_tools(&mut support_tools, store.clone())?;
        repl.register(&mut support_tools)?;
        for definition in support_tools.capabilities() {
            let name = definition.name;
            if support_tools.is_shown_always(&name) {
                tools.show_always([name.clone()]);
            }
            let tool = support_tools.get_host(&name).expect("support host tool");
            tools.register_host(Arc::new(tools::RecordedTool::new(
                tool,
                records.clone(),
                vault.clone(),
                false,
            )))?;
        }
        for definition in support_tools.definitions() {
            tools.register(
                support_tools
                    .get_model(&definition.name)
                    .expect("support model tool"),
            )?;
        }
        // MCP servers (desktop hosts with the Everything set; see `mcp`).
        // The host exists even without servers, so `set_mcp` can add some.
        // A server that fails is left out and reported by `mcp_status`.
        let mcp_state = if matches!(config.host_kind, HostKind::Desktop)
            && tool_set == vault_tools::ToolSet::Everything
        {
            let host = Arc::new(runtime.block_on(mcp::McpHost::start(
                &mcp::McpSettings::default(),
                &workspace,
            )));
            let applied = match runtime.block_on(host.reload(&config.mcp, None)) {
                Ok(diff) => (config.mcp.clone(), diff.warnings),
                Err(error) => {
                    eprintln!("agent: MCP settings: {error:#}; no MCP servers");
                    (mcp::McpSettings::default(), Vec::new())
                }
            };
            for warning in &applied.1 {
                eprintln!("agent: {warning}");
            }
            Some(McpState {
                host,
                apply: Mutex::new(()),
                applied: Mutex::new(applied),
            })
        } else {
            None
        };
        let base_tools = tools.clone();
        if let Some(state) = &mcp_state {
            for skipped in mcp::register(&mut tools, &state.host, &records, &vault) {
                eprintln!("agent: {skipped}");
            }
        }
        let model_tools = tools
            .with_execution_mode(execution_mode, enable_js)
            .definitions();
        ensure!(
            !model_tools.is_empty(),
            "The agent must expose at least one tool"
        );
        let surfaces = Surfaces::new(&tools, execution_mode, enable_js)?;

        let default_model = if config.default_model.trim().is_empty() {
            DEFAULT_MODEL.to_string()
        } else {
            config.default_model.trim().to_string()
        };
        let codex_home = codex_home();
        let models_cache = codex_home
            .as_ref()
            .map(|home| home.join("models_cache.json"));
        let mut registry = ProviderRegistry::new(
            "codex",
            codex_provider(codex_home.as_deref(), models_cache.as_deref()),
        );
        let claude_detection = claude::detect(
            config.bun_path.as_deref(),
            config.claude_code_dir.as_deref(),
        );
        let extension = match (
            &claude_detection.bun_path,
            &claude_detection.claude_code_dir,
        ) {
            (Some(bun), Some(dir)) if bun.is_file() && dir.is_dir() => {
                let extension = Arc::new(ExtensionProvider::new(ExtensionConfig {
                    id: "claude-code".into(),
                    command: bun.to_string_lossy().into_owned(),
                    args: vec![
                        "run".into(),
                        dir.join("src")
                            .join("main.ts")
                            .to_string_lossy()
                            .into_owned(),
                    ],
                    cwd: Some(dir.clone()),
                    env: Default::default(),
                }));
                registry.insert("claude-code", extension.clone())?;
                Some(extension)
            }
            _ => None,
        };
        if let Some((steps, final_text)) = config.test.scripted {
            registry.insert(
                "scripted",
                Arc::new(ScriptedProvider::with_steps(steps, final_text)),
            )?;
        }
        if replay::enabled() {
            registry.insert(replay::PROVIDER_ID, Arc::new(replay::ReplayProvider))?;
        }
        let providers = Arc::new(registry);

        let mut tool_names: Vec<_> = tools
            .definitions()
            .into_iter()
            .chain(tools.capabilities())
            .map(|definition| definition.name)
            .collect();
        tool_names.sort();
        let host_info = HostInfo {
            permission_mode,
            projects: projects.is_some(),
            kind: match config.host_kind {
                HostKind::Desktop => "desktop",
                HostKind::Hub => "hub",
            }
            .into(),
            packs: pack_names,
            tools: tool_names,
            providers: providers.ids(),
            can_run_tasks: config.host_kind == HostKind::Desktop,
            version: env!("CARGO_PKG_VERSION").into(),
            default_model: Some(default_model.clone()),
        };

        let notes = Arc::new(RuntimeNotes::without_processes(repl.clone()));
        let mut harness_config = HarnessConfig {
            execution_mode,
            enable_js,
            model: default_model.clone(),
            system_prompt: system_prompt.clone(),
            approval: permission_policy(permission_mode),
            // Preserve the embedded agent's previous finite default. BubblePaw
            // now defaults to an unlimited number of model/tool rounds.
            max_rounds: Some(32),
            // chat_read pages contain up to 20,000 Unicode characters.
            // Allow their UTF-8 / JSON representation before js applies its
            // separate output budget (the full value stays available in JS).
            max_tool_result_bytes: 256 * 1024,
            ..HarnessConfig::default()
        };
        // The Codex window is known from Codex's model cache; other models
        // compact at the default cap.
        let window = match (default_model.strip_prefix("codex/"), &models_cache) {
            (Some(model), Some(cache)) => cached_context_window(cache, model),
            _ => None,
        };
        let mut summary = SummarizingContext::new(
            store.clone(),
            providers.clone(),
            SummaryConfig {
                context_window: window,
                max_request_bytes: harness_config.max_request_bytes,
                ..SummaryConfig::default()
            },
            Some(notes),
        )?;
        if let Some(cache) = models_cache {
            // A run with another model drops the default model's window.
            summary = summary.with_model_metadata(default_model.clone(), cache);
        }
        harness_config.context = match (&desktop, &projects) {
            (Some(desktop), Some(projects)) => Arc::new(DesktopContext {
                inner: summary,
                desktop: desktop.clone(),
                projects: projects.clone(),
            }),
            _ => Arc::new(summary),
        };

        if replay::enabled() {
            // A recorded turn can hold ~190 assistant records; see replay::MAX_ROUNDS.
            harness_config.max_rounds = Some(replay::MAX_ROUNDS);
        }
        if let Some(state) = &mcp_state {
            // Each turn sees the servers' current tools: the resolver asks
            // them to refresh (for `tools/list_changed`) and registers what
            // they list now over the other tools.
            let (host, handle) = (state.host.clone(), runtime.handle().clone());
            let (records, vault) = (records.clone(), vault.clone());
            let base_tools = base_tools.clone();
            harness_config.registry_resolver = Some(Arc::new(move || {
                let refresh = host.clone();
                handle.spawn(async move {
                    refresh.refresh_tools().await;
                });
                let mut registry = base_tools.clone();
                mcp::register(&mut registry, &host, &records, &vault);
                registry
            }));
        }
        let harness = Arc::new(Harness::new(
            store.clone(),
            providers.clone(),
            tools,
            harness_config.clone(),
        ));
        drop(_context);
        Ok(Agent {
            mcp: mcp_state,
            base_tools,
            execution_mode,
            enable_js,
            surfaces: RwLock::new(Arc::new(surfaces)),
            scheduler: Mutex::new(Ok(None)),
            records,
            ui,
            host_info,
            desktop,
            projects,
            permission_mode: Mutex::new(permission_mode),
            permission_file,
            harness,
            task_harness_config: harness_config,
            store,
            outline_path: dir.join(vault_tools::outline::FILE_NAME),
            repl,
            providers,
            extension,
            claude_detection,
            vault,
            default_model,
            vault_default_runner: (config.host_kind == HostKind::Hub)
                .then_some(vault_core::AGENT_RUNNER_HUB),
            system_prompt,
            forwarders: Mutex::new(Vec::new()),
            runtime,
        })
    }

    pub fn vault(&self) -> &Arc<Vault> {
        &self.vault
    }

    /// Replaces the MCP servers (see [`mcp`]): only changed servers restart,
    /// and chats, the gateway and the relay see the new tools at once.
    /// Invalid settings are an error and change nothing. A no-op without MCP
    /// (a hub host, or a tool set other than Everything).
    pub fn set_mcp(&self, settings: mcp::McpSettings) -> Result<()> {
        let Some(state) = &self.mcp else {
            return Ok(());
        };
        let _apply = state.apply.lock().unwrap();
        let diff = self.runtime.block_on(state.host.reload(&settings, None))?;
        for warning in &diff.warnings {
            eprintln!("agent: {warning}");
        }
        *state.applied.lock().unwrap() = (settings, diff.warnings);
        let mut tools = self.base_tools.clone();
        for skipped in mcp::register(&mut tools, &state.host, &self.records, &self.vault) {
            eprintln!("agent: {skipped}");
        }
        let surfaces = Surfaces::new(&tools, self.execution_mode, self.enable_js)?;
        *self.surfaces.write().unwrap() = Arc::new(surfaces);
        Ok(())
    }

    /// Each configured MCP server's state, in name order. Empty without MCP.
    pub fn mcp_status(&self) -> Vec<mcp::ServerStatus> {
        let Some(state) = &self.mcp else {
            return Vec::new();
        };
        let (settings, warnings) = state.applied.lock().unwrap().clone();
        let tools: Vec<String> = state
            .host
            .tools()
            .into_iter()
            .map(|tool| tool.definition().name)
            .collect();
        mcp::status(&settings, &warnings, &tools)
    }

    /// The gateway and relay surfaces as of now.
    fn surfaces(&self) -> Arc<Surfaces> {
        self.surfaces.read().unwrap().clone()
    }

    /// Attaches the host's provider-independent schedule engine.
    pub fn set_scheduler(&self, scheduler: Arc<schedules::Scheduler>) {
        *self.scheduler.lock().unwrap() = Ok(Some(scheduler));
    }

    /// Preserves a host startup failure for RPC clients without disabling chats.
    pub fn set_scheduler_error(&self, error: String) {
        *self.scheduler.lock().unwrap() = Err(error);
    }

    pub fn scheduler(&self) -> Result<Arc<schedules::Scheduler>> {
        self.scheduler
            .lock()
            .unwrap()
            .clone()
            .map_err(anyhow::Error::msg)?
            .ok_or_else(|| anyhow::anyhow!("schedule runner is not attached to this host"))
    }

    /// Capabilities assembled at open, without starting providers or contacting models.
    pub fn host_info(&self) -> HostInfo {
        let mut info = self.host_info.clone();
        info.permission_mode = *self.permission_mode.lock().unwrap();
        info
    }

    /// The model of runs that name none. On a hub: the vault's default
    /// model for `hub`, else the configured `[agent] default_model`, else
    /// [`DEFAULT_MODEL`]. On a desktop host: the configured default.
    pub fn default_model(&self) -> String {
        self.vault_default_model()
            .unwrap_or_else(|| self.default_model.clone())
    }

    fn vault_default_model(&self) -> Option<String> {
        let runner = self.vault_default_runner?;
        match self.vault.agent_defaults() {
            Ok(defaults) => defaults.model_for(runner).map(str::to_owned),
            Err(error) => {
                eprintln!("agent defaults unreadable: {error}");
                None
            }
        }
    }

    pub fn claude_detection(&self) -> &claude::ClaudeDetection {
        &self.claude_detection
    }

    /// Every provider's models as `provider/model`, the default model first.
    /// Starts the Claude Code extension on first call so its models are known.
    pub fn models(&self) -> Vec<ModelInfo> {
        if let Some(extension) = &self.extension {
            if extension.models().is_empty() {
                let _ = self.runtime.block_on(async {
                    tokio::time::timeout(Duration::from_secs(30), extension.start()).await
                });
            }
        }
        let mut models = self.providers.models();
        let default_model = self.default_model();
        if !models.iter().any(|m| m.id == default_model) {
            models.insert(
                0,
                ModelInfo {
                    id: default_model.clone(),
                    name: default_model,
                },
            );
        }
        models
    }

    fn run_prompt(&self, session: &str) -> Result<String> {
        let mut prompt = self.system_prompt.clone();
        if let Some(projects) = &self.projects {
            let current = projects.ensure_session(session)?;
            prompt.push_str(&format!(
                " Current session project: {}. Folder / cwd: {}.",
                serde_json::to_string(&current.name)?,
                serde_json::to_string(&current.path)?
            ));
        }
        Ok(prompt)
    }
    fn current_policy(&self) -> ApprovalPolicy {
        permission_policy(*self.permission_mode.lock().unwrap())
    }
    /// Changes the agent-wide mode; already-running runs retain their captured policy.
    /// Startup uses the explicit config, preserving but ignoring this saved snapshot.
    pub fn permission_mode_set(&self, mode: PermissionMode) -> Result<PermissionMode> {
        let mut current = self.permission_mode.lock().unwrap();
        projects::save_json(&self.permission_file, &mode)?;
        *current = mode;
        Ok(mode)
    }
    fn project_store(&self) -> Result<&Arc<projects::Projects>> {
        self.projects
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("projects are not supported on this host"))
    }
    pub fn projects_list(&self) -> Result<Vec<Project>> {
        Ok(self.project_store()?.list())
    }
    pub fn project_add(&self, name: &str, path: &str) -> Result<Project> {
        self.project_store()?.add(name, path)
    }
    pub fn project_remove(&self, id: &str) -> Result<()> {
        self.project_store()?.remove(id)
    }
    pub fn project_rename(&self, id: &str, name: &str) -> Result<Project> {
        self.project_store()?.rename(id, name)
    }
    pub fn session_project_set(
        &self,
        session_id: &str,
        project_id: &str,
    ) -> Result<notes_agent_wire::AgentSession> {
        let projects = self.project_store()?;
        let session = self.store.session(session_id)?;
        projects.switch(session_id, project_id)?;
        self.session_dto(session)
    }
    pub fn session_dto(&self, session: Session) -> Result<notes_agent_wire::AgentSession> {
        let project = self
            .projects
            .as_ref()
            .map(|projects| projects.ensure_session(&session.id))
            .transpose()?;
        let mut dto = wire::session(session);
        dto.project_id = project.as_ref().map(|project| project.id.clone());
        dto.project_name = project.map(|project| project.name);
        Ok(dto)
    }

    /// Newest first.
    pub fn sessions(&self) -> Result<Vec<Session>> {
        for session in self.store.list_sessions()? {
            // One unreadable outline must not hide the other chats.
            if let Err(error) =
                outline_titles::refresh(&self.store, &self.outline_path, &session.id)
            {
                eprintln!("agent automatic title: {error}");
            }
        }
        self.store.list_sessions()
    }

    /// The session's outline: the notes-owned one when saved, else BubblePaw's context
    /// chapters. See [`read_session_outline`]. Errors for an unknown session.
    pub fn session_outline(&self, session: &str) -> Result<vault_tools::outline::ChatOutline> {
        outline_titles::refresh(&self.store, &self.outline_path, session)?;
        read_session_outline(&self.store, &self.outline_path, session)
    }

    pub fn create_session(&self, title: &str) -> Result<Session> {
        let session = self.store.create_session(title)?;
        if let Some(projects) = &self.projects {
            projects.ensure_session(&session.id)?;
        }
        Ok(session)
    }

    pub fn rename_session(&self, id: &str, title: &str) -> Result<Session> {
        outline_titles::rename_user(&self.store, &self.outline_path, id, title)
    }

    /// Deletes the session's history and drops the providers' mirrors of it.
    pub fn delete_session(&self, id: &str) -> Result<()> {
        self.runtime.block_on(self.harness.delete_session(id))?;
        self.repl.cancel_session(id);
        replay::forget(id);
        self.records.forget_session(id)?;
        if let Some(projects) = &self.projects {
            projects.forget_session(id)?;
        }
        Ok(())
    }

    pub fn session_state(&self, id: &str) -> Result<SessionState> {
        outline_titles::refresh(&self.store, &self.outline_path, id)?;
        let (state, position) = self.harness.snapshot_with(id, || {
            Ok(SessionState {
                session: self.store.session(id)?,
                messages: app_messages(self.store.messages(id)?),
                runs: self.store.runs(id)?,
                pending_approvals: self.harness.pending_approvals(id)?,
                position: LogPosition::default(),
                activity: self.records.activity(id)?,
                edits: self.records.edits(id)?,
                project: self
                    .projects
                    .as_ref()
                    .map(|projects| projects.ensure_session(id))
                    .transpose()?,
            })
        })?;
        Ok(SessionState { position, ..state })
    }

    /// Starts a run, or queues a follow-up when the session is busy.
    pub fn send(&self, session: &str, text: &str, model: Option<String>) -> Result<Sent> {
        self.send_as(session, text, model, desktop_client())
    }

    /// [`Agent::send`] on behalf of `client` (the events' origin).
    pub fn send_as(
        &self,
        session: &str,
        text: &str,
        model: Option<String>,
        client: ClientInfo,
    ) -> Result<Sent> {
        self.send_as_with_context(session, text, model, None, client)
    }

    pub fn send_with_context(
        &self,
        session: &str,
        text: &str,
        model: Option<String>,
        context: Option<notes_agent_wire::MessageContext>,
    ) -> Result<Sent> {
        self.send_as_with_context(session, text, model, context, desktop_client())
    }
    pub fn send_as_with_context(
        &self,
        session: &str,
        text: &str,
        model: Option<String>,
        context: Option<notes_agent_wire::MessageContext>,
        client: ClientInfo,
    ) -> Result<Sent> {
        let text = context::encode(text, context, &self.store)?;
        let model = model
            .filter(|m| !m.trim().is_empty())
            .or_else(|| self.vault_default_model());
        let model = self.replay_model(session, model)?;
        let _context = self.runtime.enter();
        let options = RunOptions {
            approval: Some(self.current_policy()),
            system_prompt: Some(self.run_prompt(session)?),
            model: model.filter(|m| !m.trim().is_empty()),
            client: Some(client.clone()),
            ..RunOptions::default()
        };
        match self.harness.start(session, &text, options) {
            Ok(run) => Ok(Sent::Started(run)),
            Err(error) if error.downcast_ref::<SessionBusy>().is_some() => Ok(Sent::FollowUp(
                self.harness.followup_from(session, &text, Some(client))?,
            )),
            Err(error) => Err(error),
        }
    }

    /// A session with a loaded dev replay always runs on the replay model, never on
    /// the caller's choice or the default model, so no real model is contacted.
    fn replay_model(&self, session: &str, model: Option<String>) -> Result<Option<String>> {
        if !replay::is_loaded(session) {
            return Ok(model);
        }
        ensure!(
            self.providers.get(replay::PROVIDER_ID).is_some(),
            "dev replay is loaded but its provider is not registered"
        );
        Ok(Some(replay::MODEL_ID.into()))
    }

    pub fn steer(&self, session: &str, text: &str) -> Result<QueuedInput> {
        self.steer_as(session, text, desktop_client())
    }

    pub fn steer_as(&self, session: &str, text: &str, client: ClientInfo) -> Result<QueuedInput> {
        let _context = self.runtime.enter();
        self.harness.steer_from(session, text, Some(client))
    }

    pub fn cancel(&self, session: &str) -> bool {
        self.cancel_as(session, desktop_client())
    }

    pub fn cancel_as(&self, session: &str, client: ClientInfo) -> bool {
        let _context = self.runtime.enter();
        self.harness.cancel_from(session, Some(client))
    }

    /// `decision` is `approve`, `approve_session` or `deny`.
    pub fn decide_approval(
        &self,
        approval_id: &str,
        decision: &str,
        reason: Option<String>,
    ) -> Result<ApprovalRecord> {
        self.decide_approval_as(approval_id, decision, reason, desktop_client())
    }

    pub fn decide_approval_as(
        &self,
        approval_id: &str,
        decision: &str,
        reason: Option<String>,
        client: ClientInfo,
    ) -> Result<ApprovalRecord> {
        let _context = self.runtime.enter();
        self.harness.decide_from(
            approval_id,
            Decision::parse(decision, reason)?,
            Some(client),
        )
    }

    /// Durable events after `after`, optionally of one session.
    pub fn events_after(&self, after: u64, session: Option<&str>) -> Result<EventsPage> {
        Ok(EventsPage {
            events: self
                .harness
                .events_after(after, session, EVENTS_AFTER_LIMIT)?,
            last_seq: self.harness.last_sequence(),
            resync: !self.harness.can_resume_from(after),
        })
    }

    /// Forwards every harness event to `deliver` until it returns false or
    /// the returned token (or [`Agent::close_event_streams`]) stops it; then
    /// `deliver` is dropped at once, without waiting for another event.
    /// Falling behind sends one `Gap` with the last durable sequence delivered.
    pub fn forward_events<F>(&self, mut deliver: F) -> CancellationToken
    where
        F: FnMut(Forwarded) -> bool + Send + 'static,
    {
        let mut events = self.harness.subscribe();
        let mut actions = self.ui.actions.subscribe();
        let mut delivered = self.harness.last_sequence();
        let stop = CancellationToken::new();
        {
            let mut forwarders = self.forwarders.lock().unwrap();
            forwarders.retain(|token| !token.is_cancelled());
            forwarders.push(stop.clone());
        }
        let stopped = stop.clone();
        self.runtime.spawn(async move {
            use tokio::sync::broadcast::error::RecvError;
            loop {
                let received = tokio::select! {
                    biased;
                    _ = stopped.cancelled() => break,
                    action = actions.recv() => {
                        match action {Ok(action)=>if !deliver(Forwarded::UiAction(action)){break},Err(RecvError::Closed)=>break,Err(RecvError::Lagged(_))=>{}}
                        continue;
                    },
                    received = events.recv() => received,
                };
                match received {
                    Ok(event) => {
                        let (durable, sequence) = (event.durable, event.sequence);
                        if !deliver(Forwarded::Event(Box::new(event))) {
                            break;
                        }
                        if durable {
                            delivered = delivered.max(sequence);
                        }
                    }
                    Err(RecvError::Lagged(skipped)) => {
                        if !deliver(Forwarded::Gap {
                            skipped,
                            after: delivered,
                        }) {
                            break;
                        }
                    }
                    Err(RecvError::Closed) => break,
                }
            }
            stopped.cancel();
            // Ending the task drops `deliver` here.
        });
        stop
    }

    /// Marks one explicitly displayed chat until this listener is cancelled.
    pub fn forward_events_for_session<F>(
        &self,
        session: String,
        mut deliver: F,
    ) -> Result<CancellationToken>
    where
        F: FnMut(Forwarded) -> bool + Send + 'static,
    {
        self.store.session(&session)?;
        let displayed = session.clone();
        let token = self.forward_events(move |event| {
            if let Forwarded::UiAction(action) = &event {
                if action.session_id != displayed {
                    return true;
                }
            }
            deliver(event)
        });
        self.ui.viewing(session, token.clone());
        Ok(token)
    }
    pub(crate) fn mark_viewing(&self, session: String, token: CancellationToken) -> Result<()> {
        self.store.session(&session)?;
        self.ui.viewing(session, token);
        Ok(())
    }
    pub fn edit_undo(&self, edit_id: &str) -> Result<notes_agent_wire::AgentEdit> {
        self.records.undo(&self.vault, edit_id)
    }

    /// Stops every [`Agent::forward_events`] task now; each drops its
    /// `deliver` (for the bridge, its stream sink, which ends the Dart stream).
    pub fn close_event_streams(&self) {
        for token in self.forwarders.lock().unwrap().drain(..) {
            token.cancel();
        }
    }

    /// Runs `code` as a `js` cell without a model, and waits for it.
    pub fn run_script(&self, session: &str, label: &str, code: &str) -> Result<Run> {
        self.run_script_as(session, label, code, desktop_client())
    }

    pub fn run_script_as(
        &self,
        session: &str,
        label: &str,
        code: &str,
        client: ClientInfo,
    ) -> Result<Run> {
        self.runtime.block_on(async {
            self.harness.start(
                session,
                label,
                RunOptions {
                    approval: Some(self.current_policy()),
                    system_prompt: Some(self.run_prompt(session)?),
                    script: Some(code.into()),
                    client: Some(client),
                    ..RunOptions::default()
                },
            )?;
            self.harness.wait_for_idle(session).await
        })
    }

    /// Starts a run with the given text and waits until the session is idle.
    pub(crate) fn run_and_wait(&self, session: &str, text: &str) -> Result<Run> {
        self.runtime.block_on(async {
            self.harness.start(
                session,
                text,
                RunOptions {
                    approval: Some(self.current_policy()),
                    system_prompt: Some(self.run_prompt(session)?),
                    client: Some(desktop_client()),
                    ..RunOptions::default()
                },
            )?;
            self.harness.wait_for_idle(session).await
        })
    }

    /// Stops runs and code environments. Also done on drop.
    pub fn shutdown(&self) {
        self.close_event_streams();
        self.runtime.block_on(async {
            let _ = self.harness.shutdown(Duration::from_secs(5)).await;
            if let Some(desktop) = &self.desktop {
                desktop.shutdown().await;
            }
            self.repl.shutdown().await;
        });
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        // Blocking on the runtime is impossible from inside a runtime.
        if tokio::runtime::Handle::try_current().is_err() {
            self.shutdown();
        }
    }
}

const SYSTEM_PROMPT: &str = concat!(
    "You are the notes agent in the user's personal notes app. ",
    "You read and edit their notes vault through the js tool: vault actions are async tools.* functions there. ",
    "Use search_notes to find notes pages and database rows, read_page to read them, and create_page to create a note. ",
    "On desktop, tools.search is the file search; tools.search_notes searches notes. ",
    "Edits are versioned and sync to the user's other devices. ",
    "Use search_notes to find dated tasks/events, update_task to change completion (done) or move an item (due), and daily_note to open or create today's dated note. ",
    "Use ui_open_page or ui_highlight when showing a page or changed blocks would help the user; do not open a page after every read. These actions require an app viewing this chat. ",
    "Link pages in your replies using [title](page:<id>) so the user can open them. ",
    "Batch independent, likely-needed reads in one cell; use Promise.all for independent calls. ",
    "Keep dependent steps and edits ordered. Take another model turn when observations determine the next step. ",
    "Return compact labeled observations for your own inspection and keep reusable data in state. ",
    "Correct failures using the returned diagnostics; never repeat an edit with an unknown outcome. ",
    "The js tool description defines the API, state lifecycle and limits."
);

/// BubblePaw records runtime notices (`[host notice] ...`) as generated user
/// messages. They are model context, not user-facing: the app never sees them.
pub(crate) fn app_messages(mut messages: Vec<Message>) -> Vec<Message> {
    messages.retain(|message| !is_runtime_notice(message));
    messages
}

pub(crate) fn is_runtime_notice(message: &Message) -> bool {
    message.role == bubblepaw_core::Role::User
        && message.origin == Some(bubblepaw_core::MessageOrigin::Generated)
}

fn execution_prompt(mode: bubblepaw_core::ExecutionMode, enable_js: bool) -> String {
    if mode == bubblepaw_core::ExecutionMode::Js {
        return SYSTEM_PROMPT.to_owned();
    }
    let mut prompt = SYSTEM_PROMPT
        .replace(
            "through the js tool: vault actions are async tools.* functions there.",
            "through the advertised native tools, using their declared argument schemas.",
        )
        .replace("tools.search is", "search is")
        .replace("tools.search_notes searches", "search_notes searches")
        .replace(
            "Batch independent, likely-needed reads in one cell; use Promise.all for independent calls.",
            "Batch independent, likely-needed native reads in one tool round.",
        )
        .replace(
            "Return compact labeled observations for your own inspection and keep reusable data in state.",
            "Keep observations compact and labeled for your own inspection.",
        )
        .replace(
            "The js tool description defines the API, state lifecycle and limits.",
            "The advertised tool descriptions define the API and limits.",
        );
    if enable_js {
        prompt.push_str(" The optional js tool composes async tools.* calls; its description defines its API, state lifecycle and limits.");
    }
    prompt
}

fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(|home| PathBuf::from(home).join(".codex"))
        })
}

/// Codex over Responses with the Codex sign-in, as the runner's `--provider
/// codex` builds it. Without a readable sign-in, a provider that reports it,
/// so the agent still opens (sessions, scripts, other providers).
fn codex_provider(home: Option<&Path>, models_cache: Option<&Path>) -> Arc<dyn Provider> {
    let built = home
        .context("no Codex home directory")
        .and_then(|home| ResponsesProvider::codex_auth(home.join("auth.json"), CODEX_URL));
    match built {
        Ok(provider) => Arc::new(provider.with_models(codex_models(models_cache))),
        Err(error) => Arc::new(Unavailable(format!("Codex sign-in unavailable: {error:#}"))),
    }
}

/// Model slugs from Codex's model cache, else the default model.
fn codex_models(cache: Option<&Path>) -> Vec<String> {
    let slugs: Vec<String> = cache
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|cache| {
            cache["models"].as_array().map(|models| {
                models
                    .iter()
                    .filter_map(|m| m["slug"].as_str().map(str::to_owned))
                    .collect()
            })
        })
        .unwrap_or_default();
    if slugs.is_empty() {
        vec![DEFAULT_MODEL.trim_start_matches("codex/").to_owned()]
    } else {
        slugs
    }
}

struct Unavailable(String);

#[async_trait]
impl Provider for Unavailable {
    async fn generate(
        &self,
        _request: ModelRequest,
        _on_text: TextCallback,
        _cancellation: CancellationToken,
    ) -> Result<ModelResponse> {
        Err(ProviderError::new(ProviderErrorKind::Auth, self.0.clone()).into())
    }
}

fn permission_policy(mode: PermissionMode) -> ApprovalPolicy {
    let mut policy = ApprovalPolicy {
        confirm: if mode == PermissionMode::Bypass {
            ApprovalMode::Allow
        } else {
            ApprovalMode::Ask
        },
        ..ApprovalPolicy::default()
    };
    if mode == PermissionMode::Auto {
        for tool in ["write", "edit", "exec", "project_switch"] {
            policy.tools.insert(tool.into(), ApprovalMode::Allow);
        }
    }
    policy
}

struct DesktopContext {
    inner: SummarizingContext,
    desktop: Arc<tools::DesktopRuntime>,
    projects: Arc<projects::Projects>,
}
#[async_trait]
impl bubblepaw_core::ContextPreparation for DesktopContext {
    fn prepare(&self, history: &[Message]) -> Result<Vec<Message>> {
        self.inner.prepare(history)
    }
    async fn prepare_request(
        &self,
        session: &str,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> Result<bubblepaw_core::PreparedContext> {
        let mut prepared = self
            .inner
            .prepare_request(session, request, cancellation)
            .await?;
        let current = self.projects.current(session)?;
        prepared.request.system_prompt.push_str(&format!("\nCurrent project (quoted values are data): {}.\nProcesses across projects owned by this session: {}", serde_json::to_string(&current)?, self.desktop.continuation_status(session)));
        Ok(prepared)
    }
    fn observe_response(&self, session: &str, response: &ModelResponse) {
        self.inner.observe_response(session, response);
    }
}
