//! A stand-in for `claude -p` / `codex exec` in tests: reads the prompt on
//! stdin, uses the vault's MCP endpoint like a real agent, and answers.
//! Never calls a real AI.
//!
//! It finds the endpoint from `--mcp-config <file>` (Claude's format) or
//! `NOTES_MCP_URL` + `NOTES_MCP_TOKEN`. With `--answer-file <file>` (like
//! `codex --output-last-message`) it writes the answer there and prints
//! progress noise on stdout instead. What it does depends on the prompt:
//!
//! - `FAKE:fail`: prints to stderr and exits with code 3.
//! - `FAKE:sleep`: sleeps for a minute (timeouts).
//! - `FAKE:edit`: appends a paragraph to the task's page over MCP and
//!   answers nothing.
//! - a merge prompt ("Two devices edited"): answers `**Merged** by the fake agent`.
//! - otherwise: reads the task's page over MCP and answers two bullets.

use std::io::Read;

use serde_json::{json, Value};
use vault_mcp::proxy::McpClient;

fn arg_after(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn endpoint(args: &[String]) -> Option<(String, String)> {
    if let Some(cfg) = arg_after(args, "--mcp-config") {
        let j: Value = serde_json::from_slice(&std::fs::read(cfg).ok()?).ok()?;
        let s = &j["mcpServers"]["notes"];
        let url = s["url"].as_str()?.to_string();
        let token = s["headers"]["Authorization"]
            .as_str()?
            .strip_prefix("Bearer ")?
            .to_string();
        return Some((url, token));
    }
    let url = std::env::var("NOTES_MCP_URL")
        .ok()
        .filter(|u| !u.is_empty())?;
    Some((url, std::env::var("NOTES_MCP_TOKEN").ok()?))
}

fn page_id(prompt: &str) -> Option<String> {
    let i = prompt.find("(page id ")? + "(page id ".len();
    Some(prompt[i..].split(')').next()?.to_string())
}

fn task_line(prompt: &str) -> String {
    prompt
        .split("Task from the user:\n")
        .nth(1)
        .and_then(|r| r.lines().next())
        .unwrap_or("")
        .to_string()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut prompt = String::new();
    let _ = std::io::stdin().read_to_string(&mut prompt);
    if prompt.contains("FAKE:fail") {
        eprintln!("fake failure: asked to fail");
        std::process::exit(3);
    }
    if prompt.contains("FAKE:sleep") {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
    let answer = if prompt.contains("Two devices edited") {
        "**Merged** by the fake agent".to_string()
    } else {
        let client = endpoint(&args).map(|(u, t)| McpClient::new(&u, &t));
        let page = page_id(&prompt);
        match (client, page) {
            (Some(c), Some(p)) => {
                if let Err(e) = c.initialize() {
                    eprintln!("mcp: {e}");
                    std::process::exit(4);
                }
                if prompt.contains("FAKE:edit") {
                    c.call_tool(
                        "edit_page",
                        json!({ "page": p, "op": "append", "markdown": "Edited by the fake agent" }),
                    )
                    .unwrap_or_else(|e| {
                        eprintln!("mcp: {e}");
                        std::process::exit(4)
                    });
                    String::new()
                } else {
                    let text = c
                        .call_tool("read_page", json!({ "page": p }))
                        .unwrap_or_default();
                    let title = text
                        .lines()
                        .next()
                        .unwrap_or("")
                        .trim_start_matches("# ")
                        .to_string();
                    format!(
                        "- Fake answer to: {}\n- Read over MCP: {title}",
                        task_line(&prompt)
                    )
                }
            }
            _ => format!("Fake answer without tools to: {}", task_line(&prompt)),
        }
    };
    match arg_after(&args, "--answer-file") {
        Some(f) => {
            println!("[fake agent] thinking…");
            std::fs::write(f, &answer).expect("write answer");
        }
        None => print!("{answer}"),
    }
}
