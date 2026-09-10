//! `Service` against a fake `launchctl`.
//!
//! The fake is a shell script written by the test; it records every argv it
//! sees and keeps the set of "bootstrapped" labels in a state file, so
//! `print` can answer 113 (not loaded) or 0 the way real launchd does.
//!
//! Everything lives in one `#[test]` because the three environment variables
//! involved (`LAUNCHKEEPER_LAUNCHCTL`, `LAUNCHKEEPER_LAUNCH_AGENTS_DIR`,
//! `LAUNCHKEEPER_DATA_DIR`) are process-wide; cargo gives this file its own
//! process, so nothing else is disturbed. Neither the real launchd nor the
//! real `~/Library/LaunchAgents` is touched. The scenarios are split into
//! plain functions rather than into separate `#[test]`s, because two `#[test]`
//! functions would run on two threads of the same process and fight over
//! those variables.
//!
//! The real-launchd counterpart lives in `tests/launchd_real.rs`, behind
//! `#[ignore]`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use launchkeeper_core::{Error, Service, Store, Task, TaskName, Trigger, paths};

/// Writes the fake `launchctl` and returns its path.
///
/// Two state files model launchd: `loaded.txt` holds the bootstrapped labels
/// and `running.txt` the ones that currently have a process. Verbs:
///
/// * `bootstrap <domain> <plist>` — records the label read out of the plist's
///   `Label` key (falling back to the file name), exits 5 if it was already
///   bootstrapped (launchd's EIO). Reading the key rather than the file name
///   matters for M3: an adopted plist keeps its own name, which is routinely
///   not `<label>.plist`.
/// * `bootout gui/<uid>/<label>` — exits 3 when the label is not loaded;
///   also clears any running process.
/// * `print gui/<uid>/<label>` — exits 113 when not loaded, else prints a
///   plausible block, including `pid = <n>` while the job is running.
/// * `kickstart gui/<uid>/<label>` — exits 3 when not loaded, else marks the
///   label running with a made-up pid.
/// * `kill <signal> gui/<uid>/<label>` — exits 3 when the label is not
///   running (which is what real launchctl does), else stops it. A
///   `stubborn-<label>` flag file makes it ignore SIGTERM, so the SIGKILL
///   escalation in `Service::stop` can be exercised.
fn write_fake_launchctl(dir: &Path) -> PathBuf {
    let path = dir.join("fake-launchctl");
    let log = dir.join("argv.log");
    let state = dir.join("loaded.txt");
    let running = dir.join("running.txt");
    let script = format!(
        r#"#!/bin/sh
LOG={log}
STATE={state}
RUNNING={running}
DIR={dir}
printf '%s\n' "$*" >> "$LOG"
[ -f "$STATE" ] || : > "$STATE"
[ -f "$RUNNING" ] || : > "$RUNNING"

unrun() {{
  grep -vx "$1" "$RUNNING" > "$RUNNING.tmp" || :
  mv "$RUNNING.tmp" "$RUNNING"
}}

verb="$1"
target="$2"
label="${{target##*/}}"

case "$verb" in
  bootstrap)
    if [ -f "{fail}" ]; then
      echo "forced failure" >&2
      exit 5
    fi
    plist="$3"
    # 和真 launchd 一样，label 取自 plist 里的 Label 键（不是文件名）：
    # 接管来的 plist 文件名和 label 常常不一样。
    label=$(tr -d '\n\t ' < "$plist" | sed -n 's:.*<key>Label</key><string>\([^<]*\)</string>.*:\1:p')
    if [ -z "$label" ]; then
      base="${{plist##*/}}"
      label="${{base%.plist}}"
    fi
    if grep -qx "$label" "$STATE"; then
      echo "Input/output error" >&2
      exit 5
    fi
    echo "$label" >> "$STATE"
    exit 0
    ;;
  bootout)
    if grep -qx "$label" "$STATE"; then
      grep -vx "$label" "$STATE" > "$STATE.tmp" || :
      mv "$STATE.tmp" "$STATE"
      unrun "$label"
      exit 0
    fi
    echo "Could not find service" >&2
    exit 3
    ;;
  print)
    if grep -qx "$label" "$STATE"; then
      echo "$target = {{"
      if grep -qx "$label" "$RUNNING"; then
        echo "	state = running"
        echo "	pid = 4242"
      else
        echo "	state = waiting"
      fi
      echo "	last exit code = 0"
      echo "}}"
      exit 0
    fi
    echo "Could not find service" >&2
    exit 113
    ;;
  kickstart)
    if grep -qx "$label" "$STATE"; then
      grep -qx "$label" "$RUNNING" || echo "$label" >> "$RUNNING"
      exit 0
    fi
    echo "Could not find service" >&2
    exit 3
    ;;
  kill)
    signal="$2"
    target="$3"
    label="${{target##*/}}"
    if grep -qx "$label" "$RUNNING"; then
      # 装死：只有 KILL 才停得下来，用来验证 stop 的升级路径
      if [ -f "$DIR/stubborn-$label" ] && [ "$signal" != "KILL" ]; then
        exit 0
      fi
      unrun "$label"
      exit 0
    fi
    echo "No such process" >&2
    exit 3
    ;;
esac
echo "unexpected verb $verb" >&2
exit 64
"#,
        log = log.display(),
        state = state.display(),
        running = running.display(),
        dir = dir.display(),
        fail = dir.join("fail-bootstrap").display(),
    );
    fs::write(&path, script).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    path
}

fn argv_log(dir: &Path) -> String {
    fs::read_to_string(dir.join("argv.log")).unwrap_or_default()
}

fn truncate_argv_log(dir: &Path) {
    fs::write(dir.join("argv.log"), "").unwrap();
}

fn pid_of(service: &Service, name: &TaskName) -> Option<u32> {
    let task = service.store().get_task(name).unwrap().unwrap();
    Service::enabled_status(&task).unwrap().and_then(|s| s.pid)
}

#[test]
fn service_against_a_fake_launchctl() {
    let data = tempfile::tempdir().unwrap();
    let agents = tempfile::tempdir().unwrap();
    let fake_dir = tempfile::tempdir().unwrap();
    let fake = write_fake_launchctl(fake_dir.path());

    // SAFETY: single-threaded test process, set once before anything reads it.
    unsafe {
        std::env::set_var(paths::DATA_DIR_ENV, data.path());
        std::env::set_var(paths::LAUNCH_AGENTS_DIR_ENV, agents.path());
        std::env::set_var(paths::RUNNER_LOG_DIR_ENV, data.path().join("runner-logs"));
        std::env::set_var(launchkeeper_core::launchctl::LAUNCHCTL_ENV, &fake);
    }

    let store = Store::open(&paths::db_path().unwrap()).unwrap();
    let service = Service::new(store, PathBuf::from("/usr/bin/true"));

    lifecycle(&service, fake_dir.path());
    start_stop_restart(&service, fake_dir.path());
    adopt_and_unadopt(&service, fake_dir.path(), agents.path());
    adopt_refuses_what_it_should(&service, agents.path());
    run_at_load_beside_a_schedule_is_listed_as_removed(&service, agents.path());
    adopt_rolls_everything_back_when_launchd_refuses(&service, fake_dir.path(), agents.path());
    external_enable_leaves_a_loaded_job_alone(&service, fake_dir.path(), agents.path());
}

// ---------------------------------------------------------------------------
// M3 §3.2: adopting a hand-written plist in place
// ---------------------------------------------------------------------------

/// Writes a plist by hand — the way a user would — into the agents dir.
fn write_agent(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    fs::write(
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

const DAILY_BODY: &str = "\
<key>Label</key><string>com.example.daily</string>
<key>ProgramArguments</key><array><string>/bin/echo</string><string>hi</string></array>
<key>WorkingDirectory</key><string>/tmp</string>
<key>EnvironmentVariables</key><dict><key>FOO</key><string>bar</string></dict>
<key>StandardOutPath</key><string>/tmp/daily.log</string>
<key>StartCalendarInterval</key><dict><key>Hour</key><integer>21</integer><key>Minute</key><integer>0</integer></dict>
";

const HOURLY_BODY: &str = "\
<key>Label</key><string>com.example.hourly</string>
<key>ProgramArguments</key><array><string>/bin/echo</string></array>
<key>StartInterval</key><integer>3600</integer>
";

const WATCHER_BODY: &str = "\
<key>Label</key><string>com.example.watcher</string>
<key>ProgramArguments</key><array><string>/bin/echo</string></array>
<key>WatchPaths</key><array><string>/tmp/inbox</string></array>
";

fn adopt_and_unadopt(service: &Service, fake_dir: &Path, agents_dir: &Path) {
    let plist = write_agent(agents_dir, "daily.plist", DAILY_BODY);
    write_agent(agents_dir, "hourly.plist", HOURLY_BODY);
    write_agent(agents_dir, "watcher.plist", WATCHER_BODY);
    let original = fs::read(&plist).unwrap();

    // ---- 列表：三个都在，第三个不可接管，前两个可以
    let found = launchkeeper_core::agents::list_external(agents_dir).unwrap();
    let labels: Vec<&str> = found.iter().map(|a| a.label.as_str()).collect();
    assert_eq!(
        labels,
        vec![
            "com.example.daily",
            "com.example.hourly",
            "com.example.watcher"
        ],
        "应当按 label 排序列出三个手写 plist"
    );
    let daily = &found[0];
    assert!(daily.adoptable.is_yes(), "{:?}", daily.adoptable);
    assert_eq!(daily.trigger_text(), "每天 21:00");
    assert_eq!(daily.command(), "/bin/echo hi");
    assert!(found[1].adoptable.is_yes());
    let watcher = &found[2];
    assert!(!watcher.adoptable.is_yes());
    assert!(
        watcher.adoptable.reason().unwrap().contains("WatchPaths"),
        "{:?}",
        watcher.adoptable
    );
    assert!(watcher.trigger.is_none());

    // 我们自己的 com.launchkeeper.* plist 不算"外部"
    assert!(!labels.iter().any(|l| l.starts_with("com.launchkeeper.")));

    // ---- 计划：只读，label 不变，diff 说得出改了什么
    let plan = service.adoption_plan("com.example.daily").unwrap();
    assert_eq!(plan.label, "com.example.daily");
    assert_eq!(plan.backup_path, plist.with_extension("plist.bak"));
    assert_eq!(plan.task.name.as_str(), "com.example.daily");
    assert!(plan.task.adopted);
    assert_eq!(plan.task.plist_path.as_deref(), Some(plist.as_path()));
    assert_eq!(plan.task.working_dir.as_deref(), Some(Path::new("/tmp")));
    assert_eq!(plan.task.env.get("FOO").map(String::as_str), Some("bar"));
    let key = |v: &[(String, String)], k: &str| v.iter().any(|(a, _)| a == k);
    assert!(key(&plan.kept, "Label"), "label 不变: {:?}", plan.kept);
    assert!(
        key(&plan.kept, "StartCalendarInterval"),
        "触发规则原样保留: {:?}",
        plan.kept
    );
    assert!(key(&plan.removed, "WorkingDirectory"));
    assert!(key(&plan.removed, "EnvironmentVariables")); // 换成 LAUNCHKEEPER_DATA_DIR
    assert!(key(&plan.added, "EnvironmentVariables"));
    assert!(key(&plan.added, "ProgramArguments")); // 换成 runner
    assert!(key(&plan.added, "LaunchkeeperManaged"));
    assert!(key(&plan.added, "ThrottleInterval"));
    // 计划本身什么都没写
    assert!(!plan.backup_path.exists());
    assert_eq!(fs::read(&plist).unwrap(), original);
    assert!(service.store().get_task(&plan.task.name).unwrap().is_none());

    // ---- 接管
    truncate_argv_log(fake_dir);
    let task = service.adopt("com.example.daily").unwrap();
    assert_eq!(task.label(), "com.example.daily", "label 必须原样保留");
    assert!(plan.backup_path.exists(), "原文件应当备份为 .bak");
    assert_eq!(
        fs::read(&plan.backup_path).unwrap(),
        original,
        ".bak 必须是原始字节"
    );
    let xml = fs::read_to_string(&plist).unwrap();
    assert!(xml.contains("<key>LaunchkeeperManaged</key>"), "{xml}");
    assert!(xml.contains("com.example.daily"), "{xml}");
    assert!(
        xml.contains("<string>/usr/bin/true</string>"),
        "runner: {xml}"
    );
    let log = argv_log(fake_dir);
    assert!(log.contains("bootstrap"), "接管后应当重新加载: {log}");
    assert!(service.is_enabled(&task.name).unwrap());

    // 接管后它就是一个普通任务：runner 按 name = label 找得到它，
    // 运行历史照常记
    let stored = service.store().get_task(&task.name).unwrap().unwrap();
    assert!(stored.adopted);
    assert_eq!(stored.plist_path.as_deref(), Some(plist.as_path()));
    service
        .store()
        .start_run(
            &task.name,
            launchkeeper_core::TriggerKind::Scheduled,
            chrono::Utc::now(),
            Path::new("/tmp/a.out"),
            Path::new("/tmp/a.err"),
        )
        .unwrap();
    assert!(service.store().last_run(&task.name).unwrap().is_some());

    // 已接管的 plist 现在被标成"我们的"，普通 enable 也能重写它
    service.enable(&task.name).unwrap();
    assert!(
        fs::read_to_string(&plist)
            .unwrap()
            .contains("LaunchkeeperManaged")
    );

    // 已经接管过的不会再出现在"可接管"里
    let again = launchkeeper_core::agents::find(agents_dir, "com.example.daily").unwrap();
    assert!(again.managed_by_launchkeeper);
    assert!(!again.adoptable.is_yes());
    assert!(matches!(
        service.adopt("com.example.daily").unwrap_err(),
        Error::Adopt(_)
    ));

    // ---- 撤销接管：字节级还原
    truncate_argv_log(fake_dir);
    service.unadopt(&task.name).unwrap();
    assert_eq!(
        fs::read(&plist).unwrap(),
        original,
        "unadopt 必须按字节还原原始 plist"
    );
    assert!(!plan.backup_path.exists(), ".bak 应当被移回去，不再留着");
    assert!(service.store().get_task(&task.name).unwrap().is_none());
    let log = argv_log(fake_dir);
    assert!(log.contains("bootout"), "{log}");
    assert!(log.contains("bootstrap"), "还原后应当重新加载原 job: {log}");

    // 撤销之后可以再接管一次
    let task = service.adopt("com.example.daily").unwrap();
    service.unadopt(&task.name).unwrap();
    assert_eq!(fs::read(&plist).unwrap(), original);

    // ---- 不重新加载的接管（界面上那个没勾的「接管后立即重新加载」）：
    // 文件改了、行也插了，但 launchd 手都没碰——正在跑的 job 不受打扰。
    truncate_argv_log(fake_dir);
    let task = service.adopt_with("com.example.daily", false).unwrap();
    let log = argv_log(fake_dir);
    assert!(
        !log.contains("bootstrap") && !log.contains("bootout"),
        "没勾重新加载就不该碰 launchd: {log}"
    );
    assert!(
        fs::read_to_string(&plist)
            .unwrap()
            .contains("LaunchkeeperManaged"),
        "plist 仍然要被改写"
    );
    assert!(service.store().get_task(&task.name).unwrap().is_some());
    service.unadopt(&task.name).unwrap();
    assert_eq!(fs::read(&plist).unwrap(), original);

    // 非接管任务不能 unadopt
    let plain = TaskName::new("plain").unwrap();
    service
        .add_task(Task::new(plain.clone(), "/usr/bin/true", Trigger::Manual))
        .unwrap();
    assert!(matches!(
        service.unadopt(&plain).unwrap_err(),
        Error::Adopt(_)
    ));
    service.remove_task(&plain).unwrap();

    // 收尾
    for f in ["daily.plist", "hourly.plist", "watcher.plist"] {
        fs::remove_file(agents_dir.join(f)).unwrap();
    }
}

/// L1: `RunAtLoad` next to a schedule is *not* representable — `Trigger`
/// holds one rule, and the interval is the one that matters — so the rewrite
/// drops it. It must at least be visible in the plan's `removed` group, which
/// is what the CLI prints and the app's 接管确认框 shows.
fn run_at_load_beside_a_schedule_is_listed_as_removed(service: &Service, agents_dir: &Path) {
    let plist = write_agent(
        agents_dir,
        "both.plist",
        "<key>Label</key><string>com.example.both</string>\n\
         <key>ProgramArguments</key><array><string>/bin/echo</string></array>\n\
         <key>RunAtLoad</key><true/>\n\
         <key>StartInterval</key><integer>3600</integer>\n",
    );

    let agent = launchkeeper_core::agents::find(agents_dir, "com.example.both").unwrap();
    assert_eq!(
        agent.trigger,
        Some(Trigger::Interval { seconds: 3600 }),
        "间隔才是那条要留住的规则"
    );
    let plan = service.adoption_plan("com.example.both").unwrap();
    let removed: Vec<&str> = plan.removed.iter().map(|(k, _)| k.as_str()).collect();
    assert!(
        removed.contains(&"RunAtLoad"),
        "接管会丢掉「登录时也跑一次」，diff 里必须看得见: {removed:?}"
    );
    assert!(plan.kept.iter().any(|(k, _)| k == "StartInterval"));

    fs::remove_file(&plist).unwrap();
}

/// The rollback path (H1): launchd refuses the rewritten plist, and adoption
/// has to undo everything it did — including putting the user's bytes back —
/// rather than leave a half-adopted job behind.
fn adopt_rolls_everything_back_when_launchd_refuses(
    service: &Service,
    fake_dir: &Path,
    agents_dir: &Path,
) {
    let plist = write_agent(agents_dir, "hourly.plist", HOURLY_BODY);
    let original = fs::read(&plist).unwrap();
    let backup = plist.with_extension("plist.bak");
    let name = TaskName::new("com.example.hourly").unwrap();

    // 让假 launchctl 的 bootstrap 失败：接管的最后一步炸掉
    let flag = fake_dir.join("fail-bootstrap");
    fs::write(&flag, "").unwrap();
    let err = service.adopt("com.example.hourly").unwrap_err();
    fs::remove_file(&flag).unwrap();

    assert!(err.to_string().contains("launchctl"), "{err}");
    assert_eq!(
        fs::read(&plist).unwrap(),
        original,
        "回滚必须把原始字节放回去"
    );
    assert!(
        !backup.exists(),
        "还原成功之后 .bak 不该还留着（它就是被移回去的那份）"
    );
    assert!(
        service.store().get_task(&name).unwrap().is_none(),
        "回滚必须把刚插进去的任务行删掉"
    );

    // 回滚干净了，所以可以直接重试——.bak 不会挡路
    let task = service.adopt("com.example.hourly").unwrap();
    service.unadopt(&task.name).unwrap();
    assert_eq!(fs::read(&plist).unwrap(), original);
    fs::remove_file(&plist).unwrap();
}

/// M7: `external_enable` on a label launchd already knows must not bootout
/// and bootstrap somebody else's *running* job.
fn external_enable_leaves_a_loaded_job_alone(
    service: &Service,
    fake_dir: &Path,
    agents_dir: &Path,
) {
    let plist = write_agent(agents_dir, "hourly.plist", HOURLY_BODY);
    let label = "com.example.hourly";
    // 上一段的 unadopt 会把原 job 重新加载回去，先回到"没加载"的起点
    service.external_disable(label).unwrap();

    truncate_argv_log(fake_dir);
    service.external_enable(label, &plist).unwrap();
    let log = argv_log(fake_dir);
    assert!(log.contains("bootstrap"), "没加载的要加载: {log}");

    // 第二次：launchd 已经认识它了，什么都不该发生
    truncate_argv_log(fake_dir);
    service.external_enable(label, &plist).unwrap();
    let log = argv_log(fake_dir);
    assert!(
        !log.contains("bootstrap") && !log.contains("bootout"),
        "已经加载的 job 不该被重启一遍: {log}"
    );

    service.external_disable(label).unwrap();
    fs::remove_file(&plist).unwrap();
}

/// The refusals that protect somebody else's files.
fn adopt_refuses_what_it_should(service: &Service, agents_dir: &Path) {
    // 未知 label
    assert!(matches!(
        service.adopt("com.nobody.here").unwrap_err(),
        Error::AgentNotFound(_)
    ));

    // 已存在的 .bak 会挡住接管：它是原件的唯一副本
    let plist = write_agent(agents_dir, "hourly.plist", HOURLY_BODY);
    let backup = plist.with_extension("plist.bak");
    fs::write(&backup, "别人的东西").unwrap();
    let err = service.adopt("com.example.hourly").unwrap_err();
    assert!(err.to_string().contains("备份文件已存在"), "{err}");
    assert_eq!(fs::read_to_string(&backup).unwrap(), "别人的东西");
    assert!(
        service
            .store()
            .get_task(&TaskName::new("com.example.hourly").unwrap())
            .unwrap()
            .is_none()
    );
    fs::remove_file(&backup).unwrap();

    // Apple / Homebrew 的 label，以及 /Applications 里的程序，一律拒绝
    for (file, body) in [
        (
            "apple.plist",
            "<key>Label</key><string>com.apple.thing</string>\n\
             <key>ProgramArguments</key><array><string>/bin/echo</string></array>\n",
        ),
        (
            "brew.plist",
            "<key>Label</key><string>homebrew.mxcl.pg</string>\n\
             <key>ProgramArguments</key><array><string>/bin/echo</string></array>\n",
        ),
        (
            "app.plist",
            "<key>Label</key><string>com.vendor.helper</string>\n\
             <key>ProgramArguments</key><array><string>/Applications/X.app/Contents/MacOS/x</string></array>\n",
        ),
        (
            "noprog.plist",
            "<key>Label</key><string>com.vendor.noprog</string>\n\
             <key>RunAtLoad</key><true/>\n",
        ),
        (
            "month.plist",
            "<key>Label</key><string>com.vendor.month</string>\n\
             <key>ProgramArguments</key><array><string>/bin/echo</string></array>\n\
             <key>StartCalendarInterval</key><dict><key>Month</key><integer>3</integer>\
             <key>Hour</key><integer>1</integer><key>Minute</key><integer>0</integer></dict>\n",
        ),
    ] {
        write_agent(agents_dir, file, body);
    }
    let found = launchkeeper_core::agents::list_external(agents_dir).unwrap();
    for label in [
        "com.apple.thing",
        "homebrew.mxcl.pg",
        "com.vendor.helper",
        "com.vendor.noprog",
        "com.vendor.month",
    ] {
        let a = found.iter().find(|a| a.label == label).expect(label);
        assert!(!a.adoptable.is_yes(), "{label} 不该可接管");
        assert!(matches!(service.adopt(label).unwrap_err(), Error::Adopt(_)));
    }
    for f in [
        "hourly.plist",
        "apple.plist",
        "brew.plist",
        "app.plist",
        "noprog.plist",
        "month.plist",
    ] {
        fs::remove_file(agents_dir.join(f)).unwrap();
    }
}

/// M1: add / enable / run_now / update / disable / remove.
fn lifecycle(service: &Service, fake_dir: &Path) {
    let name = TaskName::new("sync").unwrap();
    let mut task = Task::new(
        name.clone(),
        "/usr/bin/true",
        Trigger::Interval { seconds: 3600 },
    );
    task.description = Some("原描述".into());
    service.add_task(task.clone()).unwrap();

    // add 不碰 launchd
    assert_eq!(argv_log(fake_dir), "");
    assert!(!service.is_enabled(&name).unwrap());

    // enable：写 plist + reload（bootout 未加载 -> 视为成功，再 bootstrap）
    service.enable(&name).unwrap();
    let plist = paths::plist_path(&task).unwrap();
    assert!(plist.exists());
    assert!(service.is_enabled(&name).unwrap());
    let log = argv_log(fake_dir);
    assert!(log.contains("bootout"), "{log}");
    assert!(log.contains("bootstrap"), "{log}");

    // run_now 需要 job 已加载
    service.run_now(&name).unwrap();
    assert!(argv_log(fake_dir).contains("kickstart"), "缺 kickstart");

    // update 时已启用：先重载 launchd，再写库；即使只改了描述也重载
    task.description = Some("改过的描述".into());
    task.trigger = Trigger::Interval { seconds: 7200 };
    service.update_task(task.clone()).unwrap();
    let stored = service.store().get_task(&name).unwrap().unwrap();
    assert_eq!(stored.description.as_deref(), Some("改过的描述"));
    assert_eq!(stored.trigger, Trigger::Interval { seconds: 7200 });
    assert!(fs::read_to_string(&plist).unwrap().contains("7200"));
    assert!(service.is_enabled(&name).unwrap());

    // launchctl 失败时数据库保持不变
    let fail_flag = fake_dir.join("fail-bootstrap");
    fs::write(&fail_flag, "").unwrap();
    let mut broken = task.clone();
    broken.description = Some("不该落库".into());
    let err = service.update_task(broken).unwrap_err();
    assert!(
        err.to_string().contains("数据库未改动"),
        "错误信息应说明数据库未改动: {err}"
    );
    fs::remove_file(&fail_flag).unwrap();
    assert_eq!(
        service
            .store()
            .get_task(&name)
            .unwrap()
            .unwrap()
            .description
            .as_deref(),
        Some("改过的描述")
    );

    // 未启用时 update 不碰 launchd
    service.disable(&name).unwrap();
    assert!(!plist.exists());
    assert!(!service.is_enabled(&name).unwrap());
    let before = argv_log(fake_dir).lines().count();
    let mut offline = task.clone();
    offline.description = Some("离线改的".into());
    service.update_task(offline).unwrap();
    // 只多了 is_enabled 里的那次 print（plist 不存在时连 print 都省了）
    assert_eq!(argv_log(fake_dir).lines().count(), before);

    // run_now 在未加载时报 NotEnabled
    assert!(matches!(
        service.run_now(&name).unwrap_err(),
        Error::NotEnabled(_)
    ));

    // remove：bootout（未加载也算成功）+ 删 plist + 删库 + 删日志目录
    let log_dir = paths::task_log_dir(&name).unwrap();
    fs::create_dir_all(&log_dir).unwrap();
    fs::write(log_dir.join("x.out"), "hi").unwrap();
    service.remove_task(&name).unwrap();
    assert!(service.store().get_task(&name).unwrap().is_none());
    assert!(!plist.exists());
    assert!(!log_dir.exists());

    // 再启用一次并 remove，验证 bootout 走的是"已加载"分支
    service.add_task(task).unwrap();
    service.enable(&name).unwrap();
    assert!(service.is_enabled(&name).unwrap());
    service.remove_task(&name).unwrap();
    assert!(!service.is_enabled(&name).unwrap());
    assert!(!plist.exists());
}

/// M2.5: start / stop / restart on a `Trigger::Manual` service, plus the
/// SIGKILL escalation and the no-op paths.
fn start_stop_restart(service: &Service, fake_dir: &Path) {
    let name = TaskName::new("bridge").unwrap();
    let mut task = Task::new(name.clone(), "/usr/bin/true", Trigger::Manual);
    task.keep_alive = true;
    assert!(task.is_service());
    service.add_task(task.clone()).unwrap();

    let plist = paths::plist_path(&task).unwrap();
    assert!(!service.is_enabled(&name).unwrap());

    // stop 一个还没 enable 的任务：什么都不做，也不报错
    truncate_argv_log(fake_dir);
    service.stop(&name).unwrap();
    assert!(
        !argv_log(fake_dir).contains("kill"),
        "没加载的任务不该发信号: {}",
        argv_log(fake_dir)
    );

    // start：没 enable 就先 enable（写 plist + bootstrap），再 kickstart
    truncate_argv_log(fake_dir);
    service.start(&name).unwrap();
    let log = argv_log(fake_dir);
    assert!(log.contains("bootstrap"), "start 应当先加载: {log}");
    assert!(log.contains("kickstart"), "start 应当 kickstart: {log}");
    assert!(plist.exists());
    // KeepAlive 写成字典而不是 <true/>
    let xml = fs::read_to_string(&plist).unwrap();
    assert!(xml.contains("<key>KeepAlive</key>"), "{xml}");
    assert!(xml.contains("<key>SuccessfulExit</key>"), "{xml}");
    assert!(!xml.contains("<key>RunAtLoad</key>"), "{xml}");
    assert_eq!(pid_of(service, &name), Some(4242), "start 之后应当有 pid");

    // 已经加载了就不再 bootstrap，只 kickstart
    truncate_argv_log(fake_dir);
    service.start(&name).unwrap();
    let log = argv_log(fake_dir);
    assert!(
        !log.contains("bootstrap"),
        "已加载不该重新 bootstrap: {log}"
    );
    assert!(log.contains("kickstart"), "{log}");

    // stop：发 TERM，job 仍然加载着，只是没进程了
    truncate_argv_log(fake_dir);
    service.stop(&name).unwrap();
    let log = argv_log(fake_dir);
    assert!(log.contains("kill TERM"), "stop 应当发 SIGTERM: {log}");
    assert!(!log.contains("kill KILL"), "TERM 就够了，不该升级: {log}");
    assert!(!log.contains("bootout"), "stop 不该把 job 卸载: {log}");
    assert!(
        service.is_enabled(&name).unwrap(),
        "stop 之后仍然是 enabled"
    );
    assert_eq!(pid_of(service, &name), None);

    // 再 stop 一次：已经停了，直接返回
    truncate_argv_log(fake_dir);
    service.stop(&name).unwrap();
    assert!(
        !argv_log(fake_dir).contains("kill"),
        "已经停了就不该再发信号: {}",
        argv_log(fake_dir)
    );

    // restart：先 stop（这次是从运行中停）再 start
    service.start(&name).unwrap();
    assert_eq!(pid_of(service, &name), Some(4242));
    truncate_argv_log(fake_dir);
    service.restart(&name).unwrap();
    let log = argv_log(fake_dir);
    assert!(log.contains("kill TERM"), "restart 应当先停: {log}");
    assert!(log.contains("kickstart"), "restart 应当再起: {log}");
    assert_eq!(pid_of(service, &name), Some(4242));

    // run_now 对服务型任务等价于 start
    service.stop(&name).unwrap();
    assert_eq!(pid_of(service, &name), None);
    service.run_now(&name).unwrap();
    assert_eq!(
        pid_of(service, &name),
        Some(4242),
        "run_now 应当把服务起起来"
    );

    // 顽固进程：TERM 不管用，stop 最终升级到 KILL
    fs::write(fake_dir.join(format!("stubborn-{}", name.label())), "").unwrap();
    truncate_argv_log(fake_dir);
    let began = std::time::Instant::now();
    service.stop(&name).unwrap();
    let log = argv_log(fake_dir);
    assert!(log.contains("kill TERM"), "{log}");
    assert!(
        log.contains("kill KILL"),
        "TERM 无效时应当升级到 SIGKILL: {log}"
    );
    assert!(
        began.elapsed() >= launchkeeper_core::service::STOP_GRACE,
        "应当等满宽限期再 KILL"
    );
    assert_eq!(pid_of(service, &name), None);
    fs::remove_file(fake_dir.join(format!("stubborn-{}", name.label()))).unwrap();

    // 收尾：不留任何 job
    service.remove_task(&name).unwrap();
    assert!(!plist.exists());
    assert!(service.store().get_task(&name).unwrap().is_none());
}
