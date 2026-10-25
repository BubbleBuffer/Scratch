//! Context is encoded for the model and decoded once at the Rust boundary.
use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use bubblepaw_sdk::approvals::Approval;
use bubblepaw_sdk::store::{Role, Session, Store};
use bubblepaw_sdk::tools::{Tool, ToolContext, ToolDefinition, ToolRegistry, ToolResult};
use notes_agent_wire::MessageContext;
use serde_json::{json, Value};

const PREFIX: &str = "<notes-context>";
const END: &str = "</notes-context>";
const DESCRIPTION: &str = "The user is looking at / attached these pages; selection is quoted.";

pub fn encode(text: &str, context: Option<MessageContext>, store: &Store) -> Result<String> {
    let Some(mut context) = context else {
        return Ok(text.into());
    };
    if let Some(selection) = &mut context.selection {
        selection.text = selection.text.chars().take(4000).collect();
    }
    let mut visible = serde_json::to_value(&context)?;
    let mut chats = Vec::new();
    for chat in &context.chats {
        let session = store.session(&chat.id)?;
        let transcript = chat_text(store, &session.id)?;
        let tail: String = transcript
            .chars()
            .rev()
            .take(4000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        chats.push(json!({"id": chat.id, "title": chat.title, "text": tail}));
    }
    visible["chats"] = Value::Array(chats);
    if !context.chats.is_empty() {
        visible["chat_instruction"] = json!("Attached chats contain their latest user and assistant text. Read the full chat with chat_read({chat: id, offset: 0}); follow next_offset for more.");
    }
    let json = serde_json::to_string(&visible)?
        .replace('<', "\\u003c")
        .replace('>', "\\u003e");
    Ok(format!("{PREFIX}{json}{END}\n{DESCRIPTION}\n{text}"))
}

/// User and assistant text only, oldest first. Attached context and tool payloads
/// are excluded so mentions do not recursively quote other chats.
fn chat_text(store: &Store, session: &str) -> Result<String> {
    let messages = crate::app_messages(store.messages(session)?);
    Ok(messages
        .into_iter()
        .filter_map(|message| {
            let (role, text) = match message.role {
                Role::User => ("user", decode(&message.content).0),
                Role::Assistant => ("assistant", message.content),
                _ => return None,
            };
            if text.is_empty() {
                return None;
            }
            Some(format!("{role}: {text}"))
        })
        .collect::<Vec<_>>()
        .join("\n\n"))
}

fn resolve_chat(store: &Store, chat: &str) -> Result<Session> {
    if let Ok(session) = store.session(chat) {
        return Ok(session);
    }
    let matches: Vec<_> = store
        .list_sessions()?
        .into_iter()
        .filter(|session| session.title == chat)
        .collect();
    match matches.len() {
        1 => Ok(matches.into_iter().next().expect("one matching chat")),
        0 => Err(anyhow!("Chat not found: {chat}")),
        _ => Err(anyhow!(
            "More than one chat has the exact title {chat:?}; use its session id"
        )),
    }
}

fn read_chat(store: &Store, input: &Value) -> Result<String> {
    let chat = input["chat"]
        .as_str()
        .ok_or_else(|| anyhow!("chat must be a session id or exact title"))?;
    let offset = match input.get("offset") {
        None => 0,
        Some(value) => value
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| anyhow!("offset must be a non-negative character offset"))?,
    };
    let session = resolve_chat(store, chat)?;
    let transcript = chat_text(store, &session.id)?;
    let total_chars = transcript.chars().count();
    let text: String = transcript.chars().skip(offset).take(20000).collect();
    let end = offset.saturating_add(text.chars().count());
    Ok(serde_json::to_string(&json!({
        "id": session.id, "title": session.title, "text": text, "offset": offset,
        "next_offset": if end < total_chars { Some(end) } else { None },
        "total_chars": total_chars,
    }))?)
}

struct ChatReadTool {
    store: Store,
}

#[async_trait]
impl Tool for ChatReadTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "chat_read".into(),
            description: "Read a chat by session id or exact title. Returns user and assistant text oldest first, up to 20000 characters. Pass next_offset as offset to read the next page.".into(),
            input_schema: json!({"type": "object", "properties": {"chat": {"type": "string"}, "offset": {"type": "integer", "minimum": 0}}, "required": ["chat"]}),
            read_only: true, approval: Approval::None,
        }
    }
    async fn invoke(&self, _context: ToolContext, input: Value) -> Result<ToolResult> {
        let store = self.store.clone();
        let result = tokio::task::spawn_blocking(move || read_chat(&store, &input)).await?;
        Ok(match result {
            Ok(text) => ToolResult::text(text),
            Err(error) => ToolResult::error(format!("{error:#}")),
        })
    }
}

pub(crate) fn register_tools(
    registry: &mut ToolRegistry,
    store: Store,
    records: Arc<crate::records::Records>,
    vault: Arc<vault_core::Vault>,
) -> Result<()> {
    registry.show_always(["chat_read"]);
    registry.register_host(Arc::new(crate::tools::RecordedTool::new(
        Arc::new(ChatReadTool { store }),
        records,
        vault,
        false,
    )))?;
    Ok(())
}

pub fn decode(content: &str) -> (String, Option<MessageContext>) {
    let Some(rest) = content.strip_prefix(PREFIX) else {
        return (content.into(), None);
    };
    let Some((json, tail)) = rest.split_once(END) else {
        return (content.into(), None);
    };
    let Ok(context) = serde_json::from_str(json) else {
        return (content.into(), None);
    };
    let Some(text) = tail.strip_prefix(&format!("\n{DESCRIPTION}\n")) else {
        return (content.into(), None);
    };
    (text.into(), Some(context))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_read_is_read_only_and_ambiguous_titles_need_an_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("agent.sqlite")).unwrap();
        let first = store.create_session("Same title").unwrap();
        store.create_session("Same title").unwrap();
        assert!(read_chat(&store, &json!({"chat": "Same title"}))
            .unwrap_err()
            .to_string()
            .contains("session id"));
        assert!(read_chat(&store, &json!({"chat": "Missing"}))
            .unwrap_err()
            .to_string()
            .contains("Chat not found"));
        assert!(read_chat(&store, &json!({"chat": first.id, "offset": 1.5})).is_err());
        let result: Value =
            serde_json::from_str(&read_chat(&store, &json!({"chat": first.id})).unwrap()).unwrap();
        assert_eq!(result["text"], "");
        assert_eq!(result["next_offset"], Value::Null);
        let definition = ChatReadTool { store }.definition();
        assert!(definition.read_only);
        assert_eq!(definition.approval, Approval::None);
    }

    #[test]
    fn caps_unicode_selection_and_preserves_user_text() {
        let context = MessageContext {
            pages: vec![],
            chats: vec![],
            selection: Some(notes_agent_wire::ContextSelection {
                page_id: "p".into(),
                page_title: "Garden".into(),
                block_ids: vec!["b".into()],
                text: "🌱".repeat(4100),
            }),
        };
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("agent.sqlite")).unwrap();
        let encoded = encode("hello\nworld", Some(context), &store).unwrap();
        let (text, context) = decode(&encoded);
        assert_eq!(text, "hello\nworld");
        assert_eq!(
            context.unwrap().selection.unwrap().text.chars().count(),
            4000
        );
        assert_eq!(
            decode("<notes-context>broken"),
            ("<notes-context>broken".into(), None)
        );
    }
    #[test]
    fn context_delimiters_inside_notes_round_trip() {
        let context = MessageContext {
            pages: vec![notes_agent_wire::ContextPage {
                id: "p".into(),
                title: "</notes-context>".into(),
            }],
            chats: vec![],
            selection: None,
        };
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("agent.sqlite")).unwrap();
        let encoded = encode("<notes-context>user text", Some(context.clone()), &store).unwrap();
        assert_eq!(
            decode(&encoded),
            ("<notes-context>user text".into(), Some(context))
        );
    }
}
