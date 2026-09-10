//! End-to-end tests for `launchkeeper-runner`, run as a real subprocess.
//!
//! Each test uses its own temp directory and passes `LAUNCHKEEPER_DATA_DIR`
//! only to the *child* process (via `Command::env`), never to the test
//! process itself, so tests can run in parallel without racing on a
//! process-wide environment variable. The test process opens the `Store`
//! directly against `<tmp>/launchkeeper.db`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use launchkeeper_core::{StopReason, Store, Task, TaskName, Trigger, TriggerKind};

fn runner_bin() -> &'static str {
    env!("CARGO_BIN_EXE_launchkeeper-runner")
}

/// Writes an executable shell script at `path`.
fn write_script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

fn open_store(tmp: &Path) -> Store {
    Store::open(&tmp.join("launchkeeper.db")).unwrap()
}

fn run_runner(tmp: &Path, name: &str, manual: bool) -> std::process::Output {
    let mut cmd = Command::new(runner_bin());
    cmd.arg("run").arg(name);
    if manual {
        cmd.arg("--manual");
    }
    cmd.env("LAUNCHKEEPER_DATA_DIR", tmp);
    cmd.output().expect("spawn runner")
}

fn task_name(s: &str) -> TaskName {
    TaskName::new(s).unwrap()
}

/// Spawns the runner without waiting for it, the way launchd does.
fn spawn_runner(tmp: &Path, name: &str) -> std::process::Child {
    Command::new(runner_bin())
        .arg("run")
        .arg(name)
        .arg("--manual")
        .env("LAUNCHKEEPER_DATA_DIR", tmp)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn runner")
}

/// `/bin/kill -<sig> <pid>`. `/bin/kill` rather than `libc`, so the tests do
/// not need an unsafe block or a new dependency.
fn kill(pid: u32, sig: &str) {
    let status = Command::new("/bin/kill")
        .arg(format!("-{sig}"))
        .arg(pid.to_string())
        .status()
        .expect("run /bin/kill");
    assert!(status.success(), "kill -{sig} {pid} 失败");
}

/// True while `pid` still exists (`kill -0`). Output is discarded: probing a
/// dead pid is the expected answer here, not something to print.
fn alive(pid: u32) -> bool {
    Command::new("/bin/kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("run /bin/kill -0")
        .success()
}

/// Polls `f` every 50 ms for at most `secs`, returning what it first yields.
fn wait_for<T>(secs: u64, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        if let Some(v) = f() {
            return Some(v);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Sets up a long-running task, starts the runner and waits until the child
/// is genuinely up. Returns (runner, child pid).
///
/// "Up" is two things, and both matter: the runner has recorded the child's
/// pid, *and* the script itself has reached `touch "$LK_READY"`. Signalling
/// on the pid alone is a race — the shell may not have installed its `trap`
/// or produced any output yet, and the test would then be measuring how fast
/// `/bin/sh` starts rather than what the runner does.
fn start_long_running(
    tmp: &Path,
    store: &Store,
    name: &str,
    body: &str,
) -> (std::process::Child, u32) {
    let script = tmp.join(format!("{name}.sh"));
    write_script(&script, body);
    let ready = tmp.join(format!("{name}.ready"));

    let mut task = Task::new(task_name(name), &script, Trigger::Manual);
    task.env
        .insert("LK_READY".into(), ready.to_string_lossy().into_owned());
    store.insert_task(&task).unwrap();

    let runner = spawn_runner(tmp, name);
    let pid = wait_for(10, || store.last_run(&task_name(name)).ok().flatten()?.pid)
        .expect("runner 应当在 10 秒内记录子进程 pid");
    assert!(
        wait_for(10, || ready.exists().then_some(())).is_some(),
        "脚本应当在 10 秒内就绪（{}）",
        ready.display()
    );
    (runner, pid)
}

#[test]
fn sigterm_stops_the_child_and_records_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let tmp = dir.path();
    let store = open_store(tmp);

    // 不 trap TERM：子进程被信号杀死，exit_code 记为 None
    let (mut runner, child_pid) = start_long_running(
        tmp,
        &store,
        "service",
        "echo up\ntouch \"$LK_READY\"\nsleep 300\necho never",
    );

    kill(runner.id(), "TERM");

    let status = wait_for(15, || runner.try_wait().ok().flatten())
        .expect("runner 应当在收到 SIGTERM 后很快退出");
    assert_eq!(
        status.code(),
        Some(0),
        "runner 必须以 0 退出，否则 KeepAlive={{SuccessfulExit=false}} 会把它重新拉起来"
    );

    assert!(
        wait_for(5, || (!alive(child_pid)).then_some(())).is_some(),
        "子进程 {child_pid} 应当已经被终止"
    );

    let run = store.last_run(&task_name("service")).unwrap().unwrap();
    assert!(run.finished_at.is_some(), "停止也要写 finished_at");
    assert_eq!(run.stop_reason, Some(StopReason::Stopped));
    assert_eq!(run.exit_code, None, "被信号杀死的子进程没有退出码");
    assert!(!run.succeeded());

    let out = fs::read_to_string(&run.stdout_path).unwrap();
    assert!(out.contains("up"), "out: {out}");
    assert!(!out.contains("never"), "脚本不该跑完: {out}");
}

#[test]
fn a_child_that_handles_sigterm_keeps_its_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let tmp = dir.path();
    let store = open_store(tmp);

    let (mut runner, child_pid) = start_long_running(
        tmp,
        &store,
        "graceful",
        "trap 'echo bye; exit 7' TERM\necho up\ntouch \"$LK_READY\"\nwhile true; do sleep 1; done",
    );

    kill(runner.id(), "TERM");

    let status = wait_for(15, || runner.try_wait().ok().flatten()).expect("runner 应当退出");
    assert_eq!(status.code(), Some(0), "runner 自己仍然以 0 退出");
    assert!(
        wait_for(5, || (!alive(child_pid)).then_some(())).is_some(),
        "子进程应当已经退出"
    );

    let run = store.last_run(&task_name("graceful")).unwrap().unwrap();
    assert_eq!(run.stop_reason, Some(StopReason::Stopped));
    assert_eq!(run.exit_code, Some(7), "脚本自己处理了 TERM，退出码要留下");
    let out = fs::read_to_string(&run.stdout_path).unwrap();
    assert!(out.contains("bye"), "脚本应当有机会清理: {out}");
}

#[test]
fn sigint_stops_the_run_too() {
    let dir = tempfile::tempdir().unwrap();
    let tmp = dir.path();
    let store = open_store(tmp);

    let (mut runner, child_pid) = start_long_running(
        tmp,
        &store,
        "interruptible",
        "touch \"$LK_READY\"\nsleep 300",
    );

    kill(runner.id(), "INT");

    let status = wait_for(15, || runner.try_wait().ok().flatten()).expect("runner 应当退出");
    assert_eq!(status.code(), Some(0));
    assert!(wait_for(5, || (!alive(child_pid)).then_some(())).is_some());

    let run = store
        .last_run(&task_name("interruptible"))
        .unwrap()
        .unwrap();
    assert_eq!(run.stop_reason, Some(StopReason::Stopped));
}

#[test]
fn happy_path_manual_run_records_everything() {
    let dir = tempfile::tempdir().unwrap();
    let tmp = dir.path();
    let store = open_store(tmp);

    let script = tmp.join("script.sh");
    write_script(
        &script,
        r#"
echo "hello from stdout"
echo "custom env: $MY_CUSTOM_VAR"
echo "task: $LAUNCHKEEPER_TASK"
echo "path is $PATH" >&2
echo "oops on stderr" >&2
exit 0
"#,
    );

    let mut task = Task::new(task_name("happy"), &script, Trigger::AtLogin);
    task.env.insert("MY_CUSTOM_VAR".into(), "abc123".into());
    store.insert_task(&task).unwrap();

    let output = run_runner(tmp, "happy", true);
    assert_eq!(output.status.code(), Some(0), "stderr: {:?}", output.stderr);

    let run = store.last_run(&task_name("happy")).unwrap().unwrap();
    assert!(run.finished_at.is_some());
    assert_eq!(run.exit_code, Some(0));
    assert!(run.pid.is_some());
    assert_eq!(run.trigger_kind, TriggerKind::Manual);
    assert_eq!(run.stop_reason, Some(StopReason::Exited));

    let out = fs::read_to_string(&run.stdout_path).unwrap();
    assert!(out.contains("hello from stdout"), "out: {out}");
    assert!(out.contains("custom env: abc123"), "out: {out}");
    assert!(out.contains("task: happy"), "out: {out}");

    let err = fs::read_to_string(&run.stderr_path).unwrap();
    assert!(err.contains("oops on stderr"), "err: {err}");
    assert!(err.contains("path is"), "err: {err}");
}

#[test]
fn nonzero_exit_is_propagated_and_scheduled_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let tmp = dir.path();
    let store = open_store(tmp);

    let script = tmp.join("script.sh");
    write_script(&script, "exit 3");

    let task = Task::new(task_name("fails"), &script, Trigger::AtLogin);
    store.insert_task(&task).unwrap();

    let output = run_runner(tmp, "fails", false);
    assert_eq!(output.status.code(), Some(3));

    let run = store.last_run(&task_name("fails")).unwrap().unwrap();
    assert_eq!(run.exit_code, Some(3));
    assert_eq!(run.trigger_kind, TriggerKind::Scheduled);
}

#[test]
fn missing_script_exits_127_and_records_error() {
    let dir = tempfile::tempdir().unwrap();
    let tmp = dir.path();
    let store = open_store(tmp);

    let missing = tmp.join("does-not-exist.sh");
    let task = Task::new(task_name("ghost-script"), &missing, Trigger::AtLogin);
    store.insert_task(&task).unwrap();

    let output = run_runner(tmp, "ghost-script", true);
    assert_eq!(output.status.code(), Some(127));

    let run = store.last_run(&task_name("ghost-script")).unwrap().unwrap();
    assert_eq!(run.exit_code, Some(127));
    let err = fs::read_to_string(&run.stderr_path).unwrap();
    assert!(!err.trim().is_empty(), "expected an error message in .err");
}

#[test]
fn unknown_task_exits_78_with_no_run_row() {
    let dir = tempfile::tempdir().unwrap();
    let tmp = dir.path();
    let store = open_store(tmp);

    let output = run_runner(tmp, "no-such-task", false);
    assert_eq!(output.status.code(), Some(78));

    assert!(
        store
            .last_run(&task_name("no-such-task"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn prune_keeps_only_default_keep_runs() {
    let dir = tempfile::tempdir().unwrap();
    let tmp = dir.path();
    let store = open_store(tmp);

    let script = tmp.join("script.sh");
    write_script(&script, "exit 0");
    let task = Task::new(task_name("prunable"), &script, Trigger::AtLogin);
    store.insert_task(&task).unwrap();

    // Pre-insert 55 finished runs with real (empty) log files.
    let log_dir = tmp.join("logs").join("prunable");
    fs::create_dir_all(&log_dir).unwrap();
    let base = chrono::Utc::now() - chrono::Duration::days(1);
    for i in 0..55 {
        let started = base + chrono::Duration::seconds(i);
        let out = log_dir.join(format!("pre-{i}.out"));
        let err = log_dir.join(format!("pre-{i}.err"));
        fs::write(&out, "").unwrap();
        fs::write(&err, "").unwrap();
        let id = store
            .start_run(
                &task_name("prunable"),
                TriggerKind::Scheduled,
                started,
                &out,
                &err,
            )
            .unwrap();
        store
            .finish_run(id, started, Some(0), Some(StopReason::Exited))
            .unwrap();
    }
    assert_eq!(
        store.list_runs(&task_name("prunable"), 1000).unwrap().len(),
        55
    );

    let output = run_runner(tmp, "prunable", true);
    assert_eq!(output.status.code(), Some(0), "stderr: {:?}", output.stderr);

    let remaining = store.list_runs(&task_name("prunable"), 1000).unwrap();
    assert_eq!(remaining.len(), 50);

    // The 6 oldest pre-inserted runs (55 + 1 new - 50 kept = 6 pruned) should
    // have had their log files removed.
    let mut still_there = 0;
    for i in 0..55 {
        if log_dir.join(format!("pre-{i}.out")).exists() {
            still_there += 1;
        }
    }
    assert_eq!(still_there, 55 - 6);
}

#[test]
fn working_dir_is_honored() {
    let dir = tempfile::tempdir().unwrap();
    let tmp = dir.path();
    let store = open_store(tmp);

    let cwd_target = tmp.join("workdir");
    fs::create_dir_all(&cwd_target).unwrap();

    let script = tmp.join("script.sh");
    write_script(&script, "pwd");

    let mut task = Task::new(task_name("cwd-check"), &script, Trigger::AtLogin);
    task.working_dir = Some(cwd_target.clone());
    store.insert_task(&task).unwrap();

    let output = run_runner(tmp, "cwd-check", true);
    assert_eq!(output.status.code(), Some(0));

    let run = store.last_run(&task_name("cwd-check")).unwrap().unwrap();
    let out = fs::read_to_string(&run.stdout_path).unwrap();
    let printed = PathBuf::from(out.trim());
    // Resolve symlinks (e.g. /tmp -> /private/tmp on macOS) before comparing.
    let expected = fs::canonicalize(&cwd_target).unwrap();
    let actual = fs::canonicalize(&printed).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn timeout_kills_the_script_and_records_124() {
    let dir = tempfile::tempdir().unwrap();
    let tmp = dir.path();
    let store = open_store(tmp);

    let script = tmp.join("slow.sh");
    write_script(&script, "echo started\nsleep 30\necho never");

    let mut task = Task::new(task_name("slow"), &script, Trigger::AtLogin);
    // 3 秒而不是 1 秒：机器忙的时候 /bin/sh 起来再 echo 都可能超过 1 秒，
    // 那样测的就是调度延迟而不是超时逻辑了。
    task.timeout_secs = Some(3);
    store.insert_task(&task).unwrap();

    let began = std::time::Instant::now();
    let out = run_runner(tmp, "slow", true);
    let elapsed = began.elapsed();

    assert_eq!(
        out.status.code(),
        Some(124),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "应当在超时后立刻返回，实际用了 {elapsed:?}"
    );

    let run = store.last_run(&task_name("slow")).unwrap().unwrap();
    assert_eq!(run.exit_code, Some(124));
    assert_eq!(run.stop_reason, Some(StopReason::Timeout));
    assert!(run.finished_at.is_some());

    let err = fs::read_to_string(&run.stderr_path).unwrap();
    assert!(err.contains("超时"), "stderr 日志里应记录超时: {err:?}");
    let stdout = fs::read_to_string(&run.stdout_path).unwrap();
    assert!(stdout.contains("started"));
    assert!(!stdout.contains("never"));
}

#[test]
fn no_timeout_lets_the_script_finish() {
    let dir = tempfile::tempdir().unwrap();
    let tmp = dir.path();
    let store = open_store(tmp);

    let script = tmp.join("quick.sh");
    write_script(&script, "sleep 1\necho done");

    let task = Task::new(task_name("quick"), &script, Trigger::AtLogin);
    assert_eq!(task.timeout_secs, None);
    store.insert_task(&task).unwrap();

    let out = run_runner(tmp, "quick", true);
    assert_eq!(out.status.code(), Some(0));
    let run = store.last_run(&task_name("quick")).unwrap().unwrap();
    assert_eq!(run.exit_code, Some(0));
    assert!(
        fs::read_to_string(&run.stdout_path)
            .unwrap()
            .contains("done")
    );
}
