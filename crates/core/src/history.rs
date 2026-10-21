//! Page history from the Loro change log. Changes are sorted causally
//! (Lamport time) and grouped into versions: a new version starts when more
//! than 5 minutes pass between two changes or when the change comes from
//! another device (the `peers` map of the document says which device a Loro
//! peer belongs to).

use std::collections::{HashMap, HashSet};

use loro::{ContainerID, ExportMode, Frontiers, LoroDoc, VersionVector, ID};

use crate::doc::PageDoc;
use crate::error::{invalid, Result};
#[cfg(feature = "storage-metrics")]
use crate::storage_metrics;
use crate::types::Version;

/// Idle time that starts a new version.
pub(crate) const IDLE_SECS: i64 = 5 * 60;

/// One Loro change, reduced to what grouping needs.
#[derive(Debug, Clone)]
pub(crate) struct ChangeInfo {
    pub peer: u64,
    pub counter: i32,
    pub len: usize,
    pub lamport: u32,
    /// Seconds since the epoch.
    pub timestamp: i64,
}

/// Every change of the document.
pub(crate) fn changes(pd: &PageDoc) -> Vec<ChangeInfo> {
    let mut out = Vec::new();
    let vv = pd.doc.oplog_vv();
    for (&peer, &end) in vv.iter() {
        let mut counter = 0;
        while counter < end {
            let Some(meta) = pd.doc.get_change(ID::new(peer, counter)) else {
                break;
            };
            out.push(ChangeInfo {
                peer,
                counter: meta.id.counter,
                len: meta.len,
                lamport: meta.lamport,
                timestamp: meta.timestamp,
            });
            counter = meta.id.counter + meta.len as i32;
        }
    }
    out.sort_by_key(|c| (c.lamport, c.peer));
    out
}

/// The newest causal prefix made entirely before the cutoff. Timestamps
/// need not be monotonic (offline peers can have older clocks), so stop at
/// the first recent change rather than moving its old descendants to cold.
pub(crate) fn cutoff_root(pd: &PageDoc, cutoff_seconds: i64) -> Result<Option<Frontiers>> {
    let mut vv = VersionVector::new();
    for c in changes(pd) {
        if c.timestamp >= cutoff_seconds {
            break;
        }
        vv.set_end(ID::new(c.peer, c.counter + c.len as i32));
    }
    Ok((!vv.is_empty()).then(|| pd.doc.vv_to_frontiers(&vv)))
}

/// Preserve original op IDs and state while trimming the operation log.
/// Loro's native search may retreat across serial peer switches. At a
/// critical root (every retained change depends on it), exporting the root
/// prefix first and appending the original tail avoids that retreat.
pub(crate) fn hot_snapshot(pd: &PageDoc, requested: &Frontiers) -> Result<Vec<u8>> {
    #[cfg(feature = "storage-metrics")]
    let _h = storage_metrics::span("history.hot_snapshot");
    let bytes = {
        #[cfg(feature = "storage-metrics")]
        let _e = storage_metrics::span("loro.export_shallow_snapshot");
        pd.doc.export(ExportMode::shallow_snapshot(requested))?
    };
    let hot = LoroDoc::new();
    hot.import(&bytes)?;
    if hot.shallow_since_frontiers() == *requested {
        return Ok(bytes);
    }
    let root_vv = pd.doc.frontiers_to_vv(requested)
        .ok_or_else(|| invalid("cold root missing from history"))?;
    let critical = changes(pd).iter().all(|c| {
        let included = root_vv.get(&c.peer).copied().unwrap_or(0);
        if c.counter + c.len as i32 <= included { return true; }
        pd.doc.frontiers_to_vv(&Frontiers::from(ID::new(c.peer, c.counter.max(included))))
            .is_some_and(|vv| vv.includes_vv(&root_vv))
    });
    if !critical {
        // A concurrent offline edit can make a previous root noncritical.
        // Retaining more history hot is safe; never discard that branch.
        return Ok(bytes);
    }
    let prefix = {
        #[cfg(feature = "storage-metrics")]
        let _f = storage_metrics::span("loro.fork_at");
        pd.doc.fork_at(requested)?
    };
    let composed = LoroDoc::new();
    composed.import(&{
        #[cfg(feature = "storage-metrics")]
        let _e = storage_metrics::span("loro.export_shallow_snapshot");
        prefix.export(ExportMode::shallow_snapshot(requested))?
    })?;
    if composed.shallow_since_frontiers() != *requested {
        return Ok(bytes);
    }
    composed.import(&{
        #[cfg(feature = "storage-metrics")]
        let _e = storage_metrics::span("loro.export_updates");
        pd.doc.export(ExportMode::updates(&root_vv))?
    })?;
    pd.materialize_roots_in(&composed);
    {
        #[cfg(feature = "storage-metrics")]
        let _c = storage_metrics::span("history.verify_compare");
        if composed.oplog_vv() != pd.doc.oplog_vv()
            || composed.oplog_frontiers() != pd.doc.oplog_frontiers()
            || composed.get_deep_value() != pd.doc.get_deep_value() {
            return Err(invalid("shallow snapshot changed page state or operation IDs"));
        }
    }
    #[cfg(feature = "storage-metrics")]
    let _s = storage_metrics::span("loro.export_snapshot");
    Ok(composed.export(ExportMode::Snapshot)?)
}

/// Groups sorted changes: returns the index ranges of each group.
pub(crate) fn group(changes: &[ChangeInfo], device: &dyn Fn(u64) -> String) -> Vec<(usize, usize)> {
    let mut groups = Vec::new();
    let mut start = 0;
    for i in 1..=changes.len() {
        let split = i == changes.len()
            || device(changes[i].peer) != device(changes[i - 1].peer)
            || changes[i].timestamp - changes[i - 1].timestamp > IDLE_SECS;
        if split {
            if i > start {
                groups.push((start, i));
            }
            start = i;
        }
    }
    groups
}

fn encode_frontiers(f: &Frontiers) -> String {
    let mut ids: Vec<String> = f
        .iter()
        .map(|id| format!("{}@{}", id.peer, id.counter))
        .collect();
    ids.sort();
    ids.join(",")
}

pub(crate) fn decode_version(s: &str) -> Result<Frontiers> {
    let mut ids = Vec::new();
    for part in s.split(',').filter(|p| !p.is_empty()) {
        let (p, c) = part
            .split_once('@')
            .ok_or_else(|| invalid(format!("bad version id {s}")))?;
        let peer: u64 = p
            .parse()
            .map_err(|_| invalid(format!("bad version id {s}")))?;
        let counter: i32 = c
            .parse()
            .map_err(|_| invalid(format!("bad version id {s}")))?;
        ids.push(ID::new(peer, counter));
    }
    if ids.is_empty() {
        return Err(invalid("empty version id"));
    }
    Ok(Frontiers::from(ids))
}

fn summarize(pd: &PageDoc, cs: &[ChangeInfo]) -> String {
    let mut parts: HashSet<&'static str> = HashSet::new();
    for c in cs {
        for cid in pd
            .doc
            .get_changed_containers_in(ID::new(c.peer, c.counter), c.len)
        {
            let part = match &cid {
                ContainerID::Root { name, .. } => match name.as_str() {
                    "page" => "details",
                    "props" => "properties",
                    "schema" | "schema_order" => "schema",
                    "views" | "view_order" => "views",
                    "blocks" => "content",
                    _ => continue,
                },
                ContainerID::Normal { .. } => "content",
            };
            parts.insert(part);
        }
    }
    let order = ["content", "details", "properties", "schema", "views"];
    let list: Vec<&str> = order
        .iter()
        .copied()
        .filter(|p| parts.contains(p))
        .collect();
    if list.is_empty() {
        "No visible changes".into()
    } else {
        format!("Edited {}", list.join(", "))
    }
}

/// Versions of a page, newest first.
pub(crate) fn versions(pd: &PageDoc) -> Vec<Version> {
    let cs = changes(pd);
    let peers: HashMap<String, String> = pd.peers();
    let device = |peer: u64| -> String {
        peers
            .get(&peer.to_string())
            .cloned()
            .unwrap_or_else(|| format!("peer-{peer:016x}"))
    };
    let mut vv = VersionVector::new();
    let mut out = Vec::new();
    for (i, (s, e)) in group(&cs, &device).into_iter().enumerate() {
        for c in &cs[s..e] {
            vv.set_end(ID::new(c.peer, c.counter + c.len as i32));
        }
        let frontiers = pd.doc.vv_to_frontiers(&vv);
        let summary = if i == 0 {
            "Created".to_string()
        } else {
            summarize(pd, &cs[s..e])
        };
        out.push(Version {
            id: encode_frontiers(&frontiers),
            time_ms: cs[e - 1].timestamp * 1000,
            device: device(cs[e - 1].peer),
            summary,
            changes: (e - s) as u32,
        });
    }
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_by_idle_time_and_device() {
        let pd = PageDoc::new_page(1, "P", "t", None, 0).unwrap();
        let mut pd = pd;
        let commit = |pd: &mut PageDoc, ts: i64, title: &str| {
            pd.doc.set_next_commit_timestamp(ts);
            pd.set_title(title).unwrap();
            pd.doc.commit();
        };
        // Loro timestamps are seconds; start well after the creation commit.
        let t = crate::ids::now_ms() / 1000 + 10;
        pd.register_peer("dev-a").unwrap();
        pd.doc.commit();
        commit(&mut pd, t + 100, "a");
        commit(&mut pd, t + 200, "b"); // same version
        commit(&mut pd, t + 400, "c"); // 200 s later: same version
        commit(&mut pd, t + 800, "d"); // 400 s idle: new version
                                       // Another device.
        let mut other = PageDoc::from_bytes(&pd.snapshot().unwrap(), &[]).unwrap();
        other.doc.set_peer_id(777).unwrap();
        other.register_peer("dev-b").unwrap();
        commit(&mut other, t + 810, "e");
        let vs = versions(&other);
        let devices: Vec<&str> = vs.iter().map(|v| v.device.as_str()).collect();
        assert_eq!(vs.len(), 3, "{vs:?}");
        assert_eq!(devices[0], "dev-b");
        assert_eq!(devices[1], "dev-a");
        assert_eq!(vs[1].time_ms, (t + 800) * 1000);
        // Loading a version shows its state.
        let old = other.fork_at(&decode_version(&vs[1].id).unwrap()).unwrap();
        assert_eq!(old.meta().title, "d");
        let first = other.fork_at(&decode_version(&vs[2].id).unwrap()).unwrap();
        assert_eq!(first.meta().title, "c");
    }
}
