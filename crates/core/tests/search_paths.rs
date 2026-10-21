//! Path patterns in `in:` (see `search_path`): globs over ancestor titles,
//! quoted segments, negation, case, an exact title that contains `/`, item
//! results, printed queries that survive a rename, the plain `in:` values of
//! saved queries, and live queries that follow renames.
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use vault_core::*;

struct TempVault(PathBuf);

impl TempVault {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("vault_search_paths_{}", new_id())))
    }

    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).unwrap()
    }
}

impl Drop for TempVault {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The test's own copy of the tree. Subtrees are walked here, so the
/// expected sets do not depend on the search code under test.
#[derive(Default)]
struct Tree {
    pages: BTreeSet<PageId>,
    edges: Vec<(PageId, PageId)>,
}

impl Tree {
    fn add(&mut self, v: &Vault, parent: Option<&PageId>, title: &str) -> PageId {
        let id = v.create_page(parent.cloned(), title.into()).unwrap();
        self.pages.insert(id.clone());
        if let Some(parent) = parent {
            self.edges.push((parent.clone(), id.clone()));
        }
        id
    }

    /// The page and everything below it.
    fn subtree(&self, root: &PageId) -> BTreeSet<PageId> {
        let mut out = BTreeSet::from([root.clone()]);
        let mut stack = vec![root.clone()];
        while let Some(page) = stack.pop() {
            for (parent, child) in &self.edges {
                if *parent == page && out.insert(child.clone()) {
                    stack.push(child.clone());
                }
            }
        }
        out
    }
}

/// Work
///   Q3 notes
///     Plan
///   Meetings
/// Home
///   Meetings
/// a/b (one top-level title containing a slash)
///   Inside
struct Fixture {
    tree: Tree,
    work: PageId,
    q3: PageId,
    plan: PageId,
    meetings: PageId,
    home: PageId,
    home_meetings: PageId,
    ab: PageId,
    inside: PageId,
}

fn build(v: &Vault) -> Fixture {
    let mut tree = Tree::default();
    let work = tree.add(v, None, "Work");
    let q3 = tree.add(v, Some(&work), "Q3 notes");
    let plan = tree.add(v, Some(&q3), "Plan");
    let meetings = tree.add(v, Some(&work), "Meetings");
    let home = tree.add(v, None, "Home");
    let home_meetings = tree.add(v, Some(&home), "Meetings");
    let ab = tree.add(v, None, "a/b");
    let inside = tree.add(v, Some(&ab), "Inside");
    let fixture = Fixture {
        tree,
        work,
        q3,
        plan,
        meetings,
        home,
        home_meetings,
        ab,
        inside,
    };
    todo(v, &fixture.work, "t-work", "Work todo");
    todo(v, &fixture.plan, "t-plan", "Plan todo");
    todo(v, &fixture.meetings, "t-meet", "Meeting todo");
    todo(v, &fixture.home_meetings, "t-home", "Home meeting todo");
    v.flush().unwrap();
    fixture
}

fn todo(v: &Vault, page: &PageId, id: &str, text: &str) {
    v.apply(
        page.clone(),
        vec![BlockOp::Insert {
            parent: None,
            after: None,
            block: NewBlock {
                id: id.into(),
                kind: BlockKind::Todo,
                text: text.into(),
                marks: vec![],
                checked: false,
            },
        }],
    )
    .unwrap();
}

fn parse(v: &Vault, query: &str) -> SearchSpec {
    v.parse_search(query.into()).unwrap().spec
}

/// Page ids a page query returns.
fn ids(v: &Vault, query: &str) -> BTreeSet<PageId> {
    v.search_spec(parse(v, query), 100, 0)
        .unwrap()
        .hits
        .into_iter()
        .map(|h| h.id)
        .collect()
}

fn set(ids: &[&PageId]) -> BTreeSet<PageId> {
    ids.iter().map(|id| (*id).clone()).collect()
}

fn assert_ids(v: &Vault, query: &str, expected: BTreeSet<PageId>) {
    assert_eq!(ids(v, query), expected, "query {query:?}");
}

/// Each item's block id, or its title for task notes and rows.
fn item_keys(v: &Vault, query: &str) -> BTreeSet<String> {
    let spec = parse(v, query);
    assert!(spec.selects_items(), "{query:?} selects items");
    v.search_items(&spec, 500, 0)
        .unwrap()
        .items
        .into_iter()
        .map(|h| h.block.unwrap_or(h.title))
        .collect()
}

fn keys(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|k| (*k).to_string()).collect()
}

#[test]
fn star_matches_one_segment_and_double_star_any_depth() {
    let t = TempVault::new();
    let v = t.open();
    let f = build(&v);
    // `*` is one segment: Work's direct children, not Work itself.
    assert_ids(&v, "in:Work/*", set(&[&f.q3, &f.meetings]));
    assert_ids(&v, "in:[[Work]]/*", set(&[&f.q3, &f.meetings]));
    // `**` is any depth: all descendants, not Work itself.
    assert_ids(&v, "in:Work/**", set(&[&f.q3, &f.plan, &f.meetings]));
    // A glob at the front matches any page with that title.
    assert_ids(&v, "in:**/Meetings", set(&[&f.meetings, &f.home_meetings]));
    assert_ids(&v, "in:**/Work", set(&[&f.work]));
    assert_ids(&v, "in:Work/**/Plan", set(&[&f.plan]));
    // A partial glob inside one segment.
    assert_ids(&v, "in:Work/Q*", set(&[&f.q3]));
    // A pattern with no glob is that page and its subtree.
    assert_ids(&v, "in:Work/\"Q3 notes\"", set(&[&f.q3, &f.plan]));
    assert_ids(&v, "in:Work/Meetings", set(&[&f.meetings]));
}

#[test]
fn quoted_segments_keep_their_spaces() {
    let t = TempVault::new();
    let v = t.open();
    let f = build(&v);
    assert_ids(&v, "in:Work/\"Q3 notes\"/*", set(&[&f.plan]));
    assert_ids(&v, "in:Work/\"Q3 notes\"", set(&[&f.q3, &f.plan]));
}

#[test]
fn negation_takes_pattern_pages_out_of_the_result() {
    let t = TempVault::new();
    let v = t.open();
    let f = build(&v);
    assert_ids(&v, "in:Work -in:Work/**", set(&[&f.work]));
    assert_ids(&v, "in:Work -in:Work/*", set(&[&f.work, &f.plan]));
    let everything: BTreeSet<PageId> = f.tree.pages.clone();
    assert_eq!(
        ids(&v, "-in:Work"),
        everything
            .difference(&f.tree.subtree(&f.work))
            .cloned()
            .collect::<BTreeSet<_>>(),
        "-in:Work is every page outside Work's subtree"
    );
}

#[test]
fn patterns_ignore_case() {
    let t = TempVault::new();
    let v = t.open();
    let f = build(&v);
    assert_ids(&v, "in:work/*", set(&[&f.q3, &f.meetings]));
    assert_ids(&v, "in:**/meetings", set(&[&f.meetings, &f.home_meetings]));
    assert_ids(&v, "in:WORK/Q*", set(&[&f.q3]));
}

#[test]
fn an_exact_title_with_a_slash_is_still_a_page() {
    let t = TempVault::new();
    let v = t.open();
    let f = build(&v);
    // `a/b` names no path (no page titled `a`), and the exact title wins
    // over reading it as one: the page and its subtree.
    assert_ids(&v, "in:a/b", set(&[&f.ab, &f.inside]));
}

#[test]
fn item_searches_follow_path_patterns() {
    let t = TempVault::new();
    let v = t.open();
    let _f = build(&v);
    assert_eq!(
        item_keys(&v, "is:todo in:Work/**"),
        keys(&["t-plan", "t-meet"])
    );
    assert_eq!(
        item_keys(&v, "is:todo in:**/Meetings"),
        keys(&["t-meet", "t-home"])
    );
    assert_eq!(
        item_keys(&v, "is:todo -in:Work/**"),
        keys(&["t-work", "t-home"])
    );
}

#[test]
fn printed_query_keeps_the_first_segment_as_an_id() {
    let t = TempVault::new();
    let v = t.open();
    let f = build(&v);
    let spec = parse(&v, "in:Work/*");
    let printed = spec.to_query_string();
    assert!(
        printed.contains(&format!("[[{}]]", f.work)),
        "printed {printed:?} names Work by id"
    );
    assert!(
        !printed.contains("Work"),
        "printed {printed:?} has no title"
    );
    let before = ids(&v, "in:Work/*");
    assert_eq!(before, set(&[&f.q3, &f.meetings]));
    v.rename_page(f.work.clone(), "Renamed work".into())
        .unwrap();
    v.flush().unwrap();
    assert_eq!(ids(&v, &printed), before, "printed query after a rename");
    assert_eq!(
        ids(&v, "in:\"Renamed work\"/*"),
        before,
        "the new title reaches the same pages"
    );
}

#[test]
fn plain_in_values_keep_their_page_and_subtree_meaning() {
    let t = TempVault::new();
    let v = t.open();
    let f = build(&v);
    let work = f.tree.subtree(&f.work);
    let q3 = f.tree.subtree(&f.q3);
    let home = f.tree.subtree(&f.home);
    let union: BTreeSet<PageId> = work.union(&home).cloned().collect();
    let outside: BTreeSet<PageId> = f.tree.pages.difference(&work).cloned().collect();

    assert_ids(&v, "in:Work", work.clone());
    assert_ids(&v, "in:[[Work]]", work.clone());
    assert_ids(&v, "in:\"Q3 notes\"", q3);
    assert_ids(&v, "in:Work,Home", union);
    assert_ids(&v, "-in:Work", outside);

    let printed = parse(&v, "in:Work").to_query_string();
    assert_eq!(printed, format!("in:[[{}]]", f.work));
    let printed = parse(&v, "in:[[Work]]").to_query_string();
    assert_eq!(printed, format!("in:[[{}]]", f.work));
    let printed = parse(&v, "in:Work,Home").to_query_string();
    assert!(
        printed.contains(&format!("[[{}]]", f.work))
            && printed.contains(&format!("[[{}]]", f.home)),
        "{printed}"
    );
    let printed = parse(&v, "-in:Work").to_query_string();
    assert!(printed.contains(&format!("[[{}]]", f.work)), "{printed}");
}

#[test]
fn live_pattern_query_drops_a_renamed_child() {
    let t = TempVault::new();
    let v = t.open();
    let f = build(&v);
    let subscription = v.watch_query(parse(&v, "in:**/Meetings"), 10).unwrap();
    let initial = subscription.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(initial.initial);
    let added: BTreeSet<PageId> = initial.added.iter().map(|h| h.id.clone()).collect();
    assert_eq!(added, set(&[&f.meetings, &f.home_meetings]));

    v.rename_page(f.meetings.clone(), "Gatherings".into())
        .unwrap();
    v.flush().unwrap();
    let key = HitKey {
        source: VAULT_SOURCE.into(),
        id: f.meetings.clone(),
    };
    let update = loop {
        let next = subscription.recv_timeout(Duration::from_secs(5)).unwrap();
        if !next.removed.is_empty() {
            break next;
        }
    };
    assert!(update.removed.contains(&key), "{:?}", update.removed);
}

#[test]
fn live_pattern_query_follows_an_ancestor_rename() {
    let t = TempVault::new();
    let v = t.open();
    let f = build(&v);
    // `**/Work/*` reads ancestor titles, so renaming Work moves its children
    // out of the pattern even though they did not change.
    let subscription = v.watch_query(parse(&v, "in:**/Work/*"), 10).unwrap();
    let initial = subscription.recv_timeout(Duration::from_secs(5)).unwrap();
    let added: BTreeSet<PageId> = initial.added.iter().map(|h| h.id.clone()).collect();
    assert_eq!(added, set(&[&f.q3, &f.meetings]));

    v.rename_page(f.work.clone(), "Office".into()).unwrap();
    v.flush().unwrap();
    let mut removed = BTreeSet::new();
    while removed.len() < 2 {
        let next = subscription.recv_timeout(Duration::from_secs(5)).unwrap();
        removed.extend(next.removed.into_iter().map(|k| k.id));
    }
    assert_eq!(removed, set(&[&f.q3, &f.meetings]));
}
