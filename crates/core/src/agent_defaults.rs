//! Vault-wide agent defaults (integration spec, phase 3): the default runner
//! and the default model per runner. They live as type data on the hidden,
//! synced settings page that also holds the conflict policy
//! (`conflict_settings.rs`), under the `agent` add-on key, so every device
//! and the hub read the same value. A device may override them locally
//! (the app keeps that in its own settings, never here).
//!
//! Each runner's tool set (integration spec, phase 5) sits beside them, under
//! `tool_sets.<runner>`: `read`, `write` or `everything` (the sets of
//! `vault_tools::ToolSet`, which this crate cannot depend on). A missing
//! value is Everything. Each runner reads its own set when its engine starts.
//!
//! Runner values are `hub` and `this_device`. `this_device` means "the
//! device that starts the run"; the hub and its MCP gateway treat it as
//! `hub`. Node device ids are left to phase 6.
use crate::doc::PageDoc;
use crate::error::{invalid, Result};
use crate::vault::Vault;
use std::collections::BTreeMap;

pub const AGENT_RUNNER_HUB: &str = "hub";
pub const AGENT_RUNNER_THIS_DEVICE: &str = "this_device";
const ADDON: &str = "agent";
const RUNNER_KEY: &str = "default_runner";
const MODELS_KEY: &str = "default_models";
const TOOL_SETS_KEY: &str = "tool_sets";
/// The tool set names a runner may store (see `vault_tools::ToolSet`).
pub const AGENT_TOOL_SETS: [&str; 3] = ["read", "write", "everything"];

/// The vault's agent defaults. `None` / a missing key: not set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentDefaults {
    pub runner: Option<String>,
    /// Runner (`hub`, `this_device`) to `provider/model`.
    pub models: BTreeMap<String, String>,
}

impl AgentDefaults {
    pub fn is_empty(&self) -> bool {
        self.runner.is_none() && self.models.is_empty()
    }

    /// The default model for `runner`, if the vault sets one.
    pub fn model_for(&self, runner: &str) -> Option<&str> {
        self.models.get(runner).map(String::as_str)
    }
}

fn check_runner(runner: &str) -> Result<()> {
    if runner == AGENT_RUNNER_HUB || runner == AGENT_RUNNER_THIS_DEVICE {
        Ok(())
    } else {
        Err(invalid(format!(
            "unknown agent runner {runner:?} (expected hub or this_device)"
        )))
    }
}

fn check_tool_set(set: &str) -> Result<()> {
    if AGENT_TOOL_SETS.contains(&set) {
        Ok(())
    } else {
        Err(invalid(format!(
            "unknown tool set {set:?} (expected read, write or everything)"
        )))
    }
}

/// Runner to tool set, keeping only known runners and sets.
fn tool_sets_in(doc: &PageDoc) -> BTreeMap<String, String> {
    let data = doc.type_data_value();
    let mut out = BTreeMap::new();
    if let Some(map) = data
        .get(ADDON)
        .and_then(|a| a.get(TOOL_SETS_KEY))
        .and_then(|v| v.as_object())
    {
        for (runner, set) in map {
            if let Some(set) = clean(set.as_str()) {
                if check_runner(runner).is_ok() && check_tool_set(&set).is_ok() {
                    out.insert(runner.clone(), set);
                }
            }
        }
    }
    out
}

fn clean(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}

pub(crate) fn agent_defaults_in(doc: &PageDoc) -> AgentDefaults {
    let data = doc.type_data_value();
    let Some(agent) = data.get(ADDON) else {
        return AgentDefaults::default();
    };
    let runner =
        clean(agent.get(RUNNER_KEY).and_then(|v| v.as_str())).filter(|r| check_runner(r).is_ok());
    let mut models = BTreeMap::new();
    if let Some(map) = agent.get(MODELS_KEY).and_then(|v| v.as_object()) {
        for (runner, model) in map {
            if check_runner(runner).is_err() {
                continue;
            }
            if let Some(model) = clean(model.as_str()) {
                models.insert(runner.clone(), model);
            }
        }
    }
    AgentDefaults { runner, models }
}

impl Vault {
    /// The vault's agent defaults. Reading an empty vault does not create
    /// the settings page.
    pub fn agent_defaults(&self) -> Result<AgentDefaults> {
        match self.find_by_role("calendars".into())? {
            Some(page) => self.inner.read(&page, |doc| agent_defaults_in(doc)),
            None => Ok(AgentDefaults::default()),
        }
    }

    fn settings_page(&self) -> Result<String> {
        // Initializes the deterministic hidden page under the calendar lock,
        // as `set_conflict_auto_resolve` does.
        self.list_calendars()?;
        self.find_by_role("calendars".into())?
            .ok_or_else(|| invalid("synced settings page is missing"))
    }

    /// Sets the default runner (`hub` or `this_device`); `None` clears it.
    pub fn set_agent_default_runner(&self, runner: Option<String>) -> Result<()> {
        let runner = clean(runner.as_deref());
        if let Some(r) = &runner {
            check_runner(r)?;
        }
        let page = self.settings_page()?;
        self.inner.edit(&page, |doc| match &runner {
            Some(r) => doc.type_data_set(ADDON, &[RUNNER_KEY.into()], &serde_json::json!(r)),
            None => doc.type_data_delete(ADDON, &[RUNNER_KEY.into()]),
        })
    }

    /// Sets the default model for `runner`; `None` or empty clears it.
    pub fn set_agent_default_model(&self, runner: String, model: Option<String>) -> Result<()> {
        check_runner(&runner)?;
        let model = clean(model.as_deref());
        let page = self.settings_page()?;
        self.inner.edit(&page, |doc| {
            let path = [MODELS_KEY.to_owned(), runner.clone()];
            match &model {
                Some(m) => doc.type_data_set(ADDON, &path, &serde_json::json!(m)),
                None => {
                    if agent_defaults_in(doc).models.contains_key(&runner) {
                        doc.type_data_delete(ADDON, &path)
                    } else {
                        Ok(())
                    }
                }
            }
        })
    }

    /// Each runner's tool set, as stored (runners without one use
    /// Everything). Reading an empty vault does not create the settings page.
    pub fn agent_tool_sets(&self) -> Result<BTreeMap<String, String>> {
        match self.find_by_role("calendars".into())? {
            Some(page) => self.inner.read(&page, |doc| tool_sets_in(doc)),
            None => Ok(BTreeMap::new()),
        }
    }

    /// The tool set `runner` uses: the stored one, else `everything`.
    pub fn agent_tool_set(&self, runner: &str) -> Result<String> {
        check_runner(runner)?;
        Ok(self
            .agent_tool_sets()?
            .remove(runner)
            .unwrap_or_else(|| "everything".into()))
    }

    /// Sets the tool set of `runner`; `None` or empty clears it (Everything).
    pub fn set_agent_tool_set(&self, runner: String, set: Option<String>) -> Result<()> {
        check_runner(&runner)?;
        let set = clean(set.as_deref());
        if let Some(s) = &set {
            check_tool_set(s)?;
        }
        let page = self.settings_page()?;
        self.inner.edit(&page, |doc| {
            let path = [TOOL_SETS_KEY.to_owned(), runner.clone()];
            match &set {
                Some(s) => doc.type_data_set(ADDON, &path, &serde_json::json!(s)),
                None => {
                    if tool_sets_in(doc).contains_key(&runner) {
                        doc.type_data_delete(ADDON, &path)
                    } else {
                        Ok(())
                    }
                }
            }
        })
    }

    /// One-time migration of a device's old local values: writes them only
    /// when the vault has no agent defaults at all. Returns whether it wrote.
    pub fn seed_agent_defaults(&self, seed: AgentDefaults) -> Result<bool> {
        if !self.agent_defaults()?.is_empty() {
            return Ok(false);
        }
        let runner = clean(seed.runner.as_deref()).filter(|r| check_runner(r).is_ok());
        let models: Vec<(String, String)> = seed
            .models
            .into_iter()
            .filter(|(r, _)| check_runner(r).is_ok())
            .filter_map(|(r, m)| clean(Some(&m)).map(|m| (r, m)))
            .collect();
        if runner.is_none() && models.is_empty() {
            return Ok(false);
        }
        let page = self.settings_page()?;
        self.inner.edit(&page, |doc| {
            if !agent_defaults_in(doc).is_empty() {
                return Ok(false);
            }
            if let Some(r) = &runner {
                doc.type_data_set(ADDON, &[RUNNER_KEY.into()], &serde_json::json!(r))?;
            }
            for (r, m) in &models {
                doc.type_data_set(
                    ADDON,
                    &[MODELS_KEY.to_owned(), r.clone()],
                    &serde_json::json!(m),
                )?;
            }
            Ok(true)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn vault() -> (Temp, Vault) {
        let path = std::env::temp_dir().join(format!("agent_defaults_{}", crate::new_id()));
        let vault = Vault::open(path.to_string_lossy().into_owned()).unwrap();
        (Temp(path), vault)
    }
    fn sync(from: &Vault, to: &Vault) {
        for version in from.page_versions().unwrap() {
            let bytes = from.export_updates(version.page.clone(), vec![]).unwrap();
            to.import_updates(version.page, bytes).unwrap();
        }
    }

    #[test]
    fn empty_vault_has_no_defaults_and_reading_creates_nothing() {
        let (_dir, vault) = vault();
        let before = vault.page_versions().unwrap();
        assert!(vault.agent_defaults().unwrap().is_empty());
        assert_eq!(vault.page_versions().unwrap(), before);
    }

    #[test]
    fn runner_and_models_round_trip_and_clear() {
        let (dir, vault) = vault();
        vault.set_agent_default_runner(Some("hub".into())).unwrap();
        vault
            .set_agent_default_model("hub".into(), Some("codex/gpt-hub".into()))
            .unwrap();
        vault
            .set_agent_default_model("this_device".into(), Some(" claude-code/x ".into()))
            .unwrap();
        let d = vault.agent_defaults().unwrap();
        assert_eq!(d.runner.as_deref(), Some("hub"));
        assert_eq!(d.model_for("hub"), Some("codex/gpt-hub"));
        assert_eq!(d.model_for("this_device"), Some("claude-code/x"));
        vault.flush().unwrap();
        drop(vault);
        let vault = Vault::open(dir.0.to_string_lossy().into_owned()).unwrap();
        assert_eq!(vault.agent_defaults().unwrap(), d, "survives reopen");
        vault.set_agent_default_runner(None).unwrap();
        vault.set_agent_default_model("hub".into(), None).unwrap();
        let d = vault.agent_defaults().unwrap();
        assert_eq!(d.runner, None);
        assert_eq!(d.model_for("hub"), None);
        assert_eq!(d.model_for("this_device"), Some("claude-code/x"));
    }

    #[test]
    fn unknown_runners_are_rejected() {
        let (_dir, vault) = vault();
        assert!(vault
            .set_agent_default_runner(Some("desktop".into()))
            .is_err());
        assert!(vault
            .set_agent_default_model("node-1".into(), Some("m".into()))
            .is_err());
    }

    #[test]
    fn defaults_sync_both_directions_and_keep_the_conflict_policy() {
        let (_a_dir, a) = vault();
        let (_b_dir, b) = vault();
        a.set_conflict_auto_resolve(true).unwrap();
        a.set_agent_default_runner(Some("hub".into())).unwrap();
        a.set_agent_default_model("hub".into(), Some("codex/a".into()))
            .unwrap();
        sync(&a, &b);
        let got = b.agent_defaults().unwrap();
        assert_eq!(got.runner.as_deref(), Some("hub"));
        assert_eq!(got.model_for("hub"), Some("codex/a"));
        assert!(b.conflict_auto_resolve().unwrap());
        b.set_agent_default_runner(Some("this_device".into()))
            .unwrap();
        b.set_agent_default_model("hub".into(), Some("codex/b".into()))
            .unwrap();
        sync(&b, &a);
        let got = a.agent_defaults().unwrap();
        assert_eq!(got.runner.as_deref(), Some("this_device"));
        assert_eq!(got.model_for("hub"), Some("codex/b"));
        assert!(a.conflict_auto_resolve().unwrap());
    }

    #[test]
    fn seed_writes_only_into_an_empty_vault() {
        let (_dir, v) = vault();
        let seed = AgentDefaults {
            runner: Some("hub".into()),
            models: [("hub".to_owned(), "codex/old".to_owned())].into(),
        };
        assert!(v.seed_agent_defaults(seed.clone()).unwrap());
        assert_eq!(v.agent_defaults().unwrap(), seed);
        let other = AgentDefaults {
            runner: Some("this_device".into()),
            models: [("this_device".to_owned(), "codex/other".to_owned())].into(),
        };
        assert!(!v.seed_agent_defaults(other).unwrap());
        assert_eq!(v.agent_defaults().unwrap(), seed, "vault value kept");
        let (_dir2, empty) = vault();
        assert!(!empty.seed_agent_defaults(AgentDefaults::default()).unwrap());
        assert!(empty.agent_defaults().unwrap().is_empty());
    }

    #[test]
    fn tool_sets_per_runner_default_to_everything() {
        let (_t, v) = vault();
        assert!(v.agent_tool_sets().unwrap().is_empty());
        assert_eq!(v.agent_tool_set("hub").unwrap(), "everything");
        v.set_agent_tool_set("hub".into(), Some("read".into()))
            .unwrap();
        v.set_agent_tool_set("this_device".into(), Some("write".into()))
            .unwrap();
        assert_eq!(v.agent_tool_set("hub").unwrap(), "read");
        assert_eq!(v.agent_tool_set("this_device").unwrap(), "write");
        assert!(v
            .set_agent_tool_set("hub".into(), Some("admin".into()))
            .is_err());
        assert!(v
            .set_agent_tool_set("laptop".into(), Some("read".into()))
            .is_err());
        // Tool sets sit beside the defaults and leave them alone.
        assert!(v.agent_defaults().unwrap().is_empty());
        v.set_agent_tool_set("hub".into(), None).unwrap();
        assert_eq!(v.agent_tool_set("hub").unwrap(), "everything");
    }
}
