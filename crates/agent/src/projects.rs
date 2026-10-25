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
    publisher: Option<std::sync::Arc<vault_core::Vault>>,
}

fn published(projects: &[Project]) -> Vec<vault_core::DeviceProject> {
    projects
        .iter()
        .map(|p| vault_core::DeviceProject {
            id: p.id.clone(),
            name: p.name.clone(),
        })
        .collect()
}

/// Publish an existing device's projects even when its desktop tools are off
/// or its agent has not been opened. Local repository paths never enter sync.
pub fn publish_at(vault: &vault_core::Vault, data_dir: &Path) -> Result<()> {
    let file = data_dir.join("agent/projects.json");
    if file.exists() {
        let state: State = serde_json::from_slice(&std::fs::read(&file)?)
            .with_context(|| format!("read {}", file.display()))?;
        vault.publish_device_projects(published(&state.projects))?;
    }
    Ok(())
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
            let state: State = serde_json::from_slice(&std::fs::read(&file)?)
                .with_context(|| format!("read {}", file.display()))?;
            ensure!(
                !state.projects.is_empty(),
                "projects.json contains no projects"
            );
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
            publisher: None,
        })
    }

    pub(crate) fn open_for_device(
        file: PathBuf,
        seed: &Path,
        vault: std::sync::Arc<vault_core::Vault>,
    ) -> Result<Self> {
        let mut projects = Self::open(file, seed)?;
        vault.publish_device_projects(published(&projects.list()))?;
        projects.publisher = Some(vault);
        Ok(projects)
    }
    fn update<T>(&self, f: impl FnOnce(&mut State) -> Result<T>) -> Result<T> {
        let mut guard = self.state.lock().unwrap();
        let mut next = guard.clone();
        let out = f(&mut next)?;
        save_json(&self.file, &next)?;
        *guard = next;
        if let Some(vault) = &self.publisher {
            vault.publish_device_projects(published(&guard.projects))?;
        }
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
            let path = path.to_string_lossy().into_owned();
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
    Ok(Project {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.trim().into(),
        path: path.to_string_lossy().into_owned(),
        added_at: chrono::Utc::now().to_rfc3339(),
    })
}

fn same_path(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.to_lowercase() == right.to_lowercase()
    } else {
        left == right
    }
}
