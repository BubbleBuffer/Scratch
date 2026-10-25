//! Read-only discovery of the optional Claude Code runtime.
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct ClaudeDetection {
    pub bun_path: Option<PathBuf>,
    pub claude_code_dir: Option<PathBuf>,
}

pub fn detect(bun: Option<&Path>, extension: Option<&Path>) -> ClaudeDetection {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    let default_extension = bubblepaw_sdk::claude_code::bundled_dir();
    detect_with(
        bun,
        extension,
        std::env::var_os("PATH").as_deref(),
        home.as_deref().map(Path::new),
        std::env::var_os("NOTES_CLAUDE_CODE_DIR")
            .as_deref()
            .map(Path::new),
        &default_extension,
    )
}

/// The Bun and extension overrides a host without the app's UI (scratch-cli)
/// passes to [`detect`], so it finds Claude Code as the desktop app does.
/// The extension folder: `NOTES_CLAUDE_CODE_DIR` when set, else the app's
/// saved `agentClaudeCodeDir`; Bun: the app's saved `agentBunPath`. The
/// settings are each `ai_settings.json` in `settings_dirs`, first non-empty
/// value wins. `None` leaves [`detect`]'s own search (PATH, `~/.bun/bin`,
/// the bundled extension).
pub fn app_overrides(settings_dirs: &[PathBuf]) -> (Option<PathBuf>, Option<PathBuf>) {
    let env_dir = std::env::var_os("NOTES_CLAUDE_CODE_DIR").filter(|v| !v.is_empty());
    app_overrides_with(env_dir.as_deref().map(Path::new), settings_dirs)
}

fn app_overrides_with(
    env_extension: Option<&Path>,
    settings_dirs: &[PathBuf],
) -> (Option<PathBuf>, Option<PathBuf>) {
    let settings: Vec<serde_json::Value> = settings_dirs
        .iter()
        .filter_map(|dir| std::fs::read(dir.join("ai_settings.json")).ok())
        .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
        .collect();
    let saved = |key: &str| {
        settings.iter().find_map(|s| {
            s.get(key)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
    };
    let extension = env_extension
        .map(Path::to_owned)
        .or_else(|| saved("agentClaudeCodeDir"));
    (saved("agentBunPath"), extension)
}

fn detect_with(
    bun: Option<&Path>,
    extension: Option<&Path>,
    search_path: Option<&OsStr>,
    home: Option<&Path>,
    env_extension: Option<&Path>,
    default_extension: &Path,
) -> ClaudeDetection {
    let bun_name = if cfg!(windows) { "bun.exe" } else { "bun" };
    let bun_path = match bun {
        // An explicit override is authoritative, including when invalid.
        Some(path) => path.is_file().then(|| path.to_owned()),
        None => search_path
            .into_iter()
            .flat_map(std::env::split_paths)
            .map(|dir| dir.join(bun_name))
            .find(|path| path.is_file())
            .or_else(|| {
                home.map(|h| h.join(".bun/bin").join(bun_name))
                    .filter(|path| path.is_file())
            }),
    };
    let dir = extension.or(env_extension).unwrap_or(default_extension);
    ClaudeDetection {
        bun_path,
        claude_code_dir: dir.is_dir().then(|| dir.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_overrides_env_then_saved_settings_in_order() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let (vault_side, app_dir, empty) = (root.join("a"), root.join("b"), root.join("c"));
        for dir in [&vault_side, &app_dir, &empty] {
            std::fs::create_dir(dir).unwrap();
        }
        let write = |dir: &Path, json: serde_json::Value| {
            std::fs::write(dir.join("ai_settings.json"), json.to_string()).unwrap()
        };
        write(
            &vault_side,
            serde_json::json!({ "agentBunPath": "", "agentClaudeCodeDir": "  " }),
        );
        write(
            &app_dir,
            serde_json::json!({ "agentBunPath": "/saved/bun", "agentClaudeCodeDir": "/saved/ext" }),
        );
        let env = root.join("env-ext");
        let dirs = [empty.clone(), vault_side.clone(), app_dir.clone()];

        // The environment wins for the extension; Bun comes from the settings.
        let (bun, ext) = app_overrides_with(Some(&env), &dirs);
        assert_eq!(bun, Some(PathBuf::from("/saved/bun")));
        assert_eq!(ext, Some(env));
        // Empty saved values are skipped for the next settings file.
        let (bun, ext) = app_overrides_with(None, &dirs);
        assert_eq!(bun, Some(PathBuf::from("/saved/bun")));
        assert_eq!(ext, Some(PathBuf::from("/saved/ext")));
        // An earlier settings file wins.
        write(&vault_side, serde_json::json!({ "agentClaudeCodeDir": "/first/ext" }));
        assert_eq!(
            app_overrides_with(None, &dirs).1,
            Some(PathBuf::from("/first/ext"))
        );
        // Nothing saved: detect's own search (PATH, ~/.bun/bin, bundled).
        assert_eq!(app_overrides_with(None, &[empty]), (None, None));
        // The overrides feed detect: a saved folder that exists is found.
        let real = root.join("real-ext");
        std::fs::create_dir(&real).unwrap();
        let default = root.join("no-bundled");
        let detected = detect_with(None, Some(&real), None, None, None, &default);
        assert_eq!(detected.claude_code_dir, Some(real));
    }
    #[test]
    fn overrides_then_path_then_home_and_default_extension() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let path_dir = root.join("path");
        let home = root.join("home");
        let bin = home.join(".bun/bin");
        for dir in [&path_dir, &bin] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let name = if cfg!(windows) { "bun.exe" } else { "bun" };
        let override_bun = root.join("override");
        let path_bun = path_dir.join(name);
        let home_bun = bin.join(name);
        for path in [&override_bun, &path_bun, &home_bun] {
            std::fs::write(path, "fake").unwrap();
        }
        let explicit = root.join("explicit");
        let env_dir = root.join("env");
        let default = root.join("default");
        for dir in [&explicit, &env_dir, &default] {
            std::fs::create_dir(dir).unwrap();
        }
        let search = std::env::join_paths([&path_dir]).unwrap();
        let run = |bun, ext, env_ext| {
            detect_with(bun, ext, Some(&search), Some(&home), env_ext, &default)
        };
        let detected = run(Some(&override_bun), Some(&explicit), Some(&env_dir));
        assert_eq!(detected.bun_path, Some(override_bun.clone()));
        assert_eq!(detected.claude_code_dir, Some(explicit.clone()));
        let detected = run(None, None, Some(&env_dir));
        assert_eq!(detected.bun_path, Some(path_bun.clone()));
        assert_eq!(detected.claude_code_dir, Some(env_dir.clone()));
        std::fs::remove_file(path_bun).unwrap();
        let detected = run(None, None, None);
        assert_eq!(detected.bun_path, Some(home_bun));
        assert_eq!(detected.claude_code_dir, Some(default.clone()));
        let missing = root.join("missing");
        assert!(run(Some(&missing), None, None)
            .bun_path
            .is_none());
        assert!(run(None, Some(&missing), None)
            .claude_code_dir
            .is_none());
    }
}
