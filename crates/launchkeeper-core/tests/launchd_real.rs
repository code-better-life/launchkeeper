//! One `#[ignore]`d test against the **real** launchd, in two phases, for the
//! things a fake `launchctl` cannot prove:
//!
//! 1. that a `Trigger::Manual` + `keep_alive` job started through
//!    [`Service::start`] really runs, that [`Service::stop`] really brings it
//!    down, and that launchd does *not* restart it afterwards (which is the
//!    whole point of having the runner exit 0 — see `docs/M2.5-design.md`
//!    §3.1);
//! 2. that adoption keeps a hand-written plist's label, produces real run
//!    rows, and can be undone byte for byte (M3 §3.5).
//!
//! The two phases are one `#[test]` on purpose. They both point the
//! process-wide `LAUNCHKEEPER_*` variables at their own temp directories, and
//! two `#[test]` functions in one binary run on two threads: as separate
//! tests they would overwrite each other's data directory mid-run, and the
//! loser would write its database and plists into the winner's temp dir (or,
//! once that `TempDir` was dropped, into nothing at all).
//!
//! Run it explicitly:
//!
//! ```console
//! $ cargo test -p launchkeeper-core -- --ignored
//! ```
//!
//! Safety rails, because this one does touch the machine's launchd:
//!
//! * the label is `com.launchkeeper.test.<uuid>`, never a real task name;
//! * the plist goes into a temp directory, not `~/Library/LaunchAgents`
//!   (`launchctl bootstrap` accepts any path), so the real directory is never
//!   written to;
//! * the data dir, the logs and the runner copy are all in a temp directory;
//! * the test boots the job out and deletes everything at the end, including
//!   on the failure paths that matter.
//!
//! The runner binary is *copied* into the temp data dir before use. On this
//! machine the checkout lives on an external volume, and a binary launchd
//! exec's from there blocks on a TCC "removable volume" prompt — copying it
//! onto the boot disk first keeps the test non-interactive.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use launchkeeper_core::{
    Service, StopReason, Store, Task, TaskName, Trigger, launchctl, paths, service,
};

/// Finds the built `launchkeeper-runner` next to this test binary
/// (`target/<profile>/deps/..` -> `target/<profile>/launchkeeper-runner`).
fn built_runner() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let mut dir = exe.parent().expect("test binary has a parent");
    for _ in 0..3 {
        let candidate = dir.join("launchkeeper-runner");
        if candidate.is_file() {
            return candidate;
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => break,
        }
    }
    panic!(
        "找不到 launchkeeper-runner，先跑一次 `cargo build --workspace`（从 {} 往上找过）",
        exe.display()
    );
}

fn write_script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

fn wait_for<T>(secs: u64, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f() {
            return Some(v);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn pid_of(svc: &Service, name: &TaskName) -> Option<u32> {
    let task = svc.store().get_task(name).ok().flatten()?;
    Service::enabled_status(&task).ok().flatten()?.pid
}

/// The whole file: both phases, in order, on one thread.
#[test]
#[ignore = "talks to the real launchd; run with --ignored"]
fn against_the_real_launchd() {
    a_manual_service_starts_stops_and_stays_stopped();
    adopting_a_hand_written_plist_keeps_its_label_and_can_be_undone();
}

fn a_manual_service_starts_stops_and_stays_stopped() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let agents = tmp.path().join("agents");
    fs::create_dir_all(&data).unwrap();
    fs::create_dir_all(&agents).unwrap();

    // 唯一的临时 label，绝不撞上真实任务
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let name = TaskName::new(&format!("test.{}", &suffix[..12])).unwrap();
    let label = name.label();

    // runner 复制到临时目录（内置盘），避开外置盘的 TCC 弹框
    let runner = data.join("launchkeeper-runner");
    fs::copy(built_runner(), &runner).unwrap();
    let mut perms = fs::metadata(&runner).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&runner, perms).unwrap();

    // SAFETY: this binary runs exactly one `#[test]`, which calls this phase
    // and the next one in sequence on the same thread — no other thread can
    // be reading these while they change. The next phase re-points all three
    // at its own temp directory before it touches anything.
    unsafe {
        std::env::set_var(paths::DATA_DIR_ENV, &data);
        std::env::set_var(paths::LAUNCH_AGENTS_DIR_ENV, &agents);
        std::env::set_var(paths::RUNNER_LOG_DIR_ENV, tmp.path().join("runner-logs"));
    }

    let script = tmp.path().join("service.sh");
    write_script(&script, "echo up $$\nwhile true; do sleep 1; done");

    let store = Store::open(&paths::db_path().unwrap()).unwrap();
    let svc = Service::new(store, runner);
    let mut task = Task::new(name.clone(), &script, Trigger::Manual);
    task.keep_alive = true;
    svc.add_task(task).unwrap();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        exercise(&svc, &name);
    }));

    // 无论成功失败都清干净
    let _ = svc.remove_task(&name);
    let _ = launchctl::bootout(&label);
    assert!(
        launchctl::status(&label).unwrap().is_none(),
        "测试结束后 {label} 不该还在 launchd 里"
    );

    if let Err(p) = result {
        std::panic::resume_unwind(p);
    }
}

fn exercise(svc: &Service, name: &TaskName) {
    // start：enable（bootstrap）+ kickstart
    svc.start(name).unwrap();
    let pid = wait_for(20, || pid_of(svc, name)).expect("start 之后 20 秒内应当有 pid");
    assert!(pid > 1);

    // runner 记下了 run，脚本还在跑
    let run = wait_for(20, || {
        svc.store()
            .last_run(name)
            .ok()
            .flatten()
            .filter(|r| r.pid.is_some())
    })
    .expect("应当有一条带 pid 的运行记录");
    assert!(run.finished_at.is_none(), "服务应当还在运行");

    // stop：SIGTERM -> runner 转发 -> runner 以 0 退出 -> launchd 不重启
    svc.stop(name).unwrap();
    assert!(
        wait_for(5, || pid_of(svc, name).is_none().then_some(())).is_some(),
        "stop 之后 launchd 不该再报 pid"
    );

    let finished = wait_for(15, || {
        svc.store()
            .last_run(name)
            .ok()
            .flatten()
            .filter(|r| r.finished_at.is_some())
    })
    .expect("停止后应当写上 finished_at");
    assert_eq!(finished.id, run.id, "应当是同一条运行记录");
    assert_eq!(finished.stop_reason, Some(StopReason::Stopped));

    // 关键断言：KeepAlive={SuccessfulExit=false} 不该把它重新拉起来。
    // 观察窗口比 stop 的宽限期长，比 ThrottleInterval(60s) 短就够了 ——
    // launchd 若认为这次退出"不成功"，重启会立刻发生，不会等满 60 秒。
    let watch_until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < watch_until {
        assert_eq!(
            pid_of(svc, name),
            None,
            "手动停止之后 launchd 不该重启服务（runner 应当以 0 退出）"
        );
        std::thread::sleep(Duration::from_millis(250));
    }

    // 还是 enabled 的，能再次 start
    assert!(svc.is_enabled(name).unwrap());
    svc.start(name).unwrap();
    assert!(
        wait_for(20, || pid_of(svc, name)).is_some(),
        "停止后应当还能再启动"
    );
    svc.stop(name).unwrap();
    // stop 的宽限期常量在这里也用一下，免得它悄悄变成 0
    assert!(service::STOP_GRACE >= Duration::from_secs(1));
}

// ---------------------------------------------------------------------------
// M3 §3.5: adopting a hand-written plist, against the real launchd
// ---------------------------------------------------------------------------

/// The one thing a fake `launchctl` cannot prove about adoption: that real
/// launchd keeps running the job under its **original label** after the
/// plist has been rewritten to go through the runner, that a kickstart then
/// produces a real run row, and that `unadopt` puts the user's bytes back and
/// leaves launchd holding the original job again.
///
/// Same safety rails as the test above: the label is `com.lktest.<uuid>`, the
/// plist lives in a temp "LaunchAgents" directory (never the real one), the
/// data dir, logs and runner copy are all temporary, and the job is booted
/// out at the end whatever happens.
fn adopting_a_hand_written_plist_keeps_its_label_and_can_be_undone() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let agents = tmp.path().join("agents");
    fs::create_dir_all(&data).unwrap();
    fs::create_dir_all(&agents).unwrap();

    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let label = format!("com.lktest.{}", &suffix[..12]);
    let name = TaskName::new(&label).unwrap();

    let runner = data.join("launchkeeper-runner");
    fs::copy(built_runner(), &runner).unwrap();
    let mut perms = fs::metadata(&runner).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&runner, perms).unwrap();

    // SAFETY: same as the phase above — one `#[test]`, one thread, and this
    // phase now owns the three variables for the rest of the run.
    unsafe {
        std::env::set_var(paths::DATA_DIR_ENV, &data);
        std::env::set_var(paths::LAUNCH_AGENTS_DIR_ENV, &agents);
        std::env::set_var(paths::RUNNER_LOG_DIR_ENV, tmp.path().join("runner-logs"));
    }

    let script = tmp.path().join("hand-written.sh");
    write_script(&script, "echo adopted-run-ok");

    // A plist a user could plausibly have written by hand: its file name is
    // deliberately *not* `<label>.plist`, so the lookup has to go by Label.
    let plist = agents.join("my-job.plist");
    let original_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{label}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{script}</string>
	</array>
	<key>StandardOutPath</key>
	<string>{out}</string>
	<key>StartInterval</key>
	<integer>86400</integer>
</dict>
</plist>
"#,
        script = script.display(),
        out = tmp.path().join("hand-written.log").display(),
    );
    fs::write(&plist, &original_xml).unwrap();
    let original = fs::read(&plist).unwrap();

    let store = Store::open(&paths::db_path().unwrap()).unwrap();
    let svc = Service::new(store, runner);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        exercise_adoption(&svc, &name, &label, &plist, &original);
    }));

    let _ = svc.unadopt(&name);
    let _ = svc.remove_task(&name);
    let _ = launchctl::bootout(&label);
    assert!(
        launchctl::status(&label).unwrap().is_none(),
        "测试结束后 {label} 不该还在 launchd 里"
    );

    if let Err(p) = result {
        std::panic::resume_unwind(p);
    }
}

fn exercise_adoption(svc: &Service, name: &TaskName, label: &str, plist: &Path, original: &[u8]) {
    // 未接管也能启停：bootstrap 一个别人的 plist
    let agent = launchkeeper_core::agents::find(&paths::launch_agents_dir().unwrap(), label)
        .expect("应当在临时 LaunchAgents 目录里找到它");
    assert!(agent.adoptable.is_yes(), "{:?}", agent.adoptable);
    assert_eq!(agent.trigger, Some(Trigger::Interval { seconds: 86400 }));
    svc.external_enable(label, plist).unwrap();
    assert!(
        launchctl::status(label).unwrap().is_some(),
        "external_enable 之后 launchd 应当认识这个 label"
    );

    // 接管：label 不变，文件带标记，.bak 是原始字节
    let task = svc.adopt(label).unwrap();
    assert_eq!(task.label(), label, "接管不能改 label");
    assert!(task.adopted);
    let backup = launchkeeper_core::agents::backup_path(plist);
    assert_eq!(fs::read(&backup).unwrap(), original, ".bak 必须逐字节相同");
    let rewritten = fs::read_to_string(plist).unwrap();
    assert!(rewritten.contains("LaunchkeeperManaged"), "{rewritten}");
    assert!(rewritten.contains(label), "{rewritten}");

    // 真 launchd 仍然按原 label 认得它
    assert!(
        wait_for(10, || launchctl::status(label).unwrap()).is_some(),
        "接管后 {label} 应当仍然加载着"
    );
    assert!(svc.is_enabled(name).unwrap());

    // 现在它有运行历史了：runner 按 name = label 找得到这条任务
    svc.run_now(name).unwrap();
    let run = wait_for(30, || {
        svc.store()
            .last_run(name)
            .ok()
            .flatten()
            .filter(|r| r.finished_at.is_some())
    })
    .expect("接管后 kickstart 应当写出一条完成的运行记录");
    assert_eq!(run.exit_code, Some(0), "脚本应当成功");
    let out = fs::read_to_string(&run.stdout_path).unwrap();
    assert!(out.contains("adopted-run-ok"), "捕获的 stdout: {out:?}");

    // 撤销接管：字节级还原，原 job 重新加载
    svc.unadopt(name).unwrap();
    assert_eq!(
        fs::read(plist).unwrap(),
        original,
        "unadopt 必须按字节还原原始 plist"
    );
    assert!(!backup.exists(), ".bak 应当被移回去");
    assert!(svc.store().get_task(name).unwrap().is_none());
    assert!(
        launchctl::status(label).unwrap().is_some(),
        "还原之后原 job 应当重新加载着"
    );
}
