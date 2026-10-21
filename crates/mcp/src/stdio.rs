//! The stdio transport: one JSON-RPC message per line on stdin, one reply
//! per line on stdout. Nothing else is written to stdout.

use std::io::{BufRead, Write};

use serde_json::Value;

use crate::server::error;

/// Reads messages from `input` until it ends and answers each with
/// `handle` (None: no reply). Lines that are not JSON get a parse error.
pub fn serve(
    input: impl BufRead,
    mut output: impl Write,
    handle: impl Fn(Value) -> Option<Value>,
) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(msg),
            Err(e) => Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
        };
        if let Some(r) = reply {
            let mut s = serde_json::to_string(&r).unwrap_or_default();
            s.push('\n');
            output.write_all(s.as_bytes())?;
            output.flush()?;
        }
    }
    Ok(())
}
