use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    // With a path dependency on the runner, derive the extension's location
    // from it, rather than embedding a machine path. With the git dependency
    // (the default), a build script cannot see where Cargo checked it out, so
    // the default stays empty and the extension is found through
    // NOTES_CLAUDE_CODE_DIR at run time.
    let runner = manifest
        .lines()
        .find(|line| line.trim_start().starts_with("bubblepaw-runner ="))
        .expect("bubblepaw-runner must be a dependency");
    let extension = match runner
        .split_once("path = \"")
        .and_then(|(_, rest)| rest.split_once('"'))
    {
        Some((path, _)) => root
            .join(path)
            .canonicalize()
            .expect("bubblepaw-runner dependency must exist")
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("extensions/claude-code")
            .display()
            .to_string(),
        None => String::new(),
    };
    println!("cargo:rustc-env=NOTES_DEFAULT_CLAUDE_CODE_DIR={extension}");
}
