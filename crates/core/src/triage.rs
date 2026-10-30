//! Triage: proposals to merge duplicate or overlapping pages, and the merge.
//!
//! A triage run (an AI task, see `vault_mcp::triage`) proposes merges; the
//! model detects them, the core only validates and stores them. Proposals live
//! in the type data (add-on `triage`) of the hidden Nodes page, one map entry
//! per page pair, so they sync like any page and are left out of lists and
//! search with it. The entry key is the SORTED page id pair, so one pair has
//! one record on every device, and a dismissed pair is never proposed again.
//!
//! A pair is valid when both pages are live, of one page type, under one
//! top-level ancestor (the "project"), and neither is the other's ancestor.
//! The older page (by creation time, never the model's choice) keeps its id
//! and the newer one is trashed after its content is appended to the older.
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{invalid, not_found, Result};
use crate::ids::now_ms;
use crate::templates::TemplateCopy;
use crate::types::{Block, BlockKind, BlockOp, MarkKind, NewBlock, PageId, PropValue, PAGE_TYPE_DOCUMENT};
use crate::vault::{lock, Vault};

// The proposals' key in the Nodes page's type data.
const ADDON: &str = "triage";
/// A proposal nobody has decided.
pub const TRIAGE_PENDING: &str = "pending";
/// A proposal whose merge was run.
pub const TRIAGE_ACCEPTED: &str = "accepted";
/// A proposal the user turned down, or whose merge was undone. The pair is
/// never proposed again.
pub const TRIAGE_DISMISSED: &str = "dismissed";

const REASON_MAX: usize = 500;
const SUMMARY_MAX: usize = 240;
/// The most pages one triage run lists.
const CANDIDATE_MAX: usize = 300;

/// One proposal to merge `dropped` into `keep`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriageProposal {
    /// The sorted page id pair, `<lower>-<higher>`.
    pub id: String,
    /// The older page: it keeps its id.
    pub keep: PageId,
    /// The newer page: it is trashed by the merge.
    pub dropped: PageId,
    pub reason: String,
    /// `pending`, `accepted` or `dismissed`.
    pub status: String,
    pub created_ms: i64,
    /// The AI task row that proposed it, empty when none did.
    #[serde(default)]
    pub task: String,
    /// The `keep` page's version before the merge (for `restore_version`),
    /// set when the proposal is accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_ms: Option<i64>,
}

/// A page a triage run may look at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriageCandidate {
    pub id: PageId,
    pub title: String,
    pub page_type: String,
    /// The start of the page's text.
    pub summary: String,
    pub created_ms: i64,
}

struct Info {
    id: PageId,
    title: String,
    page_type: String,
    created_ms: i64,
    hidden: bool,
    in_trash: bool,
}

fn pair_id(a: &str, b: &str) -> String {
    if a <= b {
        format!("{a}-{b}")
    } else {
        format!("{b}-{a}")
    }
}

fn text_of(blocks: &[Block], out: &mut String) {
    for b in blocks {
        if out.chars().count() >= SUMMARY_MAX {
            return;
        }
        let t = b.text.trim();
        if !t.is_empty() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(t);
        }
        text_of(&b.children, out);
    }
}

fn summary_of(blocks: &[Block]) -> String {
    let mut out = String::new();
    text_of(blocks, &mut out);
    let flat: String = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > SUMMARY_MAX {
        format!("{}…", flat.chars().take(SUMMARY_MAX).collect::<String>())
    } else {
        flat
    }
}

impl Vault {
    fn triage_info(&self, id: &str) -> Result<Info> {
        let page = self.load_page(id.to_string())?;
        let (hidden, title_role_template) = self.inner.read(id, |d| {
            let m = d.meta();
            (m.role.is_some() || m.is_template, m.title)
        })?;
        Ok(Info {
            id: page.id,
            title: if page.title.is_empty() {
                title_role_template
            } else {
                page.title
            },
            page_type: page.page_type,
            created_ms: page.created_ms,
            hidden,
            in_trash: page.in_trash,
        })
    }

    /// The page's top-level ancestor (itself when it is top-level).
    fn triage_root(&self, id: &str) -> Result<String> {
        let db = lock(&self.inner.db);
        let mut cur = id.to_string();
        for _ in 0..10_000 {
            match db.parent_of(&cur)?.flatten() {
                Some(parent) => cur = parent,
                None => break,
            }
        }
        Ok(cur)
    }

    /// Checks that `a` and `b` may be merged and returns their infos.
    fn triage_check_pair(&self, a: &str, b: &str) -> Result<(Info, Info)> {
        if a == b {
            return Err(invalid("a page cannot be merged with itself"));
        }
        let (ia, ib) = (self.triage_info(a)?, self.triage_info(b)?);
        for i in [&ia, &ib] {
            if i.in_trash {
                return Err(invalid(format!("page {:?} is in the trash", i.title)));
            }
            if i.hidden || i.page_type == "database" {
                return Err(invalid(format!(
                    "page {:?} is a system page, template or database and cannot be merged",
                    i.title
                )));
            }
        }
        if ia.page_type != ib.page_type {
            return Err(invalid(format!(
                "pages of different types cannot be merged ({} and {})",
                ia.page_type, ib.page_type
            )));
        }
        if self.triage_root(a)? != self.triage_root(b)? {
            return Err(invalid("pages of different projects cannot be merged"));
        }
        let related = {
            let db = lock(&self.inner.db);
            db.is_ancestor(a, b)? || db.is_ancestor(b, a)?
        };
        if related {
            return Err(invalid("a page and its sub-page cannot be merged"));
        }
        Ok((ia, ib))
    }

    fn triage_read(&self, nodes: &str) -> Result<HashMap<String, TriageProposal>> {
        let json = self.type_data(nodes.to_string(), ADDON.to_string())?;
        let value: Value = serde_json::from_str(&json).unwrap_or(Value::Null);
        let mut out = HashMap::new();
        if let Value::Object(map) = value {
            for (key, entry) in map {
                if let Ok(p) = serde_json::from_value::<TriageProposal>(entry) {
                    out.insert(key, p);
                }
            }
        }
        Ok(out)
    }

    fn triage_write(&self, nodes: &str, p: &TriageProposal) -> Result<()> {
        let json = serde_json::to_string(p).map_err(|e| invalid(e.to_string()))?;
        self.type_data_set(nodes.to_string(), ADDON.to_string(), vec![p.id.clone()], json)
    }

    /// The hidden page that holds the proposals (the Nodes page).
    pub fn triage_page_id(&self) -> Result<PageId> {
        let _guard = lock(&self.inner.nodes);
        self.nodes_page()
    }

    /// Proposes merging `a` and `b`. The older page is kept. A pair that has a
    /// record already (pending, accepted or dismissed, in either order) is not
    /// proposed again: the existing record comes back unchanged. A pair that is
    /// not valid (see the module docs), or has a trashed or purged page, is an
    /// error. `task` is the AI task row that proposed it (empty for none).
    pub fn triage_propose(
        &self,
        a: PageId,
        b: PageId,
        reason: String,
        task: String,
    ) -> Result<TriageProposal> {
        let (ia, ib) = self.triage_check_pair(&a, &b)?;
        let (older, newer) = if (ia.created_ms, &ia.id) <= (ib.created_ms, &ib.id) {
            (ia, ib)
        } else {
            (ib, ia)
        };
        let id = pair_id(&a, &b);
        let _guard = lock(&self.inner.nodes);
        let nodes = self.nodes_page()?;
        if let Some(existing) = self.triage_read(&nodes)?.remove(&id) {
            return Ok(existing);
        }
        let reason = reason.trim();
        let reason = if reason.is_empty() {
            "These pages overlap".to_owned()
        } else {
            reason.chars().take(REASON_MAX).collect()
        };
        let proposal = TriageProposal {
            id,
            keep: older.id,
            dropped: newer.id,
            reason,
            status: TRIAGE_PENDING.into(),
            created_ms: now_ms(),
            task,
            merged_version: None,
            decided_ms: None,
        };
        self.triage_write(&nodes, &proposal)?;
        Ok(proposal)
    }

    /// The proposals, newest first: the pending ones only, or all of them.
    pub fn triage_proposals(&self, pending_only: bool) -> Result<Vec<TriageProposal>> {
        let Some(nodes) = self.find_by_role("nodes".into())? else {
            return Ok(Vec::new());
        };
        let mut all: Vec<TriageProposal> = self
            .triage_read(&nodes)?
            .into_values()
            .filter(|p| !pending_only || p.status == TRIAGE_PENDING)
            .collect();
        all.sort_by(|x, y| y.created_ms.cmp(&x.created_ms).then_with(|| x.id.cmp(&y.id)));
        Ok(all)
    }

    fn triage_get(&self, nodes: &str, id: &str) -> Result<TriageProposal> {
        self.triage_read(nodes)?
            .remove(id)
            .ok_or_else(|| not_found("triage proposal", id))
    }

    /// Turns a pending proposal down; its pair is never proposed again.
    /// Dismissing a dismissed proposal changes nothing; an accepted one is
    /// undone with `triage_undo`.
    pub fn triage_dismiss(&self, id: String) -> Result<TriageProposal> {
        let _guard = lock(&self.inner.nodes);
        let nodes = self.nodes_page()?;
        let mut p = self.triage_get(&nodes, &id)?;
        match p.status.as_str() {
            TRIAGE_DISMISSED => return Ok(p),
            TRIAGE_ACCEPTED => {
                return Err(invalid("the merge was done; undo it instead of dismissing"))
            }
            _ => {}
        }
        p.status = TRIAGE_DISMISSED.into();
        p.decided_ms = Some(now_ms());
        self.triage_write(&nodes, &p)?;
        Ok(p)
    }

    /// Runs the merge of a pending proposal and records the version of the
    /// kept page from before it, for `triage_undo`. The proposal is checked
    /// again: a page that was trashed, retyped or moved since is an error.
    pub fn triage_accept(&self, id: String) -> Result<TriageProposal> {
        let _guard = lock(&self.inner.nodes);
        let nodes = self.nodes_page()?;
        let mut p = self.triage_get(&nodes, &id)?;
        if p.status != TRIAGE_PENDING {
            return Err(invalid(format!("the proposal is already {}", p.status)));
        }
        let version = self.merge_pages(p.keep.clone(), p.dropped.clone())?;
        p.status = TRIAGE_ACCEPTED.into();
        p.merged_version = Some(version);
        p.decided_ms = Some(now_ms());
        self.triage_write(&nodes, &p)?;
        Ok(p)
    }

    /// Undoes an accepted merge: the kept page goes back to its version from
    /// before the merge (as a new change) and the dropped page leaves the
    /// trash. The proposal becomes dismissed, so the pair stays unproposed.
    pub fn triage_undo(&self, id: String) -> Result<TriageProposal> {
        let _guard = lock(&self.inner.nodes);
        let nodes = self.nodes_page()?;
        let mut p = self.triage_get(&nodes, &id)?;
        let Some(version) = p.merged_version.clone().filter(|_| p.status == TRIAGE_ACCEPTED)
        else {
            return Err(invalid("only an accepted merge can be undone"));
        };
        self.restore_version(p.keep.clone(), version)?;
        self.restore_page(p.dropped.clone())?;
        p.status = TRIAGE_DISMISSED.into();
        p.decided_ms = Some(now_ms());
        self.triage_write(&nodes, &p)?;
        Ok(p)
    }

    /// Merges `dropped` into `keep` as ONE history commit of `keep` (message
    /// `triage:merge`): the blocks of `dropped` go at the end under a divider
    /// and a "Merged from" line, and the properties, icon and cover `keep`
    /// lacks are copied. Then `dropped` goes to the trash. Returns `keep`'s
    /// version id from before the merge: `restore_version(keep, id)` and
    /// `restore_page(dropped)` undo it. Both pages must form a valid pair, and
    /// `dropped` must have no sub-pages (they would go to the trash with it).
    ///
    /// Links to `dropped` are not repointed; they keep pointing at the page in
    /// the trash.
    pub fn merge_pages(&self, keep: PageId, dropped: PageId) -> Result<String> {
        let (_, gone) = self.triage_check_pair(&keep, &dropped)?;
        if !self.list_children(Some(dropped.clone()), 0, 1)?.is_empty() {
            return Err(invalid(format!(
                "page {:?} has sub-pages; move them first",
                gone.title
            )));
        }
        let copy = self.inner.read(&dropped, TemplateCopy::of)?;
        let version = self
            .inner
            .edit_tagged(&keep, Some("triage:merge"), |doc| {
                let version = crate::history::encode_frontiers(&doc.doc.oplog_frontiers());
                copy.append_after_divider(doc, &gone.title)?;
                Ok(version)
            })?;
        {
            let db = lock(&self.inner.db);
            self.inner.sync_meta(&db, &keep)?;
        }
        self.trash_page(dropped)?;
        Ok(version)
    }

    /// The pages a triage run may look at, oldest first: the live pages of the
    /// subtree under `root` (the page itself included), or the listed `pages`.
    /// System pages, templates, databases and trashed pages are left out, and
    /// so is everything past 300 pages.
    pub fn triage_candidates(
        &self,
        root: Option<PageId>,
        pages: Vec<PageId>,
    ) -> Result<Vec<TriageCandidate>> {
        let ids: Vec<String> = match root {
            Some(root) => {
                let db = lock(&self.inner.db);
                db.subtree(&root)?.into_iter().map(|(id, _)| id).collect()
            }
            None => pages,
        };
        let mut out = Vec::new();
        for id in ids {
            let Ok(info) = self.triage_info(&id) else {
                continue;
            };
            if info.hidden || info.in_trash || info.page_type == "database" {
                continue;
            }
            let page = self.load_page(id.clone())?;
            out.push(TriageCandidate {
                id,
                title: info.title,
                page_type: if info.page_type.is_empty() {
                    PAGE_TYPE_DOCUMENT.to_owned()
                } else {
                    info.page_type
                },
                summary: summary_of(&page.blocks),
                created_ms: info.created_ms,
            });
        }
        out.sort_by(|a, b| a.created_ms.cmp(&b.created_ms).then_with(|| a.id.cmp(&b.id)));
        out.truncate(CANDIDATE_MAX);
        Ok(out)
    }
}

/// One change a triage run made on its own (a link or a tag), with what is
/// needed to take it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriageAction {
    /// `link` or `tag`.
    pub kind: String,
    /// The page that was changed.
    pub page: PageId,
    /// For a link, the page it points at; for a tag, the tag.
    pub target: String,
    /// A link's inserted block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<String>,
    /// A tag's previous `tags` value: `None` when the page had none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<Vec<String>>,
}

pub const TRIAGE_LINK: &str = "link";
pub const TRIAGE_TAG: &str = "tag";
const TAG_MAX: usize = 64;
const TAGS_KEY: &str = "tags";

fn links_to(blocks: &[Block], target: &str) -> bool {
    blocks.iter().any(|b| {
        (b.kind == BlockKind::LinkToPage && b.target.as_deref() == Some(target))
            || b.marks
                .iter()
                .any(|m| matches!(&m.kind, MarkKind::PageLink(p) if p == target))
            || links_to(&b.children, target)
    })
}

impl Vault {
    /// Adds a link from page `from` to page `to`: a link-to-page block at the
    /// end of `from`. Both must be live pages of one project and not the
    /// same page. `None` when `from` already links to `to`.
    pub fn triage_link(&self, from: PageId, to: PageId) -> Result<Option<TriageAction>> {
        if from == to {
            return Err(invalid("a page cannot link to itself"));
        }
        let (a, _) = self.triage_check_link(&from, &to)?;
        let page = self.load_page(a.id.clone())?;
        if links_to(&page.blocks, &to) {
            return Ok(None);
        }
        let block = crate::ids::new_id();
        self.apply(
            from.clone(),
            vec![
                BlockOp::Insert {
                    parent: None,
                    after: page.blocks.last().map(|b| b.id.clone()),
                    block: NewBlock {
                        id: block.clone(),
                        kind: BlockKind::LinkToPage,
                        text: String::new(),
                        marks: vec![],
                        checked: false,
                    },
                },
                BlockOp::SetTarget {
                    id: block.clone(),
                    target: Some(to.clone()),
                },
            ],
        )?;
        Ok(Some(TriageAction {
            kind: TRIAGE_LINK.into(),
            page: from,
            target: to,
            block: Some(block),
            previous: None,
        }))
    }

    /// Like `triage_check_pair` without the same-type rule: links may join
    /// pages of different types.
    fn triage_check_link(&self, a: &str, b: &str) -> Result<(Info, Info)> {
        let (ia, ib) = (self.triage_info(a)?, self.triage_info(b)?);
        for i in [&ia, &ib] {
            if i.in_trash || i.hidden || i.page_type == "database" {
                return Err(invalid(format!("page {:?} cannot be linked", i.title)));
            }
        }
        if self.triage_root(a)? != self.triage_root(b)? {
            return Err(invalid("pages of different projects cannot be linked"));
        }
        Ok((ia, ib))
    }

    /// Adds `tag` to the page's `tags` property. The tag is trimmed, a leading
    /// `#` dropped, and it must be 1 to 64 characters without commas. `None`
    /// when the page has the tag already (compared ignoring case).
    pub fn triage_tag(&self, page: PageId, tag: String) -> Result<Option<TriageAction>> {
        let tag = tag.trim().trim_start_matches('#').trim().to_owned();
        if tag.is_empty() || tag.chars().count() > TAG_MAX || tag.contains(',') {
            return Err(invalid("a tag is 1 to 64 characters without commas"));
        }
        let info = self.triage_info(&page)?;
        if info.in_trash || info.hidden || info.page_type == "database" {
            return Err(invalid(format!("page {:?} cannot be tagged", info.title)));
        }
        let current = self.load_page(page.clone())?.properties;
        let previous = current.iter().find(|p| p.key == TAGS_KEY).and_then(|p| match &p.value {
            PropValue::MultiSelect(v) => Some(v.clone()),
            _ => None,
        });
        if current.iter().any(|p| p.key == TAGS_KEY) && previous.is_none() {
            return Err(invalid("the page's tags property is not a tag list"));
        }
        let mut next = previous.clone().unwrap_or_default();
        if next.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
            return Ok(None);
        }
        next.push(tag.clone());
        self.set_property(page.clone(), TAGS_KEY.into(), PropValue::MultiSelect(next))?;
        Ok(Some(TriageAction {
            kind: TRIAGE_TAG.into(),
            page,
            target: tag,
            block: None,
            previous,
        }))
    }

    /// Takes back one action: a link's block is deleted (if it is still
    /// there), a tag's page gets its previous `tags` value back (the tags
    /// property removed when there was none). Changes made to the page since
    /// stay, except that a tag list is restored whole.
    pub fn triage_undo_action(&self, action: &TriageAction) -> Result<()> {
        match action.kind.as_str() {
            TRIAGE_LINK => {
                let Some(block) = &action.block else {
                    return Err(invalid("the link action has no block"));
                };
                let page = self.load_page(action.page.clone())?;
                fn has(blocks: &[Block], id: &str) -> bool {
                    blocks.iter().any(|b| b.id == id || has(&b.children, id))
                }
                if has(&page.blocks, block) {
                    self.apply(action.page.clone(), vec![BlockOp::Delete { id: block.clone() }])?;
                }
                Ok(())
            }
            TRIAGE_TAG => match &action.previous {
                Some(tags) => self.set_property(
                    action.page.clone(),
                    TAGS_KEY.into(),
                    PropValue::MultiSelect(tags.clone()),
                ),
                None => self.remove_property(action.page.clone(), TAGS_KEY.into()),
            },
            other => Err(invalid(format!("unknown triage action {other:?}"))),
        }
    }
}
