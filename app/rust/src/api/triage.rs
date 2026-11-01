//! Triage for Dart: start a triage run, list the merge proposals it made, and
//! accept, dismiss or undo them. Only type conversion happens here; the
//! proposals and the merge are `vault_core` (`Vault::triage_*`), the run is an
//! ordinary AI task (`vault_mcp::triage`).
//!
//! The proposals are stored in the hidden Nodes page (`triage_page_id`): a
//! change to that page in the change feed means the list changed.

use vault_core::TriageProposal;
use vault_mcp::triage;

use crate::api::vault::NotesVault;
use crate::convert::*;

/// A proposal to merge two pages, with their titles for display.
#[derive(Debug, Clone)]
pub struct TriageProposalDto {
    pub id: String,
    /// The older page: it keeps its id.
    pub keep_id: String,
    pub keep_title: String,
    /// The newer page: it is trashed by the merge.
    pub dropped_id: String,
    pub dropped_title: String,
    pub page_type: String,
    pub reason: String,
    /// "pending", "accepted" or "dismissed".
    pub status: String,
    pub created_ms: i64,
    /// The AI task row that proposed it, "" when none did.
    pub task: String,
    /// The kept page's version from before the merge, set once accepted;
    /// `triage_undo` uses it.
    pub merged_version: Option<String>,
}

fn dto(vault: &NotesVault, p: TriageProposal) -> TriageProposalDto {
    let page = |id: &str| vault.inner.load_page(id.to_owned()).ok();
    let keep = page(&p.keep);
    let dropped = page(&p.dropped);
    TriageProposalDto {
        page_type: keep
            .as_ref()
            .or(dropped.as_ref())
            .map(|k| k.page_type.clone())
            .unwrap_or_default(),
        keep_title: keep.map(|k| k.title).unwrap_or_default(),
        dropped_title: dropped.map(|d| d.title).unwrap_or_default(),
        id: p.id,
        keep_id: p.keep,
        dropped_id: p.dropped,
        reason: p.reason,
        status: p.status,
        created_ms: p.created_ms,
        task: p.task,
        merged_version: p.merged_version,
    }
}

/// Queues a triage run on `runner` (a PC's device id, or "hub") and returns
/// its AI task id. It reviews the pages under the top-level page
/// `scope_project_id`, or `page_ids` when that is empty. The run shows in the
/// AI tasks like any other and only proposes merges; nothing changes until a
/// proposal is accepted.
pub fn triage_start(
    vault: &NotesVault,
    scope_project_id: Option<String>,
    page_ids: Vec<String>,
    runner: String,
) -> Result<String, String> {
    triage::start(
        &vault.inner,
        scope_project_id.as_deref(),
        &page_ids,
        &runner,
    )
    .map(|t| t.id)
    .map_err(err)
}

/// The proposals still waiting for a decision, newest first.
pub fn triage_proposals(vault: &NotesVault) -> Result<Vec<TriageProposalDto>, String> {
    let all = vault.inner.triage_proposals(true).map_err(err)?;
    Ok(all.into_iter().map(|p| dto(vault, p)).collect())
}

/// Every proposal including decided ones, newest first.
pub fn triage_all_proposals(vault: &NotesVault) -> Result<Vec<TriageProposalDto>, String> {
    let all = vault.inner.triage_proposals(false).map_err(err)?;
    Ok(all.into_iter().map(|p| dto(vault, p)).collect())
}

/// Merges the newer page into the older one (one history commit, the newer
/// page goes to the trash). The result carries `merged_version` for undo.
pub fn triage_accept(vault: &NotesVault, id: String) -> Result<TriageProposalDto, String> {
    let p = vault.inner.triage_accept(id).map_err(err)?;
    Ok(dto(vault, p))
}

/// Turns a proposal down; the pair is never proposed again.
pub fn triage_dismiss(vault: &NotesVault, id: String) -> Result<TriageProposalDto, String> {
    let p = vault.inner.triage_dismiss(id).map_err(err)?;
    Ok(dto(vault, p))
}

/// Undoes an accepted merge: the kept page returns to its version before the
/// merge and the other page leaves the trash. The pair stays dismissed.
pub fn triage_undo(vault: &NotesVault, id: String) -> Result<TriageProposalDto, String> {
    let p = vault.inner.triage_undo(id).map_err(err)?;
    Ok(dto(vault, p))
}

/// The id of the hidden page that stores the proposals. A change to it in the
/// change feed means the proposal list changed.
pub fn triage_page_id(vault: &NotesVault) -> Result<String, String> {
    vault.inner.triage_page_id().map_err(err)
}

/// One link or tag a triage run applied, with the page titles for display.
#[derive(Debug, Clone)]
pub struct TriageActionDto {
    /// "link" or "tag".
    pub kind: String,
    pub page_id: String,
    pub page_title: String,
    /// A link's target page id; a tag's tag.
    pub target: String,
    /// A link's target title; "" for a tag.
    pub target_title: String,
}

/// The links and tags one triage run applied and has not undone.
#[derive(Debug, Clone)]
pub struct TriageRunDto {
    /// The AI task row of the run.
    pub task: String,
    pub actions: Vec<TriageActionDto>,
}

/// The triage runs that applied links or tags and still have them, newest
/// first.
pub fn triage_applied(vault: &NotesVault) -> Result<Vec<TriageRunDto>, String> {
    let title = |id: &str| {
        vault
            .inner
            .load_page(id.to_owned())
            .map(|p| p.title)
            .unwrap_or_default()
    };
    let runs = triage::applied_runs(&vault.inner).map_err(err)?;
    Ok(runs
        .into_iter()
        .map(|r| TriageRunDto {
            task: r.task,
            actions: r
                .actions
                .into_iter()
                .map(|a| {
                    let is_link = a.kind == vault_core::TRIAGE_LINK;
                    TriageActionDto {
                        page_title: title(&a.page),
                        target_title: if is_link { title(&a.target) } else { String::new() },
                        kind: a.kind,
                        page_id: a.page,
                        target: a.target,
                    }
                })
                .collect(),
        })
        .collect())
}

/// Takes back every link and tag one run applied, newest first, and returns
/// how many were undone.
pub fn triage_undo_applied(vault: &NotesVault, task: String) -> Result<u32, String> {
    triage::undo_actions(&vault.inner, &task)
        .map(|n| n as u32)
        .map_err(err)
}
