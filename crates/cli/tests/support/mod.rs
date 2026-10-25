//! Runs `scratch-cli` command lines in-process, through the same entry point
//! as the binary, with the agent's scripted provider.
#![allow(dead_code)]

use serde_json::json;
use vault_agent::bubblepaw_sdk::providers::ScriptedStep;
use vault_agent::bubblepaw_sdk::tools::ToolCall;
use vault_agent::TestHooks;

/// Runs one command line and returns its output; panics on an error.
pub fn cli(args: &[&str], hooks: Option<scratch_cli::Hooks>) -> String {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let mut out = Vec::new();
    if let Err(error) = scratch_cli::run_with(&args, &mut out, hooks) {
        panic!("scratch-cli {args:?}: {error:#}");
    }
    String::from_utf8(out).unwrap()
}

/// Runs one command line and returns its error; panics if it succeeds.
pub fn cli_err(args: &[&str]) -> String {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let mut out = Vec::new();
    match scratch_cli::run_with(&args, &mut out, None) {
        Ok(()) => panic!("scratch-cli {args:?} succeeded"),
        Err(error) => format!("{error:#}"),
    }
}

/// One scripted provider step: a `js` cell running `code`.
pub fn js(code: &str) -> ScriptedStep {
    ScriptedStep::Calls {
        calls: vec![ToolCall {
            id: vault_core::new_id(),
            name: "js".into(),
            arguments: json!({ "code": code }),
        }],
    }
}

/// Test hooks whose scripted provider runs `code` in one `js` cell, then
/// answers `final_text`.
pub fn scripted(code: &str, final_text: &str) -> TestHooks {
    TestHooks {
        scripted: Some((vec![js(code)], final_text.into())),
        ..TestHooks::default()
    }
}

/// The value of `key: value` in `task show` output.
pub fn field<'a>(show: &'a str, key: &str) -> &'a str {
    let prefix = format!("{key}:");
    show.lines()
        .find_map(|l| l.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| panic!("no {key} in {show}"))
        .trim()
}
