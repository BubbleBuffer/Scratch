//! Runner picker data and assignments. Device ids are the vault's device ids;
//! node-record page ids are storage details, never task destinations.

use vault_core::{
    Block, Vault, AGENT_RUNNER_HUB, AGENT_RUNNER_THIS_DEVICE, RUNNER_ONLINE_WINDOW_MS,
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
        .filter(|r| r.kind == "desktop" || r.kind == "pc")
        .map(|r| Runner {
            is_this_device: r.id == device_id,
            online: is_online(r.last_seen_ms, now_ms),
            id: r.id,
            name: r.name,
            kind: RunnerKind::Pc,
            last_seen_ms: r.last_seen_ms,
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

/// Creates a real AI task and its placeholder, pinned before it is queued.
/// `block` is an optional block in `page`, using the existing creation path's
/// placement/reuse behavior. Offline runners accept queued assignments.
pub fn assign(
    v: &Vault,
    prompt: &str,
    page: &str,
    block: Option<&str>,
    runner_id: &str,
) -> Result<tasks::Task> {
    if runner_id != jobs::NODE_HUB
        && !v
            .device_records()?
            .iter()
            .any(|r| r.id == runner_id && (r.kind == "desktop" || r.kind == "pc"))
    {
        return err(format!("unknown runner {runner_id:?}"));
    }
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
    tasks::create_on(v, prompt, &page_id, block, "agent", runner_id)
}
