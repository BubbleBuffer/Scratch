//! The jobs API (`jobs` tool on `/mcp`) end to end on an in-process hub,
//! against the agent's scripted provider (no real model): start, status,
//! result and cancel; tool sets; owners; limits; retention; restarts.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notes_hub::agent_tokens::{AgentScope, AgentTokens};
use notes_hub::{AgentSection, Config, JobLimits, Server};
use reqwest::Client;
use serde_json::{json, Value};
use vault_agent::bubblepaw_sdk::tools::ToolCall;
use vault_agent::bubblepaw_sdk::providers::ScriptedStep;
use vault_agent::TestHooks;
use vault_core::{new_id, BlockKind, Vault};

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Env {
    server: Option<Server>,
    client: Client,
    url: String,
    read: String,
    writer: String,
    other: String,
    everything: String,
    /// A device token (`hub.pair`); any `X-Device` id goes with it.
    device: String,
    _data: TempDir,
}

impl Drop for Env {
    fn drop(&mut self) {
        drop(self.server.take());
    }
}

fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: format!("call-{}", new_id()),
        name: name.into(),
        arguments,
    }
}

fn steps(calls: Vec<ToolCall>) -> Vec<ScriptedStep> {
    calls
        .into_iter()
        .map(|c| ScriptedStep::Calls { calls: vec![c] })
        .collect()
}

fn fast_limits() -> JobLimits {
    JobLimits {
        tick: Duration::from_millis(200),
        ..JobLimits::default()
    }
}

/// A hub whose job engines run `script` (each engine gets a fresh copy) and
/// then answer `final_text`. `prepare` sees the data dir before the start.
async fn env_with(
    limits: JobLimits,
    script: Vec<ScriptedStep>,
    final_text: &str,
    prepare: impl FnOnce(&Path),
) -> Env {
    let data = TempDir(std::env::temp_dir().join(format!("notes_hub_jobs_{}", new_id())));
    fs::create_dir_all(&data.0).unwrap();
    prepare(&data.0);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = Config {
        listen: listener.local_addr().unwrap().to_string(),
        data_dir: data.0.clone(),
        trust_proxy: false,
        agent: AgentSection {
            default_model: "scripted/scripted".into(),
            models: vec!["scripted/scripted".into()],
            ..AgentSection::default()
        },
        job_limits: limits,
        ..Config::default()
    };
    let store = AgentTokens::open(config.vault_dir()).unwrap();
    let read = store.add("reader", AgentScope::Read).unwrap();
    let writer = store.add("writer", AgentScope::Write).unwrap();
    let other = store.add("other", AgentScope::Write).unwrap();
    let everything = store.add("everything", AgentScope::Everything).unwrap();
    let final_text = final_text.to_string();
    let hooks: notes_hub::AgentTestHooks = Arc::new(move || TestHooks {
        scripted: Some((script.clone(), final_text.clone())),
        confirm_tool: true,
        sleep_tool: true,
        ..TestHooks::default()
    });
    let server = notes_hub::start_with(listener, config, Some(hooks))
        .await
        .unwrap();
    let url = format!("http://{}/mcp", server.addr());
    let hub = server.hub().clone();
    let device = tokio::task::spawn_blocking(move || hub.pair_legacy_token("phone"))
        .await
        .unwrap()
        .unwrap();
    Env {
        server: Some(server),
        client: Client::new(),
        url,
        read,
        writer,
        other,
        everything,
        device,
        _data: data,
    }
}

async fn env(script: Vec<ScriptedStep>, final_text: &str) -> Env {
    env_with(fast_limits(), script, final_text, |_| {}).await
}

impl Env {
    fn vault(&self) -> &Arc<Vault> {
        self.server.as_ref().unwrap().hub().vault()
    }

    async fn rpc(&self, token: &str, method: &str, params: Value) -> Value {
        self.rpc_as(token, None, method, params).await
    }

    /// [`Env::rpc`] with an `X-Device` header (a signed-in device).
    async fn rpc_as(&self, token: &str, device: Option<&str>, method: &str, params: Value) -> Value {
        let mut request = self.client.post(&self.url).bearer_auth(token);
        if let Some(device) = device {
            request = request.header("X-Device", device);
        }
        let response = request
            .json(&json!({"jsonrpc":"2.0", "id":1, "method":method, "params":params}))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success(), "{}", response.status());
        response.json().await.unwrap()
    }

    /// Calls `jobs`: Ok(the job JSON) or Err(the tool's error text).
    async fn jobs(&self, token: &str, arguments: Value) -> Result<Value, String> {
        self.jobs_as(token, None, arguments).await
    }

    /// [`Env::jobs`] as the signed-in device `device` (the device token).
    async fn device_jobs(&self, device: &str, arguments: Value) -> Result<Value, String> {
        let token = self.device.clone();
        self.jobs_as(&token, Some(device), arguments).await
    }

    /// Polls as `device` until the job has left queued and running.
    async fn device_finished(&self, device: &str, id: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let job = self
                .device_jobs(device, json!({"action":"get", "job":id}))
                .await
                .unwrap();
            if !matches!(job["status"].as_str(), Some("queued" | "running")) {
                return job;
            }
            assert!(Instant::now() < deadline, "job never ended: {job}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn jobs_as(
        &self,
        token: &str,
        device: Option<&str>,
        arguments: Value,
    ) -> Result<Value, String> {
        let reply = self
            .rpc_as(
                token,
                device,
                "tools/call",
                json!({"name":"jobs", "arguments":arguments}),
            )
            .await;
        if let Some(e) = reply.get("error") {
            return Err(e["message"].as_str().unwrap_or_default().to_string());
        }
        let text = reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if reply["result"]["isError"] == true {
            return Err(text);
        }
        Ok(serde_json::from_str(&text).unwrap_or_else(|_| panic!("not JSON: {text}")))
    }

    async fn create(&self, token: &str, prompt: &str) -> Value {
        self.jobs(token, json!({"action":"create", "prompt":prompt}))
            .await
            .unwrap()
    }

    /// Waits (through `wait`) until the job ends.
    async fn finished(&self, token: &str, id: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let job = self
                .jobs(token, json!({"action":"wait", "job":id, "timeout_ms":5000}))
                .await
                .unwrap();
            if !matches!(job["status"].as_str(), Some("queued" | "running")) {
                return job;
            }
            assert!(Instant::now() < deadline, "job never ended: {job}");
        }
    }

    /// Polls until the job has `status`.
    async fn reaches(&self, token: &str, id: &str, status: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let job = self
                .jobs(token, json!({"action":"get", "job":id}))
                .await
                .unwrap();
            if job["status"] == status {
                return job;
            }
            assert!(Instant::now() < deadline, "job never reached {status}: {job}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn page_titled(&self, title: &str) -> bool {
        let vault = self.vault().clone();
        let title = title.to_string();
        std::thread::spawn(move || {
            // By title: other pages (the run's records) may quote the title.
            vault
                .search_full(title.clone(), 50)
                .map(|hits| hits.iter().any(|h| h.title == title))
                .unwrap_or(false)
        })
        .join()
        .unwrap()
    }
}

fn sleep_call(ms: u64) -> ToolCall {
    call("test_sleep", json!({"ms": ms}))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hub_job_reports_progress_even_with_the_read_tool_set() {
    let env = env(
        steps(vec![call(
            "report_progress",
            json!({"message":"  Read the references  "}),
        )]),
        "Finished.",
    )
    .await;
    let job = env
        .jobs(
            &env.writer,
            json!({
                "action":"create", "prompt":"Read references", "tools":"read"
            }),
        )
        .await
        .unwrap();
    let id = job["id"].as_str().unwrap().to_string();
    let done = env.finished(&env.writer, &id).await;
    assert_eq!(done["status"], "completed", "{done}");
    let vault = env.vault().clone();
    tokio::task::spawn_blocking(move || {
        let row = vault.load_page(id).unwrap();
        let prop = |key: &str| {
            row.properties
                .iter()
                .find_map(|p| match &p.value {
                    vault_core::PropValue::Text(text) if p.key == key => Some(text.clone()),
                    _ => None,
                })
                .unwrap_or_default()
        };
        assert_eq!(prop("progress"), "Read the references");
        assert!(prop("progress_at").parse::<i64>().unwrap() > 0);
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn start_status_result_end_to_end() {
    let env = env(
        steps(vec![call(
            "create_page",
            json!({"title":"Made by a job", "markdown":"hello"}),
        )]),
        "Made the page.",
    )
    .await;

    let job = env.create(&env.writer, "Make a page").await;
    let id = job["id"].as_str().unwrap().to_string();
    assert!(matches!(job["status"].as_str(), Some("queued" | "running")), "{job}");
    assert_eq!(job["tools"], "write");
    assert!(job.get("owner").is_none(), "{job}");

    let done = env.finished(&env.writer, &id).await;
    assert_eq!(done["status"], "completed", "{done}");
    assert_eq!(done["result"], "Made the page.");
    assert!(done["finished"].as_i64().is_some(), "{done}");
    assert!(env.page_titled("Made by a job"));

    // get returns the same; list shows the caller's job.
    let got = env.jobs(&env.writer, json!({"action":"get", "job":id})).await.unwrap();
    assert_eq!(got["result"], "Made the page.");
    let list = env.jobs(&env.writer, json!({"action":"list"})).await.unwrap();
    assert_eq!(list["jobs"].as_array().unwrap().len(), 1);
    assert_eq!(list["jobs"][0]["id"], id.as_str());

    // The job is a row of the AI tasks database, which devices sync.
    let vault = env.vault().clone();
    let row_id = id.clone();
    let (task, body) = std::thread::spawn(move || {
        let task = vault_mcp::tasks::get(&vault, &row_id).unwrap();
        let page = vault.load_page(row_id).unwrap();
        (task, vault.blocks_markdown(page.blocks))
    })
    .join()
    .unwrap();
    assert_eq!(task.status, "done");
    assert!(body.contains("Made the page."), "{body}");

    // A desktop never claims a job.
    let vault = env.vault().clone();
    let queued = env.create(&env.writer, "Another").await;
    let claimed = std::thread::spawn(move || vault_mcp::tasks::claim_next(&vault, "desktop").unwrap())
        .join()
        .unwrap();
    assert!(claimed.is_none(), "{claimed:?}");
    env.finished(&env.writer, queued["id"].as_str().unwrap()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_cannot_use_tools_outside_its_set() {
    let env = env(
        steps(vec![call(
            "create_page",
            json!({"title":"Should not exist", "markdown":"x"}),
        )]),
        "Tried.",
    )
    .await;

    // Read callers neither see nor call jobs.
    let listed = env.rpc(&env.read, "tools/list", json!({})).await;
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(!names.contains(&"jobs"), "{names:?}");
    let refused = env
        .jobs(&env.read, json!({"action":"create", "prompt":"x"}))
        .await
        .unwrap_err();
    assert!(refused.contains("jobs"), "{refused}");
    let listed = env.rpc(&env.writer, "tools/list", json!({})).await;
    assert!(listed.to_string().contains("\"jobs\""));

    // A job can't widen the caller's set.
    let wider = env
        .jobs(&env.writer, json!({"action":"create", "prompt":"x", "tools":"everything"}))
        .await
        .unwrap_err();
    assert!(wider.contains("wider"), "{wider}");

    // Narrowed to Read, the run has no create_page.
    let job = env
        .jobs(&env.writer, json!({"action":"create", "prompt":"Make a page", "tools":"read"}))
        .await
        .unwrap();
    assert_eq!(job["tools"], "read");
    let done = env.finished(&env.writer, job["id"].as_str().unwrap()).await;
    assert!(matches!(done["status"].as_str(), Some("completed" | "failed")), "{done}");
    assert!(!env.page_titled("Should not exist"));

    // The hub runner's set narrows every job: Read there makes a Write
    // caller's job Read.
    let vault = env.vault().clone();
    std::thread::spawn(move || {
        vault
            .set_agent_tool_set("hub".into(), Some("read".into()))
            .unwrap()
    })
    .join()
    .unwrap();
    let job = env.create(&env.writer, "Make a page").await;
    assert_eq!(job["tools"], "read");
    env.finished(&env.writer, job["id"].as_str().unwrap()).await;
    assert!(!env.page_titled("Should not exist"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_never_waits_for_approval() {
    let env = env(
        steps(vec![call("test_confirm", json!({"text":"go"}))]),
        "Confirmed without asking.",
    )
    .await;
    let job = env.create(&env.everything, "Use the confirm tool").await;
    assert_eq!(job["tools"], "everything");
    let done = env.finished(&env.everything, job["id"].as_str().unwrap()).await;
    assert_eq!(done["status"], "completed", "{done}");
    assert_eq!(done["result"], "Confirmed without asking.");
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_owner_sees_and_cancels_a_job() {
    let env = env(steps(vec![sleep_call(60_000)]), "slept").await;
    let job = env.create(&env.writer, "Sleep").await;
    let id = job["id"].as_str().unwrap().to_string();
    env.reaches(&env.writer, &id, "running").await;

    for action in ["get", "wait", "cancel"] {
        let refused = env
            .jobs(&env.other, json!({"action":action, "job":id, "timeout_ms":0}))
            .await
            .unwrap_err();
        assert!(refused.contains("no such job"), "{action}: {refused}");
    }
    let theirs = env.jobs(&env.other, json!({"action":"list"})).await.unwrap();
    assert_eq!(theirs["jobs"].as_array().unwrap().len(), 0);
    let oauth_like = env.jobs(&env.everything, json!({"action":"get", "job":id})).await;
    assert!(oauth_like.is_err());

    let started = Instant::now();
    let cancelled = env
        .jobs(&env.writer, json!({"action":"cancel", "job":id}))
        .await
        .unwrap();
    assert_eq!(cancelled["status"], "cancelled", "{cancelled}");
    let again = env
        .jobs(&env.writer, json!({"action":"cancel", "job":id}))
        .await
        .unwrap_err();
    assert!(again.contains("already ended"), "{again}");

    // The run stopped: the owner's next job starts (one running per owner).
    // (The script may sleep again in the new session, so it is cancelled too.)
    let next = env.create(&env.writer, "Next").await;
    let next_id = next["id"].as_str().unwrap().to_string();
    loop {
        let job = env
            .jobs(&env.writer, json!({"action":"get", "job":next_id}))
            .await
            .unwrap();
        if job["status"] != "queued" {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(30), "never started: {job}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let _ = env
        .jobs(&env.writer, json!({"action":"cancel", "job":next_id}))
        .await;
    let first = env.jobs(&env.writer, json!({"action":"get", "job":id})).await.unwrap();
    assert_eq!(first["status"], "cancelled", "{first}");
}

#[tokio::test(flavor = "multi_thread")]
async fn queue_wait_and_run_limits() {
    let limits = JobLimits {
        max_queued: 4,
        max_wait: Duration::from_millis(500),
        max_run: Duration::from_secs(3),
        ..fast_limits()
    };
    let env = env_with(limits, steps(vec![sleep_call(60_000)]), "slept", |_| {}).await;
    let first = env.create(&env.writer, "Sleep").await;
    let first_id = first["id"].as_str().unwrap().to_string();
    env.reaches(&env.writer, &first_id, "running").await;

    // One running job per owner: the next four wait in the queue.
    let mut queued = Vec::new();
    for n in 0..4 {
        let job = env.create(&env.writer, &format!("Queued {n}")).await;
        queued.push(job["id"].as_str().unwrap().to_string());
    }
    tokio::time::sleep(Duration::from_millis(400)).await;
    for id in &queued {
        let job = env.jobs(&env.writer, json!({"action":"get", "job":id})).await.unwrap();
        assert_eq!(job["status"], "queued", "{job}");
    }
    // Four queued per hub, whoever asks.
    for token in [&env.writer, &env.other] {
        let full = env
            .jobs(token, json!({"action":"create", "prompt":"One more"}))
            .await
            .unwrap_err();
        assert!(full.contains("queue is full"), "{full}");
    }

    // wait is capped (here at 0.5 s), even when asked for a minute.
    let asked = Instant::now();
    let waited = env
        .jobs(&env.writer, json!({"action":"wait", "job":first_id, "timeout_ms":60000}))
        .await
        .unwrap();
    assert_eq!(waited["status"], "running", "{waited}");
    assert!(asked.elapsed() < Duration::from_secs(5), "{:?}", asked.elapsed());

    // The run is stopped at its time limit and the job fails.
    let ended = env.finished(&env.writer, &first_id).await;
    assert_eq!(ended["status"], "failed", "{ended}");
    assert!(ended["error"].as_str().unwrap().contains("limit"), "{ended}");

    // The queue drains, one at a time.
    for id in &queued {
        let done = env.finished(&env.writer, id).await;
        assert!(matches!(done["status"].as_str(), Some("completed" | "failed")), "{done}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ended_jobs_session_expires_after_the_keep_time_but_the_row_stays() {
    let limits = JobLimits {
        keep: Duration::from_secs(1),
        ..fast_limits()
    };
    let env = env_with(limits, vec![], "done", |_| {}).await;
    let job = env.create(&env.writer, "Quick").await;
    let id = job["id"].as_str().unwrap().to_string();
    let done = env.finished(&env.writer, &id).await;
    assert_eq!(done["status"], "completed", "{done}");

    // The session (transcript and logs) expires; the row and its outcome stay.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let vault = env.vault().clone();
        let job_id = id.clone();
        let cleared = std::thread::spawn(move || {
            vault_mcp::jobs::get(&vault, &job_id)
                .unwrap()
                .map(|j| j.session.is_empty())
        })
        .join()
        .unwrap();
        if cleared == Some(true) {
            break;
        }
        assert!(Instant::now() < deadline, "the session never expired");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let kept = env.finished(&env.writer, &id).await;
    assert_eq!(kept["status"], "completed", "{kept}");
    let vault = env.vault().clone();
    let trashed = std::thread::spawn(move || vault.load_page(id).unwrap().in_trash)
        .join()
        .unwrap();
    assert!(!trashed);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restart_marks_running_jobs_interrupted() {
    let mut ids = (String::new(), String::new());
    let env = env_with(fast_limits(), vec![], "done", |data| {
        let vault = Vault::open(data.join("vault").to_string_lossy().into_owned()).unwrap();
        let running =
            vault_mcp::jobs::create(&vault, "Was running", "agent:writer", "write", "hub").unwrap();
        vault_mcp::jobs::mark_running(&vault, &running.id, "old-session").unwrap();
        let queued =
            vault_mcp::jobs::create(&vault, "Was queued", "agent:writer", "write", "hub").unwrap();
        ids = (running.id, queued.id);
    })
    .await;
    let interrupted = env.reaches(&env.writer, &ids.0, "interrupted").await;
    assert!(interrupted["error"].as_str().unwrap().contains("restarted"), "{interrupted}");
    // Not queued again; a queued job still runs.
    let done = env.finished(&env.writer, &ids.1).await;
    assert_eq!(done["status"], "completed", "{done}");
    let still = env.jobs(&env.writer, json!({"action":"get", "job":ids.0})).await.unwrap();
    assert_eq!(still["status"], "interrupted");
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_calls_are_refused() {
    let env = env(vec![], "done").await;
    for (args, want) in [
        (json!({}), "needs an action"),
        (json!({"action":"nope"}), "unknown jobs action"),
        (json!({"action":"create"}), "needs a prompt"),
        (json!({"action":"get"}), "job id"),
        (json!({"action":"get", "job":"nope"}), "no such job"),
    ] {
        let e = env.jobs(&env.writer, args.clone()).await.unwrap_err();
        assert!(e.contains(want), "{args}: {e}");
    }
    // A plain page id is no job.
    let vault = env.vault().clone();
    let page = std::thread::spawn(move || vault.create_page(None, "Not a job".into()).unwrap())
        .join()
        .unwrap();
    let e = env
        .jobs(&env.writer, json!({"action":"get", "job":page}))
        .await
        .unwrap_err();
    assert!(e.contains("no such job"), "{e}");
}

/// The page's blocks as markdown (off the async executor).
fn page_markdown(vault: &Arc<Vault>, page: &str) -> String {
    let (vault, page) = (vault.clone(), page.to_string());
    std::thread::spawn(move || {
        let doc = vault.load_page(page).unwrap();
        vault.blocks_markdown(doc.blocks)
    })
    .join()
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_asks_ai_about_a_page_as_a_hub_job() {
    let env = env(vec![], "The answer from the hub.").await;
    let vault = env.vault().clone();
    let (page, first) = std::thread::spawn(move || {
        let page = vault.create_page(None, "Trip plan".into()).unwrap();
        let ids = vault
            .insert_markdown(page.clone(), None, None, true, "First line\n\nSecond line".into())
            .unwrap();
        (page, ids[0].clone())
    })
    .join()
    .unwrap();

    // A device token without X-Device is no agent token; a wrong token
    // with X-Device is refused too.
    for (token, device) in [(env.device.as_str(), None), ("not-a-token", Some("laptop"))] {
        let mut request = env.client.post(&env.url).bearer_auth(token);
        if let Some(device) = device {
            request = request.header("X-Device", device);
        }
        let response = request
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401, "{token:?} {device:?}");
    }
    // A device calls only jobs on /mcp.
    let token = env.device.clone();
    let reply = env
        .rpc_as(
            &token,
            Some("laptop"),
            "tools/call",
            json!({"name":"search_notes", "arguments":{"query":"Trip"}}),
        )
        .await;
    assert!(reply.to_string().contains("only the jobs tool"), "{reply}");

    // The job runs with the hub runner's own set (default Everything) and
    // the scripted provider's answer replaces the placeholder after the
    // chosen block.
    let job = env
        .device_jobs(
            "laptop",
            json!({"action":"create", "prompt":"Summarize this page",
                   "page":page, "after":first}),
        )
        .await
        .unwrap();
    assert_eq!(job["tools"], "everything", "{job}");
    assert_eq!(job["page"], json!(page), "{job}");
    let id = job["id"].as_str().unwrap().to_string();
    let done = env.device_finished("laptop", &id).await;
    assert_eq!(done["status"], "completed", "{done}");
    assert_eq!(done["result"], "The answer from the hub.", "{done}");
    let md = page_markdown(env.vault(), &page);
    assert!(md.contains("The answer from the hub."), "{md}");
    assert!(!md.contains("AI task"), "placeholder left: {md}");
    let answer_at = md.find("The answer from the hub.").unwrap();
    assert!(md.find("First line").unwrap() < answer_at, "{md}");
    assert!(answer_at < md.find("Second line").unwrap(), "{md}");

    // With tool_sets.hub = read, a device's job is Read.
    let vault = env.vault().clone();
    std::thread::spawn(move || {
        vault
            .set_agent_tool_set("hub".into(), Some("read".into()))
            .unwrap()
    })
    .join()
    .unwrap();
    let job = env
        .device_jobs("laptop", json!({"action":"create", "prompt":"Again", "page":page}))
        .await
        .unwrap();
    assert_eq!(job["tools"], "read", "{job}");
    env.device_finished("laptop", job["id"].as_str().unwrap()).await;

    // after without page, and a missing page, are refused.
    let e = env
        .device_jobs("laptop", json!({"action":"create", "prompt":"x", "after":first}))
        .await
        .unwrap_err();
    assert!(e.contains("page"), "{e}");
    let e = env
        .device_jobs("laptop", json!({"action":"create", "prompt":"x", "page":"nope"}))
        .await
        .unwrap_err();
    assert!(e.contains("no such page"), "{e}");
}

#[tokio::test(flavor = "multi_thread")]
async fn any_device_cancels_a_device_job_but_agents_do_not_see_it() {
    let env = env(steps(vec![sleep_call(60_000)]), "slept").await;
    let vault = env.vault().clone();
    let page = std::thread::spawn(move || vault.create_page(None, "A task".into()).unwrap())
        .join()
        .unwrap();
    let job = env
        .device_jobs("laptop", json!({"action":"create", "prompt":"Do this task", "page":page}))
        .await
        .unwrap();
    let id = job["id"].as_str().unwrap().to_string();
    // Another signed-in device, with its own token (a device token stays
    // bound to the device that first used it).
    let hub = env.server.as_ref().unwrap().hub().clone();
    let phone = tokio::task::spawn_blocking(move || hub.pair_legacy_token("phone2"))
        .await
        .unwrap()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let job = env
            .jobs_as(&phone, Some("phone"), json!({"action":"get", "job":id}))
            .await
            .unwrap();
        if job["status"] == "running" {
            break;
        }
        assert!(Instant::now() < deadline, "never ran: {job}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // An agent token neither sees nor cancels it.
    let refused = env
        .jobs(&env.writer, json!({"action":"cancel", "job":id}))
        .await
        .unwrap_err();
    assert!(refused.contains("no such job"), "{refused}");
    let listed = env
        .jobs_as(&phone, Some("phone"), json!({"action":"list"}))
        .await
        .unwrap();
    assert_eq!(listed["jobs"].as_array().unwrap().len(), 1, "{listed}");
    // Another device (the one that unassigned the task) cancels it.
    let cancelled = env
        .jobs_as(&phone, Some("phone"), json!({"action":"cancel", "job":id}))
        .await
        .unwrap();
    assert_eq!(cancelled["status"], "cancelled", "{cancelled}");
    let md = page_markdown(env.vault(), &page);
    assert!(md.contains("AI task cancelled"), "{md}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_leaves_its_todo_subtasks_nested_under_its_placeholder() {
    // Three todo replacements, then the final reply. Each replacement mirrors
    // into the placeholder; the run drains them before the job completes.
    // The todo tool takes the whole list as {items: [...]}; the saved state is
    // the bare array the mirror reads.
    let todo = |items: Value| call("todo", json!({"items": items}));
    let env = env(
        steps(vec![
            todo(json!([
                {"text": "Read the notes", "status": "pending"},
                {"text": "Draft", "status": "pending"}
            ])),
            todo(json!([
                {"text": "Read the notes", "status": "done"},
                {"text": "Draft", "status": "in_progress"}
            ])),
            todo(json!([
                {"text": "Read the notes", "status": "done"},
                {"text": "Draft", "status": "done"},
                {"text": "Publish", "status": "pending"}
            ])),
        ]),
        "All done.",
    )
    .await;
    let vault = env.vault().clone();
    let (page, first) = std::thread::spawn(move || {
        let page = vault.create_page(None, "Writing plan".into()).unwrap();
        let ids = vault
            .insert_markdown(page.clone(), None, None, true, "First line".into())
            .unwrap();
        (page, ids[0].clone())
    })
    .join()
    .unwrap();
    let job = env
        .device_jobs(
            "laptop",
            json!({"action":"create", "prompt":"Write the post",
                   "page":page, "after":first}),
        )
        .await
        .unwrap();
    let id = job["id"].as_str().unwrap().to_string();
    let done = env.device_finished("laptop", &id).await;
    assert_eq!(done["status"], "completed", "{done}");
    assert_eq!(done["result"], "All done.", "{done}");

    let vault = env.vault().clone();
    let doc = std::thread::spawn(move || vault.load_page(page).unwrap())
        .join()
        .unwrap();
    assert_eq!(doc.blocks[0].text, "First line");
    let at = doc
        .blocks
        .iter()
        .position(|b| b.text.starts_with("AI task done"))
        .unwrap_or_else(|| panic!("no placeholder left"));
    assert_eq!(at, 1, "placeholder moved");
    let subtasks: Vec<(String, bool)> = doc.blocks[at]
        .children
        .iter()
        .filter(|c| c.kind == BlockKind::Todo)
        .map(|c| (c.text.clone(), c.checked))
        .collect();
    assert_eq!(
        subtasks,
        vec![
            ("Read the notes".to_string(), true),
            ("Draft".to_string(), true),
            ("Publish".to_string(), false),
        ]
    );
    // The final reply goes after the placeholder, not inside it.
    assert_eq!(doc.blocks[at + 1].text, "All done.");
}
