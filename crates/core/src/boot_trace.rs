//! Boot timing trace (instrumentation, branch perf/boot-timing): one line per
//! step with the time since the trace began, written to
//! `<vault>/.vault/boot-timing.log`. Never fails the caller.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub struct Trace {
    file: PathBuf,
    label: &'static str,
    start: Instant,
    last: Instant,
    lines: String,
}

impl Trace {
    /// A trace for `label` writing next to the vault at `root`.
    pub fn new(root: &Path, label: &'static str) -> Trace {
        let now = Instant::now();
        Trace {
            file: root.join(".vault").join("boot-timing.log"),
            label,
            start: now,
            last: now,
            lines: String::new(),
        }
    }

    /// Records the end of step `name`: its own duration and the total so far.
    pub fn mark(&mut self, name: &str) {
        let now = Instant::now();
        let step = now.duration_since(self.last).as_secs_f64() * 1000.0;
        let total = now.duration_since(self.start).as_secs_f64() * 1000.0;
        self.lines
            .push_str(&format!("rust {}: {name} step={step:.1}ms total={total:.1}ms\n", self.label));
        self.last = now;
    }

    /// Appends the lines to the log file. `truncate` starts a fresh file.
    pub fn finish(&mut self, truncate: bool) {
        let mut o = std::fs::OpenOptions::new();
        o.create(true);
        if truncate {
            o.write(true).truncate(true);
        } else {
            o.append(true);
        }
        if let Ok(mut f) = o.open(&self.file) {
            let _ = f.write_all(self.lines.as_bytes());
        }
        self.lines.clear();
    }
}
