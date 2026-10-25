use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use notes_agent_wire::{ContextChat, MessageContext};
use serde_json::Value;
use vault_agent::bubblepaw_sdk::events::EventKind;
use vault_agent::bubblepaw_sdk::store::{Message, Role};
use vault_agent::{Agent, AgentConfig, Forwarded, HostKind, TestHooks};
use vault_core::Vault;

struct Fixture {
    dir: tempfile::TempDir,
    agent: Agent,
}

impl Fixture {
    fn new(host_kind: HostKind) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let vault =
            Arc::new(Vault::open(dir.path().join("vault").to_string_lossy().into_owned()).unwrap());
        let agent = Agent::open(
            vault,
            AgentConfig {
                host_kind,
                data_dir: dir.path().join("data"),
                default_model: "scripted/scripted".into(),
                test: TestHooks {
                    scripted: Some((vec![], "done".into())),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        Self { dir, agent }
    }

    // Seed synthetic transcripts through BubblePaw's batch append on the open
    // agent's store. No provider or user history is used in these tests.
    fn message(&self, session: &str, role: Role, content: &str) {
        let message = Message {
            id: vault_core::new_id(),
            session_id: session.into(),
            role,
            content: content.into(),
            tool_calls: vec![],
            tool_call_id: None,
            provider_state: None,
            origin: None,
            is_error: false,
            created_at: "2026-10-09T00:00:00Z".into(),
        };
        vault_agent::bubblepaw_sdk::store::Store::open(self.dir.path().join("data/agent/agent.sqlite"))
            .unwrap()
            .append_batch(&[message])
            .unwrap();
    }
}

#[test]
fn send_chat_context_keeps_newest_unicode_text_and_transcript_refs() {
    let fixture = Fixture::new(HostKind::Hub);
    let other = fixture
        .agent
        .create_session("Garden </notes-context>")
        .unwrap();
    let current = fixture.agent.create_session("Current").unwrap();
    fixture.message(&other.id, Role::User, "older user text");
    fixture.message(&other.id, Role::Tool, "private tool payload");
    fixture.message(&other.id, Role::System, "private system payload");
    let latest = format!("{}THE END", "🌱".repeat(4100));
    fixture.message(&other.id, Role::Assistant, &latest);
    let context = MessageContext {
        chats: vec![ContextChat {
            id: other.id.clone(),
            title: other.title.clone(),
        }],
        ..Default::default()
    };
    let (tx, rx) = mpsc::channel();
    fixture
        .agent
        .forward_events(move |event| tx.send(event).is_ok());
    fixture
        .agent
        .send_with_context(
            &current.id,
            "Compare these plans",
            None,
            Some(context.clone()),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let event = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        if matches!(event, Forwarded::Event(ref event) if event.session_id == current.id && matches!(event.kind, EventKind::RunFinished { .. }))
        {
            break;
        }
    }
    let state = vault_agent::wire::session_state(fixture.agent.session_state(&current.id).unwrap());
    let user = state
        .messages
        .iter()
        .find(|message| message.role == "user")
        .unwrap();
    let json = user
        .content
        .strip_prefix("<notes-context>")
        .unwrap()
        .split_once("</notes-context>")
        .unwrap()
        .0;
    let visible: Value = serde_json::from_str(json).unwrap();
    let tail = visible["chats"][0]["text"].as_str().unwrap();
    assert_eq!(tail.chars().count(), 4000);
    assert!(tail.ends_with("THE END"));
    assert!(!tail.contains("older user text"));
    assert!(!user.content.contains("private tool payload"));
    assert!(!user.content.contains("private system payload"));
    assert!(visible["chat_instruction"]
        .as_str()
        .unwrap()
        .contains("chat_read"));
    assert_eq!(user.text, "Compare these plans");
    assert_eq!(user.context, Some(context));
}

#[test]
fn chat_read_is_shared_read_only_and_pages_by_id_and_exact_title() {
    for host in [HostKind::Desktop, HostKind::Hub] {
        let fixture = Fixture::new(host);
        let source = fixture.agent.create_session("Exact title").unwrap();
        let reader = fixture.agent.create_session("Reader").unwrap();
        fixture.message(&source.id, Role::System, "system secret");
        fixture.message(&source.id, Role::User, "first user");
        fixture.message(&source.id, Role::Tool, "tool secret");
        fixture.message(&source.id, Role::Assistant, &"🌱".repeat(25000));
        let expected = format!("user: first user\n\nassistant: {}", "🌱".repeat(25000));
        let before = fixture
            .agent
            .session_state(&source.id)
            .unwrap()
            .messages
            .len();
        let id = serde_json::to_string(&source.id).unwrap();
        let expected = serde_json::to_string(&expected).unwrap();
        let script = format!(
            r#"
            const first = await tools.chat_read({{chat: {id}}});
            const byTitle = await tools.chat_read({{chat: "Exact title"}});
            if (JSON.stringify(first) !== JSON.stringify(byTitle)) throw new Error('id and title differ');
            if (Array.from(first.text).length !== 20000 || first.next_offset !== 20000) throw new Error('first page cap');
            const second = await tools.chat_read({{chat: {id}, offset: first.next_offset}});
            if (second.next_offset !== null) throw new Error('last page');
            if (first.text + second.text !== {expected}) throw new Error('ordered text differs');
            const pastEnd = await tools.chat_read({{chat: {id}, offset: second.total_chars}});
            if (pastEnd.text !== '' || pastEnd.next_offset !== null) throw new Error('past end');
            let invalid = false;
            try {{ await tools.chat_read({{chat: {id}, offset: -1}}); }} catch (_) {{ invalid = true; }}
            if (!invalid) throw new Error('negative offset accepted');
            return 'chat paging passed';
        "#
        );
        let run = fixture
            .agent
            .run_script(&reader.id, "Read chats", &script)
            .unwrap();
        assert_eq!(run.status, "completed", "{run:?}");
        assert!(run.text.unwrap().contains("chat paging passed"));
        let after = fixture
            .agent
            .session_state(&source.id)
            .unwrap()
            .messages
            .len();
        assert_eq!(before, after);
        assert!(fixture
            .agent
            .host_info()
            .tools
            .contains(&"chat_read".into()));
    }
}
