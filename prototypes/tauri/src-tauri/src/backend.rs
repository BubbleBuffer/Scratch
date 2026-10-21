//! Everything the interface asks of the core, independent of Tauri: opening
//! (and on first launch seeding) the vault, the "Bake-off showcase" page, and
//! one `dispatch` entry point by command name. The Tauri commands in lib.rs
//! and the headless test bridge (examples/bridge.rs) both call this.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use serde::Serialize;
use serde_json::{json, Value};
use vault_core::{seed, BlockKind, BlockOp, Mark, MarkKind, NewBlock, Vault};

/// Pages and big-page blocks of the bake-off seed.
pub const SEED_PAGES: u32 = 10_000;
pub const SEED_BIG_BLOCKS: u32 = 2_000;
/// Written after seeding finished, so an interrupted seed is redone.
const SEED_MARKER: &str = "seed-complete";
/// Written after the showcase page was created (once per vault).
const SHOWCASE_MARKER: &str = "showcase-created";
pub const SHOWCASE_TITLE: &str = "Bake-off showcase";

pub struct Backend {
    /// App data folder; the vault lives in `<dir>/vault`.
    pub dir: PathBuf,
    vault: OnceLock<Arc<Vault>>,
    /// Serialises `open` so two calls cannot seed or open twice.
    opening: Mutex<()>,
    #[cfg_attr(target_os = "android", allow(dead_code))]
    log_file: Mutex<Option<std::fs::File>>,
    pages: u32,
    big_blocks: u32,
}

#[derive(Serialize)]
pub struct VaultStatus {
    /// True when the next `open_vault` has to seed (about 15 s on desktop).
    pub needs_seed: bool,
    pub path: String,
}

#[derive(Serialize)]
pub struct OpenInfo {
    pub count: u32,
    pub seeded: bool,
    pub seed_ms: u64,
    pub open_ms: f64,
}

impl Backend {
    pub fn new(dir: PathBuf) -> Backend {
        Backend::with_seed(dir, SEED_PAGES, SEED_BIG_BLOCKS)
    }

    /// A backend with a different seed size (the test bridge can ask for a
    /// smaller vault; the app always uses the spec's 10,000 / 2,000).
    pub fn with_seed(dir: PathBuf, pages: u32, big_blocks: u32) -> Backend {
        Backend {
            dir,
            vault: OnceLock::new(),
            opening: Mutex::new(()),
            log_file: Mutex::new(None),
            pages,
            big_blocks,
        }
    }

    fn vault_dir(&self) -> PathBuf {
        self.dir.join("vault")
    }

    pub fn vault(&self) -> Result<Arc<Vault>, String> {
        self.vault.get().cloned().ok_or_else(|| "vault not open".to_string())
    }

    pub fn is_open(&self) -> bool {
        self.vault.get().is_some()
    }

    pub fn status(&self) -> VaultStatus {
        let dir = self.vault_dir();
        VaultStatus {
            needs_seed: self.vault.get().is_none() && !dir.join(SEED_MARKER).exists(),
            path: dir.to_string_lossy().into_owned(),
        }
    }

    /// Opens the vault, seeding it and creating the showcase page first when
    /// this is the first launch. Blocking: call it off the UI thread.
    pub fn open(&self) -> Result<OpenInfo, String> {
        let _guard = self.opening.lock().map_err(|e| e.to_string())?;
        let dir = self.vault_dir();
        let mut seeded = false;
        let mut seed_ms = 0;
        if self.vault.get().is_none() && !dir.join(SEED_MARKER).exists() {
            // A half-written seed from an interrupted first launch: start over.
            if dir.exists() {
                std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
            }
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let t = Instant::now();
            seed::generate(dir.to_string_lossy().into_owned(), self.pages, self.big_blocks)
                .map_err(String::from)?;
            std::fs::write(dir.join(SEED_MARKER), b"ok").map_err(|e| e.to_string())?;
            seed_ms = t.elapsed().as_millis() as u64;
            seeded = true;
        }
        let t = Instant::now();
        if self.vault.get().is_none() {
            let v = Vault::open(dir.to_string_lossy().into_owned()).map_err(String::from)?;
            let _ = self.vault.set(Arc::new(v));
        }
        let open_ms = t.elapsed().as_secs_f64() * 1000.0;
        let v = self.vault()?;
        if !dir.join(SHOWCASE_MARKER).exists() {
            create_showcase(&v)?;
            v.flush().map_err(String::from)?;
            std::fs::write(dir.join(SHOWCASE_MARKER), b"ok").map_err(|e| e.to_string())?;
        }
        let count = v.page_count().map_err(String::from)?;
        Ok(OpenInfo { count, seeded, seed_ms, open_ms })
    }

    /// One measurement line, with the `BAKEOFF` prefix: logcat on Android,
    /// stdout plus `<dir>/bakeoff.log` elsewhere.
    pub fn log(&self, line: &str) {
        #[cfg(target_os = "android")]
        {
            android_log::write(&format!("BAKEOFF {line}"));
        }
        #[cfg(not(target_os = "android"))]
        {
            use std::io::Write;
            println!("BAKEOFF {line}");
            if let Ok(mut f) = self.log_file.lock() {
                if f.is_none() {
                    let _ = std::fs::create_dir_all(&self.dir);
                    *f = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(self.dir.join("bakeoff.log"))
                        .ok();
                }
                if let Some(f) = f.as_mut() {
                    let _ = writeln!(f, "BAKEOFF {line}");
                }
            }
        }
    }

    /// Runs one interface command by name with its JSON arguments (the same
    /// names and argument names as the Tauri commands). Blocking.
    pub fn dispatch(&self, cmd: &str, args: &Value) -> Result<Value, String> {
        let s = |k: &str| -> Result<String, String> {
            args.get(k).and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("missing {k}"))
        };
        let n = |k: &str| -> Result<u32, String> {
            args.get(k).and_then(Value::as_u64).map(|x| x as u32).ok_or_else(|| format!("missing {k}"))
        };
        match cmd {
            "vault_status" => js(self.status()),
            "open_vault" => js(self.open()?),
            "page_count" => Ok(json!(to(self.vault()?.page_count())?)),
            "list_pages" => js(to(self.vault()?.list_pages(None, n("offset")?, n("limit")?))?),
            "search" => js(to(self.vault()?.search(s("query")?, n("limit")?))?),
            "create_page" => Ok(json!(to(self.vault()?.create_page(None, s("title")?))?)),
            "rename_page" => {
                to(self.vault()?.rename_page(s("id")?, s("title")?))?;
                Ok(Value::Null)
            }
            "load_page" => js(to(self.vault()?.load_page(s("id")?))?),
            "apply" => {
                let ops: Vec<BlockOp> = serde_json::from_value(args.get("ops").cloned().unwrap_or(Value::Null))
                    .map_err(|e| e.to_string())?;
                to(self.vault()?.apply(s("id")?, ops))?;
                Ok(Value::Null)
            }
            "flush" => {
                if let Ok(v) = self.vault() {
                    to(v.flush())?;
                }
                Ok(Value::Null)
            }
            "bakeoff_log" => {
                self.log(&s("line")?);
                Ok(Value::Null)
            }
            _ => Err(format!("unknown command {cmd}")),
        }
    }
}

fn to<T>(v: Result<T, vault_core::VaultError>) -> Result<T, String> {
    v.map_err(String::from)
}

fn js<T: Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

/// Creates the "Bake-off showcase" page: one block of every kind in scope,
/// inline styles and a nested list, all through `vault.apply`.
fn create_showcase(v: &Vault) -> Result<(), String> {
    use BlockKind::*;
    let id = v.create_page(None, SHOWCASE_TITLE.to_string()).map_err(String::from)?;
    let mut ops = Vec::new();
    let mut last_top: Option<String> = None;
    let mut n = 0;
    // (kind, text, marks, checked, children)
    type Spec = (BlockKind, &'static str, Vec<(&'static str, MarkKind)>, bool, Vec<(BlockKind, &'static str, bool)>);
    let link = || MarkKind::Link("https://example.com".to_string());
    let specs: Vec<Spec> = vec![
        (Paragraph, "This page has one block of every kind the prototype supports. Text can be bold, italic, struck through, inline code or a link, and styles can combine.",
            vec![("bold", MarkKind::Bold), ("italic", MarkKind::Italic), ("struck through", MarkKind::Strike), ("inline code", MarkKind::Code), ("a link", link()), ("styles can combine", MarkKind::Bold), ("can combine", MarkKind::Italic)], false, vec![]),
        (Heading1, "Heading 1", vec![], false, vec![]),
        (Paragraph, "Headings come in three sizes. Type # and a space at the start of a block to make one.", vec![("#", MarkKind::Code)], false, vec![]),
        (Heading2, "Lists", vec![], false, vec![]),
        (Bulleted, "A bulleted list item", vec![], false, vec![
            (Bulleted, "A nested item", false),
            (Bulleted, "Another nested item", false),
        ]),
        (Bulleted, "Press Tab to indent, Shift+Tab to outdent", vec![("Tab", MarkKind::Bold), ("Shift+Tab", MarkKind::Bold)], false, vec![]),
        (Numbered, "First numbered item", vec![], false, vec![]),
        (Numbered, "Second numbered item", vec![], false, vec![(Numbered, "A nested numbered item", false)]),
        (Numbered, "Third numbered item", vec![], false, vec![]),
        (Heading3, "To-do list", vec![], false, vec![]),
        (Todo, "Write the showcase page", vec![], true, vec![]),
        (Todo, "Compare it with the Flutter prototype", vec![], false, vec![]),
        (Quote, "Simplicity is prerequisite for reliability. (Edsger W. Dijkstra)", vec![("Edsger W. Dijkstra", MarkKind::Italic)], false, vec![]),
        (Divider, "", vec![], false, vec![]),
        (Code, "fn main() {\n    println!(\"Hello from the bake-off\");\n}", vec![], false, vec![]),
        (Paragraph, "Type / for the block menu, or select text to format it.", vec![("/", MarkKind::Code), ("select text", MarkKind::Italic)], false, vec![]),
    ];
    for (kind, text, marks, checked, children) in specs {
        n += 1;
        let bid = format!("show{n}");
        let marks = marks
            .into_iter()
            .map(|(sub, kind)| {
                let at = text.find(sub).expect("showcase mark text");
                let start = text[..at].chars().count() as u32;
                Mark { start, end: start + sub.chars().count() as u32, kind }
            })
            .collect();
        ops.push(BlockOp::Insert {
            parent: None,
            after: last_top.clone(),
            block: NewBlock { id: bid.clone(), kind, text: text.to_string(), marks, checked },
        });
        let mut last_child: Option<String> = None;
        for (ck, ct, cc) in children {
            n += 1;
            let cid = format!("show{n}");
            ops.push(BlockOp::Insert {
                parent: Some(bid.clone()),
                after: last_child.clone(),
                block: NewBlock { id: cid.clone(), kind: ck, text: ct.to_string(), marks: vec![], checked: cc },
            });
            last_child = Some(cid);
        }
        last_top = Some(bid);
    }
    v.apply(id, ops).map_err(String::from)
}

#[cfg(target_os = "android")]
mod android_log {
    use std::ffi::{c_char, c_int, CString};

    #[link(name = "log")]
    extern "C" {
        fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
    }

    pub fn write(line: &str) {
        const INFO: c_int = 4;
        let tag = CString::new("BAKEOFF").unwrap();
        let Ok(text) = CString::new(line.replace('\0', "")) else { return };
        unsafe {
            __android_log_write(INFO, tag.as_ptr(), text.as_ptr());
        }
    }
}
