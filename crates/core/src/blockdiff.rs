//! Turning one block tree into another with `BlockOp`s: used by `rescan`
//! (external markdown edits), `restore_version`, duplicates and templates.

use std::collections::{HashMap, HashSet};

use crate::ids::new_id;
use crate::types::{Block, BlockId, BlockOp, NewBlock};

/// Ops that create `blocks` (with their ids) in an empty page.
pub(crate) fn insert_ops(blocks: &[Block]) -> Vec<BlockOp> {
    fn walk(b: &[Block], parent: Option<&BlockId>, ops: &mut Vec<BlockOp>) {
        let mut prev: Option<BlockId> = None;
        for x in b {
            ops.push(BlockOp::Insert {
                parent: parent.cloned(),
                after: prev.clone(),
                block: new_block(x),
            });
            if x.target.is_some() {
                ops.push(BlockOp::SetTarget {
                    id: x.id.clone(),
                    target: x.target.clone(),
                });
            }
            if x.color.is_some() {
                ops.push(BlockOp::SetColor {
                    id: x.id.clone(),
                    color: x.color.clone(),
                });
            }
            if x.width.is_some() {
                ops.push(BlockOp::SetWidth {
                    id: x.id.clone(),
                    width: x.width,
                });
            }
            if x.numbered_start.is_some() {
                ops.push(BlockOp::SetNumberedStart {
                    id: x.id.clone(),
                    numbered_start: x.numbered_start,
                });
            }
            walk(&x.children, Some(&x.id), ops);
            prev = Some(x.id.clone());
        }
    }
    let mut ops = Vec::new();
    walk(blocks, None, &mut ops);
    ops
}

fn new_block(b: &Block) -> NewBlock {
    NewBlock {
        id: b.id.clone(),
        kind: b.kind.clone(),
        text: b.text.clone(),
        marks: b.marks.clone(),
        checked: b.checked,
    }
}

fn same_content(a: &Block, b: &Block) -> bool {
    a.kind == b.kind
        && a.text == b.text
        && a.marks == b.marks
        && a.checked == b.checked
        && a.target == b.target
        && a.numbered_start == b.numbered_start
}

fn flatten<'a>(b: &'a [Block], out: &mut Vec<&'a Block>) {
    for x in b {
        out.push(x);
        flatten(&x.children, out);
    }
}

/// Gives the blocks of `parsed` (which have no ids yet) the ids of the
/// matching blocks of `current`: equal blocks (longest common subsequence in
/// document order) keep their id, and changed blocks between two matches
/// take the ids of the unmatched old blocks there, in order, when their kinds
/// agree. Other blocks get new ids.
pub(crate) fn assign_ids(current: &[Block], parsed: &mut [Block]) {
    let mut cur = Vec::new();
    flatten(current, &mut cur);
    let ids: Vec<String> = {
        let mut new = Vec::new();
        flatten(parsed, &mut new);
        let (n, m) = (cur.len(), new.len());
        let mut matched: Vec<Option<usize>> = vec![None; m];
        // Common prefix and suffix.
        let mut p = 0;
        while p < n && p < m && same_content(cur[p], new[p]) {
            matched[p] = Some(p);
            p += 1;
        }
        let mut s = 0;
        while s < n - p && s < m - p && same_content(cur[n - 1 - s], new[m - 1 - s]) {
            matched[m - 1 - s] = Some(n - 1 - s);
            s += 1;
        }
        let (a, b) = (&cur[p..n - s], &new[p..m - s]);
        if !a.is_empty() && !b.is_empty() && a.len() * b.len() <= 4_000_000 {
            // LCS table over the middle.
            let w = b.len() + 1;
            let mut t = vec![0u32; (a.len() + 1) * w];
            for i in (0..a.len()).rev() {
                for j in (0..b.len()).rev() {
                    t[i * w + j] = if same_content(a[i], b[j]) {
                        t[(i + 1) * w + j + 1] + 1
                    } else {
                        t[(i + 1) * w + j].max(t[i * w + j + 1])
                    };
                }
            }
            let (mut i, mut j) = (0, 0);
            while i < a.len() && j < b.len() {
                if same_content(a[i], b[j]) {
                    matched[p + j] = Some(p + i);
                    i += 1;
                    j += 1;
                } else if t[(i + 1) * w + j] >= t[i * w + j + 1] {
                    i += 1;
                } else {
                    j += 1;
                }
            }
        }
        // Pair up the unmatched blocks between consecutive matches.
        let mut used: HashSet<usize> = matched.iter().flatten().copied().collect();
        let mut j = 0;
        let mut prev_cur = 0usize;
        while j < m {
            if let Some(ci) = matched[j] {
                prev_cur = ci + 1;
                j += 1;
                continue;
            }
            let gap_start = j;
            while j < m && matched[j].is_none() {
                j += 1;
            }
            let next_cur = if j < m { matched[j].unwrap() } else { n };
            let mut olds: Vec<usize> = (prev_cur..next_cur).filter(|c| !used.contains(c)).collect();
            olds.reverse();
            for k in gap_start..j {
                if let Some(pos) = olds.iter().rposition(|&c| cur[c].kind == new[k].kind) {
                    let c = olds.remove(pos);
                    // Keep order: drop olds before the one taken.
                    olds.truncate(pos);
                    used.insert(c);
                    matched[k] = Some(c);
                }
            }
        }
        matched
            .iter()
            .map(|x| match x {
                Some(c) => cur[*c].id.clone(),
                None => new_id(),
            })
            .collect()
    };
    fn set(b: &mut [Block], ids: &mut std::vec::IntoIter<String>) {
        for x in b {
            x.id = ids.next().unwrap_or_else(new_id);
            set(&mut x.children, ids);
        }
    }
    set(parsed, &mut ids.into_iter());
}

/// The ops that turn `current` into `target` (both with ids): moves and
/// field edits keep the ids that exist in both, blocks only in `target` are
/// inserted (with their ids), blocks only in `current` deleted.
pub(crate) fn diff_ops(current: &[Block], target: &[Block]) -> Vec<BlockOp> {
    let mut cur_list = Vec::new();
    flatten(current, &mut cur_list);
    let cur: HashMap<&str, &Block> = cur_list.iter().map(|b| (b.id.as_str(), *b)).collect();
    let mut tgt_list = Vec::new();
    flatten(target, &mut tgt_list);
    let keep: HashSet<&str> = tgt_list.iter().map(|b| b.id.as_str()).collect();
    let deleted: HashSet<String> = cur_list
        .iter()
        .filter(|b| !keep.contains(b.id.as_str()))
        .map(|b| b.id.clone())
        .collect();

    // Model of the current tree, updated as ops are emitted.
    let mut parent: HashMap<String, Option<String>> = HashMap::new();
    let mut children: HashMap<Option<String>, Vec<String>> = HashMap::new();
    fn model(
        b: &[Block],
        p: Option<&String>,
        parent: &mut HashMap<String, Option<String>>,
        children: &mut HashMap<Option<String>, Vec<String>>,
    ) {
        for x in b {
            parent.insert(x.id.clone(), p.cloned());
            children.entry(p.cloned()).or_default().push(x.id.clone());
            model(&x.children, Some(&x.id), parent, children);
        }
    }
    model(current, None, &mut parent, &mut children);

    struct St<'a> {
        ops: Vec<BlockOp>,
        cur: HashMap<&'a str, &'a Block>,
        deleted: HashSet<String>,
        parent: HashMap<String, Option<String>>,
        children: HashMap<Option<String>, Vec<String>>,
    }
    impl St<'_> {
        fn place(&mut self, id: &str, p: Option<&String>, after: Option<&String>) {
            let sib = self.children.entry(p.cloned()).or_default();
            let at = match after {
                Some(a) => sib
                    .iter()
                    .position(|x| x == a)
                    .map(|i| i + 1)
                    .unwrap_or(sib.len()),
                None => 0,
            };
            sib.insert(at, id.to_string());
            self.parent.insert(id.to_string(), p.cloned());
        }
        fn in_place(&self, id: &str, p: Option<&String>, after: Option<&String>) -> bool {
            if self.parent.get(id).map(|x| x.as_ref()) != Some(p) {
                return false;
            }
            let live: Vec<&String> = self
                .children
                .get(&p.cloned())
                .map(|v| v.iter().filter(|x| !self.deleted.contains(*x)).collect())
                .unwrap_or_default();
            let pos = live.iter().position(|x| x.as_str() == id);
            match after {
                None => pos == Some(0),
                Some(a) => match (live.iter().position(|x| *x == a), pos) {
                    (Some(ai), Some(pi)) => pi == ai + 1,
                    _ => false,
                },
            }
        }
        fn walk(&mut self, blocks: &[Block], p: Option<&String>) {
            let mut prev: Option<String> = None;
            for b in blocks {
                match self.cur.get(b.id.as_str()).copied() {
                    Some(old) => {
                        if !self.in_place(&b.id, p, prev.as_ref()) {
                            let op = self.parent.get(&b.id).cloned().flatten();
                            if let Some(v) = self.children.get_mut(&op) {
                                v.retain(|x| x != &b.id);
                            }
                            self.place(&b.id, p, prev.as_ref());
                            self.ops.push(BlockOp::Move {
                                id: b.id.clone(),
                                parent: p.cloned(),
                                after: prev.clone(),
                            });
                        }
                        if old.kind != b.kind {
                            self.ops.push(BlockOp::SetKind {
                                id: b.id.clone(),
                                kind: b.kind.clone(),
                            });
                        }
                        if old.text != b.text || old.marks != b.marks {
                            self.ops.push(BlockOp::SetText {
                                id: b.id.clone(),
                                text: b.text.clone(),
                                marks: b.marks.clone(),
                            });
                        }
                        if old.checked != b.checked {
                            self.ops.push(BlockOp::SetChecked {
                                id: b.id.clone(),
                                checked: b.checked,
                            });
                        }
                        if old.target != b.target {
                            self.ops.push(BlockOp::SetTarget {
                                id: b.id.clone(),
                                target: b.target.clone(),
                            });
                        }
                        if old.color != b.color {
                            self.ops.push(BlockOp::SetColor {
                                id: b.id.clone(),
                                color: b.color.clone(),
                            });
                        }
                        if old.width != b.width {
                            self.ops.push(BlockOp::SetWidth {
                                id: b.id.clone(),
                                width: b.width,
                            });
                        }
                        if old.numbered_start != b.numbered_start {
                            self.ops.push(BlockOp::SetNumberedStart {
                                id: b.id.clone(),
                                numbered_start: b.numbered_start,
                            });
                        }
                    }
                    None => {
                        self.ops.push(BlockOp::Insert {
                            parent: p.cloned(),
                            after: prev.clone(),
                            block: new_block(b),
                        });
                        if b.target.is_some() {
                            self.ops.push(BlockOp::SetTarget {
                                id: b.id.clone(),
                                target: b.target.clone(),
                            });
                        }
                        if b.color.is_some() {
                            self.ops.push(BlockOp::SetColor {
                                id: b.id.clone(),
                                color: b.color.clone(),
                            });
                        }
                        if b.width.is_some() {
                            self.ops.push(BlockOp::SetWidth {
                                id: b.id.clone(),
                                width: b.width,
                            });
                        }
                        if b.numbered_start.is_some() {
                            self.ops.push(BlockOp::SetNumberedStart {
                                id: b.id.clone(),
                                numbered_start: b.numbered_start,
                            });
                        }
                        self.place(&b.id, p, prev.as_ref());
                    }
                }
                self.walk(&b.children, Some(&b.id));
                prev = Some(b.id.clone());
            }
        }
    }
    let mut st = St {
        ops: Vec::new(),
        cur,
        deleted,
        parent,
        children,
    };
    st.walk(target, None);
    // Delete the topmost removed blocks (their removed children go with them).
    for b in &cur_list {
        if st.deleted.contains(&b.id) {
            let p = st.parent.get(&b.id).cloned().flatten();
            if !p.as_ref().is_some_and(|p| st.deleted.contains(p)) {
                st.ops.push(BlockOp::Delete { id: b.id.clone() });
            }
        }
    }
    st.ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::BlockKind;

    fn b(id: &str, text: &str, children: Vec<Block>) -> Block {
        Block {
            id: id.into(),
            kind: BlockKind::Paragraph,
            text: text.into(),
            marks: vec![],
            checked: false,
            children,
            target: None,
            color: None,
            width: None,
            numbered_start: None,
        }
    }

    #[test]
    fn assigns_ids_to_unchanged_and_edited_blocks() {
        let cur = vec![
            b("a", "one", vec![]),
            b("b", "two", vec![b("b1", "child", vec![])]),
            b("c", "three", vec![]),
        ];
        let mut parsed = vec![
            b("", "one", vec![]),
            b("", "two!", vec![b("", "child", vec![])]),
            b("", "new", vec![]),
            b("", "three", vec![]),
        ];
        assign_ids(&cur, &mut parsed);
        assert_eq!(parsed[0].id, "a");
        assert_eq!(parsed[1].id, "b");
        assert_eq!(parsed[1].children[0].id, "b1");
        assert_eq!(parsed[3].id, "c");
        assert!(!["a", "b", "b1", "c"].contains(&parsed[2].id.as_str()));
    }
}
