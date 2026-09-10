//! `launchkeeper-runner`: the process launchd actually executes.
//!
//! Design contract: `docs/M1-design.md` §9 and `docs/M2.5-design.md` §4.
//! launchd's `ProgramArguments` always points here, never at the user's
//! script directly, so that every run gets logged and recorded in the
//! database regardless of how it was triggered.
//!
//! Stopping a service goes through this process: `launchctl kill TERM` sends
//! SIGTERM here, the runner forwards it to the script, waits for it, records
//! the run as [`StopReason::Stopped`] and then **exits 0**. That exit code is
//! load-bearing — a service plist carries `KeepAlive = {SuccessfulExit =
//! false}`, so any other exit would have launchd restart the job right back
//! (measured, `docs/M2.5-design.md` §3.1).

use std::io::Write;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chrono::Utc;
use launchkeeper_core::logs::RunLogs;
use launchkeeper_core::{RunId, StopReason, Store, Task, TaskName, TriggerKind, env, logs, paths};

const EX_USAGE: i32 = 64;
const EX_CONFIG: i32 = 78;
/// The script could not be started (or its logs could not be created).
const EX_CANNOT_START: i32 = 127;
/// The script hit its per-task timeout and was killed. Same convention as
/// coreutils `timeout(1)`.
const EX_TIMEOUT: i32 = 124;

/// How often the child is polled, both for completion and for a pending stop
/// signal.
const POLL_INTERVAL: Duration = Duration::from_millis(200);
/// Grace period between SIGTERM and SIGKILL.
const KILL_GRACE: Duration = Duration::from_secs(5);

fn diag(msg: &str) {
    let ts = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    eprintln!("[{ts}] {msg}");
}

fn usage() -> &'static str {
    "usage: launchkeeper-runner run <name> [--manual]\n       launchkeeper-runner --help | --version"
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = run(&args);
    std::process::exit(code);
}

/// Installs the SIGTERM/SIGINT handlers and returns the flag they raise.
///
/// Registered before the child is spawned so that a signal arriving in the
/// gap between spawn and the first poll is not lost: `signal-hook`'s flag is
/// sticky, so [`wait_for_child`] sees it on its very first iteration.
///
/// A failure here is not fatal — the run still happens, it just loses the
/// graceful-stop path — so it is only reported.
fn install_stop_handlers() -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    for sig in [
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGHUP,
    ] {
        if let Err(e) = signal_hook::flag::register(sig, Arc::clone(&flag)) {
            diag(&format!("注册信号 {sig} 处理器失败: {e}"));
        }
    }
    flag
}

fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("--help") | Some("-h") => {
            println!("{}", usage());
            0
        }
        Some("--version") => {
            println!("launchkeeper-runner {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("run") => run_task(&args[1..]),
        _ => {
            eprintln!("{}", usage());
            EX_USAGE
        }
    }
}

fn run_task(args: &[String]) -> i32 {
    let mut name_arg: Option<&str> = None;
    let mut manual = false;
    for a in args {
        match a.as_str() {
            "--manual" => manual = true,
            other if !other.starts_with('-') && name_arg.is_none() => name_arg = Some(other),
            _ => {
                eprintln!("{}", usage());
                return EX_USAGE;
            }
        }
    }
    let Some(name_str) = name_arg else {
        eprintln!("{}", usage());
        return EX_USAGE;
    };

    // 先装信号处理器，再做任何可能耗时的事（开库、建日志），这样
    // `launchctl kill TERM` 落在启动阶段也不会把 runner 直接打死。
    let stop = install_stop_handlers();

    let name = match TaskName::new(name_str) {
        Ok(n) => n,
        Err(e) => {
            diag(&format!("非法任务名 {name_str:?}: {e}"));
            return EX_CONFIG;
        }
    };

    let db_path = match paths::db_path() {
        Ok(p) => p,
        Err(e) => {
            diag(&format!("无法确定数据库路径: {e}"));
            return EX_CONFIG;
        }
    };
    let store = match Store::open(&db_path) {
        Ok(s) => s,
        Err(e) => {
            diag(&format!("打开数据库失败: {e}"));
            return EX_CONFIG;
        }
    };

    let task = match store.get_task(&name) {
        Ok(Some(t)) => t,
        Ok(None) => {
            diag(&format!("任务不存在: {name}"));
            return EX_CONFIG;
        }
        Err(e) => {
            diag(&format!("读取任务失败: {e}"));
            return EX_CONFIG;
        }
    };

    let started_at = Utc::now();
    let run_logs = match logs::create_run_logs(&name, started_at) {
        Ok(p) => p,
        Err(e) => {
            diag(&format!("创建日志文件失败: {e}"));
            return EX_CONFIG;
        }
    };

    let kind = if manual {
        TriggerKind::Manual
    } else {
        TriggerKind::Scheduled
    };

    let run_id = match store.start_run(
        &name,
        kind,
        started_at,
        &run_logs.stdout_path,
        &run_logs.stderr_path,
    ) {
        Ok(id) => id,
        Err(e) => {
            diag(&format!("记录运行开始失败: {e}"));
            return EX_CONFIG;
        }
    };

    let exit_code = execute(&store, &name, run_id, &task, run_logs, &stop);

    match store.prune_runs(&name, launchkeeper_core::DEFAULT_KEEP_RUNS) {
        Ok(pruned) => {
            for r in &pruned {
                logs::remove_run_logs(r);
            }
        }
        Err(e) => diag(&format!("裁剪历史运行失败: {e}")),
    }

    exit_code
}

/// Runs the task's script, records pid/finish/stop reason, and returns the
/// exit code this process itself should exit with.
///
/// `stop` is raised by the SIGTERM/SIGINT handler; when it is, the script is
/// terminated and the runner still returns 0 (see the module docs).
fn execute(
    store: &Store,
    name: &TaskName,
    run_id: RunId,
    task: &Task,
    run_logs: RunLogs,
    stop: &AtomicBool,
) -> i32 {
    let RunLogs {
        stdout: stdout_file,
        stderr: stderr_file,
        ..
    } = run_logs;
    // Keep a second handle open on the same file so we can still write an
    // error message to it after the first handle has been consumed by
    // `Stdio::from` below.
    let mut stderr_for_errors = match stderr_file.try_clone() {
        Ok(f) => f,
        Err(e) => {
            diag(&format!("复制 stderr 句柄失败: {e}"));
            let _ = store.finish_run(
                run_id,
                Utc::now(),
                Some(EX_CANNOT_START),
                Some(StopReason::Exited),
            );
            return EX_CANNOT_START;
        }
    };

    let working_dir = task.working_dir.clone().unwrap_or_else(home_dir);

    let path_override = task.env.get("PATH").cloned();
    let path_value = path_override.unwrap_or_else(env::effective_path);

    let mut cmd = Command::new(&task.script_path);
    cmd.args(&task.args)
        .current_dir(&working_dir)
        .env("PATH", &path_value)
        .env("LAUNCHKEEPER_TASK", name.as_str())
        .env("LAUNCHKEEPER_RUN_ID", run_id.0.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file));
    for (k, v) in &task.env {
        if k == "PATH" {
            continue;
        }
        cmd.env(k, v);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("启动脚本失败: {} ({e})", task.script_path.display());
            diag(&msg);
            let _ = writeln!(stderr_for_errors, "{msg}");
            let _ = store.finish_run(
                run_id,
                Utc::now(),
                Some(EX_CANNOT_START),
                Some(StopReason::Exited),
            );
            return EX_CANNOT_START;
        }
    };

    if let Err(e) = store.set_run_pid(run_id, child.id()) {
        diag(&format!("记录 pid 失败: {e}"));
    }

    let outcome = match wait_for_child(&mut child, task.timeout_secs, stop) {
        Ok(o) => o,
        Err(e) => {
            diag(&format!("等待子进程失败: {e}"));
            let _ = store.finish_run(run_id, Utc::now(), Some(EX_CANNOT_START), None);
            return EX_CANNOT_START;
        }
    };

    let finished_at = Utc::now();
    match outcome {
        Outcome::Exited(status) => {
            let exit_code = status.code();
            if let Err(e) =
                store.finish_run(run_id, finished_at, exit_code, Some(StopReason::Exited))
            {
                diag(&format!("记录运行结束失败: {e}"));
            }
            match exit_code {
                Some(code) => code,
                None => 128 + status.signal().unwrap_or(0),
            }
        }
        Outcome::TimedOut => {
            let secs = task.timeout_secs.unwrap_or(0);
            let msg = format!("超时 {secs} 秒，已终止脚本（exit {EX_TIMEOUT}）");
            diag(&msg);
            let _ = writeln!(stderr_for_errors, "[launchkeeper] {msg}");
            let _ = store.finish_run(
                run_id,
                finished_at,
                Some(EX_TIMEOUT),
                Some(StopReason::Timeout),
            );
            EX_TIMEOUT
        }
        Outcome::Stopped(status) => {
            // 子进程自己处理了 TERM 就有退出码，被信号杀死就没有。两种都记下来，
            // 但 runner 自己一律以 0 退出：这才是 launchd 不再重启的信号。
            let exit_code = status.and_then(|s| s.code());
            let msg = "收到停止信号，已终止脚本".to_string();
            diag(&msg);
            let _ = writeln!(stderr_for_errors, "[launchkeeper] {msg}");
            if let Err(e) =
                store.finish_run(run_id, finished_at, exit_code, Some(StopReason::Stopped))
            {
                diag(&format!("记录运行结束失败: {e}"));
            }
            0
        }
    }
}

/// How a run ended, from the runner's point of view.
enum Outcome {
    /// The script finished on its own.
    Exited(ExitStatus),
    /// `timeout_secs` expired and the script was killed.
    TimedOut,
    /// A stop signal arrived and the script was terminated. The status is the
    /// child's, `None` if it could not be collected.
    Stopped(Option<ExitStatus>),
}

/// Polls `child` every [`POLL_INTERVAL`], watching for three things at once:
/// the child exiting, the optional timeout expiring, and `stop` being raised
/// by the SIGTERM/SIGINT handler.
///
/// The two termination paths (timeout and stop) share the same kill sequence
/// — SIGTERM, then SIGKILL after [`KILL_GRACE`] — and differ only in which
/// [`Outcome`] they report. There is no blocking `wait` branch even when
/// there is no timeout, because the signal flag has to keep being checked.
fn wait_for_child(
    child: &mut Child,
    timeout_secs: Option<u32>,
    stop: &AtomicBool,
) -> std::io::Result<Outcome> {
    let deadline = timeout_secs.map(|s| Instant::now() + Duration::from_secs(u64::from(s)));
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Outcome::Exited(status));
        }
        if stop.load(Ordering::Relaxed) {
            return Ok(Outcome::Stopped(terminate(child)?));
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            terminate(child)?;
            return Ok(Outcome::TimedOut);
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// SIGTERM, then SIGKILL after [`KILL_GRACE`] if the child is still there.
/// Returns the child's exit status once it is reaped.
fn terminate(child: &mut Child) -> std::io::Result<Option<ExitStatus>> {
    signal(child.id(), "TERM");
    let grace = Instant::now() + KILL_GRACE;
    while Instant::now() < grace {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    signal(child.id(), "KILL");
    Ok(child.wait().ok())
}

/// `/bin/kill -<sig> <pid>`, best effort.
fn signal(pid: u32, sig: &str) {
    let result = Command::new("/bin/kill")
        .arg(format!("-{sig}"))
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if let Err(e) = result {
        diag(&format!("发送 SIG{sig} 给 pid {pid} 失败: {e}"));
    }
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}
