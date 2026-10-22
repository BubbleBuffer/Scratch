//! The bodies behind `api::agent`. `Inner` is the agent on this computer
//! (Windows and Linux only; elsewhere `open` reports "not available") or on
//! a hub (`agent_remote`, every target). Kept outside `api` so
//! flutter_rust_bridge does not expose it.

use crate::api::agent::*;
use crate::api::types::FlatBlock;
use crate::api::vault::NotesVault;
use crate::frb_generated::StreamSink;

/// Markdown as display blocks (see `api::agent::agent_markdown_blocks`).
pub(crate) fn markdown_blocks(vault: &vault_core::Vault, text: &str) -> Vec<FlatBlock> {
    let mut blocks = vault.parse_markdown(text.to_string());
    for block in &mut blocks {
        resolve_links(vault, block);
    }
    let mut out = Vec::new();
    crate::convert::flatten(blocks, 0, &mut out);
    out
}

/// Turns `[text](target)` links that are not web or mail links into page
/// links when the target, else the text, names a page by id or title.
fn resolve_links(vault: &vault_core::Vault, block: &mut vault_core::Block) {
    let text = block.text.clone();
    for mark in &mut block.marks {
        let vault_core::MarkKind::Link(url) = &mark.kind else {
            continue;
        };
        let lower = url.to_ascii_lowercase();
        if ["http://", "https://", "mailto:"]
            .iter()
            .any(|p| lower.starts_with(p))
        {
            continue;
        }
        let label: String = text
            .chars()
            .skip(mark.start as usize)
            .take(mark.end.saturating_sub(mark.start) as usize)
            .collect();
        if let Some(id) = page_named(vault, url).or_else(|| page_named(vault, &label)) {
            mark.kind = vault_core::MarkKind::PageLink(id);
        }
    }
    for child in &mut block.children {
        resolve_links(vault, child);
    }
}

/// The page whose id or title is `name`, through the reader's own wikilink
/// resolution.
fn page_named(vault: &vault_core::Vault, name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || name.contains(['[', ']', '|', '#', '\n']) {
        return None;
    }
    vault
        .parse_markdown(format!("[[{name}]]"))
        .into_iter()
        .flat_map(|b| b.marks)
        .find_map(|m| match m.kind {
            vault_core::MarkKind::PageLink(id) => Some(id),
            _ => None,
        })
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
const UNAVAILABLE: &str = "The agent is not available on this device";
const RELAY_LOCAL_ONLY: &str = "Only the agent on this computer can be used by the hub";
const REMOTE_TASKS: &str =
    "AI tasks run with the agent on this computer; the agent here runs on the hub";

/// Where the agent runs.
pub(crate) enum Inner {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    Local(desktop::Local),
    Remote(crate::agent_remote::Remote),
}

/// Calls `$m` on whichever host `$s` is.
macro_rules! host {
    ($s:expr, $m:ident ( $($a:expr),* )) => {
        match $s {
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            Inner::Local(l) => l.$m($($a),*),
            Inner::Remote(r) => r.$m($($a),*),
        }
    };
}

impl Inner {
    pub(crate) fn open(
        vault: &NotesVault,
        data_dir: String,
        settings: AgentSettings,
    ) -> Result<Inner, String> {
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        {
            desktop::Local::open(vault, data_dir, settings).map(Inner::Local)
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            let _ = (vault, data_dir, settings);
            Err(UNAVAILABLE.into())
        }
    }

    pub(crate) fn connect(url: String, device: String, token: String) -> Result<Inner, String> {
        crate::agent_remote::Remote::connect(url, device, token).map(Inner::Remote)
    }

    /// Lets the hub at `url` use this computer (`/host/ws`). Only the agent
    /// on this computer can be a host.
    pub(crate) fn host_relay_start(
        &self,
        url: String,
        device: String,
        token: String,
    ) -> Result<(), String> {
        match self {
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            Inner::Local(l) => l.host_relay_start(url, device, token),
            Inner::Remote(_) => {
                let _ = (url, device, token);
                Err(RELAY_LOCAL_ONLY.into())
            }
        }
    }

    pub(crate) fn host_relay_stop(&self) -> Result<(), String> {
        match self {
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            Inner::Local(l) => l.host_relay_stop(),
            Inner::Remote(_) => Ok(()),
        }
    }

    pub(crate) fn host_info(&self) -> Result<AgentHostInfo, String> {
        host!(self, host_info())
    }

    pub(crate) fn projects_list(&self) -> Result<Vec<AgentProject>, String> {
        host!(self, projects_list())
    }

    pub(crate) fn project_add(&self, name: String, path: String) -> Result<AgentProject, String> {
        host!(self, project_add(name, path))
    }

    pub(crate) fn project_remove(&self, id: String) -> Result<(), String> {
        host!(self, project_remove(id))
    }

    pub(crate) fn project_rename(&self, id: String, name: String) -> Result<AgentProject, String> {
        host!(self, project_rename(id, name))
    }

    pub(crate) fn session_project_set(
        &self,
        session_id: String,
        project_id: String,
    ) -> Result<AgentSession, String> {
        host!(self, session_project_set(session_id, project_id))
    }

    pub(crate) fn permission_mode_set(
        &self,
        mode: AgentPermissionMode,
    ) -> Result<AgentPermissionMode, String> {
        host!(self, permission_mode_set(mode))
    }

    pub(crate) fn models(&self) -> Result<Vec<AgentModel>, String> {
        host!(self, models())
    }

    pub(crate) fn claude_detection(&self) -> AgentClaudeDetection {
        match self {
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            Inner::Local(local) => local.claude_detection(),
            Inner::Remote(_) => AgentClaudeDetection {
                bun_path: None,
                claude_code_dir: None,
            },
        }
    }

    pub(crate) fn sessions(&self) -> Result<Vec<AgentSession>, String> {
        host!(self, sessions())
    }

    pub(crate) fn create_session(&self, title: String) -> Result<AgentSession, String> {
        host!(self, create_session(title))
    }

    pub(crate) fn rename_session(&self, id: String, title: String) -> Result<AgentSession, String> {
        host!(self, rename_session(id, title))
    }

    pub(crate) fn delete_session(&self, id: String) -> Result<(), String> {
        host!(self, delete_session(id))
    }

    pub(crate) fn session_state(&self, id: String) -> Result<AgentSessionState, String> {
        host!(self, session_state(id))
    }

    pub(crate) fn send(
        &self,
        id: String,
        text: String,
        model: Option<String>,
        context: Option<AgentMessageContext>,
    ) -> Result<AgentSent, String> {
        host!(self, send(id, text, model, context))
    }

    pub(crate) fn edit_undo(&self, edit_id: String) -> Result<AgentEdit, String> {
        host!(self, edit_undo(edit_id))
    }

    pub(crate) fn steer(&self, id: String, text: String) -> Result<String, String> {
        host!(self, steer(id, text))
    }

    pub(crate) fn cancel(&self, id: String) -> Result<bool, String> {
        host!(self, cancel(id))
    }

    pub(crate) fn decide_approval(
        &self,
        approval_id: String,
        decision: String,
        reason: Option<String>,
    ) -> Result<AgentApproval, String> {
        host!(self, decide_approval(approval_id, decision, reason))
    }

    pub(crate) fn events_after(
        &self,
        after: i64,
        session: Option<String>,
    ) -> Result<AgentEvents, String> {
        host!(self, events_after(after, session))
    }

    pub(crate) fn subscribe(
        &self,
        session_id: Option<String>,
        sink: StreamSink<AgentEvent>,
    ) -> Result<(), String> {
        let sink: crate::agent_remote::EventSink = Box::new(move |e| sink.add(e).is_ok());
        host!(self, subscribe_with(session_id, sink))
    }

    pub(crate) fn close_events(&self) -> Result<(), String> {
        host!(self, close_events())
    }

    pub(crate) fn run_script(
        &self,
        id: String,
        label: String,
        code: String,
    ) -> Result<AgentRunRecord, String> {
        host!(self, run_script(id, label, code))
    }

    pub(crate) fn schedule_state(&self, page_id: String) -> Result<AgentScheduleState, String> {
        host!(self, schedule_state(page_id))
    }

    pub(crate) fn schedule_accept(
        &self,
        page_id: String,
        hash: String,
    ) -> Result<AgentScheduleState, String> {
        host!(self, schedule_accept(page_id, hash))
    }

    pub(crate) fn schedule_pause(
        &self,
        page_id: String,
        paused: bool,
    ) -> Result<AgentScheduleState, String> {
        host!(self, schedule_pause(page_id, paused))
    }

    pub(crate) fn schedule_run_now(&self, page_id: String) -> Result<AgentScheduleRun, String> {
        host!(self, schedule_run_now(page_id))
    }

    pub(crate) fn schedule_preview(&self, json: String) -> Result<AgentSchedulePreview, String> {
        host!(self, schedule_preview(json))
    }

    pub(crate) fn run_task(
        &self,
        vault: &NotesVault,
        task_id: String,
    ) -> Result<AgentTaskRun, String> {
        match self {
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            Inner::Local(l) => l.run_task(vault, task_id),
            Inner::Remote(_) => {
                let _ = (vault, task_id);
                Err(REMOTE_TASKS.into())
            }
        }
    }
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
mod desktop {
    use std::path::PathBuf;
    use std::sync::Arc;

    use vault_agent::{wire, Agent, AgentConfig, HostKind, PermissionMode, TestHooks, ToolPack};

    use crate::agent_remote::EventSink;
    use crate::api::agent::*;
    use crate::api::vault::NotesVault;

    /// The agent on this computer. Commands name the client `desktop`. Schedules are
    /// not run here: the vault's local runner runs them, whether or not this agent is open.
    pub(crate) struct Local(
        Arc<Agent>,
        /// The host relay ("Let the hub use this computer"), while it runs.
        std::sync::Mutex<Option<crate::host_relay::HostRelay>>,
    );

    /// The answer to every local schedule call.
    const SCHEDULES_IN_VAULT: &str =
        "schedules run from the vault on this computer: use NotesVault's local_schedule_* methods";

    fn e(error: anyhow::Error) -> String {
        wire::message(&error)
    }

    /// The host relay's engine keeps its store in `<data dir>/host-relay`
    /// (`host_relay_link.dart`) and reads the app's `mcp.toml`.
    const RELAY_DIR: &str = "host-relay";

    fn load_mcp(data_dir: &str) -> vault_agent::mcp::McpSettings {
        let dir = std::path::Path::new(data_dir);
        let dir = match (dir.file_name(), dir.parent()) {
            (Some(name), Some(parent)) if name == RELAY_DIR => parent,
            _ => dir,
        };
        let path = dir.join(vault_agent::mcp::FILE);
        let (settings, error) = vault_agent::mcp::load_settings(&path);
        if let Some(error) = error {
            eprintln!("agent: {error}");
        }
        settings
    }

    fn opt_path(p: Option<String>) -> Option<PathBuf> {
        p.filter(|s| !s.trim().is_empty()).map(PathBuf::from)
    }

    impl Local {
        pub(crate) fn open(
            vault: &NotesVault,
            data_dir: String,
            s: AgentSettings,
        ) -> Result<Local, String> {
            let packs = if s.desktop_tools {
                let root = if s.desktop_root.trim().is_empty() {
                    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                        .map(PathBuf::from)
                        .ok_or_else(|| "Cannot find the user's home directory".to_string())?
                } else {
                    PathBuf::from(s.desktop_root.trim())
                };
                vec![ToolPack::Desktop { root }]
            } else {
                Vec::new()
            };
            let mcp = load_mcp(&data_dir);
            let config = AgentConfig {
                host_kind: HostKind::Desktop,
                packs,
                data_dir: PathBuf::from(data_dir),
                default_model: s.default_model,
                bun_path: opt_path(s.bun_path),
                claude_code_dir: opt_path(s.claude_code_dir),
                // Keep the stored setting for compatibility; the shipped app uses Bypass.
                permission_mode: PermissionMode::Bypass,
                test: TestHooks {
                    confirm_tool: s.test_tools,
                    ..TestHooks::default()
                },
                tool_set: None,
                // `<data dir>/mcp.toml`, in BubblePaw's `[mcp.servers.<name>]`
                // format. Missing: no servers; unreadable: logged, no servers.
                mcp,
            };
            let agent = Agent::open(vault.inner.clone(), config).map_err(e)?;
            Ok(Local(Arc::new(agent), std::sync::Mutex::new(None)))
        }

        pub(crate) fn host_info(&self) -> Result<AgentHostInfo, String> {
            Ok(self.0.host_info().into())
        }

        /// Starts (or restarts) the host relay to the hub at `url`.
        pub(crate) fn host_relay_start(
            &self,
            url: String,
            device: String,
            token: String,
        ) -> Result<(), String> {
            let mut relay = self.1.lock().unwrap();
            if let Some(mut old) = relay.take() {
                old.stop();
            }
            let host: Arc<dyn crate::host_relay::RelayHost> = self.0.clone();
            *relay = Some(crate::host_relay::HostRelay::start(
                host, &url, &device, &token,
            )?);
            Ok(())
        }

        pub(crate) fn host_relay_stop(&self) -> Result<(), String> {
            if let Some(mut relay) = self.1.lock().unwrap().take() {
                relay.stop();
            }
            Ok(())
        }

        /// The relay's host surface (tests).
        #[cfg(test)]
        pub(crate) fn relay_host(&self) -> Arc<dyn crate::host_relay::RelayHost> {
            self.0.clone()
        }

        pub(crate) fn claude_detection(&self) -> AgentClaudeDetection {
            let found = self.0.claude_detection();
            AgentClaudeDetection {
                bun_path: found
                    .bun_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned()),
                claude_code_dir: found
                    .claude_code_dir
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned()),
            }
        }

        pub(crate) fn projects_list(&self) -> Result<Vec<AgentProject>, String> {
            Ok(self
                .0
                .projects_list()
                .map_err(e)?
                .into_iter()
                .map(Into::into)
                .collect())
        }

        pub(crate) fn project_add(
            &self,
            name: String,
            path: String,
        ) -> Result<AgentProject, String> {
            self.0.project_add(&name, &path).map(Into::into).map_err(e)
        }

        pub(crate) fn project_remove(&self, id: String) -> Result<(), String> {
            self.0.project_remove(&id).map_err(e)
        }

        pub(crate) fn project_rename(
            &self,
            id: String,
            name: String,
        ) -> Result<AgentProject, String> {
            self.0.project_rename(&id, &name).map(Into::into).map_err(e)
        }

        pub(crate) fn session_project_set(
            &self,
            session_id: String,
            project_id: String,
        ) -> Result<AgentSession, String> {
            self.0
                .session_project_set(&session_id, &project_id)
                .map(Into::into)
                .map_err(e)
        }

        pub(crate) fn permission_mode_set(
            &self,
            _mode: AgentPermissionMode,
        ) -> Result<AgentPermissionMode, String> {
            self.0
                .permission_mode_set(PermissionMode::Bypass)
                .map(Into::into)
                .map_err(e)
        }

        pub(crate) fn models(&self) -> Result<Vec<AgentModel>, String> {
            Ok(self
                .0
                .models()
                .into_iter()
                .map(|m| wire::model(m).into())
                .collect())
        }

        pub(crate) fn sessions(&self) -> Result<Vec<AgentSession>, String> {
            self.0
                .sessions()
                .map_err(e)?
                .into_iter()
                .map(|s| self.0.session_dto(s).map(Into::into))
                .collect::<anyhow::Result<_>>()
                .map_err(e)
        }

        pub(crate) fn create_session(&self, title: String) -> Result<AgentSession, String> {
            self.0
                .create_session(&title)
                .and_then(|s| self.0.session_dto(s))
                .map(Into::into)
                .map_err(e)
        }

        pub(crate) fn rename_session(
            &self,
            id: String,
            title: String,
        ) -> Result<AgentSession, String> {
            self.0
                .rename_session(&id, &title)
                .and_then(|s| self.0.session_dto(s))
                .map(Into::into)
                .map_err(e)
        }

        pub(crate) fn delete_session(&self, id: String) -> Result<(), String> {
            self.0.delete_session(&id).map_err(e)
        }

        pub(crate) fn session_state(&self, id: String) -> Result<AgentSessionState, String> {
            self.0
                .session_state(&id)
                .map(|s| wire::session_state(s).into())
                .map_err(e)
        }

        pub(crate) fn send(
            &self,
            id: String,
            text: String,
            model: Option<String>,
            context: Option<AgentMessageContext>,
        ) -> Result<AgentSent, String> {
            self.0
                .send_with_context(&id, &text, model, context.map(Into::into))
                .map(|s| wire::sent(s).into())
                .map_err(e)
        }

        pub(crate) fn edit_undo(&self, edit_id: String) -> Result<AgentEdit, String> {
            self.0.edit_undo(&edit_id).map(Into::into).map_err(e)
        }

        pub(crate) fn steer(&self, id: String, text: String) -> Result<String, String> {
            self.0.steer(&id, &text).map(|q| q.id).map_err(e)
        }

        pub(crate) fn cancel(&self, id: String) -> Result<bool, String> {
            Ok(self.0.cancel(&id))
        }

        pub(crate) fn decide_approval(
            &self,
            approval_id: String,
            decision: String,
            reason: Option<String>,
        ) -> Result<AgentApproval, String> {
            self.0
                .decide_approval(&approval_id, &decision, reason)
                .map(|a| wire::approval(a).into())
                .map_err(e)
        }

        pub(crate) fn events_after(
            &self,
            after: i64,
            session: Option<String>,
        ) -> Result<AgentEvents, String> {
            self.0
                .events_after(after.max(0) as u64, session.as_deref())
                .map(|p| wire::events_page(p).into())
                .map_err(e)
        }

        pub(crate) fn subscribe_with(
            &self,
            session_id: Option<String>,
            mut sink: EventSink,
        ) -> Result<(), String> {
            if let Some(session_id) = session_id {
                self.0
                    .forward_events_for_session(session_id, move |f| {
                        sink(wire::forwarded(f).into())
                    })
                    .map_err(e)?;
            } else {
                self.0
                    .forward_events(move |f| sink(wire::forwarded(f).into()));
            }
            Ok(())
        }

        pub(crate) fn close_events(&self) -> Result<(), String> {
            self.0.close_event_streams();
            Ok(())
        }

        pub(crate) fn run_script(
            &self,
            id: String,
            label: String,
            code: String,
        ) -> Result<AgentRunRecord, String> {
            self.0
                .run_script(&id, &label, &code)
                .map(|r| wire::run(r).into())
                .map_err(e)
        }

        pub(crate) fn run_task(
            &self,
            vault: &NotesVault,
            task_id: String,
        ) -> Result<AgentTaskRun, String> {
            let r = self.0.run_task(&vault.inner, &task_id).map_err(e)?;
            Ok(AgentTaskRun {
                session_id: r.session_id,
                ok: r.ok,
                text: r.text,
            })
        }

        pub(crate) fn schedule_state(&self, _page_id: String) -> Result<AgentScheduleState, String> {
            Err(SCHEDULES_IN_VAULT.into())
        }

        pub(crate) fn schedule_accept(
            &self,
            _page_id: String,
            _hash: String,
        ) -> Result<AgentScheduleState, String> {
            Err(SCHEDULES_IN_VAULT.into())
        }

        pub(crate) fn schedule_pause(
            &self,
            _page_id: String,
            _paused: bool,
        ) -> Result<AgentScheduleState, String> {
            Err(SCHEDULES_IN_VAULT.into())
        }

        pub(crate) fn schedule_run_now(&self, _page_id: String) -> Result<AgentScheduleRun, String> {
            Err(SCHEDULES_IN_VAULT.into())
        }

        pub(crate) fn schedule_preview(
            &self,
            _json: String,
        ) -> Result<AgentSchedulePreview, String> {
            Err(SCHEDULES_IN_VAULT.into())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_relay_offers_the_desktop_pack_and_runs_it_without_approval() {
            let root = std::env::temp_dir()
                .join(format!("notes_bridge_relay_host_{}", vault_core::new_id()));
            let files = root.join("files");
            std::fs::create_dir_all(&files).unwrap();
            let vault = NotesVault::open(
                root.join("vault").to_string_lossy().into_owned(),
                None,
            )
            .unwrap();
            let agent = Local::open(
                &vault,
                root.join("state").to_string_lossy().into_owned(),
                AgentSettings {
                    default_model: String::new(),
                    bun_path: None,
                    claude_code_dir: None,
                    permission_mode: AgentPermissionMode::Manual,
                    desktop_tools: true,
                    desktop_root: files.to_string_lossy().into_owned(),
                    test_tools: false,
                },
            )
            .unwrap();
            let host = agent.relay_host();
            let names: Vec<String> = host.tools(None).unwrap().into_iter().map(|t| t.name).collect();
            for tool in ["read", "write", "edit", "exec"] {
                assert!(names.iter().any(|n| n == tool), "{tool} missing from {names:?}");
            }
            let session = host.open_session().unwrap();
            let target = files.join("from-hub.txt");
            let result = host
                .call(
                    &session,
                    "write",
                    serde_json::json!({"path": "from-hub.txt", "content": "hello"}),
                    notes_agent_wire::host::Caller { id: "oauth:test".into(), name: None },
                    vault_agent::bubblepaw_core::CancellationToken::new(),
                )
                .unwrap();
            assert!(!result.is_error && !result.pending, "{}", result.content);
            assert_eq!(std::fs::read_to_string(&target).unwrap(), "hello");
            host.close_session(&session).unwrap();
            assert!(agent.host_relay_stop().is_ok());
            drop(host);
            drop(agent);
            drop(vault);
            std::fs::remove_dir_all(root).ok();
        }

        #[test]
        fn local_agent_schedule_calls_point_to_the_vault() {
            let root = std::env::temp_dir().join(format!(
                "notes_bridge_agent_schedules_{}",
                vault_core::new_id()
            ));
            let data = root.join("state");
            let vault = NotesVault::open(
                root.join("vault").to_string_lossy().into_owned(),
                None,
            )
            .unwrap();
            let agent = Local::open(
                &vault,
                data.to_string_lossy().into_owned(),
                AgentSettings {
                    default_model: String::new(),
                    bun_path: None,
                    claude_code_dir: None,
                    permission_mode: AgentPermissionMode::Bypass,
                    desktop_tools: false,
                    desktop_root: String::new(),
                    test_tools: false,
                },
            )
            .unwrap();
            assert_eq!(agent.host_info().unwrap().kind, "desktop");
            let error = agent.schedule_state("page".into()).unwrap_err();
            assert!(error.contains("NotesVault"), "{error}");
            assert!(agent.sessions().unwrap().is_empty());
            assert!(!data.join("agent/schedules.json").exists());
            drop(agent);
            drop(vault);
            std::fs::remove_dir_all(root).ok();
        }

        #[test]
        fn local_projects_survive_reopen_and_bridge_stays_bypass() {
            let root = std::env::temp_dir()
                .join(format!("notes_bridge_projects_{}", vault_core::new_id()));
            let first = root.join("first");
            let second = root.join("second");
            std::fs::create_dir_all(&first).unwrap();
            std::fs::create_dir_all(&second).unwrap();
            let vault = NotesVault {
                inner: Arc::new(
                    vault_core::Vault::open(root.join("vault").to_string_lossy().into_owned())
                        .unwrap(),
                ),
                runner: crate::local_runner::LocalRunner::Unconfigured,
            };
            let settings = AgentSettings {
                default_model: String::new(),
                bun_path: None,
                claude_code_dir: None,
                permission_mode: AgentPermissionMode::Manual,
                desktop_tools: true,
                desktop_root: first.to_string_lossy().into_owned(),
                test_tools: false,
            };
            let data = root.join("state").to_string_lossy().into_owned();
            let agent = Local::open(&vault, data.clone(), settings.clone()).unwrap();
            assert_eq!(
                agent.host_info().unwrap().permission_mode,
                AgentPermissionMode::Bypass
            );
            let home = agent.projects_list().unwrap().remove(0);
            assert_eq!(home.name, "Home");
            let second_project = agent
                .project_add("Second".into(), second.to_string_lossy().into_owned())
                .unwrap();
            let session = agent.create_session("First chat".into()).unwrap();
            assert_eq!(session.project_id.as_deref(), Some(home.id.as_str()));
            let session = agent
                .session_project_set(session.id, second_project.id.clone())
                .unwrap();
            assert_eq!(session.project_name.as_deref(), Some("Second"));
            agent
                .project_rename(second_project.id, "Renamed".into())
                .unwrap();
            assert_eq!(
                agent.sessions().unwrap()[0].project_name.as_deref(),
                Some("Renamed")
            );
            assert_eq!(
                agent
                    .permission_mode_set(AgentPermissionMode::Manual)
                    .unwrap(),
                AgentPermissionMode::Bypass
            );
            drop(agent);

            let agent = Local::open(&vault, data, settings).unwrap();
            assert_eq!(
                agent.host_info().unwrap().permission_mode,
                AgentPermissionMode::Bypass
            );
            assert_eq!(agent.projects_list().unwrap().len(), 2);
            let new_session = agent.create_session("Next chat".into()).unwrap();
            assert_eq!(new_session.project_name.as_deref(), Some("Renamed"));
            agent.project_remove(home.id).unwrap();
            assert_eq!(agent.projects_list().unwrap().len(), 1);
            drop(agent);
            drop(vault);
            std::fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn local_open_ignores_legacy_manual_mode_without_changing_its_file() {
            let root = std::env::temp_dir()
                .join(format!("notes_bridge_legacy_mode_{}", vault_core::new_id()));
            let data_dir = root.join("state");
            let agent_dir = data_dir.join("agent");
            std::fs::create_dir_all(&agent_dir).unwrap();
            let mode_file = agent_dir.join("permission-mode.json");
            let legacy = b"\"manual\"\n";
            std::fs::write(&mode_file, legacy).unwrap();
            let vault = NotesVault {
                inner: Arc::new(
                    vault_core::Vault::open(root.join("vault").to_string_lossy().into_owned())
                        .unwrap(),
                ),
                runner: crate::local_runner::LocalRunner::Unconfigured,
            };
            let settings = AgentSettings {
                default_model: String::new(),
                bun_path: None,
                claude_code_dir: None,
                permission_mode: AgentPermissionMode::Manual,
                desktop_tools: true,
                desktop_root: root.to_string_lossy().into_owned(),
                test_tools: false,
            };
            let agent = Local::open(
                &vault,
                data_dir.to_string_lossy().into_owned(),
                settings.clone(),
            )
            .unwrap();
            assert_eq!(
                agent.host_info().unwrap().permission_mode,
                AgentPermissionMode::Bypass
            );
            assert_eq!(settings.permission_mode, AgentPermissionMode::Manual);
            assert_eq!(std::fs::read(&mode_file).unwrap(), legacy.to_vec());
            drop(agent);
            drop(vault);
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
