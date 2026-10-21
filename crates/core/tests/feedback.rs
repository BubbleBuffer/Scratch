use std::{fs, path::PathBuf, sync::Arc};
use vault_core::*;

struct TempVault(PathBuf);

impl TempVault {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("vault-feedback-{}", new_id())))
    }
    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).unwrap()
    }
}

impl Drop for TempVault {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_hidden(v: &Vault, ids: &[String]) {
    for page in v.list_pages(None, 0, 100).unwrap() {
        assert!(!ids.contains(&page.id));
    }
    for query in ["quasar", "qsr", "fdbck", "Feedback", "", "type:document"] {
        for hit in v.search(query.into(), 100).unwrap() {
            assert!(!ids.contains(&hit.id), "search {query:?} found {}", hit.id);
        }
        for hit in v.search_full(query.into(), 100).unwrap() {
            assert!(
                !ids.contains(&hit.id),
                "full search {query:?} found {}",
                hit.id
            );
        }
    }
    let parsed = v.parse_search("quasar sort:updated".into()).unwrap();
    let results = v.search_spec(parsed.spec, 100, 0).unwrap();
    assert!(results.hits.is_empty());
    assert_eq!(results.total_bounded, 0);
}

#[test]
fn feedback_is_lazy_fixed_hidden_and_addressable() {
    let t = TempVault::new();
    let v = t.open();
    assert_eq!(v.find_by_role("feedback".into()).unwrap(), None);
    let visible = v.create_page(None, "Ordinary page".into()).unwrap();
    let feedback = v.feedback_page().unwrap();
    assert_eq!(feedback, "000000000000000000000000FB");
    assert_eq!(v.feedback_page().unwrap(), feedback);
    assert_eq!(v.load_page(feedback.clone()).unwrap().title, "Feedback");
    assert_eq!(
        v.load_page(feedback.clone()).unwrap().role.as_deref(),
        Some("feedback")
    );
    let report = v
        .create_page(Some(feedback.clone()), "Quasar report".into())
        .unwrap();
    let nested = v
        .create_page(Some(report.clone()), "Quasar details".into())
        .unwrap();
    for id in [&feedback, &report, &nested] {
        v.insert_markdown(
            id.clone(),
            None,
            None,
            true,
            "quasar confidential body".into(),
        )
        .unwrap();
    }
    v.flush().unwrap();
    let ids = vec![feedback.clone(), report.clone(), nested.clone()];
    assert_hidden(&v, &ids);
    assert_eq!(v.page_count().unwrap(), 1);
    assert_eq!(
        v.list_children(None, 0, 100)
            .unwrap()
            .iter()
            .map(|p| &p.id)
            .collect::<Vec<_>>(),
        vec![&visible]
    );
    assert_eq!(
        v.list_children(Some(feedback.clone()), 0, 100).unwrap()[0].id,
        report
    );
    assert_eq!(
        v.list_pages(Some(feedback.clone()), 0, 100).unwrap()[0].id,
        report
    );
    assert_eq!(
        v.list_children(Some(report.clone()), 0, 100).unwrap()[0].id,
        nested
    );
    assert!(v.load_page(report.clone()).unwrap().blocks[0]
        .text
        .contains("quasar"));
    v.reindex().unwrap();
    assert_hidden(&v, &ids);
    drop(v);
    let reopened = t.open();
    assert_eq!(reopened.feedback_page().unwrap(), feedback);
    assert_hidden(&reopened, &ids);
}

#[test]
fn moving_subtrees_and_changing_roles_refreshes_warm_search() {
    let t = TempVault::new();
    let v = t.open();
    let feedback = v.feedback_page().unwrap();
    let parent = v.create_page(None, "Quasar parent".into()).unwrap();
    let child = v
        .create_page(Some(parent.clone()), "Quasar child".into())
        .unwrap();
    assert_eq!(v.search("qsr".into(), 100).unwrap().len(), 2);
    v.move_page(parent.clone(), Some(feedback.clone()), None)
        .unwrap();
    assert_hidden(&v, &[feedback.clone(), parent.clone(), child.clone()]);
    assert_eq!(v.page_count().unwrap(), 0);
    v.move_page(parent.clone(), None, None).unwrap();
    assert_eq!(v.search("qsr".into(), 100).unwrap().len(), 2);
    v.set_page_role(parent.clone(), Some("feedback".into()))
        .unwrap();
    assert_hidden(&v, &[feedback, parent.clone(), child]);
    v.set_page_role(parent, None).unwrap();
    assert_eq!(v.search("qsr".into(), 100).unwrap().len(), 2);
}

#[test]
fn concurrent_initialization_returns_one_feedback_page() {
    let t = TempVault::new();
    let v = Arc::new(t.open());
    std::thread::scope(|scope| {
        let handles = (0..4)
            .map(|_| scope.spawn(|| v.feedback_page().unwrap()))
            .collect::<Vec<_>>();
        for handle in handles {
            assert_eq!(handle.join().unwrap(), "000000000000000000000000FB");
        }
    });
    assert_eq!(v.page_count().unwrap(), 0);
}
