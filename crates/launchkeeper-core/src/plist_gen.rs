//! LaunchAgent plist generation.
//!
//! The plist answers exactly one question: *when* should the job be started.
//! Everything about *how* to run the script (cwd, env, args) lives in the
//! database and is read back by `launchkeeper-runner`, so there is only ever
//! one source of truth for execution details.

use std::path::Path;

use plist::{Dictionary, Value};

use crate::error::{Error, Result};
use crate::task::{CalendarEntry, Task, Trigger};

/// Throttle written into every generated plist, in seconds.
pub const THROTTLE_INTERVAL: i64 = 60;

/// The key that marks a plist Launchkeeper is allowed to write even though
/// its label is not `com.launchkeeper.*` — i.e. one it adopted in place
/// (`docs/M3-design.md` §3.1). launchd ignores keys it does not know, so
/// this costs nothing at run time.
pub const MANAGED_KEY: &str = "LaunchkeeperManaged";

/// Inputs the task itself does not carry.
#[derive(Debug, Clone, Copy)]
pub struct PlistOptions<'a> {
    /// Absolute path to the `launchkeeper-runner` binary.
    pub runner_path: &'a Path,
    /// Where launchd redirects the runner's own stdout/stderr, i.e.
    /// `paths::runner_log_path(name)`. Must be on the boot disk.
    pub runner_log: &'a Path,
    /// The data directory, i.e. `paths::data_dir()`. Written into the plist
    /// as `EnvironmentVariables.LAUNCHKEEPER_DATA_DIR` so the runner opens the
    /// same database as the process that generated the plist.
    pub data_dir: &'a Path,
}

fn calendar_dict(e: &CalendarEntry) -> Dictionary {
    let mut d = Dictionary::new();
    d.insert("Minute".into(), Value::Integer(i64::from(e.minute).into()));
    d.insert("Hour".into(), Value::Integer(i64::from(e.hour).into()));
    if let Some(w) = e.weekday {
        d.insert("Weekday".into(), Value::Integer(i64::from(w).into()));
    }
    if let Some(day) = e.day {
        d.insert("Day".into(), Value::Integer(i64::from(day).into()));
    }
    d
}

/// Builds the plist dictionary for `task`.
///
/// Keys written: `Label`, `ProgramArguments`, `StandardOutPath`,
/// `StandardErrorPath`, `ThrottleInterval`, `ProcessType`,
/// `EnvironmentVariables` (only `LAUNCHKEEPER_DATA_DIR`), plus at most one
/// of `RunAtLoad` / `StartInterval` / `StartCalendarInterval` (a
/// [`Trigger::Manual`] task gets none of them), plus `KeepAlive` when the
/// task asks for it.
///
/// `KeepAlive` has two shapes, per the measurements in
/// `docs/M2.5-design.md` §3:
///
/// * a `Manual` task gets the dictionary `{SuccessfulExit = false}`, so that
///   launchd restarts it after a crash or a signal but leaves it alone once
///   the runner exits 0 — which is how "stop" is implemented without
///   booting the job out;
/// * an `AtLogin` task keeps the plain `true` it has had since M1.
pub fn build_plist(task: &Task, opts: &PlistOptions<'_>) -> Dictionary {
    let mut d = Dictionary::new();
    d.insert("Label".into(), Value::String(task.label()));
    // Always the installed runner, never the user's script — see CLAUDE.md.
    // The third argument is the task *name*, which for an adopted task is
    // the label verbatim; the runner looks the row up by it either way.
    d.insert(
        "ProgramArguments".into(),
        Value::Array(vec![
            Value::String(opts.runner_path.to_string_lossy().into_owned()),
            Value::String("run".into()),
            Value::String(task.name.as_str().to_string()),
        ]),
    );
    if task.adopted {
        // The permission slip that lets `write_plist` overwrite this file
        // again next time, even though its label is somebody else's.
        d.insert(MANAGED_KEY.into(), Value::Boolean(true));
    }

    match &task.trigger {
        Trigger::AtLogin => {
            d.insert("RunAtLoad".into(), Value::Boolean(true));
        }
        Trigger::Interval { seconds } => {
            d.insert(
                "StartInterval".into(),
                Value::Integer(i64::from(*seconds).into()),
            );
        }
        Trigger::Calendar { entries } => {
            let value = if entries.len() == 1 {
                Value::Dictionary(calendar_dict(&entries[0]))
            } else {
                Value::Array(
                    entries
                        .iter()
                        .map(|e| Value::Dictionary(calendar_dict(e)))
                        .collect(),
                )
            };
            d.insert("StartCalendarInterval".into(), value);
        }
        // 手动启停：一个触发键都不写。
        Trigger::Manual => {}
    }

    if task.keep_alive {
        let value = if task.is_service() {
            let mut ka = Dictionary::new();
            ka.insert("SuccessfulExit".into(), Value::Boolean(false));
            Value::Dictionary(ka)
        } else {
            Value::Boolean(true)
        };
        d.insert("KeepAlive".into(), value);
    }

    let runner_log = opts.runner_log.to_string_lossy().into_owned();
    d.insert("StandardOutPath".into(), Value::String(runner_log.clone()));
    d.insert("StandardErrorPath".into(), Value::String(runner_log));
    d.insert(
        "ThrottleInterval".into(),
        Value::Integer(THROTTLE_INTERVAL.into()),
    );
    d.insert("ProcessType".into(), Value::String("Background".into()));
    let mut env = Dictionary::new();
    env.insert(
        crate::paths::DATA_DIR_ENV.into(),
        Value::String(opts.data_dir.to_string_lossy().into_owned()),
    );
    d.insert("EnvironmentVariables".into(), Value::Dictionary(env));
    d
}

/// True when Launchkeeper is allowed to write `dest`, i.e. when the file
/// there (if any) is one of ours.
///
/// Ours means either of:
///
/// * the file name starts with `com.launchkeeper.` — a plist only this
///   program ever creates; or
/// * the file parses as a plist dictionary carrying
///   [`MANAGED_KEY`]` = true`, which is the marker adoption writes.
///
/// A `dest` that does not exist yet is writable: there is nothing to
/// clobber. A file that exists but cannot be parsed as a plist dictionary is
/// **not** ours — a half-written or hand-edited file somebody cares about is
/// exactly what this check is for.
///
/// # Errors
/// I/O errors other than "does not exist".
pub fn is_ours(dest: &Path) -> Result<bool> {
    let name = dest.file_name().unwrap_or_default().to_string_lossy();
    if name.starts_with(crate::task::LABEL_PREFIX) {
        return Ok(true);
    }
    match std::fs::metadata(dest) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(e) => return Err(Error::io(dest, e)),
        Ok(_) => {}
    }
    let Ok(value) = plist::Value::from_file(dest) else {
        return Ok(false);
    };
    Ok(value
        .as_dictionary()
        .and_then(|d| d.get(MANAGED_KEY))
        .and_then(Value::as_boolean)
        .unwrap_or(false))
}

/// Writes the plist for `task` to `dest` atomically: a temporary file in the
/// same directory is written first and then renamed into place.
///
/// Refuses to overwrite a plist that is not ours ([`is_ours`]) — the
/// "只写自己的 plist" rule of CLAUDE.md, enforced at the one place bytes
/// actually reach `~/Library/LaunchAgents`. The single exception is
/// [`write_plist_adopting`], which the adoption path uses *after* it has
/// backed the original up.
///
/// # Errors
/// [`Error::PlistNotOurs`], I/O and plist serialization errors.
pub fn write_plist(task: &Task, opts: &PlistOptions<'_>, dest: &Path) -> Result<()> {
    if !is_ours(dest)? {
        return Err(Error::PlistNotOurs(dest.to_path_buf()));
    }
    write_plist_adopting(task, opts, dest)
}

/// [`write_plist`] without the ownership check.
///
/// Only [`crate::Service::adopt`] may call this, and only once it has copied
/// the original file to `<plist>.bak`: taking a foreign plist over is the one
/// legitimate way to write a file that is not ours yet, and the plist this
/// writes carries [`MANAGED_KEY`] so every later write goes through the
/// checked path above.
///
/// # Errors
/// I/O and plist serialization errors.
pub(crate) fn write_plist_adopting(
    task: &Task,
    opts: &PlistOptions<'_>,
    dest: &Path,
) -> Result<()> {
    let dict = build_plist(task, opts);
    let dir = dest.parent().ok_or_else(|| {
        Error::InvalidTask(format!("plist 目标路径没有父目录: {}", dest.display()))
    })?;
    std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    let file_name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "plist".to_string());
    let tmp = dir.join(format!(".{file_name}.tmp-{}", std::process::id()));
    plist::to_file_xml(&tmp, &Value::Dictionary(dict))?;
    std::fs::rename(&tmp, dest).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        Error::io(dest, e)
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::task::{CalendarEntry, Task, TaskName, Trigger};

    fn task(trigger: Trigger) -> Task {
        Task::new(TaskName::new("sync").unwrap(), "/usr/bin/true", trigger)
    }

    fn opts() -> (PathBuf, PathBuf) {
        (
            PathBuf::from("/Applications/Launchkeeper.app/Contents/MacOS/launchkeeper-runner"),
            PathBuf::from("/tmp/lk-logs/sync"),
        )
    }

    fn build(t: &Task) -> Dictionary {
        let (runner, log_dir) = opts();
        build_plist(
            t,
            &PlistOptions {
                runner_path: &runner,
                runner_log: &log_dir.join("runner.log"),
                data_dir: Path::new("/tmp/lk-data"),
            },
        )
    }

    fn int(d: &Dictionary, k: &str) -> i64 {
        d.get(k)
            .unwrap_or_else(|| panic!("missing {k}"))
            .as_signed_integer()
            .unwrap()
    }

    #[test]
    fn common_keys() {
        let d = build(&task(Trigger::AtLogin));
        assert_eq!(
            d.get("Label").unwrap().as_string().unwrap(),
            "com.launchkeeper.sync"
        );
        let args: Vec<&str> = d
            .get("ProgramArguments")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_string().unwrap())
            .collect();
        assert_eq!(
            args,
            vec![
                "/Applications/Launchkeeper.app/Contents/MacOS/launchkeeper-runner",
                "run",
                "sync"
            ]
        );
        assert_eq!(
            d.get("StandardOutPath").unwrap().as_string().unwrap(),
            "/tmp/lk-logs/sync/runner.log"
        );
        assert_eq!(
            d.get("StandardErrorPath").unwrap().as_string().unwrap(),
            "/tmp/lk-logs/sync/runner.log"
        );
        assert_eq!(int(&d, "ThrottleInterval"), 60);
        assert_eq!(
            d.get("ProcessType").unwrap().as_string().unwrap(),
            "Background"
        );
        // 只带 LAUNCHKEEPER_DATA_DIR，任务自定义 env 不进 plist
        let env = d
            .get("EnvironmentVariables")
            .unwrap()
            .as_dictionary()
            .unwrap();
        assert_eq!(env.len(), 1);
        assert_eq!(
            env.get("LAUNCHKEEPER_DATA_DIR")
                .unwrap()
                .as_string()
                .unwrap(),
            "/tmp/lk-data"
        );
        // 其他执行细节不进 plist
        for absent in ["WorkingDirectory", "Program"] {
            assert!(d.get(absent).is_none(), "{absent} 不该出现在 plist 里");
        }
    }

    #[test]
    fn at_login_sets_run_at_load_only() {
        let d = build(&task(Trigger::AtLogin));
        assert!(d.get("RunAtLoad").unwrap().as_boolean().unwrap());
        assert!(d.get("StartInterval").is_none());
        assert!(d.get("StartCalendarInterval").is_none());
        assert!(d.get("KeepAlive").is_none());
    }

    #[test]
    fn interval_sets_start_interval_only() {
        let d = build(&task(Trigger::Interval { seconds: 1800 }));
        assert_eq!(int(&d, "StartInterval"), 1800);
        assert!(d.get("RunAtLoad").is_none());
        assert!(d.get("StartCalendarInterval").is_none());
    }

    #[test]
    fn single_calendar_entry_is_a_dict() {
        let d = build(&task(Trigger::Calendar {
            entries: vec![CalendarEntry::daily(21, 0)],
        }));
        let cal = d
            .get("StartCalendarInterval")
            .unwrap()
            .as_dictionary()
            .unwrap();
        assert_eq!(cal.len(), 2);
        assert_eq!(int(cal, "Hour"), 21);
        assert_eq!(int(cal, "Minute"), 0);
        assert!(cal.get("Weekday").is_none());
        assert!(cal.get("Day").is_none());
        assert!(d.get("RunAtLoad").is_none());
        assert!(d.get("StartInterval").is_none());
    }

    #[test]
    fn multiple_calendar_entries_become_an_array() {
        let d = build(&task(Trigger::Calendar {
            entries: vec![
                CalendarEntry {
                    minute: 30,
                    hour: 9,
                    weekday: Some(1),
                    day: None,
                },
                CalendarEntry {
                    minute: 0,
                    hour: 8,
                    weekday: None,
                    day: Some(1),
                },
            ],
        }));
        let arr = d.get("StartCalendarInterval").unwrap().as_array().unwrap();
        assert_eq!(arr.len(), 2);
        let first = arr[0].as_dictionary().unwrap();
        assert_eq!(int(first, "Minute"), 30);
        assert_eq!(int(first, "Hour"), 9);
        assert_eq!(int(first, "Weekday"), 1);
        assert!(first.get("Day").is_none());
        let second = arr[1].as_dictionary().unwrap();
        assert_eq!(int(second, "Day"), 1);
        assert!(second.get("Weekday").is_none());
    }

    #[test]
    fn keep_alive_is_written_only_when_true() {
        let mut t = task(Trigger::AtLogin);
        t.keep_alive = true;
        let d = build(&t);
        assert!(d.get("KeepAlive").unwrap().as_boolean().unwrap());
    }

    #[test]
    fn manual_writes_no_trigger_key_at_all() {
        let d = build(&task(Trigger::Manual));
        for absent in [
            "RunAtLoad",
            "StartInterval",
            "StartCalendarInterval",
            "KeepAlive",
        ] {
            assert!(
                d.get(absent).is_none(),
                "{absent} 不该出现在手动任务的 plist 里"
            );
        }
        // 公共键还是照写
        assert_eq!(
            d.get("Label").unwrap().as_string().unwrap(),
            "com.launchkeeper.sync"
        );
        assert_eq!(int(&d, "ThrottleInterval"), 60);
    }

    #[test]
    fn manual_with_keep_alive_uses_the_successful_exit_dictionary() {
        let mut t = task(Trigger::Manual);
        t.keep_alive = true;
        let d = build(&t);
        let ka = d.get("KeepAlive").unwrap().as_dictionary().unwrap();
        assert_eq!(ka.len(), 1);
        assert!(!ka.get("SuccessfulExit").unwrap().as_boolean().unwrap());
        // 仍然没有任何触发键：启动只靠 kickstart（以及 bootstrap 时 launchd
        // 自己拉起，见 docs/M2.5-design.md §3.1）
        assert!(d.get("RunAtLoad").is_none());
        assert!(d.get("StartInterval").is_none());
        assert!(d.get("StartCalendarInterval").is_none());
    }

    /// M3 §3.1: an adopted task's plist keeps its original label and gains
    /// the marker that makes it writable again next time.
    #[test]
    fn an_adopted_task_gets_its_own_label_and_the_managed_marker() {
        let mut t = Task::new(
            TaskName::new("com.example.sync").unwrap(),
            "/usr/bin/true",
            Trigger::Interval { seconds: 60 },
        );
        t.adopted = true;
        t.plist_path = Some(PathBuf::from("/x/sync.plist"));
        let d = build(&t);
        assert_eq!(
            d.get("Label").unwrap().as_string().unwrap(),
            "com.example.sync"
        );
        assert!(d.get(MANAGED_KEY).unwrap().as_boolean().unwrap());
        // The runner still is the program, with the task name as argument —
        // which for an adopted task is the label.
        let args: Vec<&str> = d
            .get("ProgramArguments")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_string().unwrap())
            .collect();
        assert_eq!(args[1], "run");
        assert_eq!(args[2], "com.example.sync");

        // A normal task has no marker.
        assert!(build(&task(Trigger::AtLogin)).get(MANAGED_KEY).is_none());
    }

    #[test]
    fn ownership_is_decided_by_name_then_by_marker() {
        let dir = tempfile::tempdir().unwrap();

        // Nothing there yet: writable.
        assert!(is_ours(&dir.path().join("com.someone.else.plist")).unwrap());

        // Somebody else's plist: not ours.
        let foreign = dir.path().join("com.someone.else.plist");
        let mut d = Dictionary::new();
        d.insert("Label".into(), Value::String("com.someone.else".into()));
        plist::to_file_xml(&foreign, &Value::Dictionary(d.clone())).unwrap();
        assert!(!is_ours(&foreign).unwrap());

        // The same file with the marker: ours.
        d.insert(MANAGED_KEY.into(), Value::Boolean(true));
        plist::to_file_xml(&foreign, &Value::Dictionary(d.clone())).unwrap();
        assert!(is_ours(&foreign).unwrap());

        // Marker present but false is not a permission slip.
        d.insert(MANAGED_KEY.into(), Value::Boolean(false));
        plist::to_file_xml(&foreign, &Value::Dictionary(d)).unwrap();
        assert!(!is_ours(&foreign).unwrap());

        // Our own name needs no marker, and is decided without reading.
        let ours = dir.path().join("com.launchkeeper.sync.plist");
        std::fs::write(&ours, "not even a plist").unwrap();
        assert!(is_ours(&ours).unwrap());

        // An unparseable file under a foreign name is emphatically not ours.
        let junk = dir.path().join("com.someone.junk.plist");
        std::fs::write(&junk, "not even a plist").unwrap();
        assert!(!is_ours(&junk).unwrap());
    }

    #[test]
    fn write_plist_refuses_to_clobber_a_foreign_plist() {
        let dir = tempfile::tempdir().unwrap();
        let (runner, log_dir) = opts();
        let runner_log = log_dir.join("runner.log");
        let o = PlistOptions {
            runner_path: &runner,
            runner_log: &runner_log,
            data_dir: Path::new("/tmp/lk-data"),
        };

        let foreign = dir.path().join("com.someone.else.plist");
        let original = "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict>\
             <key>Label</key><string>com.someone.else</string></dict></plist>";
        std::fs::write(&foreign, original).unwrap();

        let mut adopted = Task::new(
            TaskName::new("com.someone.else").unwrap(),
            "/usr/bin/true",
            Trigger::Manual,
        );
        adopted.adopted = true;
        adopted.plist_path = Some(foreign.clone());

        let err = write_plist(&adopted, &o, &foreign).unwrap_err();
        assert!(matches!(err, Error::PlistNotOurs(_)), "got {err:?}");
        assert_eq!(std::fs::read_to_string(&foreign).unwrap(), original);

        // The adoption path may write it — that is the one exception, and it
        // leaves the marker behind, so the checked path works from then on.
        write_plist_adopting(&adopted, &o, &foreign).unwrap();
        assert!(is_ours(&foreign).unwrap());
        write_plist(&adopted, &o, &foreign).unwrap();
    }

    #[test]
    fn write_plist_is_atomic_and_reparses() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("com.launchkeeper.sync.plist");
        let (runner, log_dir) = opts();
        let t = task(Trigger::Interval { seconds: 60 });
        let o = PlistOptions {
            runner_path: &runner,
            runner_log: &log_dir.join("runner.log"),
            data_dir: Path::new("/tmp/lk-data"),
        };
        write_plist(&t, &o, &dest).unwrap();
        write_plist(&t, &o, &dest).unwrap(); // 覆盖写也要成功
        let text = std::fs::read_to_string(&dest).unwrap();
        assert!(text.starts_with("<?xml"), "应当是 XML plist: {text}");
        let parsed = plist::Value::from_file(&dest).unwrap();
        let d = parsed.into_dictionary().unwrap();
        assert_eq!(
            d.get("Label").unwrap().as_string().unwrap(),
            "com.launchkeeper.sync"
        );
        // 没有留下临时文件
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n != "com.launchkeeper.sync.plist")
            .collect();
        assert!(leftovers.is_empty(), "残留文件: {leftovers:?}");
    }
}
