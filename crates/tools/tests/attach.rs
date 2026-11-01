//! attach_file against a local vault: by data and by path (local paths are
//! enabled in this test binary, as the desktop app and notes-mcp do), image
//! and file blocks, placement, the size caps and image validation. URL
//! fetching is covered by the unit tests in src/attach.rs (a local server
//! under a test-only loopback flag).

use base64::Engine;
use serde_json::{json, Value};
use vault_core::{BlockKind, Vault};
use vault_tools::call;

const PNG: &[u8] =
    b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89";

struct Temp(std::path::PathBuf);

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp() -> (Temp, Vault) {
    let dir =
        Temp(std::env::temp_dir().join(format!("vault_tools_attach_{}", vault_core::new_id())));
    let vault = Vault::open(dir.0.to_string_lossy().into_owned()).unwrap();
    (dir, vault)
}

fn ok(v: &Vault, name: &str, args: Value) -> String {
    call(v, name, &args).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn fails(v: &Vault, name: &str, args: Value) -> String {
    match call(v, name, &args) {
        Ok(out) => panic!("{name} should fail, gave: {out}"),
        Err(e) => e.0,
    }
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn block_id(text: &str) -> String {
    text.lines()
        .next()
        .and_then(|l| l.rsplit(": ").next())
        .unwrap()
        .to_string()
}

fn page(v: &Vault, title: &str, markdown: &str) -> String {
    let id = v.create_page(None, title.into()).unwrap();
    if !markdown.is_empty() {
        v.insert_markdown(id.clone(), None, None, true, markdown.into())
            .unwrap();
    }
    id
}

#[test]
fn attaches_an_image_by_data_at_the_end() {
    let (_d, v) = temp();
    let id = page(&v, "Pics", "first\n\nsecond");
    let out = ok(
        &v,
        "attach_file",
        json!({"page":"Pics","data":b64(PNG),"caption":"A dot"}),
    );
    assert!(out.contains("as an image block"), "{out}");
    assert!(out.contains("attachment: ../.vault/blobs/"), "{out}");
    let blocks = v.load_page(id).unwrap().blocks;
    let last = blocks.last().unwrap();
    assert_eq!(last.id, block_id(&out));
    assert_eq!(last.kind, BlockKind::Image);
    assert_eq!(last.text, "A dot");
    let blob = last.target.clone().unwrap();
    assert_eq!(
        std::fs::read(v.attachment_path(blob.clone()).unwrap()).unwrap(),
        PNG
    );
    // The stored name gets an image extension, so its MIME type is right.
    let info = v.attachment_info(blob).unwrap();
    assert_eq!(info.mime, "image/png");
    // read_page shows the image with its caption.
    let read = ok(&v, "read_page", json!({"page":"Pics"}));
    assert!(read.contains("A dot"), "{read}");
}

#[test]
fn attaches_after_a_block() {
    let (_d, v) = temp();
    let id = page(&v, "Order", "one\n\ntwo\n\nthree");
    let blocks = v.load_page(id.clone()).unwrap().blocks;
    let first = blocks[0].id.clone();
    let out = ok(
        &v,
        "attach_file",
        json!({"page":id,"data":b64(PNG),"after_block":first}),
    );
    let blocks = v.load_page(id).unwrap().blocks;
    assert_eq!(blocks.len(), 4);
    assert_eq!(blocks[1].id, block_id(&out));
    assert_eq!(blocks[1].kind, BlockKind::Image);
    let e = fails(
        &v,
        "attach_file",
        json!({"page":"Order","data":b64(PNG),"after_block":"nope"}),
    );
    assert!(e.contains("no block nope"), "{e}");
}

#[test]
fn attaches_a_file_and_keeps_a_sanitised_name() {
    let (_d, v) = temp();
    let id = page(&v, "Docs", "");
    let out = ok(
        &v,
        "attach_file",
        json!({"page":"Docs","data":b64(b"col1,col2\n1,2\n"),"name":"../../report:2026.csv"}),
    );
    assert!(out.contains("as a file block"), "{out}");
    let b = v.load_page(id).unwrap().blocks.pop().unwrap();
    assert_eq!(b.kind, BlockKind::File);
    assert_eq!(b.text, "report_2026.csv");
    let info = v.attachment_info(b.target.unwrap()).unwrap();
    assert_eq!(info.name, "report_2026.csv");
    // Image bytes stored as a file when asked.
    let out = ok(
        &v,
        "attach_file",
        json!({"page":"Docs","data":b64(PNG),"as":"file","name":"dot.png"}),
    );
    assert!(out.contains("as a file block"), "{out}");
}

#[test]
fn attaches_by_local_path_when_enabled() {
    vault_tools::allow_local_paths(true);
    let (d, v) = temp();
    let id = page(&v, "Local", "");
    let file =
        d.0.join("..")
            .join(format!("attach_src_{}.png", vault_core::new_id()));
    std::fs::write(&file, PNG).unwrap();
    let out = ok(
        &v,
        "attach_file",
        json!({"page":"Local","path":file.to_string_lossy()}),
    );
    std::fs::remove_file(&file).unwrap();
    assert!(out.contains("as an image block"), "{out}");
    let b = v.load_page(id).unwrap().blocks.pop().unwrap();
    assert_eq!(b.kind, BlockKind::Image);
    let e = fails(
        &v,
        "attach_file",
        json!({"page":"Local","path":d.0.join("missing.png").to_string_lossy()}),
    );
    assert!(e.contains("cannot read"), "{e}");
}

#[test]
fn refuses_oversized_input() {
    vault_tools::allow_local_paths(true);
    let (d, v) = temp();
    page(&v, "Big", "");
    let over = vec![0u8; vault_tools::attach::MAX_DATA_BYTES + 1];
    let e = fails(&v, "attach_file", json!({"page":"Big","data":b64(&over)}));
    assert!(e.contains("10 MB"), "{e}");
    let file =
        d.0.join("..")
            .join(format!("attach_big_{}.bin", vault_core::new_id()));
    let f = std::fs::File::create(&file).unwrap();
    f.set_len(vault_tools::attach::MAX_BYTES as u64 + 1)
        .unwrap();
    drop(f);
    let e = fails(
        &v,
        "attach_file",
        json!({"page":"Big","path":file.to_string_lossy()}),
    );
    std::fs::remove_file(&file).unwrap();
    assert!(e.contains("25 MB"), "{e}");
}

#[test]
fn validates_images_and_arguments() {
    let (_d, v) = temp();
    page(&v, "Check", "");
    let e = fails(
        &v,
        "attach_file",
        json!({"page":"Check","data":b64(b"just text"),"as":"image"}),
    );
    assert!(e.contains("not an image"), "{e}");
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>";
    let e = fails(&v, "attach_file", json!({"page":"Check","data":b64(svg)}));
    assert!(e.contains("svg refused"), "{e}");
    let e = fails(
        &v,
        "attach_file",
        json!({"page":"Check","data":b64(svg),"as":"image"}),
    );
    assert!(e.contains("svg refused"), "{e}");
    let clean = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><circle r=\"1\"/></svg>";
    let out = ok(
        &v,
        "attach_file",
        json!({"page":"Check","data":b64(clean),"name":"c"}),
    );
    assert!(out.contains("c.svg"), "{out}");
    let e = fails(&v, "attach_file", json!({"page":"Check"}));
    assert!(e.contains("exactly one of path, url or data"), "{e}");
    let e = fails(
        &v,
        "attach_file",
        json!({"page":"Check","data":"AA==","url":"https://example.com/a.png"}),
    );
    assert!(e.contains("exactly one"), "{e}");
    let e = fails(
        &v,
        "attach_file",
        json!({"page":"Check","url":"http://example.com/a.png"}),
    );
    assert!(e.contains("https"), "{e}");
    let e = fails(
        &v,
        "attach_file",
        json!({"page":"Check","url":"https://127.0.0.1/a.png"}),
    );
    assert!(e.contains("private"), "{e}");
    let e = fails(
        &v,
        "attach_file",
        json!({"page":"Check","data":"not base64!"}),
    );
    assert!(e.contains("base64"), "{e}");
    let e = fails(&v, "attach_file", json!({"page":"Missing","data":b64(PNG)}));
    assert!(!e.is_empty());
}
