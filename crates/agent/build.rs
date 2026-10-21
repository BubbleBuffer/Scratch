use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    // This project's runner is a path dependency. Derive the extension's
    // location from that dependency, rather than embedding a machine path.
    let runner = manifest
        .lines()
        .find(|line| line.trim_start().starts_with("bubblepaw-runner ="))
        .and_then(|line| line.split_once("path = \""))
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(path, _)| root.join(path))
        .expect("bubblepaw-runner must declare its path dependency");
    let runner = runner
        .canonicalize()
        .expect("bubblepaw-runner dependency must exist");
    let extension = runner
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("extensions/claude-code");
    println!(
        "cargo:rustc-env=NOTES_DEFAULT_CLAUDE_CODE_DIR={}",
        extension.display()
    );
}
