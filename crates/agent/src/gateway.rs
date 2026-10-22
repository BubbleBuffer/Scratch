//! MCP uses BubblePaw's declarations and single-tool dispatch. Gateway policy
//! constrains both native entrypoints and capabilities composed inside JS.

use anyhow::{ensure, Context, Result};
use async_trait::async_trait;
use bubblepaw_core::{
    ApprovalMode, ApprovalPolicy, CancellationToken, ClientInfo, ExecutionMode, Harness,
    HarnessConfig, RunOptions, SessionBusy, Store, Tool, ToolContext, ToolDefinition, ToolRegistry,
    ToolResult,
};
use serde_json::{json, Value};
use std::{collections::BTreeSet, sync::Arc, time::Duration};

pub(crate) struct GatewayRuntime {
    source: ToolRegistry,
    execution_mode: ExecutionMode,
    enable_js: bool,
    vault_adapters: BTreeSet<String>,
}

const HOSTS_HELP: &str = "Hosts: hub (online, default).\n";

struct Hosts;

#[async_trait]
impl Tool for Hosts {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "hosts".into(),
            description: "Lists available execution hosts and their online/default status.".into(),
            input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
            read_only: true,
            approval: Default::default(),
        }
    }

    async fn invoke(&self, _context: ToolContext, _input: Value) -> Result<ToolResult> {
        Ok(ToolResult::text(
            json!([{"name":"hub","online":true,"default":true}]).to_string(),
        ))
    }
}

fn allowed_surface(name: &str) -> bool {
    !name.starts_with("history_")
        && !name.starts_with("web_")
        && !name.starts_with("ui_")
        && !matches!(name, "web" | "todo" | "question" | "reload")
}

impl GatewayRuntime {
    pub(crate) fn new(
        source: &ToolRegistry,
        execution_mode: ExecutionMode,
        enable_js: bool,
    ) -> Result<Self> {
        for name in ["hosts", "js_help"] {
            ensure!(
                source.get(name).is_none(),
                "Gateway tool name collision: {name}"
            );
        }
        // Only these names were registered as our vault adapters. Other vault
        // surfaces/aliases are reserved too, but a host using one is a real
        // collision and must not disappear from the additional host-tool list.
        let vault_adapters: BTreeSet<String> =
            vault_tools::definitions_for(vault_tools::Surface::InAppAgent)
                .map(|tool| tool.name.to_owned())
                .collect();
        let vault_names = vault_tools::definitions()
            .iter()
            .chain(vault_tools::hidden())
            .map(|tool| tool.name)
            .chain(vault_tools::aliases().iter().map(|alias| alias.name));
        for name in vault_names {
            ensure!(
                source.get(name).is_none() || vault_adapters.contains(name),
                "Gateway tool name collision with vault tool: {name}"
            );
        }
        let names = source
            .capabilities()
            .into_iter()
            .filter(|tool| allowed_surface(&tool.name))
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        let mut source = source.restricted_capabilities(&names);
        source.register_host(Arc::new(Hosts))?;
        source.show_always(["hosts"]);
        Ok(Self {
            source,
            execution_mode,
            enable_js,
            vault_adapters,
        })
    }

    fn registry(&self, policy: &ApprovalPolicy) -> ToolRegistry {
        let names = self
            .source
            .capabilities()
            .into_iter()
            .filter(|tool| policy.mode(&tool.name, tool.approval) == ApprovalMode::Allow)
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        self.source.restricted_capabilities(&names)
    }

    fn harness(&self, store: Store, policy: ApprovalPolicy) -> Result<Harness> {
        Ok(Harness::new(
            store,
            // Single-tool runs never invoke this provider, including errors.
            Arc::new(crate::Unavailable("MCP only executes tools".into())),
            self.registry(&noninteractive(policy.clone())),
            HarnessConfig {
                execution_mode: self.execution_mode,
                enable_js: self.enable_js,
                approval: noninteractive(policy),
                max_tool_result_bytes: 256 * 1024,
                ..HarnessConfig::default()
            },
        ))
    }

    fn definitions(&self, harness: &Harness, session: &str) -> Result<Vec<ToolDefinition>> {
        Ok(harness
            .effective_tool_definitions(session, &RunOptions::default())?
            .into_iter()
            // The HTTP gateway serves its vault MCP surface and hosts itself.
            // Keep the adapters in the registry for JS composition only.
            .filter(|tool| tool.name != "hosts" && !self.vault_adapters.contains(&tool.name))
            .collect())
    }
}

fn noninteractive(mut policy: ApprovalPolicy) -> ApprovalPolicy {
    if policy.confirm == ApprovalMode::Ask {
        policy.confirm = ApprovalMode::Deny;
    }
    for mode in policy.tools.values_mut() {
        if *mode == ApprovalMode::Ask {
            *mode = ApprovalMode::Deny;
        }
    }
    policy
}

fn session_missing(store: &Store, session: &str) -> Result<bool> {
    match store.session(session) {
        Ok(_) => Ok(false),
        Err(error)
            if matches!(
                error.downcast_ref::<rusqlite::Error>(),
                Some(rusqlite::Error::QueryReturnedNoRows)
            ) =>
        {
            Ok(true)
        }
        Err(error) => Err(error),
    }
}

fn signature(tool: &ToolDefinition) -> String {
    let required = tool.input_schema["required"].as_array();
    let fields = tool.input_schema["properties"]
        .as_object()
        .map(|properties| {
            properties
                .keys()
                .map(|name| {
                    if required.is_some_and(|items| items.iter().any(|item| item == name)) {
                        name.clone()
                    } else {
                        format!("{name}?")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    format!("tools.{}({{{fields}}})", tool.name)
}

impl crate::Agent {
    /// Additional gateway entrypoints, selected by BubblePaw for this session.
    /// Vault MCP tools and `hosts` are owned by the HTTP host instead.
    pub fn gateway_tool_definitions(&self, session: &str) -> Result<Vec<ToolDefinition>> {
        let gateway = &self.surfaces().gateway;
        let harness = gateway.harness(self.store.clone(), self.current_policy())?;
        gateway.definitions(&harness, session)
    }

    /// Discover the normal gateway surface before an MCP session is initialized.
    /// The private session never executes tools and is removed on success/error;
    /// listing repeatedly cannot accumulate sessions or allocate JS heaps.
    pub fn gateway_tool_definitions_for_discovery(&self) -> Result<Vec<ToolDefinition>> {
        self.discover(&self.surfaces().gateway, self.current_policy())
    }

    fn discover(
        &self,
        gateway: &GatewayRuntime,
        policy: ApprovalPolicy,
    ) -> Result<Vec<ToolDefinition>> {
        let harness = gateway.harness(self.store.clone(), policy)?;
        let session = self.store.create_session("MCP tool discovery")?;
        let definitions = gateway.definitions(&harness, &session.id);
        self.runtime
            .block_on(harness.delete_session(&session.id))
            .context("remove private gateway discovery session")?;
        definitions
    }

    /// The host relay's tools (the hub's `/mcp/agent?host=<this computer>`):
    /// this host's own tools as native entrypoints, under Bypass, with the
    /// gateway's exclusions. `None` lists them for discovery.
    pub fn relay_tool_definitions(&self, session: Option<&str>) -> Result<Vec<ToolDefinition>> {
        let policy = crate::permission_policy(crate::PermissionMode::Bypass);
        let relay = &self.surfaces().relay;
        match session {
            Some(session) => {
                let harness = relay.harness(self.store.clone(), policy)?;
                relay.definitions(&harness, session)
            }
            None => self.discover(relay, policy),
        }
    }

    /// Runs one relay tool under Bypass, like [`Self::gateway_call_as`];
    /// `cancel` stops it.
    pub fn relay_call(
        &self,
        session: &str,
        name: &str,
        input: Value,
        client: ClientInfo,
        cancel: CancellationToken,
    ) -> Result<ToolResult> {
        let policy = crate::permission_policy(crate::PermissionMode::Bypass);
        let surfaces = self.surfaces();
        self.call_on(
            &surfaces.relay,
            policy,
            session,
            name,
            input,
            client,
            cancel,
        )
    }

    /// The actual BubblePaw js schema and generated reference, with only this
    /// host's currently enabled, noninteractive gateway capabilities visible.
    pub fn gateway_js_definition(&self) -> ToolDefinition {
        self.surfaces()
            .gateway
            .registry(&self.current_policy())
            .definitions()
            .into_iter()
            .find(|tool| tool.name == "js")
            .expect("agent js tool")
    }

    /// No query lists callable signatures and descriptions. A query returns
    /// matching capabilities' full schemas/docs; `js` returns runtime guidance.
    pub fn gateway_js_help(&self, query: Option<&str>) -> String {
        let registry = self.surfaces().gateway.registry(&self.current_policy());
        let query = query.unwrap_or("").trim();
        if matches!(
            query.to_ascii_lowercase().as_str(),
            "js" | "runtime" | "code"
        ) {
            let js = registry
                .definitions()
                .into_iter()
                .find(|tool| tool.name == "js")
                .unwrap();
            return format!(
                "{HOSTS_HELP}{}\nInput schema: {}",
                js.description, js.input_schema
            );
        }
        let mut output = if query.is_empty() {
            format!("{HOSTS_HELP}Use js_help with a tool name or search phrase for full schemas; query js for the runtime reference.\n")
        } else {
            HOSTS_HELP.to_owned()
        };
        let mut found = false;
        let terms = query.to_ascii_lowercase();
        for tool in registry.capabilities() {
            let haystack = format!("{} {}", tool.name, tool.description).to_ascii_lowercase();
            if !query.is_empty() && !terms.split_whitespace().all(|term| haystack.contains(term)) {
                continue;
            }
            found = true;
            let docs = if query.is_empty() {
                tool.description
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                tool.description.clone()
            };
            output.push_str(&format!("{}: {docs}\n", signature(&tool)));
            if !query.is_empty() {
                output.push_str(&format!("Input schema: {}\n", tool.input_schema));
            }
        }
        if !found {
            output.push_str("No matching enabled tools.");
        }
        output
    }

    /// Execute one advertised entrypoint without a model turn. Tool errors and
    /// pending effects retain BubblePaw's ToolResult flags. Admission/storage/
    /// cancellation failures remain errors. The HTTP host owns authentication
    /// and session binding; never accept a client-supplied chat ID here.
    pub fn gateway_call_as(
        &self,
        session: &str,
        name: &str,
        input: Value,
        client: ClientInfo,
    ) -> Result<ToolResult> {
        let policy = self.current_policy();
        self.call_on(
            &self.surfaces().gateway,
            policy,
            session,
            name,
            input,
            client,
            CancellationToken::new(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn call_on(
        &self,
        gateway: &GatewayRuntime,
        policy: ApprovalPolicy,
        session: &str,
        name: &str,
        input: Value,
        client: ClientInfo,
        cancel: CancellationToken,
    ) -> Result<ToolResult> {
        self.store.session(session)?;
        if let Some(subject) = client
            .id
            .strip_prefix("oauth:")
            .filter(|subject| !subject.is_empty())
        {
            let author = client
                .name
                .as_deref()
                .filter(|name| !name.is_empty())
                .unwrap_or(subject);
            self.records
                .bind_session_actor(session, &client.id, format!("oauth:{author}"))?;
        }
        let harness = gateway.harness(self.store.clone(), policy)?;
        let tool_names = gateway
            .definitions(&harness, session)?
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        self.runtime.block_on(harness.call_tool(
            session,
            name,
            input,
            RunOptions {
                tool_names: Some(tool_names),
                client: Some(client),
                ..RunOptions::default()
            },
            cancel,
        ))
    }

    /// Explicitly stop an expired/disconnected MCP session, close every owned
    /// JS heap, then remove its history and notes-owned records. Does not stop
    /// other sessions, the shared CodeMode, or any model/provider process.
    pub fn gateway_close_session(&self, session: &str) -> Result<()> {
        let harness = self
            .surfaces()
            .gateway
            .harness(self.store.clone(), self.current_policy())?;
        self.runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    // An admitted call may still be entering its first cell.
                    // Repeat cancellation/close until both its run and heaps
                    // are gone, rather than relying on one racy heap snapshot.
                    harness.cancel(session);
                    let status = self.repl.status(session);
                    let environments = status["environments"]
                        .as_array()
                        .context("invalid JS environment status")?;
                    if environments.is_empty() {
                        // The durable deletion can have succeeded on a prior
                        // close attempt whose notes/project cleanup then failed.
                        // Only the precise missing-row case is idempotent.
                        if session_missing(&self.store, session)? {
                            break Ok(());
                        }
                        match harness.delete_session(session).await {
                            Ok(()) => break Ok(()),
                            Err(error) if error.downcast_ref::<SessionBusy>().is_some() => {}
                            Err(error)
                                if error.to_string() == format!("Unknown session: {session}")
                                    && session_missing(&self.store, session)? =>
                            {
                                // Another close may have won after our lookup.
                                break Ok(());
                            }
                            Err(error) => break Err(error),
                        }
                    } else {
                        for environment in environments {
                            let name = environment["name"]
                                .as_str()
                                .context("JS environment has no name")?;
                            if let Err(error) = self.repl.control(
                                session,
                                json!({"action":"close","environment":name,"cancel":true}),
                            ) {
                                // A previous iteration already requested close;
                                // status retains it until its worker joins.
                                if error.to_string() != "Environment is closing" {
                                    return Err(error);
                                }
                            }
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .context("timed out closing MCP session")?
        })?;
        self.records.forget_session(session)?;
        if let Some(projects) = &self.projects {
            projects.forget_session(session)?;
        }
        Ok(())
    }
}
