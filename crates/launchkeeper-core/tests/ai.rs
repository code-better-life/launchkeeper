//! `launchkeeper_core::ai` against a local mock HTTP server (M4 §1).
//!
//! **No test here reaches the network and no test here touches the real
//! keychain.** The provider tests point `AiConfig::base_url` at a
//! `127.0.0.1` [`TcpListener`] this file owns, and the key always comes from
//! `LAUNCHKEEPER_AI_API_KEY` — the override that outranks the keychain
//! exists precisely so that a test (or CI, or a headless run) never has to
//! unlock a login keychain or dismiss an authorization dialog.
//!
//! This file lives under `tests/` rather than in `src/ai.rs` for the same
//! reason `paths_env.rs` does: it calls `std::env::set_var`, which is an
//! `unsafe fn` and cannot appear under the crate's `#![forbid(unsafe_code)]`.
//! An integration test is its own crate and does not inherit that attribute.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock, mpsc};

use chrono::{TimeZone, Utc};
use launchkeeper_core::ai::{
    self, AiConfig, AiProvider, ExplainInput, Lang, MAX_LOG_BYTES, Prompt,
};
use launchkeeper_core::store::{Run, RunId, StopReason, TriggerKind};
use launchkeeper_core::{Error, Service, Store, Task, TaskName, Trigger, paths};

/// One request the mock server saw.
#[derive(Debug)]
struct Captured {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: String,
}

/// A one-shot HTTP/1.1 responder on `127.0.0.1:0`.
///
/// Deliberately hand-rolled on `std::net` rather than pulled in as a
/// dev-dependency: the whole contract under test is "we send these headers
/// and this JSON body to this path, and we read that field back out", and
/// forty lines of `TcpListener` say that more directly than a mocking DSL —
/// while adding nothing to the dependency tree of a crate that ships inside
/// a signed app bundle.
struct MockServer {
    base_url: String,
    rx: mpsc::Receiver<Captured>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl MockServer {
    /// Answers exactly one request with `status` and `body`, then stops.
    fn once(status: u16, body: &str) -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let (tx, rx) = mpsc::channel();
        let body = body.to_string();
        let handle = std::thread::spawn(move || {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            match serve(stream, status, &body) {
                Ok(c) => {
                    let _ = tx.send(c);
                }
                Err(e) => eprintln!("mock server: {e}"),
            }
        });
        MockServer {
            base_url: format!("http://127.0.0.1:{port}"),
            rx,
            handle: Some(handle),
        }
    }

    /// The request the client made. Panics if none arrived.
    fn captured(mut self) -> Captured {
        let c = self
            .rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("mock server 没有收到请求");
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        c
    }
}

fn serve(mut stream: TcpStream, status: u16, body: &str) -> std::io::Result<Captured> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    let mut headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let len: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;

    let reason = if (200..300).contains(&status) {
        "OK"
    } else {
        "Error"
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()?;

    Ok(Captured {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&buf).into_owned(),
    })
}

// ---------------------------------------------------------------------------

/// Serializes the tests that mutate process-wide environment variables.
///
/// Environment variables are per *process*, and cargo runs the tests in one
/// file on several threads of one process — so two tests each pointing
/// `LAUNCHKEEPER_DATA_DIR` at their own `tempdir` will, run together, have
/// one of them open a database in the other's directory and then watch it be
/// deleted underneath (`disk I/O error`, seen for real while writing this).
/// `paths_env.rs` avoids the same trap by keeping everything in a single
/// `#[test]`; this file has several distinct scenarios, so it takes a lock
/// instead. Poisoning is recovered from: a failing test must report its own
/// assertion, not turn every later test into a mutex panic.
fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn task(name: &str) -> Task {
    let mut t = Task::new(
        TaskName::new(name).unwrap(),
        "/usr/bin/true",
        Trigger::Interval { seconds: 3600 },
    );
    t.env.insert("SECRET_TOKEN".into(), "do-not-leak".into());
    t
}

fn input(name: &str) -> ExplainInput {
    ExplainInput {
        task: task(name),
        runs: vec![],
        log_tail: "boom\n".into(),
        lang: Lang::ZhCn,
    }
}

fn anthropic_cfg(base: &str) -> AiConfig {
    AiConfig {
        provider: AiProvider::Anthropic,
        base_url: Some(base.to_string()),
        model: "claude-sonnet-5".into(),
    }
}

fn openai_cfg(base: &str) -> AiConfig {
    AiConfig {
        provider: AiProvider::OpenAiCompatible,
        base_url: Some(format!("{base}/v1")),
        model: "gpt-4o-mini".into(),
    }
}

#[test]
fn anthropic_request_and_response_round_trip() {
    let server = MockServer::once(
        200,
        r#"{"id":"msg_01","type":"message","role":"assistant",
            "content":[{"type":"text","text":"这个任务每小时跑一次。"}],
            "model":"claude-sonnet-5","stop_reason":"end_turn"}"#,
    );
    let cfg = anthropic_cfg(&server.base_url);
    let insight = ai::explain(&cfg, "sk-ant-test", &input("hourly")).expect("explain");

    assert_eq!(insight.content, "这个任务每小时跑一次。");
    assert_eq!(insight.model, "claude-sonnet-5");
    assert_eq!(insight.task_name.as_str(), "hourly");
    assert_eq!(insight.prompt_hash.len(), 64);

    let req = server.captured();
    assert_eq!(req.method, "POST");
    assert_eq!(req.path, "/v1/messages");
    // The Anthropic auth shape: `x-api-key` + a version header, never a
    // bearer token.
    assert_eq!(
        req.headers.get("x-api-key").map(String::as_str),
        Some("sk-ant-test")
    );
    assert_eq!(
        req.headers.get("anthropic-version").map(String::as_str),
        Some("2023-06-01")
    );
    assert!(!req.headers.contains_key("authorization"));

    let body: serde_json::Value = serde_json::from_str(&req.body).expect("请求体是 JSON");
    assert_eq!(body["model"], "claude-sonnet-5");
    // Anthropic takes the system prompt as its own top-level field, not as a
    // `messages` entry.
    assert!(body["system"].as_str().unwrap().contains("Launchkeeper"));
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    assert_eq!(body["messages"][0]["role"], "user");
    assert!(body["max_tokens"].as_u64().unwrap() > 0);
    // The redaction rule, verified on the bytes that actually left: the name
    // is there, the value is not.
    assert!(req.body.contains("SECRET_TOKEN"));
    assert!(!req.body.contains("do-not-leak"), "环境变量的值不能上网");
}

#[test]
fn openai_request_and_response_round_trip() {
    let server = MockServer::once(
        200,
        r#"{"id":"chatcmpl-1","object":"chat.completion",
            "choices":[{"index":0,"message":{"role":"assistant","content":"Runs hourly."},
            "finish_reason":"stop"}]}"#,
    );
    let cfg = openai_cfg(&server.base_url);
    let insight = ai::explain(&cfg, "sk-openai-test", &input("hourly")).expect("explain");

    assert_eq!(insight.content, "Runs hourly.");
    assert_eq!(insight.model, "gpt-4o-mini");

    let req = server.captured();
    assert_eq!(req.path, "/v1/chat/completions");
    // The OpenAI auth shape: a bearer token, and no Anthropic headers.
    assert_eq!(
        req.headers.get("authorization").map(String::as_str),
        Some("Bearer sk-openai-test")
    );
    assert!(!req.headers.contains_key("x-api-key"));
    assert!(!req.headers.contains_key("anthropic-version"));

    let body: serde_json::Value = serde_json::from_str(&req.body).expect("请求体是 JSON");
    // OpenAI takes the system prompt as the first `messages` entry.
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs[1]["role"], "user");
    assert!(!req.body.contains("do-not-leak"));
}

#[test]
fn an_http_error_carries_the_status_and_the_providers_own_message() {
    let server = MockServer::once(
        401,
        r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#,
    );
    let cfg = anthropic_cfg(&server.base_url);
    let err = ai::explain(&cfg, "wrong", &input("hourly")).unwrap_err();
    match err {
        Error::AiHttp { status, message } => {
            assert_eq!(status, 401);
            assert_eq!(message, "invalid x-api-key");
        }
        other => panic!("期望 AiHttp，得到 {other:?}"),
    }
    // And the rendered message is the one the user reads in a toast.
    let rendered = Error::AiHttp {
        status: 429,
        message: "rate limited".into(),
    }
    .to_string();
    assert!(rendered.contains("429"));
    assert!(rendered.contains("rate limited"));
}

#[test]
fn a_gateway_that_answers_html_still_produces_a_readable_error() {
    let server = MockServer::once(502, "<html><body>Bad Gateway</body></html>");
    let cfg = openai_cfg(&server.base_url);
    let err = ai::explain(&cfg, "k", &input("hourly")).unwrap_err();
    match err {
        Error::AiHttp { status, message } => {
            assert_eq!(status, 502);
            assert!(message.contains("Bad Gateway"), "得到 {message:?}");
        }
        other => panic!("期望 AiHttp，得到 {other:?}"),
    }
}

#[test]
fn test_connection_is_one_short_request() {
    let server = MockServer::once(
        200,
        r#"{"content":[{"type":"text","text":"ok"}],"model":"claude-sonnet-5"}"#,
    );
    let cfg = anthropic_cfg(&server.base_url);
    assert_eq!(ai::test_connection(&cfg, "sk-ant-test").unwrap(), "ok");

    let req = server.captured();
    let body: serde_json::Value = serde_json::from_str(&req.body).unwrap();
    // A ping must not cost an explanation's worth of tokens, and must not
    // carry a task's data at all.
    assert!(body["max_tokens"].as_u64().unwrap() <= 32);
    assert_eq!(body["messages"][0]["content"], "ping");
}

/// The env override is the whole reason no test in this repository has to
/// touch the login keychain.
#[test]
fn the_env_variable_overrides_the_keychain_and_wins_when_set() {
    let _env = env_lock();
    // SAFETY: this integration test is single-threaded within its own
    // process for the duration of these mutations (`--test-threads` does not
    // isolate env, so nothing else in this file reads the variable).
    unsafe { std::env::set_var(ai::API_KEY_ENV, "  sk-from-env  ") };
    // Trimmed, and answered without the keychain being consulted at all.
    assert_eq!(ai::get_api_key().unwrap(), Some("sk-from-env".to_string()));
    assert!(ai::has_api_key().unwrap());

    // Blank is treated as absent rather than as an empty key, so an
    // `export LAUNCHKEEPER_AI_API_KEY=` in a shell rc does not shadow the
    // keychain with nothing.
    unsafe { std::env::set_var(ai::API_KEY_ENV, "   ") };
    // (Falls through to the keychain, which this machine may or may not have
    // an entry in — so only the *not the blank string* part is asserted.)
    assert_ne!(ai::get_api_key().unwrap(), Some("   ".to_string()));

    unsafe { std::env::remove_var(ai::API_KEY_ENV) };
}

#[test]
fn explaining_without_a_key_fails_before_any_request() {
    // A server that will answer if anyone asks — nobody should.
    let server = MockServer::once(200, r#"{"content":[{"type":"text","text":"x"}]}"#);
    let cfg = anthropic_cfg(&server.base_url);
    assert!(matches!(
        ai::explain(&cfg, "", &input("hourly")).unwrap_err(),
        Error::AiNoKey
    ));
}

/// `ai.json` round-trips through the shared data directory, which is what
/// lets the CLI and the app agree on a provider without either owning the
/// file.
#[test]
fn the_config_file_round_trips_in_the_data_dir() {
    let _env = env_lock();
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: single-threaded test process; every path lookup below happens
    // after this.
    unsafe { std::env::set_var(paths::DATA_DIR_ENV, dir.path()) };

    // Nothing written yet: the default, not an error.
    assert_eq!(ai::load_config().unwrap(), AiConfig::default());
    assert_eq!(ai::config_path().unwrap(), dir.path().join("ai.json"));

    let cfg = AiConfig {
        provider: AiProvider::OpenAiCompatible,
        base_url: Some("http://127.0.0.1:11434/v1".into()),
        model: "llama3.2".into(),
    };
    ai::save_config(&cfg).unwrap();
    assert_eq!(ai::load_config().unwrap(), cfg);

    // The on-disk spelling is the stable one, not serde's default
    // `open_ai_compatible`.
    let raw = std::fs::read_to_string(dir.path().join("ai.json")).unwrap();
    assert!(raw.contains("\"openai_compatible\""), "得到 {raw}");
    // No temp file left behind.
    let strays: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".tmp"))
        .collect();
    assert!(strays.is_empty(), "残留临时文件: {strays:?}");

    // A hand-broken file reads as the default rather than failing.
    std::fs::write(dir.path().join("ai.json"), "{ not json").unwrap();
    assert_eq!(ai::load_config().unwrap(), AiConfig::default());
    // ...and can be overwritten normally.
    ai::save_config(&cfg).unwrap();
    assert_eq!(ai::load_config().unwrap(), cfg);

    unsafe { std::env::remove_var(paths::DATA_DIR_ENV) };
}

/// The service-level contract: gather, call, store — and, without
/// `refresh`, do not call at all.
#[test]
fn explain_task_stores_the_answer_and_reuses_it_until_refresh() {
    let _env = env_lock();
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: single-threaded test process.
    unsafe {
        std::env::set_var(paths::DATA_DIR_ENV, dir.path());
        std::env::set_var(
            paths::LAUNCH_AGENTS_DIR_ENV,
            dir.path().join("LaunchAgents"),
        );
        std::env::set_var(paths::RUNNER_LOG_DIR_ENV, dir.path().join("RunnerLogs"));
        std::env::set_var(ai::API_KEY_ENV, "sk-test");
    }

    let store = Store::open(&paths::db_path().unwrap()).unwrap();
    let name = TaskName::new("sync").unwrap();
    store.insert_task(&task("sync")).unwrap();

    // One finished run, with real log files on disk for the tail to read.
    let out = dir.path().join("run.out");
    let err = dir.path().join("run.err");
    std::fs::write(&out, "downloaded 3 files\n").unwrap();
    std::fs::write(&err, "Traceback: ConnectionRefusedError\n").unwrap();
    let id = store
        .start_run(
            &name,
            TriggerKind::Scheduled,
            Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            &out,
            &err,
        )
        .unwrap();
    store
        .finish_run(
            id,
            Utc.timestamp_opt(1_700_000_030, 0).unwrap(),
            Some(1),
            Some(StopReason::Exited),
        )
        .unwrap();
    let service = Service::without_runner(store);

    let server = MockServer::once(
        200,
        r#"{"content":[{"type":"text","text":"连不上对象存储。"}],"model":"claude-sonnet-5"}"#,
    );
    let cfg = anthropic_cfg(&server.base_url);
    let first = service
        .explain_task(&name, &cfg, Lang::ZhCn, false)
        .expect("explain_task");
    assert_eq!(first.content, "连不上对象存储。");
    // Stored, and readable back through the read-only accessor.
    assert_eq!(service.insight(&name).unwrap().as_ref(), Some(&first));

    // Both log streams reached the prompt, labelled.
    let req = server.captured();
    assert!(req.body.contains("downloaded 3 files"));
    assert!(req.body.contains("ConnectionRefusedError"));
    assert!(req.body.contains("--- stderr ---"));
    // The run's outcome is described too, not just its log.
    assert!(req.body.contains("exit 1"));

    // Without `refresh`, the second call must not make a request at all —
    // there is no server listening now, so a request would fail outright.
    let again = service
        .explain_task(&name, &cfg, Lang::ZhCn, false)
        .expect("缓存命中时不应该发请求");
    assert_eq!(again, first);

    // With `refresh`, it calls again and overwrites.
    let server = MockServer::once(
        200,
        r#"{"content":[{"type":"text","text":"第二次的结论。"}],"model":"claude-sonnet-5"}"#,
    );
    let cfg = anthropic_cfg(&server.base_url);
    let refreshed = service
        .explain_task(&name, &cfg, Lang::ZhCn, true)
        .expect("refresh");
    assert_eq!(refreshed.content, "第二次的结论。");
    assert_eq!(service.insight(&name).unwrap().unwrap(), refreshed);

    // Deleting the task takes the insight with it (FK cascade).
    service.store().delete_task(&name).unwrap();
    assert!(matches!(
        service.insight(&name).unwrap_err(),
        Error::TaskNotFound(_)
    ));
    assert_eq!(service.store().get_insight(&name).unwrap(), None);

    unsafe {
        std::env::remove_var(paths::DATA_DIR_ENV);
        std::env::remove_var(paths::LAUNCH_AGENTS_DIR_ENV);
        std::env::remove_var(paths::RUNNER_LOG_DIR_ENV);
        std::env::remove_var(ai::API_KEY_ENV);
    }
}

/// A giant log must not become a giant request, whatever the caller hands in.
#[test]
fn a_huge_log_tail_is_bounded_in_the_request_that_is_actually_sent() {
    let server = MockServer::once(200, r#"{"content":[{"type":"text","text":"ok"}]}"#);
    let cfg = anthropic_cfg(&server.base_url);
    let mut i = input("noisy");
    i.log_tail = "A".repeat(4 * 1024 * 1024); // 4 MiB
    ai::explain(&cfg, "sk", &i).expect("explain");

    let req = server.captured();
    // The prompt preamble is well under a kilobyte, so anything much beyond
    // the log budget means the cap did not apply.
    assert!(
        req.body.len() < MAX_LOG_BYTES + 8 * 1024,
        "请求体 {} 字节，日志上限应当是 {MAX_LOG_BYTES}",
        req.body.len()
    );
}

/// A run whose log files are gone (pruned, or on a volume that is not
/// mounted) still explains — with less evidence, not with an error.
#[test]
fn a_missing_log_file_is_not_a_failure() {
    let run = Run {
        id: RunId(1),
        task_name: TaskName::new("gone").unwrap(),
        started_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
        finished_at: Some(Utc.timestamp_opt(1_700_000_001, 0).unwrap()),
        exit_code: Some(0),
        pid: None,
        stdout_path: PathBuf::from("/nonexistent/nope.out"),
        stderr_path: PathBuf::from(""),
        trigger_kind: TriggerKind::Manual,
        stop_reason: Some(StopReason::Exited),
    };
    let i = ExplainInput {
        task: task("gone"),
        runs: vec![run],
        log_tail: String::new(),
        lang: Lang::En,
    };
    let p: Prompt = ai::build_prompt(&i);
    assert!(p.user.contains("(empty)"));
    assert!(p.user.contains("exit 0"));
}
