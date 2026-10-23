//! Desktop projects and session cwd assignments, stored beside agent.sqlite.
use crate::Project;
use anyhow::{anyhow, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Clone, Serialize, Deserialize, Default)]
struct State {
    projects: Vec<Project>,
    #[serde(default)]
    sessions: BTreeMap<String, String>,
    #[serde(default)]
    recent: Vec<String>,
}

pub struct Projects {
    file: PathBuf,
    state: Mutex<State>,
}

pub(crate) fn save_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(&temp, path).with_context(|| format!("save {}", path.display()))?;
    Ok(())
}

impl Projects {
    pub fn open(file: PathBuf, seed: &Path) -> Result<Self> {
        let state = if file.exists() {
            let mut state: State = serde_json::from_slice(&std::fs::read(&file)?)
                .with_context(|| format!("read {}", file.display()))?;
            ensure!(
                !state.projects.is_empty(),
                "projects.json contains no projects"
            );
            // Older files keep the verbatim prefix on disk; the next save writes the clean form.
            for project in &mut state.projects {
                project.path = stored_path(&project.path);
            }
            state
        } else {
            let project = make_project("Home", seed)?;
            let state = State {
                recent: vec![project.id.clone()],
                projects: vec![project],
                ..State::default()
            };
            save_json(&file, &state)?;
            state
        };
        Ok(Self {
            file,
            state: Mutex::new(state),
        })
    }
    fn update<T>(&self, f: impl FnOnce(&mut State) -> Result<T>) -> Result<T> {
        let mut guard = self.state.lock().unwrap();
        let mut next = guard.clone();
        let out = f(&mut next)?;
        save_json(&self.file, &next)?;
        *guard = next;
        Ok(out)
    }
    pub fn list(&self) -> Vec<Project> {
        self.state.lock().unwrap().projects.clone()
    }
    pub fn current(&self, session: &str) -> Result<Project> {
        let state = self.state.lock().unwrap();
        let id = state.sessions.get(session).or_else(|| state.recent.first());
        state
            .projects
            .iter()
            .find(|p| Some(&p.id) == id)
            .or_else(|| state.projects.first())
            .cloned()
            .ok_or_else(|| anyhow!("no projects"))
    }
    pub fn ensure_session(&self, session: &str) -> Result<Project> {
        {
            let state = self.state.lock().unwrap();
            if let Some(project) = state
                .sessions
                .get(session)
                .and_then(|id| state.projects.iter().find(|p| &p.id == id))
            {
                return Ok(project.clone());
            }
        }
        self.update(|state| {
            let id = state.sessions.get(session).or_else(|| state.recent.first());
            let project = state
                .projects
                .iter()
                .find(|p| Some(&p.id) == id)
                .or_else(|| state.projects.first())
                .cloned()
                .ok_or_else(|| anyhow!("no projects"))?;
            state.sessions.insert(session.into(), project.id.clone());
            Ok(project)
        })
    }
    pub fn forget_session(&self, session: &str) -> Result<()> {
        self.update(|state| {
            state.sessions.remove(session);
            Ok(())
        })
    }
    pub fn switch(&self, session: &str, project: &str) -> Result<Project> {
        self.update(|state| {
            let selected = state
                .projects
                .iter()
                .find(|p| p.id == project)
                .or_else(|| state.projects.iter().find(|p| p.name == project))
                .cloned()
                .ok_or_else(|| anyhow!("unknown project {project:?}"))?;
            state.sessions.insert(session.into(), selected.id.clone());
            state.recent.retain(|id| id != &selected.id);
            state.recent.insert(0, selected.id.clone());
            Ok(selected)
        })
    }
    pub fn add(&self, name: &str, path: &str) -> Result<Project> {
        let project = make_project(name, Path::new(path))?;
        self.update(|state| {
            ensure!(
                !state
                    .projects
                    .iter()
                    .any(|p| same_path(&p.path, &project.path)),
                "folder is already a project"
            );
            unique_name(state, &project.name, None)?;
            state.projects.push(project.clone());
            Ok(project)
        })
    }
    /// The agent may choose a fresh display name; direct user adds stay strict.
    pub fn open_project(&self, session: &str, path: &str, name: Option<&str>) -> Result<Project> {
        let path = std::fs::canonicalize(path).context("project folder does not exist")?;
        ensure!(path.is_dir(), "project path must be a folder");
        let derived = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Project".into());
        let base = name.unwrap_or(&derived).trim();
        ensure!(!base.is_empty(), "project name must not be empty");
        self.update(|state| {
            let path = stored_path(&path.to_string_lossy());
            ensure!(
                !state.projects.iter().any(|p| same_path(&p.path, &path)),
                "folder is already a project"
            );
            let mut selected = base.to_owned();
            for index in 2.. {
                if !state
                    .projects
                    .iter()
                    .any(|p| p.name.to_lowercase() == selected.to_lowercase())
                {
                    break;
                }
                selected = format!("{base} ({index})");
            }
            let project = Project {
                id: uuid::Uuid::new_v4().to_string(),
                name: selected,
                path,
                added_at: chrono::Utc::now().to_rfc3339(),
            };
            state.projects.push(project.clone());
            state.sessions.insert(session.into(), project.id.clone());
            state.recent.insert(0, project.id.clone());
            Ok(project)
        })
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<Project> {
        self.update(|state| {
            unique_name(state, name, Some(id))?;
            let project = state
                .projects
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or_else(|| anyhow!("unknown project {id:?}"))?;
            project.name = name.trim().into();
            Ok(project.clone())
        })
    }
    pub fn remove(&self, id: &str) -> Result<()> {
        self.update(|state| {
            ensure!(
                state.projects.iter().any(|p| p.id == id),
                "unknown project {id:?}"
            );
            ensure!(state.projects.len() > 1, "cannot remove the last project");
            state.projects.retain(|p| p.id != id);
            state.recent.retain(|recent| recent != id);
            let replacement = state
                .recent
                .iter()
                .find(|recent| state.projects.iter().any(|p| &p.id == *recent))
                .cloned()
                .unwrap_or_else(|| state.projects[0].id.clone());
            for current in state.sessions.values_mut() {
                if current == id {
                    *current = replacement.clone();
                }
            }
            Ok(())
        })
    }
}
fn unique_name(state: &State, name: &str, except: Option<&str>) -> Result<()> {
    let name = name.trim();
    ensure!(!name.is_empty(), "project name must not be empty");
    ensure!(
        !state
            .projects
            .iter()
            .any(|p| Some(p.id.as_str()) != except && p.name.to_lowercase() == name.to_lowercase()),
        "project name already exists"
    );
    Ok(())
}
fn make_project(name: &str, path: &Path) -> Result<Project> {
    ensure!(!name.trim().is_empty(), "project name must not be empty");
    let path = std::fs::canonicalize(path).context("project folder does not exist")?;
    ensure!(path.is_dir(), "project path must be a folder");
    Ok(project_at(name, &path))
}

/// A new project for a canonical folder, stored without the verbatim prefix.
fn project_at(name: &str, canonical: &Path) -> Project {
    Project {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.trim().into(),
        path: stored_path(&canonical.to_string_lossy()),
        added_at: chrono::Utc::now().to_rfc3339(),
    }
}

/// The folder path as stored and shown. `std::fs::canonicalize` returns Windows
/// verbatim paths (`\\?\C:\…`, `\\?\UNC\server\share\…`); this drops that prefix.
/// Other paths come back unchanged, and so do verbatim paths the prefix cannot
/// be dropped from safely: a `/` in one is part of a name, and a volume GUID
/// (`\\?\Volume{…}`) has no drive form.
fn stored_path(path: &str) -> String {
    let Some(rest) = path.strip_prefix(r"\\?\") else {
        return path.to_owned();
    };
    if rest.contains('/') {
        return path.to_owned();
    }
    let bytes = rest.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\' {
        return rest.to_owned();
    }
    match rest.strip_prefix(r"UNC\") {
        Some(share) if !share.is_empty() => format!(r"\\{share}"),
        _ => path.to_owned(),
    }
}

fn same_path(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.to_lowercase() == right.to_lowercase()
    } else {
        left == right
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: &str, path: &str) -> Project {
        Project {
            id: id.into(),
            name: id.into(),
            path: path.into(),
            added_at: "2026-10-23T00:00:00Z".into(),
        }
    }

    #[test]
    fn verbatim_prefix_is_dropped_only_where_it_is_safe() {
        let cases = [
            (r"\\?\C:\Users\Example", r"C:\Users\Example"),
            (r"\\?\c:\", r"c:\"),
            (r"\\?\UNC\server\share\notes", r"\\server\share\notes"),
            (r"C:\Users\Example", r"C:\Users\Example"),
            (r"\\server\share\notes", r"\\server\share\notes"),
            (r"/srv/notes", r"/srv/notes"),
            (r"\\?\Volume{1234}\notes", r"\\?\Volume{1234}\notes"),
            (r"\\?\UNC\", r"\\?\UNC\"),
            (r"\\?\C:", r"\\?\C:"),
            (r"\\?\C:\a/b", r"\\?\C:\a/b"),
            (r"\\?\é:\x", r"\\?\é:\x"),
            ("", ""),
        ];
        for (input, want) in cases {
            assert_eq!(stored_path(input), want, "input {input:?}");
        }
    }

    #[test]
    fn new_projects_store_the_folder_without_the_verbatim_prefix() {
        let project = project_at("Home", Path::new(r"\\?\C:\Users\Example"));
        assert_eq!(project.path, r"C:\Users\Example");
        assert_eq!(project.name, "Home");
    }

    #[test]
    fn a_new_home_project_is_a_plain_canonical_folder() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("projects.json");
        let projects = Projects::open(file, dir.path()).unwrap();
        let home = projects.list().remove(0);
        let canonical = std::fs::canonicalize(dir.path()).unwrap();
        assert_eq!(home.path, stored_path(&canonical.to_string_lossy()));
        assert!(!home.path.starts_with(r"\\?\"));
    }

    #[test]
    fn loaded_projects_drop_the_prefix_and_the_next_save_writes_it_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("projects.json");
        let state = State {
            projects: vec![
                project("a", r"\\?\C:\Users\Example"),
                project("b", r"\\?\UNC\server\share\notes"),
                project("c", r"D:\Work"),
            ],
            ..State::default()
        };
        save_json(&file, &state).unwrap();

        let projects = Projects::open(file.clone(), dir.path()).unwrap();
        let paths: Vec<String> = projects.list().into_iter().map(|p| p.path).collect();
        assert_eq!(
            paths,
            [r"C:\Users\Example", r"\\server\share\notes", r"D:\Work"]
        );

        projects.rename("c", "Work").unwrap();
        let saved: State = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        let saved: Vec<&str> = saved.projects.iter().map(|p| p.path.as_str()).collect();
        assert_eq!(
            saved,
            [r"C:\Users\Example", r"\\server\share\notes", r"D:\Work"]
        );
    }
}
