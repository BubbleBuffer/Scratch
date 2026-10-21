//! A `chat_ref` block keeps its chat payload in `target` across a vault reopen.

use std::fs;
use std::path::PathBuf;

use vault_core::*;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("vault_core_{tag}_{}", new_id()));
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).expect("open")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn chat_ref_payload_survives_a_reopen() {
    let t = TempDir::new("chat_ref");
    let summary = "Cursor fix, driver route dropped.";
    let payload = concat!(
        r#"{"v":1,"level":"chapter","sessionId":"e5445cc0-bf53-4854-8b45-690a8e61bbcc","#,
        r#""firstMessageId":"m-first","lastMessageId":"m-last","#,
        r#""chatTitle":"Cacheonlynebula","chapterTitle":"Replay bubbleplay","#,
        r#""summary":"Cursor fix, driver route dropped.","decision":"Keep the cursor local."}"#,
    );
    let page = {
        let v = t.open();
        let page = v.create_page(None, "Notes".into()).unwrap();
        v.apply(
            page.clone(),
            vec![BlockOp::Insert {
                parent: None,
                after: None,
                block: NewBlock {
                    id: "chat-1".into(),
                    kind: BlockKind::ChatRef,
                    text: summary.into(),
                    marks: vec![],
                    checked: false,
                },
            }],
        )
        .unwrap();
        v.apply(
            page.clone(),
            vec![BlockOp::SetTarget {
                id: "chat-1".into(),
                target: Some(payload.into()),
            }],
        )
        .unwrap();
        page
    };

    let v = t.open();
    let loaded = v.load_page(page).unwrap();
    let block = loaded
        .blocks
        .iter()
        .find(|b| b.id == "chat-1")
        .expect("the chat_ref block");
    assert_eq!(block.kind, BlockKind::ChatRef);
    assert_eq!(block.text, summary);
    assert_eq!(block.target.as_deref(), Some(payload));
    assert!(v
        .search("Cursor".into(), 10)
        .unwrap()
        .iter()
        .any(|hit| hit.id == loaded.id));
    assert!(
        v.search("Cacheonlynebula".into(), 10).unwrap().is_empty(),
        "the target payload is display data, only block text enters notes search"
    );
}
