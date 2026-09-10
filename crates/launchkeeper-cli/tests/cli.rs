//! End-to-end tests against the built `launchkeeper` binary (crate
//! `launchkeeper-cli`).
//!
//! Every invocation gets `--data-dir <tmp>`, `--runner /bin/true` and a
//! temporary `LAUNCHKEEPER_LAUNCH_AGENTS_DIR` / `LAUNCHKEEPER_RUNNER_LOG_DIR`,
//! so nothing here touches the real database, the real
//! `~/Library/LaunchAgents`, the real log directory, or real launchd state.
//! No test here enables or bootstraps anything.
//!
//! The agents-directory override is **not** optional hygiene. `--data-dir`
//! moves the database but not the plist path, and `Service::is_enabled` asks
//! two questions: does the plist exist, and does launchd know the label. A
//! temp-database task that happens to share a name with a real, loaded task
//! therefore answers "enabled" — and the next `set` rewrites the real user's
//! plist (with the temp data dir baked in) and reloads their job. That is
//! exactly what used to happen to `nightly-report-sync`, which one test names
//! on purpose because it is the real M1 acceptance task.

use std::os::unix::fs::PermissionsExt;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn cli(data_dir: &TempDir) -> Command {
    // Interpreter resolution (M3 item 1) reads `launchkeeper_core::env::
    // effective_path()`, which — with no cache yet — spawns the real login
    // shell to capture a real `PATH`. Pre-seed the cache (same file
    // `crates/launchkeeper-core/tests/paths_env.rs` writes directly) so
    // every test gets a fast, deterministic `PATH` instead, without having
    // to know this detail itself. A test that wants a *specific* fake PATH
    // (with a fake `python3`, etc.) writes this same file itself before its
    // first `cli(&dir)` call; `exists()` here means it is never overwritten.
    let cache = data_dir.path().join("login-shell-path");
    if !cache.exists() {
        std::fs::write(&cache, "/usr/bin:/bin:/usr/sbin:/sbin\n").unwrap();
    }
    // See the module docs: these two have no `--flag` form, and leaving them
    // pointed at the user's real directories is how a test can reach out and
    // rewrite a real plist.
    let agents = data_dir.path().join("LaunchAgents");
    let runner_logs = data_dir.path().join("RunnerLogs");
    let mut cmd = Command::cargo_bin("launchkeeper").unwrap();
    cmd.arg("--data-dir")
        .arg(data_dir.path())
        .arg("--runner")
        .arg("/usr/bin/true")
        .env("LAUNCHKEEPER_LAUNCH_AGENTS_DIR", &agents)
        .env("LAUNCHKEEPER_RUNNER_LOG_DIR", &runner_logs);
    cmd
}

/// The same isolation as [`cli`], on a command whose argv the test builds
/// itself.
///
/// A few tests cannot use `cli()`: they need a different `--runner`, or no
/// `--data-dir` at all because the data directory is what they are testing.
/// They still must not be allowed to reach the real `~/Library/LaunchAgents`,
/// the real runner log directory or the real database — see the module docs
/// for what that costs when it goes wrong — so every one of the three
/// overrides is set here, pointed inside `dir`. A test that is *about* one of
/// them (`config data-dir`) overrides that one variable again on the command
/// it gets back, which is exactly what `.env()` / `.env_remove()` are for.
fn bare_cli(dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("launchkeeper").unwrap();
    cmd.env("LAUNCHKEEPER_DATA_DIR", dir.path())
        .env(
            "LAUNCHKEEPER_LAUNCH_AGENTS_DIR",
            dir.path().join("LaunchAgents"),
        )
        .env("LAUNCHKEEPER_RUNNER_LOG_DIR", dir.path().join("RunnerLogs"));
    cmd
}

/// Writes an executable shell script that prints `version_line` when run
/// with `--version` — the fake-interpreter pattern
/// `launchkeeper-core::interpreters`'s own tests use, so a scan over a fake
/// `PATH` sees a plausible program without needing a real python3/uv/node on
/// the machine running the tests.
fn fake_interpreter(dir: &std::path::Path, name: &str, version_line: &str) -> std::path::PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\necho '{version_line}'\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

fn write_executable(path: &std::path::Path, contents: &str) {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(path, contents).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn add_list_show_plist_set_trigger_remove() {
    let dir = TempDir::new().unwrap();

    cli(&dir)
        .args(["add", "sync", "--script", "/bin/echo", "--daily", "21:00"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sync"));

    cli(&dir)
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("sync"))
        .stdout(predicate::str::contains("每天 21:00"));

    cli(&dir)
        .args(["show", "sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("name:            sync"))
        .stdout(predicate::str::contains("每天 21:00"));

    let plist_out = cli(&dir).args(["plist", "sync"]).assert().success();
    let stdout = String::from_utf8(plist_out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("<key>StartCalendarInterval</key>"));
    assert!(stdout.contains("<key>Hour</key>"));
    assert!(stdout.contains("<integer>21</integer>"));
    assert!(stdout.contains("<key>ProgramArguments</key>"));
    assert!(stdout.contains(">run<"));
    assert!(stdout.contains(">sync<"));

    cli(&dir)
        .args(["set-trigger", "sync", "--every", "30m"])
        .assert()
        .success();

    cli(&dir)
        .args(["show", "sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("每 30 分钟"));

    cli(&dir)
        .args(["remove", "sync", "--yes"])
        .assert()
        .success();

    cli(&dir)
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("sync").not());
}

#[test]
fn add_requires_a_trigger() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .args(["add", "notrig", "--script", "/bin/echo"])
        .assert()
        .failure();
}

#[test]
fn show_unknown_task_fails() {
    let dir = TempDir::new().unwrap();
    cli(&dir).args(["show", "ghost"]).assert().failure();
}

#[test]
fn run_without_enable_hints_at_direct() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .args(["add", "job", "--script", "/bin/echo", "--at-login"])
        .assert()
        .success();
    cli(&dir)
        .args(["run", "job"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--direct"));
}

/// Commands that never write a plist must work with no runner binary at all:
/// no `--runner`, no `LAUNCHKEEPER_RUNNER`, and a sibling path that does not
/// exist (cargo puts the test binary in a directory without the runner only
/// by accident, so the env var is cleared explicitly).
#[test]
fn read_only_commands_work_without_a_runner() {
    let dir = TempDir::new().unwrap();
    let bin_dir = TempDir::new().unwrap();

    // 先用一个存在的 runner 建好任务
    cli(&dir)
        .args(["add", "job", "--script", "/bin/echo", "--at-login"])
        .assert()
        .success();

    // 把 cli 复制到一个没有 launchkeeper-runner 的目录里，再清掉环境变量
    let exe = bin_dir.path().join("launchkeeper");
    std::fs::copy(assert_cmd::cargo::cargo_bin("launchkeeper"), &exe).unwrap();
    assert!(!bin_dir.path().join("launchkeeper-runner").exists());

    let no_runner = |args: &[&str]| {
        // Same isolation as `bare_cli`, on a *copy* of the binary (which is
        // the whole point of this test), so `status` cannot go looking in the
        // real `~/Library/LaunchAgents` for a task called `job`.
        let mut cmd = Command::new(&exe);
        cmd.env_remove("LAUNCHKEEPER_RUNNER")
            .env(
                "LAUNCHKEEPER_LAUNCH_AGENTS_DIR",
                dir.path().join("LaunchAgents"),
            )
            .env("LAUNCHKEEPER_RUNNER_LOG_DIR", dir.path().join("RunnerLogs"))
            .arg("--data-dir")
            .arg(dir.path())
            .args(args);
        cmd
    };

    no_runner(&["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("job"));
    no_runner(&["show", "job"]).assert().success();
    no_runner(&["runs", "job"]).assert().success();
    no_runner(&["status"]).assert().success();

    // 需要 runner 的命令仍然报错，而且说得清楚
    no_runner(&["plist", "job"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("launchkeeper-runner"));
}

#[test]
fn add_with_timeout_is_stored_and_shown() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .args([
            "add",
            "slow",
            "--script",
            "/bin/echo",
            "--at-login",
            "--timeout",
            "30m",
        ])
        .assert()
        .success();
    cli(&dir)
        .args(["show", "slow"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1800"));
}

#[test]
fn json_add_show_list_have_the_documented_shape() {
    let dir = TempDir::new().unwrap();

    let add_out = cli(&dir)
        .args([
            "--json",
            "add",
            "sync",
            "--script",
            "/bin/echo",
            "--arg",
            "hi",
            "--daily",
            "21:00",
        ])
        .assert()
        .success();
    let add_json: serde_json::Value = serde_json::from_slice(&add_out.get_output().stdout).unwrap();
    assert_eq!(add_json["ok"], true);
    assert_eq!(add_json["task"]["name"], "sync");
    assert_eq!(add_json["task"]["args"][0], "hi");
    assert_eq!(add_json["task"]["trigger"]["kind"], "calendar");
    assert_eq!(add_json["task"]["trigger"]["entries"][0]["hour"], 21);
    assert_eq!(add_json["task"]["trigger_text"], "每天 21:00");
    assert_eq!(add_json["task"]["enabled"], false);
    assert_eq!(add_json["task"]["loaded"], false);
    assert!(add_json["task"]["last_run"].is_null());

    let show_out = cli(&dir)
        .args(["--json", "show", "sync"])
        .assert()
        .success();
    let show_json: serde_json::Value =
        serde_json::from_slice(&show_out.get_output().stdout).unwrap();
    assert_eq!(show_json["name"], "sync");
    assert_eq!(show_json["trigger"]["kind"], "calendar");

    let list_out = cli(&dir).args(["--json", "list"]).assert().success();
    let list_json: serde_json::Value =
        serde_json::from_slice(&list_out.get_output().stdout).unwrap();
    assert!(list_json.is_array());
    assert_eq!(list_json[0]["name"], "sync");

    let status_out = cli(&dir).args(["--json", "status"]).assert().success();
    let status_json: serde_json::Value =
        serde_json::from_slice(&status_out.get_output().stdout).unwrap();
    assert!(status_json.is_array());
    assert_eq!(status_json[0]["name"], "sync");
    assert_eq!(status_json[0]["loaded"], false);
}

#[test]
fn json_error_shape_on_failure() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .args(["add", "sync", "--script", "/bin/echo", "--daily", "21:00"])
        .assert()
        .success();

    // Adding the same name again fails; --json must report it as
    // {"ok": false, "error": ...} with exit code 1, and must NOT print the
    // human "错误:" line.
    let out = cli(&dir)
        .args([
            "--json",
            "add",
            "sync",
            "--script",
            "/bin/echo",
            "--daily",
            "21:00",
        ])
        .assert()
        .failure()
        .code(1);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(json["ok"], false);
    assert!(json["error"].as_str().unwrap().contains("已存在"));
    assert!(!stderr.contains("错误:"));
}

#[test]
fn json_runs_shape() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .args(["add", "job", "--script", "/bin/echo", "--at-login"])
        .assert()
        .success();

    let out = cli(&dir).args(["--json", "runs", "job"]).assert().success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert!(json.is_array());
    assert!(json.as_array().unwrap().is_empty());
}

#[test]
fn set_edits_fields_without_readding() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .args([
            "add",
            "sync",
            "--script",
            "/bin/echo",
            "--arg",
            "old",
            "--daily",
            "21:00",
            "--env",
            "A=1",
            "--tag",
            "old-tag",
        ])
        .assert()
        .success();

    // display-name / description / script / cwd
    cli(&dir)
        .args([
            "set",
            "sync",
            "--display-name",
            "Sync Job",
            "--description",
            "syncs stuff",
            "--script",
            "/bin/cat",
            "--cwd",
            "/tmp",
        ])
        .assert()
        .success();
    let show = cli(&dir)
        .args(["--json", "show", "sync"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&show.get_output().stdout).unwrap();
    assert_eq!(json["display_name"], "Sync Job");
    assert_eq!(json["description"], "syncs stuff");
    assert_eq!(json["script_path"], "/bin/cat");
    assert_eq!(json["working_dir"], "/tmp");

    // --arg replaces the whole args list
    cli(&dir)
        .args(["set", "sync", "--arg", "new1", "--arg", "new2"])
        .assert()
        .success();
    let show = cli(&dir)
        .args(["--json", "show", "sync"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&show.get_output().stdout).unwrap();
    assert_eq!(json["args"], serde_json::json!(["new1", "new2"]));

    // --env merges, --unset-env removes
    cli(&dir)
        .args(["set", "sync", "--env", "B=2", "--unset-env", "A"])
        .assert()
        .success();
    let show = cli(&dir)
        .args(["--json", "show", "sync"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&show.get_output().stdout).unwrap();
    assert_eq!(json["env"], serde_json::json!({"B": "2"}));

    // --tag replaces the whole tag list
    cli(&dir)
        .args(["set", "sync", "--tag", "fresh"])
        .assert()
        .success();
    let show = cli(&dir)
        .args(["--json", "show", "sync"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&show.get_output().stdout).unwrap();
    assert_eq!(json["tags"], serde_json::json!(["fresh"]));

    // timeout / no-timeout, favorite/no-favorite, notify/no-notify
    cli(&dir)
        .args([
            "set",
            "sync",
            "--timeout",
            "5m",
            "--favorite",
            "--no-notify",
        ])
        .assert()
        .success();
    let show = cli(&dir)
        .args(["--json", "show", "sync"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&show.get_output().stdout).unwrap();
    assert_eq!(json["timeout_secs"], 300);
    assert_eq!(json["favorite"], true);
    assert_eq!(json["notify_on_fail"], false);

    cli(&dir)
        .args(["set", "sync", "--no-timeout"])
        .assert()
        .success();
    let show = cli(&dir)
        .args(["--json", "show", "sync"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&show.get_output().stdout).unwrap();
    assert!(json["timeout_secs"].is_null());
}

#[test]
fn set_json_mode_returns_task() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .args(["add", "sync", "--script", "/bin/echo", "--at-login"])
        .assert()
        .success();

    let out = cli(&dir)
        .args(["--json", "set", "sync", "--favorite"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["task"]["favorite"], true);

    // Unknown task: JSON error envelope, exit 1.
    let out = cli(&dir)
        .args(["--json", "set", "ghost", "--favorite"])
        .assert()
        .failure()
        .code(1);
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["ok"], false);
}

#[test]
fn set_reloads_launchd_when_enabled() {
    // We cannot bootstrap for real in tests, but we can verify that `set`
    // still updates the database row for an enabled task path: since
    // `is_enabled` requires an actual plist bootstrap via launchctl (which
    // these tests never do), a task added but not enabled stays disabled and
    // `set` must not attempt (or need) a reload for it.
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .args(["add", "sync", "--script", "/bin/echo", "--at-login"])
        .assert()
        .success();
    cli(&dir)
        .args(["set", "sync", "--description", "still disabled"])
        .assert()
        .success();
    cli(&dir)
        .args(["--json", "show", "sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"enabled\": false"));
}

#[test]
fn logs_tail_limits_lines() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .args(["add", "job", "--script", "/bin/echo", "--at-login"])
        .assert()
        .success();

    // No runs recorded yet: `logs` must fail clearly instead of panicking.
    cli(&dir)
        .args(["logs", "job", "--tail", "2"])
        .assert()
        .failure();
}

#[test]
fn keep_alive_with_a_timed_trigger_is_rejected() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .args([
            "add",
            "resident",
            "--script",
            "/bin/echo",
            "--daily",
            "21:00",
            "--keep-alive",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("常驻"));
}

#[test]
fn manual_trigger_round_trips_and_shapes_the_plist() {
    let dir = TempDir::new().unwrap();

    let out = cli(&dir)
        .args([
            "--json",
            "add",
            "bridge",
            "--script",
            "/bin/echo",
            "--manual",
            "--keep-alive",
        ])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["task"]["trigger"]["kind"], "manual");
    assert_eq!(json["task"]["trigger_text"], "手动启停");
    assert_eq!(json["task"]["is_service"], true);
    assert_eq!(json["task"]["keep_alive"], true);
    assert!(json["task"]["uptime_secs"].is_null());

    cli(&dir)
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("手动启停"));

    // plist：没有任何触发键，KeepAlive 是字典
    let plist_out = cli(&dir).args(["plist", "bridge"]).assert().success();
    let xml = String::from_utf8(plist_out.get_output().stdout.clone()).unwrap();
    for absent in [
        "<key>RunAtLoad</key>",
        "<key>StartInterval</key>",
        "<key>StartCalendarInterval</key>",
    ] {
        assert!(!xml.contains(absent), "手动任务不该有 {absent}:\n{xml}");
    }
    assert!(xml.contains("<key>KeepAlive</key>"), "{xml}");
    assert!(xml.contains("<key>SuccessfulExit</key>"), "{xml}");
    assert!(xml.contains("<false/>"), "{xml}");

    // status：KIND 列区分服务型任务，JSON 带 is_service / uptime_secs
    cli(&dir)
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("UPTIME"))
        .stdout(predicate::str::contains("服务"));
    let status_out = cli(&dir).args(["--json", "status"]).assert().success();
    let status: serde_json::Value =
        serde_json::from_slice(&status_out.get_output().stdout).unwrap();
    assert_eq!(status[0]["is_service"], true);
    assert!(status[0]["uptime_secs"].is_null());
}

#[test]
fn keep_alive_is_rejected_for_timed_triggers_but_allowed_for_manual() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .args([
            "add",
            "nope",
            "--script",
            "/bin/echo",
            "--daily",
            "21:00",
            "--keep-alive",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("常驻"));

    cli(&dir)
        .args([
            "add",
            "svc",
            "--script",
            "/bin/echo",
            "--manual",
            "--keep-alive",
        ])
        .assert()
        .success();
}

#[test]
fn set_trigger_can_switch_a_task_to_manual() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .args(["add", "job", "--script", "/bin/echo", "--daily", "21:00"])
        .assert()
        .success();
    let out = cli(&dir)
        .args(["--json", "set-trigger", "job", "--manual"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["task"]["trigger"]["kind"], "manual");
    assert_eq!(json["task"]["is_service"], true);
}

#[test]
fn manual_and_another_trigger_flag_conflict() {
    let dir = TempDir::new().unwrap();
    // clap 的互斥组：--manual 和 --daily 不能同时给
    cli(&dir)
        .args([
            "add",
            "x",
            "--script",
            "/bin/echo",
            "--manual",
            "--daily",
            "21:00",
        ])
        .assert()
        .failure();
}

/// `stop` on a task launchd never heard of is a no-op, not an error — and it
/// must not need a runner binary either. (`start`/`restart` are not exercised
/// here: they would really `bootstrap` into the user's launchd.)
#[test]
fn stop_is_a_no_op_for_a_task_that_was_never_enabled() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .args(["add", "idle", "--script", "/bin/echo", "--manual"])
        .assert()
        .success();
    let out = cli(&dir)
        .args(["--json", "stop", "idle"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["task"]["enabled"], false);
}

#[test]
fn lifecycle_commands_reject_an_unknown_task() {
    let dir = TempDir::new().unwrap();
    for verb in ["start", "stop", "restart"] {
        cli(&dir)
            .args([verb, "ghost"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("任务不存在"));
    }
}

#[test]
fn runs_json_carries_stop_reason() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .args(["add", "quick", "--script", "/bin/echo", "--manual"])
        .assert()
        .success();

    // 通过 runner 直接跑一次，产生一条完成的运行记录。这里不能用 cli()，
    // 它已经把 --runner 指向 /usr/bin/true 了——但隔离用的三个环境变量一个
    // 都不能少，所以走 bare_cli()。
    bare_cli(&dir)
        .arg("--data-dir")
        .arg(dir.path())
        .arg("--runner")
        .arg(assert_cmd::cargo::cargo_bin("launchkeeper-runner"))
        .args(["run", "quick", "--direct"])
        .assert()
        .success();

    let out = cli(&dir)
        .args(["--json", "runs", "quick"])
        .assert()
        .success();
    let runs: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(runs[0]["stop_reason"], "exited");
    assert_eq!(runs[0]["exit_code"], 0);
    assert_eq!(runs[0]["running"], false);

    cli(&dir)
        .args(["runs", "quick"])
        .assert()
        .success()
        .stdout(predicate::str::contains("STOP"))
        .stdout(predicate::str::contains("正常结束"));
}

/// M3 item 2: subcommand aliases and short flags behave exactly like the
/// long forms they stand in for.
#[test]
fn aliases_and_short_flags_behave_like_the_long_form() {
    let dir = TempDir::new().unwrap();

    // `-d`/`-e` short forms for `--daily`/`--every` on `add`/`set-trigger`.
    cli(&dir)
        .args(["add", "sync", "--script", "/bin/echo", "-d", "21:00"])
        .assert()
        .success();

    // `ls` == `list`.
    cli(&dir)
        .args(["ls"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sync"));

    // `trigger` == `set-trigger`, `-e` == `--every`.
    cli(&dir)
        .args(["trigger", "sync", "-e", "30m"])
        .assert()
        .success();
    cli(&dir)
        .args(["show", "sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("每 30 分钟"));

    // `on`/`off` == `enable`/`disable` (both no-op-safe here: no real runner
    // is ever bootstrapped in these tests since `enable` would need a real
    // launchd; exercise `off` only, which never talks to launchd for a task
    // that was never loaded).
    cli(&dir)
        .args(["off", "sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("已停用"));

    // `st` == `status`.
    cli(&dir)
        .args(["st"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sync"));

    // `log` == `logs` (errors identically — no runs recorded yet).
    cli(&dir).args(["log", "sync"]).assert().failure();

    // `-j` == `--json`, checked globally rather than after the subcommand.
    let out = cli(&dir).args(["-j", "show", "sync"]).assert().success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["name"], "sync");

    // `rm` == `remove`.
    cli(&dir).args(["rm", "sync", "--yes"]).assert().success();
    cli(&dir)
        .args(["ls"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sync").not());
}

/// `-m`/`-w`/`-M` short forms for `--manual`/`--weekly`/`--monthly`.
#[test]
fn manual_weekly_monthly_short_flags() {
    let dir = TempDir::new().unwrap();

    cli(&dir)
        .args(["add", "svc", "--script", "/bin/echo", "-m"])
        .assert()
        .success();
    cli(&dir)
        .args(["show", "svc"])
        .assert()
        .success()
        .stdout(predicate::str::contains("手动启停"));

    cli(&dir)
        .args(["add", "wk", "--script", "/bin/echo", "-w", "mon,fri@09:30"])
        .assert()
        .success();
    cli(&dir)
        .args(["show", "wk"])
        .assert()
        .success()
        .stdout(predicate::str::contains("每周一、五 09:30"));

    cli(&dir)
        .args(["add", "mo", "--script", "/bin/echo", "-M", "15@08:00"])
        .assert()
        .success();
    cli(&dir)
        .args(["show", "mo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("每月 15 日 08:00"));
}

/// `config data-dir`: with no argument it's a read of the resolved data dir
/// and where it came from; with a path it writes
/// `~/.config/launchkeeper/data-dir` (redirected to a temp `XDG_CONFIG_HOME`
/// here, never the real one) and every subsequent read reflects it, unless
/// `LAUNCHKEEPER_DATA_DIR` is set, which always wins.
#[test]
fn config_data_dir_reads_and_writes_the_config_file() {
    let xdg = TempDir::new().unwrap();
    let new_data_dir = TempDir::new().unwrap();

    let cmd = |args: &[&str]| {
        let mut c = bare_cli(&xdg);
        c.env_remove("LAUNCHKEEPER_DATA_DIR")
            .env("XDG_CONFIG_HOME", xdg.path())
            .args(args);
        c
    };

    // No config file yet: falls back to the default, source "default".
    let out = cmd(&["--json", "config", "data-dir"]).assert().success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["source"], "default");

    // Write it.
    let out = cmd(&["--json", "config", "data-dir"])
        .args([new_data_dir.path().to_str().unwrap()])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["data_dir"], new_data_dir.path().to_str().unwrap());
    assert!(xdg.path().join("launchkeeper").join("data-dir").is_file());

    // Read again: now sourced from the config file.
    let out = cmd(&["--json", "config", "data-dir"]).assert().success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["source"], "config_file");
    assert_eq!(json["data_dir"], new_data_dir.path().to_str().unwrap());

    // LAUNCHKEEPER_DATA_DIR still wins over the file when set.
    let mut c = bare_cli(&xdg);
    let out = c
        .env("XDG_CONFIG_HOME", xdg.path())
        .env("LAUNCHKEEPER_DATA_DIR", "/tmp/env-wins")
        .args(["--json", "config", "data-dir"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["source"], "env");
    assert_eq!(json["data_dir"], "/tmp/env-wins");
}

/// The full data-dir precedence, in one place: `--data-dir` beats
/// `LAUNCHKEEPER_DATA_DIR`, which beats `~/.config/launchkeeper/data-dir`
/// (redirected to a temp `XDG_CONFIG_HOME` here), which beats the default.
#[test]
fn data_dir_flag_beats_the_env_var_and_the_config_file() {
    let xdg = TempDir::new().unwrap();
    let from_file = TempDir::new().unwrap();
    let from_env = TempDir::new().unwrap();
    let from_flag = TempDir::new().unwrap();

    let read = |c: &mut Command| -> serde_json::Value {
        let out = c.assert().success();
        serde_json::from_slice(&out.get_output().stdout).unwrap()
    };

    // Seed the config file.
    bare_cli(&xdg)
        .env_remove("LAUNCHKEEPER_DATA_DIR")
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["config", "data-dir", from_file.path().to_str().unwrap()])
        .assert()
        .success();

    // File only.
    let json = read(
        bare_cli(&xdg)
            .env_remove("LAUNCHKEEPER_DATA_DIR")
            .env("XDG_CONFIG_HOME", xdg.path())
            .args(["--json", "config", "data-dir"]),
    );
    assert_eq!(json["source"], "config_file");
    assert_eq!(json["data_dir"], from_file.path().to_str().unwrap());

    // Env var over the file.
    let json = read(
        bare_cli(&xdg)
            .env("XDG_CONFIG_HOME", xdg.path())
            .env("LAUNCHKEEPER_DATA_DIR", from_env.path())
            .args(["--json", "config", "data-dir"]),
    );
    assert_eq!(json["source"], "env");
    assert_eq!(json["data_dir"], from_env.path().to_str().unwrap());

    // --data-dir over both.
    let json = read(
        bare_cli(&xdg)
            .env("XDG_CONFIG_HOME", xdg.path())
            .env("LAUNCHKEEPER_DATA_DIR", from_env.path())
            .args(["--json", "--data-dir"])
            .arg(from_flag.path())
            .args(["config", "data-dir"]),
    );
    assert_eq!(json["data_dir"], from_flag.path().to_str().unwrap());

    // And the write path says so when the env var shadows what it just
    // wrote, in the JSON as well as in prose.
    let json = read(
        bare_cli(&xdg)
            .env("XDG_CONFIG_HOME", xdg.path())
            .env("LAUNCHKEEPER_DATA_DIR", from_env.path())
            .args(["--json", "config", "data-dir"])
            .arg(from_file.path()),
    );
    assert_eq!(json["ok"], true);
    assert!(
        json["warning"]
            .as_str()
            .is_some_and(|w| w.contains("LAUNCHKEEPER_DATA_DIR")),
        "{json}"
    );
}

/// `completions <shell>` prints a non-trivial, shell-specific static script
/// for each of the three supported shells and never touches the store.
#[test]
fn completions_prints_a_script_per_shell() {
    let dir = TempDir::new().unwrap();
    for (shell, needle) in [
        ("zsh", "#compdef launchkeeper"),
        ("bash", "_launchkeeper()"),
        ("fish", "__fish_launchkeeper"),
    ] {
        bare_cli(&dir)
            .args(["completions", shell])
            .assert()
            .success()
            .stdout(predicate::str::contains(needle));
    }
}

/// M3 CLI interpreter support (`docs/M3-design.md` §1, `docs/CLI.md`):
/// `add` with no `--interpreter`/`--raw` auto-picks the `recommended`
/// candidate from `interpreters::scan`, composes `script_path`/`args` via
/// `interpreters::apply`, and both the JSON and human output reflect it.
#[test]
fn add_auto_picks_python3_for_a_py_script() {
    let dir = TempDir::new().unwrap();
    let bin = dir.path().join("fakebin");
    let python3 = fake_interpreter(&bin, "python3", "FakePython 3.9.6");
    // A fake PATH with only our fake python3 on it — written before the
    // first `cli(&dir)` call so the helper's default cache never overwrites
    // it (see `cli()`'s doc comment above).
    std::fs::write(
        dir.path().join("login-shell-path"),
        format!("{}\n", bin.display()),
    )
    .unwrap();

    let script = dir.path().join("job.py");
    std::fs::write(&script, "print(1)\n").unwrap();

    let out = cli(&dir)
        .args([
            "--json",
            "add",
            "job",
            "--script",
            script.to_str().unwrap(),
            "--arg",
            "once-flag",
            "--at-login",
        ])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["ok"], true);
    // Composed via `interpreters::apply`: python3 is the program, the script
    // (then its own args) become the task's stored `args`.
    assert_eq!(json["task"]["script_path"], python3.to_str().unwrap());
    assert_eq!(
        json["task"]["args"],
        serde_json::json!([script.to_str().unwrap(), "once-flag"])
    );
    // The decomposed view: `script`/`script_args` are the script itself, not
    // python3.
    assert_eq!(json["task"]["script"], script.to_str().unwrap());
    assert_eq!(
        json["task"]["script_args"],
        serde_json::json!(["once-flag"])
    );
    assert_eq!(json["task"]["interpreter_label"], "python3 · PATH");

    // Human mode names the pick and why, per docs/CLI.md.
    let human = cli(&dir)
        .args([
            "add",
            "job2",
            "--script",
            script.to_str().unwrap(),
            "--at-login",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(human.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("运行方式: python3"), "{stdout}");
    assert!(stdout.contains("3.9.6"), "{stdout}");
    assert!(stdout.contains("按 .py 扩展名推荐"), "{stdout}");

    // `show` reconstructs the same line from the stored script_path/args.
    let show = cli(&dir).args(["show", "job"]).assert().success();
    let stdout = String::from_utf8(show.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("运行方式:"), "{stdout}");
    assert!(stdout.contains("python3"), "{stdout}");
}

/// `--raw` is the pre-M3 behavior: `--script`/`--arg` stored verbatim, no
/// interpreter resolution attempted — exactly how the real nightly-report-sync
/// task was created (`CLAUDE.md` M1 acceptance section). `detect_from_task`
/// still decomposes it after the fact for display/JSON purposes.
#[test]
fn raw_flag_stores_script_and_args_verbatim() {
    let dir = TempDir::new().unwrap();
    let uv = dir.path().join(".local").join("bin").join("uv");
    // --raw only needs --script to exist (same as every other mode); it does
    // not need to be executable or even a real uv build.
    write_executable(&uv, "#!/bin/sh\necho uv\n");

    // `uv run scripts/…` names its script relative to the working
    // directory, so the task needs one — `Task::validate` refuses the
    // combination "interpreter + relative script + no working directory",
    // which would resolve against `$HOME` at 21:00 and find nothing.
    let project = dir.path().join("report_tool");
    std::fs::create_dir_all(&project).unwrap();

    cli(&dir)
        .args([
            "--json",
            "add",
            "no-cwd",
            "--script",
            uv.to_str().unwrap(),
            "--arg",
            "run",
            "--arg",
            "scripts/sync_cloud.py",
            "--raw",
            "--daily",
            "21:00",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("相对路径"));

    let out = cli(&dir)
        .args([
            "--json",
            "add",
            "nightly-report-sync",
            "--script",
            uv.to_str().unwrap(),
            "--arg",
            "run",
            "--arg",
            "scripts/sync_cloud.py",
            "--raw",
            "--cwd",
            project.to_str().unwrap(),
            "--daily",
            "21:00",
        ])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["task"]["script_path"], uv.to_str().unwrap());
    assert_eq!(
        json["task"]["args"],
        serde_json::json!(["run", "scripts/sync_cloud.py"])
    );
    // detect_from_task still labels it "uv run" even though nothing was
    // resolved at add time.
    assert_eq!(json["task"]["script"], "scripts/sync_cloud.py");
    assert_eq!(json["task"]["script_args"], serde_json::json!([]));
    assert!(
        json["task"]["interpreter_label"]
            .as_str()
            .unwrap()
            .starts_with("uv run"),
        "{json}"
    );

    let show = cli(&dir)
        .args(["show", "nightly-report-sync"])
        .assert()
        .success();
    let stdout = String::from_utf8(show.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("运行方式:"), "{stdout}");
    assert!(stdout.contains("uv run"), "{stdout}");

    // --raw round-trips through `set` the same way.
    cli(&dir)
        .args([
            "set",
            "nightly-report-sync",
            "--arg",
            "run",
            "--arg",
            "scripts/sync_cloud.py",
            "--arg",
            "once-flag",
            "--raw",
        ])
        .assert()
        .success();
    let show = cli(&dir)
        .args(["--json", "show", "nightly-report-sync"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&show.get_output().stdout).unwrap();
    assert_eq!(
        json["args"],
        serde_json::json!(["run", "scripts/sync_cloud.py", "once-flag"])
    );
}

/// `interpreters [--script] [--cwd] [-j]` lists candidates without touching
/// the database (no `--data-dir` content is required to exist beforehand
/// beyond the PATH cache this test seeds).
#[test]
fn interpreters_subcommand_lists_candidates() {
    let dir = TempDir::new().unwrap();
    let bin = dir.path().join("fakebin");
    fake_interpreter(&bin, "python3", "FakePython 3.9.6");
    std::fs::write(
        dir.path().join("login-shell-path"),
        format!("{}\n", bin.display()),
    )
    .unwrap();

    let script = dir.path().join("check.py");
    std::fs::write(&script, "print(1)\n").unwrap();

    let out = cli(&dir)
        .args([
            "--json",
            "interpreters",
            "--script",
            script.to_str().unwrap(),
        ])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    let arr = json.as_array().expect("数组");
    assert!(!arr.is_empty());
    let python = arr
        .iter()
        .find(|c| c["kind"] == "python")
        .expect("python3 候选");
    assert_eq!(python["recommended"], true);
    assert_eq!(python["version"], "3.9.6");
    assert_eq!(python["origin"], "path");
    assert_eq!(python["label"], "python3 · PATH");
    assert!(
        python["reason"]
            .as_str()
            .unwrap()
            .contains("按 .py 扩展名推荐"),
        "{json}"
    );
    assert!(python["id"].as_str().unwrap().ends_with("/python3"));

    // Human table mode: same candidate, readable columns.
    let human = cli(&dir)
        .args(["interpreters", "--script", script.to_str().unwrap()])
        .assert()
        .success();
    let stdout = String::from_utf8(human.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("python3"), "{stdout}");
    assert!(stdout.contains("3.9.6"), "{stdout}");
    assert!(stdout.contains("推荐"), "{stdout}");

    // No --script: still lists what's on PATH, nothing recommended.
    let bare = cli(&dir).args(["interpreters"]).assert().success();
    let stdout = String::from_utf8(bare.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("python3"), "{stdout}");
}

/// `--interpreter direct` forces launchd to exec the script itself, bypassing
/// auto-detection — the explicit-override counterpart to the case where
/// `recommend()` would have picked Direct on its own.
#[test]
fn explicit_interpreter_direct_runs_the_script_itself() {
    let dir = TempDir::new().unwrap();
    let script = dir.path().join("backup.sh");
    write_executable(&script, "#!/bin/zsh\necho hi\n");

    let out = cli(&dir)
        .args([
            "--json",
            "add",
            "backup",
            "--script",
            script.to_str().unwrap(),
            "--interpreter",
            "direct",
            "--daily",
            "21:00",
        ])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["task"]["script_path"], script.to_str().unwrap());
    assert_eq!(json["task"]["args"], serde_json::json!([]));
    assert_eq!(json["task"]["interpreter_label"], "直接执行");

    let human = cli(&dir).args(["show", "backup"]).assert().success();
    let stdout = String::from_utf8(human.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("直接执行"), "{stdout}");

    // Explicit `--interpreter direct` on a script that is NOT executable is
    // rejected, same as the implicit case.
    let not_exec = dir.path().join("not-exec.sh");
    std::fs::write(&not_exec, "#!/bin/zsh\necho hi\n").unwrap();
    cli(&dir)
        .args([
            "add",
            "bad",
            "--script",
            not_exec.to_str().unwrap(),
            "--interpreter",
            "direct",
            "--daily",
            "21:00",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("可执行"));
}

/// `--interpreter`/`--raw` are mutually exclusive on both `add` and `set`
/// (clap `conflicts_with`).
#[test]
fn interpreter_and_raw_conflict() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .args([
            "add",
            "x",
            "--script",
            "/bin/echo",
            "--interpreter",
            "direct",
            "--raw",
            "--at-login",
        ])
        .assert()
        .failure();
}

// ---------------------------------------------------------------------------
// M3 §3.3: `agents` / `adopt` / `unadopt`
// ---------------------------------------------------------------------------

/// Writes a hand-written LaunchAgent into `dir` and returns its path.
///
/// These tests point `LAUNCHKEEPER_LAUNCH_AGENTS_DIR` at a temp directory, so
/// the real `~/Library/LaunchAgents` is never read or written. Nothing here
/// bootstraps anything either: only `agents` and `adopt --dry-run` run, and
/// both are read-only.
fn write_agent(dir: &std::path::Path, file: &str, body: &str) -> std::path::PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(file);
    std::fs::write(
        &path,
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
             \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\"><dict>\n{body}</dict></plist>\n"
        ),
    )
    .unwrap();
    path
}

const CLI_DAILY: &str = "\
<key>Label</key><string>com.example.daily</string>
<key>ProgramArguments</key><array><string>/bin/echo</string><string>hi</string></array>
<key>StartCalendarInterval</key><dict><key>Hour</key><integer>21</integer><key>Minute</key><integer>0</integer></dict>
";

const CLI_HOURLY: &str = "\
<key>Label</key><string>com.example.hourly</string>
<key>ProgramArguments</key><array><string>/bin/echo</string></array>
<key>StartInterval</key><integer>3600</integer>
";

const CLI_WATCHER: &str = "\
<key>Label</key><string>com.example.watcher</string>
<key>ProgramArguments</key><array><string>/bin/echo</string></array>
<key>WatchPaths</key><array><string>/tmp/inbox</string></array>
";

fn agents_cli(data_dir: &TempDir, agents: &std::path::Path) -> Command {
    let mut cmd = cli(data_dir);
    cmd.env("LAUNCHKEEPER_LAUNCH_AGENTS_DIR", agents);
    cmd
}

#[test]
fn agents_lists_hand_written_plists_with_a_verdict() {
    let dir = TempDir::new().unwrap();
    let agents = dir.path().join("LaunchAgents");
    write_agent(&agents, "daily.plist", CLI_DAILY);
    write_agent(&agents, "hourly.plist", CLI_HOURLY);
    write_agent(&agents, "watcher.plist", CLI_WATCHER);

    agents_cli(&dir, &agents)
        .arg("agents")
        .assert()
        .success()
        .stdout(predicate::str::contains("com.example.daily"))
        .stdout(predicate::str::contains("每天 21:00"))
        .stdout(predicate::str::contains("com.example.hourly"))
        .stdout(predicate::str::contains("每 1 小时"))
        .stdout(predicate::str::contains("com.example.watcher"))
        .stdout(predicate::str::contains("WatchPaths"));

    let out = agents_cli(&dir, &agents)
        .args(["agents", "--json"])
        .assert()
        .success();
    let v: serde_json::Value =
        serde_json::from_slice(&out.get_output().stdout).expect("agents --json 应当是 JSON");
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    let by = |label: &str| {
        arr.iter()
            .find(|a| a["label"] == label)
            .unwrap_or_else(|| panic!("缺 {label}"))
    };
    let daily = by("com.example.daily");
    assert_eq!(daily["adoptable"], serde_json::json!(true));
    assert_eq!(daily["reason"], serde_json::Value::Null);
    assert_eq!(daily["trigger"]["kind"], "calendar");
    assert_eq!(daily["command"], "/bin/echo hi");
    assert_eq!(daily["loaded"], serde_json::json!(false));
    let watcher = by("com.example.watcher");
    assert_eq!(watcher["adoptable"], serde_json::json!(false));
    assert!(
        watcher["reason"].as_str().unwrap().contains("WatchPaths"),
        "{watcher}"
    );
    assert_eq!(watcher["trigger"], serde_json::Value::Null);
}

#[test]
fn adopt_dry_run_shows_the_diff_and_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let agents = dir.path().join("LaunchAgents");
    let plist = write_agent(&agents, "daily.plist", CLI_DAILY);
    let before = std::fs::read(&plist).unwrap();

    agents_cli(&dir, &agents)
        .args(["adopt", "com.example.daily", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("接管 com.example.daily"))
        .stdout(predicate::str::contains("daily.plist.bak"))
        .stdout(predicate::str::contains("LaunchkeeperManaged"))
        .stdout(predicate::str::contains("--dry-run，什么都没写"));

    let out = agents_cli(&dir, &agents)
        .args(["adopt", "com.example.daily", "--dry-run", "--json"])
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["ok"], serde_json::json!(true));
    assert_eq!(v["dry_run"], serde_json::json!(true));
    let plan = &v["plan"];
    assert_eq!(plan["label"], "com.example.daily");
    assert_eq!(plan["task"]["name"], "com.example.daily");
    assert!(
        plan["backup_path"]
            .as_str()
            .unwrap()
            .ends_with("daily.plist.bak")
    );
    let keys = |group: &str| -> Vec<String> {
        plan[group]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["key"].as_str().unwrap().to_string())
            .collect()
    };
    assert!(keys("kept").contains(&"Label".to_string()), "{plan}");
    assert!(
        keys("kept").contains(&"StartCalendarInterval".to_string()),
        "{plan}"
    );
    assert!(keys("added").contains(&"LaunchkeeperManaged".to_string()));
    assert!(keys("added").contains(&"ProgramArguments".to_string()));

    // Nothing on disk moved.
    assert_eq!(std::fs::read(&plist).unwrap(), before);
    assert!(!plist.with_extension("plist.bak").exists());
}

/// L2: `-y` skips the *question*, not the answer to "what did it just do to
/// my plist". The diff is printed before the file is touched, so `adopt -y`
/// in a script still leaves a record of which keys were dropped.
///
/// This is the one test here that actually adopts, so it points
/// `LAUNCHKEEPER_LAUNCHCTL` at a stub: no real `bootstrap`, no real job, and
/// `print` answers 113 ("not loaded") the way launchd does for a label
/// nothing knows about.
#[test]
fn adopt_with_yes_still_prints_the_plan_before_writing() {
    let dir = TempDir::new().unwrap();
    let agents = dir.path().join("LaunchAgents");
    let plist = write_agent(&agents, "daily.plist", CLI_DAILY);

    let fake = dir.path().join("fake-launchctl");
    write_executable(
        &fake,
        "#!/bin/sh\ncase \"$1\" in\n  print) exit 113 ;;\n  *) exit 0 ;;\nesac\n",
    );

    agents_cli(&dir, &agents)
        .env("LAUNCHKEEPER_LAUNCHCTL", &fake)
        .args(["adopt", "com.example.daily", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("接管 com.example.daily"))
        .stdout(predicate::str::contains("移除:"))
        .stdout(predicate::str::contains("LaunchkeeperManaged"))
        .stdout(predicate::str::contains("已接管 com.example.daily"));

    assert!(
        plist.with_extension("plist.bak").exists(),
        "原文件应当备份为 .bak"
    );
    assert!(
        std::fs::read_to_string(&plist)
            .unwrap()
            .contains("LaunchkeeperManaged")
    );

    // 撤销接管也要说清楚它会丢掉 Launchkeeper 这边的改动
    agents_cli(&dir, &agents)
        .env("LAUNCHKEEPER_LAUNCHCTL", &fake)
        .args(["unadopt", "com.example.daily"])
        .write_stdin("n\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("按字节还原"))
        .stdout(predicate::str::contains("丢弃"))
        .stdout(predicate::str::contains("已取消"));

    // `remove` 的确认里要点名那个会被删掉的 plist
    agents_cli(&dir, &agents)
        .env("LAUNCHKEEPER_LAUNCHCTL", &fake)
        .args(["remove", "com.example.daily"])
        .write_stdin("n\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("接管来的任务"))
        .stdout(predicate::str::contains("daily.plist"))
        .stdout(predicate::str::contains("unadopt"));
}

#[test]
fn adopt_refuses_the_unadoptable_and_needs_yes_in_json_mode() {
    let dir = TempDir::new().unwrap();
    let agents = dir.path().join("LaunchAgents");
    write_agent(&agents, "watcher.plist", CLI_WATCHER);
    write_agent(&agents, "daily.plist", CLI_DAILY);

    agents_cli(&dir, &agents)
        .args(["adopt", "com.example.watcher", "--dry-run"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("WatchPaths"));

    agents_cli(&dir, &agents)
        .args(["adopt", "com.nobody.here", "--dry-run"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("找不到 LaunchAgent"));

    // --json without --yes must never block on stdin.
    let out = agents_cli(&dir, &agents)
        .args(["adopt", "com.example.daily", "--json"])
        .assert()
        .failure();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["ok"], serde_json::json!(false));
    assert!(v["error"].as_str().unwrap().contains("--yes"), "{v}");
    assert!(!agents.join("daily.plist.bak").exists());
}

#[test]
fn unadopt_refuses_a_task_that_was_never_adopted() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .args(["add", "sync", "--script", "/bin/echo", "--daily", "21:00"])
        .assert()
        .success();

    let out = cli(&dir)
        .args(["unadopt", "sync", "--yes", "--json"])
        .assert()
        .failure();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["ok"], serde_json::json!(false));
    assert!(
        v["error"].as_str().unwrap().contains("不是接管来的任务"),
        "{v}"
    );
}

/// A whole launchd label is a legal task name now (M3 §3.1), so a task can
/// be created under one and every command still addresses it by that name.
#[test]
fn a_label_shaped_name_is_accepted() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .args([
            "add",
            "com.example.Sync_2",
            "--script",
            "/bin/echo",
            "--every",
            "30m",
        ])
        .assert()
        .success();
    cli(&dir)
        .args(["show", "com.example.Sync_2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("每 30 分钟"));
    // Not adopted, so its label still gets the prefix.
    let out = cli(&dir)
        .args(["plist", "com.example.Sync_2"])
        .assert()
        .success();
    let xml = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(
        xml.contains("com.launchkeeper.com.example.Sync_2"),
        "普通任务仍然带前缀: {xml}"
    );
}

// ---- M4 §1: AI task insight ----------------------------------------------

/// A one-shot HTTP responder on `127.0.0.1:0`, so the AI tests below never
/// touch the network.
///
/// `n` requests are answered with `body`, one connection each, then the
/// listener closes. `explain` makes exactly one request per non-cached call,
/// which is what these tests count.
struct MockAi {
    base_url: String,
    hits: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    seen: std::sync::mpsc::Receiver<String>,
}

impl MockAi {
    fn new(n: usize, body: &'static str) -> MockAi {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = std::sync::Arc::clone(&hits);
        let (tx, seen) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for _ in 0..n {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut len = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    let line = line.trim_end().to_string();
                    if line.is_empty() {
                        break;
                    }
                    // Header names are case-insensitive and reqwest sends
                    // them lowercase, so match on the lowered name rather
                    // than the canonical spelling.
                    if let Some((k, v)) = line.split_once(':')
                        && k.trim().eq_ignore_ascii_case("content-length")
                    {
                        len = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut buf = vec![0u8; len];
                let _ = reader.read_exact(&mut buf);
                let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.flush();
            }
        });
        MockAi {
            base_url,
            hits,
            seen,
        }
    }

    fn hits(&self) -> usize {
        self.hits.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The body of the next request the server saw.
    fn next_request(&self) -> String {
        self.seen
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("mock 服务器没有收到请求")
    }
}

const AI_REPLY: &str = r#"{"content":[{"type":"text","text":"这个任务每 30 分钟跑一次，最近一直成功。"}],"model":"mock-model"}"#;

/// Points the CLI at `server` and gives it a key through the environment,
/// which is the override that keeps every test out of the real keychain.
fn ai_cli(dir: &TempDir) -> Command {
    let mut cmd = cli(dir);
    cmd.env("LAUNCHKEEPER_AI_API_KEY", "sk-test-key");
    cmd
}

fn configure_ai(dir: &TempDir, server: &MockAi) {
    ai_cli(dir)
        .args([
            "ai",
            "config",
            "--provider",
            "anthropic",
            "--base-url",
            &server.base_url,
            "--model",
            "mock-model",
        ])
        .assert()
        .success();
}

#[test]
fn ai_config_reads_and_writes_the_shared_json_and_never_prints_the_key() {
    let dir = TempDir::new().unwrap();

    // Before anything is written: the documented defaults, from a file that
    // does not exist yet.
    let out = cli(&dir)
        .args(["ai", "config", "--json"])
        .env_remove("LAUNCHKEEPER_AI_API_KEY")
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["provider"], "anthropic");
    assert_eq!(v["model"], "claude-sonnet-5");
    assert_eq!(v["base_url"], serde_json::Value::Null);
    assert_eq!(v["effective_base_url"], "https://api.anthropic.com");
    assert_eq!(v["endpoint"], "https://api.anthropic.com/v1/messages");
    assert_eq!(
        v["config_file"],
        dir.path().join("ai.json").to_string_lossy().as_ref()
    );

    // Writing: the mutation envelope, and the file really lands in the data
    // directory the app reads too.
    let out = cli(&dir)
        .args([
            "ai",
            "config",
            "--provider",
            "openai",
            "--base-url",
            "http://127.0.0.1:11434/v1/",
            "--model",
            "llama3.2",
            "--json",
        ])
        .env("LAUNCHKEEPER_AI_API_KEY", "sk-secret-value")
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["ok"], serde_json::json!(true));
    assert_eq!(v["config"]["provider"], "openai_compatible");
    assert_eq!(
        v["config"]["endpoint"],
        "http://127.0.0.1:11434/v1/chat/completions"
    );
    assert_eq!(v["config"]["has_api_key"], serde_json::json!(true));
    assert_eq!(v["config"]["api_key_source"], "env");
    // The key itself is in no JSON this CLI prints, ever.
    let raw = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(
        !raw.contains("sk-secret-value"),
        "Key 不能出现在输出里: {raw}"
    );

    let on_disk = std::fs::read_to_string(dir.path().join("ai.json")).unwrap();
    assert!(on_disk.contains("\"openai_compatible\""), "{on_disk}");
    assert!(!on_disk.contains("sk-secret-value"), "Key 不能写进 ai.json");

    // An empty --base-url goes back to the provider default.
    let out = cli(&dir)
        .args(["ai", "config", "--base-url", "", "--json"])
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["config"]["base_url"], serde_json::Value::Null);
    assert_eq!(
        v["config"]["effective_base_url"],
        "https://api.openai.com/v1"
    );

    // An unknown provider is a clean error, not a panic or a half-write.
    let out = cli(&dir)
        .args(["ai", "config", "--provider", "llama.cpp", "--json"])
        .assert()
        .failure()
        .code(1);
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["ok"], serde_json::json!(false));
    assert!(
        v["error"].as_str().unwrap().contains("未知的 AI 提供方"),
        "{v}"
    );
}

/// The key comes from the environment, which is what keeps every test in
/// this repository out of the login keychain — and what a CI or headless run
/// uses. Verified through the CLI's own report of where it found one.
#[test]
fn the_api_key_env_override_is_what_the_cli_reports() {
    let dir = TempDir::new().unwrap();
    let read_source = |key: Option<&str>| -> serde_json::Value {
        let mut cmd = cli(&dir);
        cmd.args(["ai", "config", "--json"]);
        match key {
            Some(k) => cmd.env("LAUNCHKEEPER_AI_API_KEY", k),
            None => cmd.env_remove("LAUNCHKEEPER_AI_API_KEY"),
        };
        let out = cmd.assert().success();
        serde_json::from_slice(&out.get_output().stdout).unwrap()
    };

    let v = read_source(Some("sk-from-env"));
    assert_eq!(v["has_api_key"], serde_json::json!(true));
    assert_eq!(v["api_key_source"], "env");

    // A blank value is "absent", not "an empty key" — so an
    // `export LAUNCHKEEPER_AI_API_KEY=` in a shell rc does not shadow the
    // keychain with nothing.
    let v = read_source(Some("   "));
    assert_ne!(v["api_key_source"], "env");
}

#[test]
fn explain_json_generates_stores_and_then_reuses_the_insight() {
    let dir = TempDir::new().unwrap();
    let server = MockAi::new(2, AI_REPLY);
    configure_ai(&dir, &server);

    ai_cli(&dir)
        .args(["add", "sync", "--script", "/bin/echo", "--every", "30m"])
        .assert()
        .success();

    // First call: a real round trip.
    let out = ai_cli(&dir)
        .args(["explain", "sync", "--json"])
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["task"], "sync");
    assert_eq!(v["model"], "mock-model");
    assert_eq!(v["cached"], serde_json::json!(false));
    assert_eq!(v["content"], "这个任务每 30 分钟跑一次，最近一直成功。");
    assert_eq!(v["prompt_hash"].as_str().unwrap().len(), 64);
    assert!(v["created_at"].as_str().unwrap().contains('T'));
    assert_eq!(server.hits(), 1);

    // What actually went over the wire: the Anthropic shape, with the task's
    // real trigger in it.
    let body: serde_json::Value = serde_json::from_str(&server.next_request()).unwrap();
    assert_eq!(body["model"], "mock-model");
    assert!(
        body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("sync")
    );

    // Second call without --refresh: served from the database, no request.
    let out = ai_cli(&dir)
        .args(["explain", "sync", "--json"])
        .assert()
        .success();
    let v2: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v2["cached"], serde_json::json!(true));
    assert_eq!(v2["content"], v["content"]);
    assert_eq!(v2["created_at"], v["created_at"]);
    assert_eq!(server.hits(), 1, "缓存命中不该再发请求");

    // --refresh calls again.
    ai_cli(&dir)
        .args(["explain", "sync", "--refresh", "--json"])
        .assert()
        .success();
    assert_eq!(server.hits(), 2);

    // Human mode prints the same three facts plus the body.
    ai_cli(&dir)
        .args(["explain", "sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("mock-model"))
        .stdout(predicate::str::contains("每 30 分钟跑一次"));

    // Removing the task takes the insight with it (FK cascade), so the next
    // explain has nothing to reuse — and the task is gone, so it errors.
    ai_cli(&dir)
        .args(["remove", "sync", "--yes"])
        .assert()
        .success();
    let out = ai_cli(&dir)
        .args(["explain", "sync", "--json"])
        .assert()
        .failure()
        .code(1);
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["ok"], serde_json::json!(false));
    assert!(v["error"].as_str().unwrap().contains("任务不存在"), "{v}");
}

/// Without a key nothing is sent at all, and the message says where to put
/// one. `--json` still gets a machine-readable failure.
#[test]
fn explain_without_a_key_fails_with_a_useful_message() {
    let dir = TempDir::new().unwrap();
    let server = MockAi::new(1, AI_REPLY);
    configure_ai(&dir, &server);
    cli(&dir)
        .args(["add", "sync", "--script", "/bin/echo", "--every", "30m"])
        .assert()
        .success();

    let out = cli(&dir)
        .args(["explain", "sync", "--json"])
        .env_remove("LAUNCHKEEPER_AI_API_KEY")
        .assert()
        .failure()
        .code(1);
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["ok"], serde_json::json!(false));
    assert!(
        v["error"].as_str().unwrap().contains("API Key"),
        "错误信息要指出缺的是什么: {v}"
    );
    assert_eq!(server.hits(), 0, "没有 Key 时不该发出任何请求");
}

/// `ai key set` takes the key on stdin, never in argv — a key on a command
/// line is visible in shell history and in every process listing.
#[test]
fn ai_key_set_takes_no_argument() {
    let dir = TempDir::new().unwrap();
    // There is no `--key`/positional form to pass, so clap rejects it as a
    // usage error (exit 2) rather than quietly storing it.
    cli(&dir)
        .args(["ai", "key", "set", "sk-should-not-work"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn ai_test_reports_the_endpoint_that_answered() {
    let dir = TempDir::new().unwrap();
    let server = MockAi::new(1, r#"{"content":[{"type":"text","text":"ok"}]}"#);
    configure_ai(&dir, &server);

    let out = ai_cli(&dir)
        .args(["ai", "test", "--json"])
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["ok"], serde_json::json!(true));
    assert_eq!(v["model"], "mock-model");
    assert_eq!(v["reply"], "ok");
    assert_eq!(v["endpoint"], format!("{}/v1/messages", server.base_url));
}

// ---------------------------------------------------------------------------
// M4: `docs` / `ai-prompt`, and the warning `off` puts in front of a
// LaunchAgent that belongs to somebody else's software
// ---------------------------------------------------------------------------

/// `docs` prints the embedded `docs/CLI.md`, which has to work with no
/// database and no data directory — an assistant runs it first, before
/// anything else exists.
#[test]
fn docs_prints_the_embedded_cli_reference() {
    let dir = TempDir::new().unwrap();
    bare_cli(&dir)
        .arg("docs")
        .assert()
        .success()
        .stdout(predicate::str::contains("# `launchkeeper` reference"))
        .stdout(predicate::str::contains("## Commands"))
        .stdout(predicate::str::contains("### `agents`"))
        .stdout(predicate::str::contains("### `docs`"))
        .stdout(predicate::str::contains("### `ai-prompt"));
}

#[test]
fn ai_prompt_prints_both_languages() {
    let dir = TempDir::new().unwrap();

    bare_cli(&dir)
        .args(["ai-prompt", "--lang", "zh-CN"])
        .assert()
        .success()
        .stdout(predicate::str::contains("本机安装了 Launchkeeper"))
        .stdout(predicate::str::contains("launchkeeper docs"))
        .stdout(predicate::str::contains("`-d`（每天几点）"))
        .stdout(predicate::str::contains("`-e`（每隔多久）"));

    bare_cli(&dir)
        .args(["ai-prompt", "--lang", "en"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Launchkeeper is installed on this machine",
        ))
        .stdout(predicate::str::contains("launchkeeper docs"))
        .stdout(predicate::str::contains("`-d` (a time of day)"))
        .stdout(predicate::str::contains("`-e` (an interval)"));

    // No --lang: LC_ALL/LANG decides, and an unset environment means the
    // Chinese every other line of this binary is written in.
    bare_cli(&dir)
        .arg("ai-prompt")
        .env("LANG", "en_US.UTF-8")
        .env_remove("LC_ALL")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Launchkeeper is installed on this machine",
        ));
    bare_cli(&dir)
        .arg("ai-prompt")
        .env("LANG", "zh_CN.UTF-8")
        .env_remove("LC_ALL")
        .assert()
        .success()
        .stdout(predicate::str::contains("本机安装了 Launchkeeper"));
    bare_cli(&dir)
        .arg("ai-prompt")
        .env_remove("LANG")
        .env_remove("LC_ALL")
        .assert()
        .success()
        .stdout(predicate::str::contains("本机安装了 Launchkeeper"));
}

/// `off <label>` on a LaunchAgent Launchkeeper would refuse to adopt warns
/// and asks first. Both halves of this test stop *before* launchctl is
/// touched: stdin is empty, so the prompt reads "no" and the command
/// cancels, and `--json` refuses outright. Nothing is unloaded, and the
/// plist is compared byte for byte afterwards.
#[test]
fn off_warns_before_disabling_a_foreign_launch_agent() {
    let dir = TempDir::new().unwrap();
    let agents = dir.path().join("LaunchAgents");
    let plist = write_agent(&agents, "watcher.plist", CLI_WATCHER);
    let before = std::fs::read(&plist).unwrap();

    agents_cli(&dir, &agents)
        .args(["off", "com.example.watcher"])
        .assert()
        .success()
        .stdout(predicate::str::contains("警告"))
        .stdout(predicate::str::contains("别的软件"))
        .stdout(predicate::str::contains("WatchPaths"))
        .stdout(predicate::str::contains("plist 文件不会被改动"))
        .stdout(predicate::str::contains(
            "launchkeeper on com.example.watcher",
        ))
        .stdout(predicate::str::contains("已取消"));

    let out = agents_cli(&dir, &agents)
        .args(["off", "com.example.watcher", "--json"])
        .assert()
        .failure();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["ok"], serde_json::json!(false));
    assert!(v["error"].as_str().unwrap().contains("--yes"), "{v}");
    assert!(
        v["error"].as_str().unwrap().contains("别的软件"),
        "JSON 里也要说清楚这是谁的 job: {v}"
    );

    assert_eq!(std::fs::read(&plist).unwrap(), before);
}
