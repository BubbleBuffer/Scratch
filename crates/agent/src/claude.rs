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
