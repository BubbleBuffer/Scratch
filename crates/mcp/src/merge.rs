//! AI-proposed merges of sync conflicts: the prompt that shows the agent
//! both versions of a block, and the agent's answer read back as text and
//! marks for an ordinary agent edit. Nothing here changes the
//! vault: the app shows the proposal and applies it only when the user
//! accepts.

use serde::{Deserialize, Serialize};
use vault_core::{Block, Conflict, Mark, Vault};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeProposal {
    pub text: String,
    pub marks: Vec<Mark>,
    /// The complete parsed proposal, including every descendant. Callers
    /// applying just `text`/`marks` leave the existing children intact.
    pub blocks: Vec<Block>,
}

fn block_md(v: &Vault, b: &Option<Block>) -> String {
    match b {
        None => "(deleted)".into(),
        Some(b) => v.blocks_markdown(vec![b.clone()]),
    }
}

/// The prompt for a conflict: the page title, the merged block as it is
/// now and each device's version.
pub fn prompt(v: &Vault, c: &Conflict) -> String {
    let page = v.load_page(c.page.clone()).ok();
    let title = page.as_ref().map(|p| p.title.clone()).unwrap_or_default();
    let current = page
        .as_ref()
        .and_then(|p| find(&p.blocks, &c.block).cloned());
    let mut out = format!(
        "Two devices edited the same block of the note \"{title}\" at the same time. \
         The edited version wins; merge only text that both sides edited. \
         Keep children intact, including unchanged descendants.\n\n"
    );
    for (i, s) in c.sides.iter().enumerate() {
        out.push_str(&format!(
            "Version {} (device {}):\n<version>\n{}\n</version>\n\n",
            i + 1,
            short(&s.device),
            block_md(v, &s.block)
        ));
    }
    out.push_str(&format!(
        "Automatic merge, as the block is now:\n<current>\n{}\n</current>\n\n",
        block_md(v, &current)
    ));
    out.push_str(
        "Reply with only the merged block's text as markdown inline formatting (bold, italic, \
         links), without the list marker or heading prefix, and nothing else.",
    );
    out
}

fn short(d: &str) -> &str {
    &d[..d.len().min(8)]
}

fn find<'a>(blocks: &'a [Block], id: &str) -> Option<&'a Block> {
    blocks.iter().find_map(|b| {
        if b.id == id {
            Some(b)
        } else {
            find(&b.children, id)
        }
    })
}

/// Reads an agent's answer: code fences and a leading list marker or
/// heading prefix are dropped; several paragraphs become one text with
/// line breaks (and lose their inline marks). `blocks` always preserves the
/// complete parsed tree for agent callers.
pub fn parse(v: &Vault, answer: &str) -> MergeProposal {
    let mut s = answer.trim();
    if let Some(rest) = s.strip_prefix("```") {
        let rest = rest.split_once('\n').map(|x| x.1).unwrap_or("");
        s = rest.trim_end().strip_suffix("```").unwrap_or(rest).trim();
    }
    let blocks = v.parse_markdown(s.to_string());
    let (text, marks) = match blocks.len() {
        0 => (String::new(), vec![]),
        1 => (blocks[0].text.clone(), blocks[0].marks.clone()),
        _ => (blocks
                .iter()
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"), vec![]),
    };
    MergeProposal { text, marks, blocks }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    fn vault() -> (Temp, Vault) {
        let path = std::env::temp_dir().join(format!("conflict_merge_{}", vault_core::new_id()));
        let vault = Vault::open(path.to_string_lossy().into_owned()).unwrap();
        (Temp(path), vault)
    }
    #[test]
    fn merge_prompt_materialization_keeps_nested_children() {
        let (_dir, vault) = vault();
        let blocks = vault.parse_markdown("- Parent\n  - Child\n    - Grandchild".into());
        let markdown = block_md(&vault, &Some(blocks[0].clone()));
        assert!(markdown.contains("Parent"));
        assert!(markdown.contains("Child"));
        assert!(markdown.contains("Grandchild"));
    }
    #[test]
    fn merge_proposal_keeps_complete_parsed_subtree() {
        let (_dir, vault) = vault();
        let proposal = parse(&vault, "- **Parent**\n  - Child\n    - Grandchild");
        assert_eq!(proposal.text, "Parent");
        assert_eq!(proposal.marks.len(), 1);
        assert_eq!(proposal.blocks[0].children[0].text, "Child");
        assert_eq!(proposal.blocks[0].children[0].children[0].text, "Grandchild");
    }
}
