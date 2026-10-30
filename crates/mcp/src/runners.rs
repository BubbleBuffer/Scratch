//! Runner picker data and assignments. Device ids are the vault's device ids;
//! node-record page ids are storage details, never task destinations.

use vault_core::{
    Block, DeviceProject, Vault, AGENT_RUNNER_HUB, AGENT_RUNNER_THIS_DEVICE,
    RUNNER_ONLINE_WINDOW_MS,
};

use crate::{err, jobs, tasks, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerKind {
    Hub,
    Pc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runner {
    pub id: String,
    pub name: String,
    pub kind: RunnerKind,
    pub online: bool,
    pub last_seen_ms: Option<i64>,
    pub is_this_device: bool,
    /// The projects a PC runner has open, as its record publishes them. The
    /// hub has none.
    pub projects: Vec<DeviceProject>,
}

/// Whether a device record's kind is a PC, the only kind that runs code tasks.
fn is_pc(kind: &str) -> bool {
    kind == "desktop" || kind == "pc"
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerList {
    pub runners: Vec<Runner>,
    pub default_runner_id: String,
}

/// A timestamp exactly five minutes old is offline. Future timestamps are
/// treated as age zero, so modest clock skew does not mark a live device off.
pub fn is_online(last_seen_ms: Option<i64>, now_ms: i64) -> bool {
    last_seen_ms.is_some_and(|seen| now_ms.saturating_sub(seen) < RUNNER_ONLINE_WINDOW_MS)
}

/// The hub first, then PCs sorted by name and device id. `hub_usable` describes
/// the app's usable hub connection (AgentStore.usableHosts), not last-seen
/// liveness. `this_device_usable` is false on platforms without a local agent.
/// The current app ignores the retired per-device default override.
pub fn list(v: &Vault, hub_usable: bool, this_device_usable: bool) -> Result<RunnerList> {
    list_at(v, hub_usable, this_device_usable, jobs::now_ms())
}

/// [`list`] at a supplied clock instant, for deterministic boundary checks.
pub fn list_at(
    v: &Vault,
    hub_usable: bool,
    this_device_usable: bool,
    now_ms: i64,
) -> Result<RunnerList> {
    let device_id = v.device_id();
    let records = v.device_records()?;
    let hub_seen = records
        .iter()
        .filter(|r| r.kind == "hub")
        .filter_map(|r| r.last_seen_ms)
        .max();
    let mut pcs: Vec<Runner> = records
        .into_iter()
        .filter(|r| is_pc(&r.kind))
        .map(|r| Runner {
            is_this_device: r.id == device_id,
            online: is_online(r.last_seen_ms, now_ms),
            id: r.id,
            name: r.name,
            kind: RunnerKind::Pc,
            last_seen_ms: r.last_seen_ms,
            projects: r.projects,
        })
        .collect();
    pcs.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    let mut runners = vec![Runner {
        id: jobs::NODE_HUB.into(),
        name: "Hub".into(),
        kind: RunnerKind::Hub,
        online: is_online(hub_seen, now_ms),
        last_seen_ms: hub_seen,
        is_this_device: false,
        projects: Vec::new(),
    }];
    runners.extend(pcs);

    // ai_settings.dart resolveAgentHost + AgentStore.defaultHost: Android
    // always uses the hub. Desktop follows the synced value, then its local
    // built-in default when the wanted host is not usable here.
    let wanted = v.agent_defaults()?.runner;
    let default_runner_id =
        if !this_device_usable || (wanted.as_deref() == Some(AGENT_RUNNER_HUB) && hub_usable) {
            jobs::NODE_HUB.to_owned()
        } else {
            debug_assert!(wanted
                .as_deref()
                .is_none_or(|w| w == AGENT_RUNNER_HUB || w == AGENT_RUNNER_THIS_DEVICE));
            device_id
        };
    Ok(RunnerList {
        runners,
        default_runner_id,
    })
}

/// Checks that a task on `runner_id` may use `project`, a project id, before
/// anything is written. An empty `project` is always allowed. A project needs a
/// PC runner whose record lists it: the hub has no projects, and a runner that
/// is not a PC, or is unknown, or does not list the project, is an error.
pub fn check_project(v: &Vault, runner_id: &str, project: &str) -> Result<()> {
    if project.is_empty() {
        return Ok(());
    }
    if runner_id == jobs::NODE_HUB {
        return err("a hub task cannot use a project");
    }
    let records = v.device_records()?;
    let Some(record) = records.iter().find(|r| r.id == runner_id) else {
        return err(format!("unknown runner {runner_id:?}"));
    };
    if !is_pc(&record.kind) {
        return err(format!("runner {runner_id:?} is not a PC"));
    }
    if !record.projects.iter().any(|p| p.id == project) {
        return err(format!(
            "unknown project {project:?} on runner {runner_id:?}"
        ));
    }
    Ok(())
}

/// Checks that `runner_id` is `hub` or the device id of a PC.
pub fn check_runner(v: &Vault, runner_id: &str) -> Result<()> {
    if runner_id != jobs::NODE_HUB
        && !v
            .device_records()?
            .iter()
            .any(|r| r.id == runner_id && is_pc(&r.kind))
    {
        return err(format!("unknown runner {runner_id:?}"));
    }
    Ok(())
}

/// Creates a real AI task and its placeholder, pinned before it is queued.
/// `block` is an optional block in `page`, using the existing creation path's
/// placement/reuse behavior. Offline runners accept queued assignments.
/// `project` is a project id of the PC runner (see [`check_project`]); `None`
/// assigns no project.
pub fn assign(
    v: &Vault,
    prompt: &str,
    page: &str,
    block: Option<&str>,
    runner_id: &str,
    project: Option<&str>,
) -> Result<tasks::Task> {
    check_runner(v, runner_id)?;
    let page_id = page.to_owned();
    if let Some(block) = block {
        fn contains(blocks: &[Block], id: &str) -> bool {
            blocks
                .iter()
                .any(|b| b.id == id || contains(&b.children, id))
        }
        if !contains(&v.load_page(page_id.clone())?.blocks, block) {
            return err(format!("block {block:?} does not belong to page {page:?}"));
        }
    }
    tasks::create_for_project(v, prompt, &page_id, block, "agent", runner_id, project)
}
