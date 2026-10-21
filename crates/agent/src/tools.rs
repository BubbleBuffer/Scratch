//! `vault_tools` as BubblePaw host tools, reached from `js` as `tools.<name>()`.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use bubblepaw_core::ToolDispatcher;
use bubblepaw_core::{Approval, Tool, ToolContext, ToolDefinition, ToolRegistry, ToolResult};
use bubblepaw_runner::tools::{ProcessTools, WorkspaceTools};
use serde_json::Value;
use vault_core::Vault;

/// Vault tools whose full signatures the `js` description always shows.
pub const ALWAYS_SHOWN: &[&str] = &[
    "search_notes",
    "read_page",
    "create_page",
    "edit_page",
    "update_task",
    "daily_note",
];

/// One `vault_tools` action. Name, description, schema, read-only flag and
/// approval class are carried over unchanged.
pub struct VaultTool {
    def: &'static vault_tools::ToolDef,
    vault: Arc<Vault>,
}

impl VaultTool {
    pub fn new(def: &'static vault_tools::ToolDef, vault: Arc<Vault>) -> Self {
        Self { def, vault }
    }
}

#[async_trait]
impl Tool for VaultTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.def.name.into(),
            description: self.def.description.into(),
            input_schema: self.def.input_schema_for(vault_tools::Surface::InAppAgent),
            read_only: self.def.read_only,
            approval: match self.def.approval {
                vault_tools::Approval::None => Approval::None,
                vault_tools::Approval::Confirm => Approval::Confirm,
            },
        }
    }

    async fn invoke(&self, _context: ToolContext, input: Value) -> Result<ToolResult> {
        let vault = self.vault.clone();
        let name = self.def.name;
        let result = tokio::task::spawn_blocking(move || {
            vault_tools::call_on(vault_tools::Surface::InAppAgent, &vault, name, &input)
        })
        .await?;
        Ok(match result {
            Ok(text) => ToolResult::text(text),
            Err(error) => ToolResult::error(error.0),
        })
    }
}

/// Registers the vault tools in `set` as host tools and marks the
/// always-shown ones among them.
pub fn register_vault_tools(
    registry: &mut ToolRegistry,
    vault: Arc<Vault>,
    records: Arc<crate::records::Records>,
    set: vault_tools::ToolSet,
) -> Result<()> {
    registry.show_always(ALWAYS_SHOWN.iter().copied().filter(|n| set.allows(n)));
    for def in vault_tools::definitions_for(vault_tools::Surface::InAppAgent)
        .filter(|def| set.allows(def.name))
    {
        registry.register_host(Arc::new(RecordedTool::new(
            Arc::new(VaultTool::new(def, vault.clone())),
            records.clone(),
            vault.clone(),
            !def.read_only,
        )))?;
    }
    Ok(())
}

pub(crate) struct RecordedTool {
    inner: Arc<dyn Tool>,
    records: Arc<crate::records::Records>,
    vault: Arc<Vault>,
    mutator: bool,
}
impl RecordedTool {
    pub fn new(
        inner: Arc<dyn Tool>,
        records: Arc<crate::records::Records>,
        vault: Arc<Vault>,
        mutator: bool,
    ) -> Self {
        Self {
            inner,
            records,
            vault,
            mutator,
        }
    }
    async fn invoke_recorded(
        &self,
        context: ToolContext,
        input: Value,
        dispatcher: Option<ToolDispatcher>,
    ) -> Result<ToolResult> {
        let name = self.inner.definition().name;
        let (seq, call_id) = self
            .records
            .activity_start(&context, &name, &input, &self.vault)?;
        // A synchronous vault tool is executed in one blocking worker so the
        // before/action/after lock never crosses an async suspension.
        if self.mutator {
            let records = self.records.clone();
            let vault = self.vault.clone();
            let context = context.clone();
            let name = name.clone();
            let actor = records.session_actor(&context.session_id);
            return tokio::task::spawn_blocking(move || {
                let _lock = records.mutation.lock().unwrap();
                let before = match records.before(&vault, &name, &input) {
                    Ok(before) => before,
                    Err(error) => {
                        records.activity_finish(seq, true, None)?;
                        return Ok(ToolResult::error(format!("{error:#}")));
                    }
                };
                let call = || {
                    vault_tools::call_on(vault_tools::Surface::InAppAgent, &vault, &name, &input)
                };
                let result = match actor.as_deref() {
                    Some(actor) => vault.with_actor(actor, call),
                    None => call(),
                };
                let edit_id = match &result {
                    Ok(text) => records.record(&vault, &context, &call_id, &name, before, text)?,
                    Err(_) if before.is_some() => {
                        records.record(&vault, &context, &call_id, &name, before, "")?
                    }
                    Err(_) => None,
                };
                records.activity_finish(seq, result.is_err(), edit_id)?;
                Ok(match result {
                    Ok(text) => ToolResult::text(text),
                    Err(error) => ToolResult::error(error.0),
                })
            })
            .await?;
        }
        let result = match dispatcher {
            Some(dispatcher) => {
                self.inner
                    .invoke_with_dispatcher(context, input, dispatcher)
                    .await
            }
            None => self.inner.invoke(context, input).await,
        };
        self.records.activity_finish(
            seq,
            result
                .as_ref()
                .map(|result| result.is_error)
                .unwrap_or(true),
            None,
        )?;
        result
    }
}
#[async_trait]
impl Tool for RecordedTool {
    fn definition(&self) -> ToolDefinition {
        self.inner.definition()
    }
    async fn invoke(&self, context: ToolContext, input: Value) -> Result<ToolResult> {
        self.invoke_recorded(context, input, None).await
    }
    async fn invoke_with_dispatcher(
        &self,
        context: ToolContext,
        input: Value,
        dispatcher: ToolDispatcher,
    ) -> Result<ToolResult> {
        self.invoke_recorded(context, input, Some(dispatcher)).await
    }
    fn interrupt_wait(&self, session: &str) -> usize {
        self.inner.interrupt_wait(session)
    }
    fn cancel_session(&self, session: &str) -> usize {
        self.inner.cancel_session(session)
    }
}

/// Native tools are cached per project root. Process handles keep their owning
/// registry even when the session changes projects.
struct RootTools {
    native: ToolRegistry,
    processes: Arc<ProcessTools>,
}

pub struct DesktopRuntime {
    projects: Arc<crate::projects::Projects>,
    roots: std::sync::Mutex<std::collections::HashMap<String, Arc<RootTools>>>,
    handles: std::sync::Mutex<std::collections::HashMap<String, Arc<RootTools>>>,
}
impl DesktopRuntime {
    fn root(&self, project: &crate::Project) -> Result<Arc<RootTools>> {
        let mut roots = self.roots.lock().unwrap();
        if let Some(root) = roots.get(&project.path) {
            return Ok(root.clone());
        }
        let workspace = WorkspaceTools::new(&project.path)?;
        let processes = Arc::new(ProcessTools::new(workspace.clone()));
        let mut native = ToolRegistry::new();
        workspace.register(&mut native, false)?;
        processes.register(&mut native)?;
        processes.register_wait(&mut native)?;
        let root = Arc::new(RootTools { native, processes });
        roots.insert(project.path.clone(), root.clone());
        Ok(root)
    }
    pub fn continuation_status(&self, session: &str) -> Value {
        let roots = self.roots.lock().unwrap();
        let mut entries = Vec::new();
        let mut omitted = 0;
        for root in roots.values() {
            let status = root.processes.continuation_status(session);
            omitted += status["omitted"].as_u64().unwrap_or(0);
            if let Some(values) = status["entries"].as_array() {
                entries.extend(values.iter().cloned());
            }
        }
        entries.sort_by_key(|entry| entry["status"] != "running");
        omitted += entries.len().saturating_sub(8) as u64;
        entries.truncate(8);
        serde_json::json!({"entries":entries,"omitted":omitted})
    }
    pub async fn shutdown(&self) {
        let roots: Vec<_> = self.roots.lock().unwrap().values().cloned().collect();
        for root in roots {
            root.processes.shutdown().await;
        }
    }
}
struct DesktopTool {
    desktop: Arc<DesktopRuntime>,
    source: &'static str,
    definition: ToolDefinition,
}
impl DesktopTool {
    fn target(&self, session: &str, input: &Value) -> Result<(Arc<RootTools>, crate::Project)> {
        let project = self.desktop.projects.current(session)?;
        if matches!(self.source, "read" | "write" | "edit" | "search" | "list") {
            if let Some(path) = input.get("path").and_then(Value::as_str) {
                check_project_path(&project, path)?;
            }
        }
        let root = input
            .get("handle")
            .and_then(Value::as_str)
            .and_then(|handle| self.desktop.handles.lock().unwrap().get(handle).cloned());
        Ok((
            match root {
                Some(root) => root,
                None => self.desktop.root(&project)?,
            },
            project,
        ))
    }
    fn finish(
        &self,
        result: Result<ToolResult>,
        root: Arc<RootTools>,
        project: &crate::Project,
    ) -> Result<ToolResult> {
        let result = result.map_err(|error| {
            if format!("{error:#}").contains("escapes workspace root") {
                anyhow::anyhow!(
                    "outside project \"{}\" ({}); use project_open to request access",
                    project.name,
                    project.path
                )
            } else {
                error
            }
        })?;
        if self.source == "exec" {
            if let Ok(value) = serde_json::from_str::<Value>(&result.content) {
                if let Some(handle) = value["handle"].as_str() {
                    self.desktop
                        .handles
                        .lock()
                        .unwrap()
                        .insert(handle.into(), root);
                }
            }
        }
        Ok(result)
    }
}
#[async_trait]
impl Tool for DesktopTool {
    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }
    async fn invoke(&self, context: ToolContext, input: Value) -> Result<ToolResult> {
        let (root, project) = self.target(&context.session_id, &input)?;
        let result = root
            .native
            .get_host(self.source)
            .expect("native tool")
            .invoke(context, input)
            .await;
        self.finish(result, root, &project)
    }
    async fn invoke_with_dispatcher(
        &self,
        context: ToolContext,
        input: Value,
        dispatcher: ToolDispatcher,
    ) -> Result<ToolResult> {
        let (root, project) = self.target(&context.session_id, &input)?;
        let result = root
            .native
            .get_host(self.source)
            .expect("native tool")
            .invoke_with_dispatcher(context, input, dispatcher)
            .await;
        self.finish(result, root, &project)
    }
    fn interrupt_wait(&self, session: &str) -> usize {
        self.desktop
            .roots
            .lock()
            .unwrap()
            .values()
            .map(|root| {
                root.native
                    .get_host(self.source)
                    .expect("native tool")
                    .interrupt_wait(session)
            })
            .sum()
    }
    fn cancel_session(&self, session: &str) -> usize {
        self.desktop
            .roots
            .lock()
            .unwrap()
            .values()
            .map(|root| {
                root.native
                    .get_host(self.source)
                    .expect("native tool")
                    .cancel_session(session)
            })
            .sum()
    }
}

struct ProjectTool {
    projects: Arc<crate::projects::Projects>,
    name: &'static str,
}
#[async_trait]
impl Tool for ProjectTool {
    fn definition(&self) -> ToolDefinition {
        use serde_json::json;
        let (description, properties, required) = match self.name {
            "project_list" => (
                "List projects and the current session project.",
                json!({}),
                vec![],
            ),
            "project_current" => (
                "Get the current session project and working folder.",
                json!({}),
                vec![],
            ),
            "project_switch" => (
                "Switch the current session to an existing project by id or exact name.",
                json!({"project":{"type":"string"}}),
                vec!["project"],
            ),
            "project_open" => (
                "Open a folder: add it as a project and switch this session to it.",
                json!({"path":{"type":"string"},"name":{"type":"string"}}),
                vec!["path"],
            ),
            _ => unreachable!(),
        };
        ToolDefinition {
            name: self.name.into(),
            description: description.into(),
            input_schema: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
            read_only: matches!(self.name, "project_list" | "project_current"),
            approval: if matches!(self.name, "project_switch" | "project_open") {
                Approval::Confirm
            } else {
                Approval::None
            },
        }
    }
    async fn invoke(&self, context: ToolContext, input: Value) -> Result<ToolResult> {
        let string = |key: &str| {
            input
                .get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("{key} must be a string"))
        };
        let value = match self.name {
            "project_list" => {
                serde_json::json!({"projects": self.projects.list(), "current":self.projects.current(&context.session_id)?})
            }
            "project_current" => serde_json::to_value(self.projects.current(&context.session_id)?)?,
            "project_switch" => serde_json::to_value(
                self.projects
                    .switch(&context.session_id, string("project")?)?,
            )?,
            "project_open" => serde_json::to_value(self.projects.open_project(
                &context.session_id,
                string("path")?,
                input.get("name").map(|_| string("name")).transpose()?,
            )?)?,
            _ => unreachable!(),
        };
        Ok(ToolResult::text(serde_json::to_string(&value)?))
    }
}

pub fn register_desktop_tools(
    registry: &mut ToolRegistry,
    projects: Arc<crate::projects::Projects>,
    definition_root: &std::path::Path,
) -> Result<Arc<DesktopRuntime>> {
    register_desktop_tools_recorded(registry, projects, definition_root, None)
}
pub(crate) fn register_desktop_tools_recorded(
    registry: &mut ToolRegistry,
    projects: Arc<crate::projects::Projects>,
    definition_root: &std::path::Path,
    recording: Option<(Arc<crate::records::Records>, Arc<Vault>)>,
) -> Result<Arc<DesktopRuntime>> {
    let desktop = Arc::new(DesktopRuntime {
        projects,
        roots: Default::default(),
        handles: Default::default(),
    });
    // Definitions are static. Use the agent's existing internal workspace so
    // a moved/unavailable project never prevents listing or repairing projects.
    // Project roots and process pools are constructed only when invoked.
    let workspace = WorkspaceTools::new(definition_root)?;
    let processes = Arc::new(ProcessTools::new(workspace.clone()));
    let mut native = ToolRegistry::new();
    workspace.register(&mut native, false)?;
    processes.register(&mut native)?;
    processes.register_wait(&mut native)?;
    for name in [
        "read", "write", "edit", "search", "list", "exec", "poll", "wait", "cancel",
    ] {
        let mut definition = native.get_host(name).expect("native tool").definition();
        definition.approval = if matches!(name, "write" | "edit" | "exec") {
            Approval::Confirm
        } else {
            Approval::None
        };
        if name == "exec" {
            definition.description.push_str(" The working directory defaults to the current project's folder. This is not a sandbox: commands can access files outside the project.");
        }
        let tool: Arc<dyn Tool> = Arc::new(DesktopTool {
            desktop: desktop.clone(),
            source: name,
            definition,
        });
        registry.register_host(match &recording {
            Some((records, vault)) => Arc::new(RecordedTool::new(
                tool,
                records.clone(),
                vault.clone(),
                false,
            )),
            None => tool,
        })?;
        registry.show_always([name]);
    }
    for name in [
        "project_list",
        "project_current",
        "project_switch",
        "project_open",
    ] {
        let tool: Arc<dyn Tool> = Arc::new(ProjectTool {
            projects: desktop.projects.clone(),
            name,
        });
        registry.register_host(match &recording {
            Some((records, vault)) => Arc::new(RecordedTool::new(
                tool,
                records.clone(),
                vault.clone(),
                false,
            )),
            None => tool,
        })?;
        registry.show_always([name]);
    }
    Ok(desktop)
}

fn check_project_path(project: &crate::Project, path: &str) -> Result<()> {
    use std::path::{Component, Path, PathBuf};
    let root = Path::new(&project.path);
    let joined = root.join(path);
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    let outside = || {
        anyhow::anyhow!(
            "outside project \"{}\" ({}); use project_open to request access",
            project.name,
            project.path
        )
    };
    anyhow::ensure!(path_is_inside(&normalized, root), "{}", outside());
    let mut ancestor = joined.as_path();
    loop {
        match std::fs::canonicalize(ancestor) {
            Ok(resolved) => {
                anyhow::ensure!(path_is_inside(&resolved, root), "{}", outside());
                break;
            }
            Err(_) => match ancestor.parent() {
                Some(parent) => ancestor = parent,
                None => break,
            },
        }
    }
    Ok(())
}

fn path_is_inside(path: &std::path::Path, root: &std::path::Path) -> bool {
    if cfg!(windows) {
        fn normal(path: &std::path::Path) -> std::path::PathBuf {
            let text = path.to_string_lossy().to_lowercase();
            let text = if let Some(unc) = text.strip_prefix(r"\\?\unc\") {
                format!(r"\\{unc}")
            } else {
                text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
            };
            std::path::PathBuf::from(text)
        }
        normal(path).starts_with(normal(root))
    } else {
        path.starts_with(root)
    }
}

#[cfg(test)]
mod presentation_tests {
    use super::*;
    #[test]
    fn every_new_host_tool_signature_is_always_shown() {
        let dir = tempfile::tempdir().unwrap();
        let vault =
            Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into_owned()).unwrap());
        let records = Arc::new(
            crate::records::Records::open(
                &dir.path().join("records.sqlite"),
                bubblepaw_core::Store::in_memory().unwrap(),
            )
            .unwrap(),
        );
        let mut registry = ToolRegistry::new();
        register_vault_tools(
            &mut registry,
            vault.clone(),
            records.clone(),
            vault_tools::ToolSet::Everything,
        )
        .unwrap();
        crate::ui::register(
            &mut registry,
            Arc::new(crate::ui::Ui::new()),
            vault,
            records,
        )
        .unwrap();
        for name in [
            "search_notes",
            "update_task",
            "edit_page",
            "daily_note",
            "ui_open_page",
            "ui_highlight",
        ] {
            assert!(registry.get_host(name).is_some(), "missing {name}");
            assert!(registry.is_shown_always(name), "not shown: {name}");
        }
        assert!(registry.get_host("edit_undo").is_none());
    }
}
