use std::{fs, path::PathBuf};

use vault_core::*;
use vault_mcp::{runners, tasks};

const NEW_KEYS: [&str; 3] = ["project", "worktree_branch", "worktree_path"];

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("vault_code_tasks_{}", new_id())))
    }
    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).unwrap()
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn an_older_tasks_database_gains_the_project_properties_and_keeps_its_own() {
    let t = Temp::new();
    let v = t.open();
    // The schema older builds wrote: no code-task properties. The database's
    // owner added a status option and a property of their own.
    let mut old: Vec<PropDef> = tasks::schema()
        .into_iter()
        .filter(|d| !NEW_KEYS.contains(&d.key.as_str()))
        .collect();
    for d in old.iter_mut().filter(|d| d.key == "status") {
        d.options.push(SelectOption {
            id: "waiting".into(),
            name: "Waiting".into(),
            color: "blue".into(),
        });
    }
    old.push(PropDef {
        key: "mood".into(),
        name: "Mood".into(),
        kind: PropKind::Text,
        options: vec![],
        config: String::new(),
    });
    let coll = v.create_page(None, tasks::TITLE.into()).unwrap();
    v.make_collection(coll.clone()).unwrap();
    v.set_schema(coll.clone(), old.clone()).unwrap();
    v.set_property(
        coll.clone(),
        tasks::MARKER.into(),
        PropValue::Checkbox(true),
    )
    .unwrap();
    let existing_row = v
        .create_page(Some(coll.clone()), "Existing work".into())
        .unwrap();
    v.set_property(
        existing_row.clone(),
        "status".into(),
        PropValue::Select("waiting".into()),
    )
    .unwrap();
    v.set_property(
        existing_row.clone(),
        "mood".into(),
        PropValue::Text("keep this value".into()),
    )
    .unwrap();
    let existing_before = v.load_page(existing_row.clone()).unwrap();

    tasks::migrate_schema(&v, &coll).unwrap();
    let migrated = v.get_schema(coll.clone()).unwrap();
    for key in NEW_KEYS {
        assert!(
            migrated
                .iter()
                .any(|d| d.key == key && d.kind == PropKind::Text),
            "{key}"
        );
    }
    assert_eq!(v.load_page(existing_row).unwrap(), existing_before);
    // Every property and option the old schema had is still there.
    for d in &old {
        let have = migrated.iter().find(|x| x.key == d.key).unwrap();
        assert_eq!(have.name, d.name, "{}", d.key);
        for o in &d.options {
            assert!(have.options.contains(o), "{} {}", d.key, o.id);
        }
    }

    // Running the migration again changes nothing.
    tasks::migrate_schema(&v, &coll).unwrap();
    assert_eq!(v.get_schema(coll.clone()).unwrap(), migrated);

    // A code task can be created on the migrated database.
    v.publish_device_projects(vec![DeviceProject {
        id: "app".into(),
        name: "App".into(),
    }])
    .unwrap();
    let page = v.create_page(None, "Plan".into()).unwrap();
    let task = tasks::create_for_project(&v, "Fix it", &page, None, "", "", Some("app")).unwrap();
    assert_eq!(task.project, "app");
    assert_eq!(task.worktree_branch, "");
    assert_eq!(task.worktree_path, "");
    assert_eq!(v.get_schema(coll).unwrap(), migrated);
    assert_eq!(
        runners::list_at(&v, true, true, 0).unwrap().runners.len(),
        2
    );
}
