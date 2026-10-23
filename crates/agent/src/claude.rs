//! Read-only discovery of the optional Claude Code runtime.
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct ClaudeDetection {
    pub bun_path: Option<PathBuf>,
    pub claude_code_dir: Option<PathBuf>,
    /// The `bun` override that is not a file. It is ignored, and `bun_path` is
    /// found without it.
    pub ignored_bun_override: Option<PathBuf>,
}

pub fn detect(bun: Option<&Path>, extension: Option<&Path>) -> ClaudeDetection {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    detect_with(
        bun,
        extension,
        std::env::var_os("PATH").as_deref(),
        home.as_deref().map(Path::new),
        std::env::var_os("NOTES_CLAUDE_CODE_DIR")
            .as_deref()
            .map(Path::new),
        Path::new(env!("NOTES_DEFAULT_CLAUDE_CODE_DIR")),
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
    // A valid override wins. An invalid one is ignored and reported, so detection
    // falls back to PATH and ~/.bun/bin rather than hiding Claude Code.
    let (bun_path, ignored_bun_override) = match bun {
        Some(path) if path.is_file() => (Some(path.to_owned()), None),
        ignored => (
            search_path
                .into_iter()
                .flat_map(std::env::split_paths)
                .map(|dir| dir.join(bun_name))
                .find(|path| path.is_file())
                .or_else(|| {
                    home.map(|h| h.join(".bun/bin").join(bun_name))
                        .filter(|path| path.is_file())
                }),
            ignored.map(Path::to_owned),
        ),
    };
    let dir = extension.or(env_extension).unwrap_or(default_extension);
    ClaudeDetection {
        bun_path,
        claude_code_dir: dir.is_dir().then(|| dir.to_owned()),
        ignored_bun_override,
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
        let missing = root.join("missing");
        let detected = run(None, None, Some(&env_dir));
        assert_eq!(detected.bun_path, Some(path_bun.clone()));
        assert_eq!(detected.claude_code_dir, Some(env_dir.clone()));
        // The invalid override is ignored and reported. PATH wins over ~/.bun/bin
        // even though both exist.
        let detected = run(Some(&missing), None, None);
        assert_eq!(detected.bun_path, Some(path_bun.clone()));
        assert_eq!(detected.ignored_bun_override, Some(missing.clone()));
        std::fs::remove_file(path_bun).unwrap();
        let detected = run(None, None, None);
        assert_eq!(detected.bun_path, Some(home_bun.clone()));
        assert_eq!(detected.claude_code_dir, Some(default.clone()));
        // With PATH empty of bun, the invalid override falls back to ~/.bun/bin.
        let detected = run(Some(&missing), None, None);
        assert_eq!(detected.bun_path, Some(home_bun));
        assert_eq!(detected.ignored_bun_override, Some(missing.clone()));
        assert!(run(None, Some(&missing), None).claude_code_dir.is_none());
    }

    #[test]
    fn invalid_bun_override_is_reported_even_when_nothing_is_found() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let empty = root.join("empty");
        std::fs::create_dir(&empty).unwrap();
        let missing = root.join("missing");
        let valid = root.join("valid-bun");
        std::fs::write(&valid, "fake").unwrap();

        let detected = detect_with(Some(&missing), None, None, Some(&empty), None, &empty);
        assert_eq!(detected.bun_path, None);
        assert_eq!(detected.ignored_bun_override, Some(missing));

        // A folder is not an executable, so it is ignored like a missing file.
        let detected = detect_with(Some(&empty), None, None, Some(&empty), None, &empty);
        assert_eq!(detected.bun_path, None);
        assert_eq!(detected.ignored_bun_override, Some(empty.clone()));

        let detected = detect_with(Some(&valid), None, None, Some(&empty), None, &empty);
        assert_eq!(detected.bun_path, Some(valid));
        assert_eq!(detected.ignored_bun_override, None);

        let detected = detect_with(None, None, None, Some(&empty), None, &empty);
        assert_eq!(detected.bun_path, None);
        assert_eq!(detected.ignored_bun_override, None);
    }
}
