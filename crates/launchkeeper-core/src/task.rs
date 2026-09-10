//! The task model: names, triggers and the [`Task`] record itself.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Every plist Launchkeeper owns is labelled `com.launchkeeper.<name>`.
pub const LABEL_PREFIX: &str = "com.launchkeeper.";

/// Maximum length of a task name.
///
/// Raised from 64 to 128 in M3 (`docs/M3-design.md` §3.1): an adopted task's
/// name *is* the original launchd label, and reverse-DNS labels get long.
pub const MAX_NAME_LEN: usize = 128;

/// A validated task name: `^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$`.
///
/// M3 relaxed this from the M1 rule (`^[a-z0-9][a-z0-9.-]{0,63}$` with no
/// empty dot segment) so that a whole launchd label — `com.example.Sync_2`
/// — can be a task name: adopting a plist in place keeps its label, and the
/// label is what the runner looks the task up by. Every name that was legal
/// before is still legal, so no stored task and no script that types a name
/// is affected.
///
/// No `/` and no leading `.` are accepted, so a name can never escape the
/// directory it is joined onto (log dir, plist file name).
///
/// The `test.` segment is reserved for integration tests; no other reserved
/// namespace exists today.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct TaskName(String);

impl TaskName {
    /// Validates `s` and returns the name.
    ///
    /// # Errors
    /// [`Error::InvalidName`] when the name is empty, too long, starts with a
    /// character other than `[A-Za-z0-9]`, or contains anything outside
    /// `[A-Za-z0-9._-]`.
    pub fn new(s: &str) -> Result<TaskName> {
        let bad = |m: &str| Error::InvalidName(format!("{s:?}: {m}"));
        if s.is_empty() {
            return Err(bad("不能为空"));
        }
        if s.len() > MAX_NAME_LEN {
            return Err(bad(&format!("长度超过 {MAX_NAME_LEN}")));
        }
        let first = s.as_bytes()[0];
        if !first.is_ascii_alphanumeric() {
            return Err(bad("必须以字母或数字开头"));
        }
        for c in s.chars() {
            let ok = c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_';
            if !ok {
                return Err(bad(&format!("含非法字符 {c:?}，只允许 A-Z a-z 0-9 . - _")));
            }
        }
        Ok(TaskName(s.to_string()))
    }

    /// The bare name, without the label prefix.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The launchd label Launchkeeper would give a task of this name:
    /// `com.launchkeeper.<name>`.
    ///
    /// Not the right thing to call for a task that might be adopted — an
    /// adopted task keeps the label it already had. Use [`Task::label`].
    pub fn label(&self) -> String {
        format!("{LABEL_PREFIX}{}", self.0)
    }
}

impl fmt::Display for TaskName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for TaskName {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        TaskName::new(&s).map_err(serde::de::Error::custom)
    }
}

/// One `StartCalendarInterval` entry. Absent fields mean "every".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct CalendarEntry {
    /// Minute, 0-59.
    pub minute: u8,
    /// Hour, 0-23.
    pub hour: u8,
    /// Weekday 0-7; both 0 and 7 mean Sunday (launchd semantics).
    pub weekday: Option<u8>,
    /// Day of month, 1-31.
    pub day: Option<u8>,
}

impl CalendarEntry {
    /// A plain daily entry at `hour:minute`.
    pub fn daily(hour: u8, minute: u8) -> CalendarEntry {
        CalendarEntry {
            minute,
            hour,
            weekday: None,
            day: None,
        }
    }
}

/// When a task should run: the three shapes launchd schedules, plus
/// [`Trigger::Manual`] for a long-running service the user starts and stops
/// by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    /// `RunAtLoad`: fires when the agent is loaded (i.e. at login).
    AtLogin,
    /// `StartInterval`: every `seconds` seconds.
    Interval {
        /// Interval in seconds, at least 1.
        seconds: u32,
    },
    /// `StartCalendarInterval`: one or more wall-clock rules.
    Calendar {
        /// Non-empty list of calendar rules.
        entries: Vec<CalendarEntry>,
    },
    /// No automatic trigger at all: the plist carries none of `RunAtLoad` /
    /// `StartInterval` / `StartCalendarInterval`, and the job only ever runs
    /// because someone called [`crate::Service::start`] (i.e. `launchctl
    /// kickstart`).
    ///
    /// Combined with `keep_alive` this is a service: launchd writes
    /// `KeepAlive = {SuccessfulExit = false}`, which — measured on real
    /// launchd, see `docs/M2.5-design.md` §3.1 — also makes `bootstrap`
    /// start the job immediately.
    Manual,
}

const WEEKDAYS: [&str; 7] = ["日", "一", "二", "三", "四", "五", "六"];

fn weekday_name(w: u8) -> &'static str {
    WEEKDAYS[(w % 7) as usize]
}

impl Trigger {
    /// Checks field ranges and non-emptiness.
    ///
    /// # Errors
    /// [`Error::InvalidTrigger`] with a human-readable reason.
    pub fn validate(&self) -> Result<()> {
        let bad = |m: String| Err(Error::InvalidTrigger(m));
        match self {
            Trigger::AtLogin | Trigger::Manual => Ok(()),
            Trigger::Interval { seconds } => {
                if *seconds < 1 {
                    return bad("间隔秒数必须 >= 1".into());
                }
                Ok(())
            }
            Trigger::Calendar { entries } => {
                if entries.is_empty() {
                    return bad("日历触发至少要有一条规则".into());
                }
                for e in entries {
                    if e.minute > 59 {
                        return bad(format!("minute 必须在 0-59，收到 {}", e.minute));
                    }
                    if e.hour > 23 {
                        return bad(format!("hour 必须在 0-23，收到 {}", e.hour));
                    }
                    if let Some(w) = e.weekday
                        && w > 7
                    {
                        return bad(format!("weekday 必须在 0-7，收到 {w}"));
                    }
                    if let Some(d) = e.day
                        && !(1..=31).contains(&d)
                    {
                        return bad(format!("day 必须在 1-31，收到 {d}"));
                    }
                }
                Ok(())
            }
        }
    }

    /// A short Chinese description, e.g. `每天 21:00`, `每周一、五 09:30`,
    /// `每月 1 日 08:00`, `每 30 分钟`, `登录时`, `手动启停`.
    pub fn describe(&self) -> String {
        match self {
            Trigger::AtLogin => "登录时".to_string(),
            Trigger::Manual => "手动启停".to_string(),
            Trigger::Interval { seconds } => {
                let s = *seconds;
                if s >= 86400 && s % 86400 == 0 {
                    format!("每 {} 天", s / 86400)
                } else if s >= 3600 && s % 3600 == 0 {
                    format!("每 {} 小时", s / 3600)
                } else if s >= 60 && s % 60 == 0 {
                    format!("每 {} 分钟", s / 60)
                } else {
                    format!("每 {s} 秒")
                }
            }
            Trigger::Calendar { entries } => describe_calendar(entries),
        }
    }
}

fn hhmm(e: &CalendarEntry) -> String {
    format!("{:02}:{:02}", e.hour, e.minute)
}

fn describe_calendar(entries: &[CalendarEntry]) -> String {
    if entries.is_empty() {
        return "日历（无规则）".to_string();
    }
    let mut parts: Vec<String> = Vec::new();

    // 每天：weekday 和 day 都为空，按时间聚合
    let mut daily: Vec<String> = entries
        .iter()
        .filter(|e| e.weekday.is_none() && e.day.is_none())
        .map(hhmm)
        .collect();
    // 先排序再去重，否则只有相邻的重复时间会被合并
    daily.sort();
    daily.dedup();
    if !daily.is_empty() {
        parts.push(format!("每天 {}", daily.join("、")));
    }

    // 每周：同一时间的多个 weekday 合并
    let mut weekly: Vec<(String, Vec<u8>)> = Vec::new();
    for e in entries
        .iter()
        .filter(|e| e.weekday.is_some() && e.day.is_none())
    {
        let t = hhmm(e);
        let w = e.weekday.unwrap();
        match weekly.iter_mut().find(|(k, _)| *k == t) {
            Some((_, ws)) => {
                if !ws.iter().any(|x| x % 7 == w % 7) {
                    ws.push(w);
                }
            }
            None => weekly.push((t, vec![w])),
        }
    }
    for (t, mut ws) in weekly {
        ws.sort_by_key(|w| w % 7);
        let names: Vec<&str> = ws.iter().map(|w| weekday_name(*w)).collect();
        parts.push(format!("每周{} {t}", names.join("、")));
    }

    // 每月：同一时间的多个 day 合并
    let mut monthly: Vec<(String, Vec<u8>)> = Vec::new();
    for e in entries
        .iter()
        .filter(|e| e.day.is_some() && e.weekday.is_none())
    {
        let t = hhmm(e);
        let d = e.day.unwrap();
        match monthly.iter_mut().find(|(k, _)| *k == t) {
            Some((_, ds)) => {
                if !ds.contains(&d) {
                    ds.push(d);
                }
            }
            None => monthly.push((t, vec![d])),
        }
    }
    for (t, mut ds) in monthly {
        ds.sort_unstable();
        let names: Vec<String> = ds.iter().map(|d| d.to_string()).collect();
        parts.push(format!("每月 {} 日 {t}", names.join("、")));
    }

    // weekday + day 同时指定：launchd 要求两者都匹配，单独描述
    for e in entries
        .iter()
        .filter(|e| e.weekday.is_some() && e.day.is_some())
    {
        parts.push(format!(
            "每月 {} 日且周{} {}",
            e.day.unwrap(),
            weekday_name(e.weekday.unwrap()),
            hhmm(e)
        ));
    }

    parts.join("，")
}

/// A Launchkeeper task. The database is the source of truth for everything
/// here except "is it loaded", which belongs to launchd.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// Unique name; drives the launchd label and the log directory.
    pub name: TaskName,
    /// Human-facing name shown in lists.
    pub display_name: String,
    /// Optional longer description.
    pub description: Option<String>,
    /// Absolute path to the executable or script the runner will start.
    pub script_path: PathBuf,
    /// Arguments passed to `script_path`.
    pub args: Vec<String>,
    /// Working directory; the runner falls back to `$HOME` when `None`.
    pub working_dir: Option<PathBuf>,
    /// Extra environment variables, applied on top of the inherited env.
    pub env: BTreeMap<String, String>,
    /// When the task runs.
    pub trigger: Trigger,
    /// Maps to launchd `KeepAlive`. Only meaningful together with
    /// [`Trigger::AtLogin`]; see [`Task::validate`].
    pub keep_alive: bool,
    /// Wall-clock limit for one run, in seconds. `None` means no limit.
    /// The runner enforces it and reports exit code 124 on expiry.
    pub timeout_secs: Option<u32>,
    /// Free-form tags for filtering.
    pub tags: Vec<String>,
    /// Pinned to the menu bar.
    pub favorite: bool,
    /// Post a macOS notification when a run fails.
    pub notify_on_fail: bool,
    /// Creation timestamp (UTC).
    pub created_at: DateTime<Utc>,
    /// Last modification timestamp (UTC).
    pub updated_at: DateTime<Utc>,
    /// True when this task was adopted in place from a plist somebody else
    /// wrote (`docs/M3-design.md` §3): its [`Task::label`] is `name` verbatim
    /// instead of `com.launchkeeper.<name>`, and its plist lives at
    /// [`Task::plist_path`] instead of the path the label would imply.
    pub adopted: bool,
    /// Where the adopted plist lives. Only meaningful with `adopted`; `None`
    /// falls back to the usual `<launch_agents_dir>/<label>.plist`.
    pub plist_path: Option<PathBuf>,
}

impl Task {
    /// A task with sane defaults: `display_name` = name, no args, no env,
    /// `notify_on_fail` on, both timestamps set to now.
    pub fn new(name: TaskName, script_path: impl Into<PathBuf>, trigger: Trigger) -> Task {
        let now = Utc::now();
        Task {
            display_name: name.as_str().to_string(),
            name,
            description: None,
            script_path: script_path.into(),
            args: Vec::new(),
            working_dir: None,
            env: BTreeMap::new(),
            trigger,
            keep_alive: false,
            timeout_secs: None,
            tags: Vec::new(),
            favorite: false,
            notify_on_fail: true,
            created_at: now,
            updated_at: now,
            adopted: false,
            plist_path: None,
        }
    }

    /// The launchd label of this task: the name verbatim when the task was
    /// [adopted](Task::adopted) in place (it keeps the label its plist
    /// already had), `com.launchkeeper.<name>` otherwise.
    pub fn label(&self) -> String {
        if self.adopted {
            self.name.as_str().to_string()
        } else {
            self.name.label()
        }
    }

    /// True when this task is a long-running service the user starts and
    /// stops by hand, i.e. its trigger is [`Trigger::Manual`].
    ///
    /// `AtLogin` + `keep_alive` is also a resident process, but it has an
    /// automatic trigger and is presented as a scheduled task; only `Manual`
    /// gets the start/stop/restart affordances.
    pub fn is_service(&self) -> bool {
        matches!(self.trigger, Trigger::Manual)
    }

    /// Validates the parts of the task that have rules: the trigger, that
    /// `script_path` is absolute, that `timeout_secs` (when set) is at least
    /// 1, and that `keep_alive` is only combined with [`Trigger::AtLogin`] or
    /// [`Trigger::Manual`].
    ///
    /// # Errors
    /// [`Error::InvalidTrigger`] or [`Error::InvalidTask`].
    pub fn validate(&self) -> Result<()> {
        self.trigger.validate()?;
        if !self.script_path.is_absolute() {
            return Err(Error::InvalidTask(format!(
                "script_path 必须是绝对路径: {}",
                self.script_path.display()
            )));
        }
        // `script_path` being absolute is not enough when it *is* an
        // interpreter: `python3 scripts/sync.py` passes the check above and
        // still cannot work, because the relative script is resolved against
        // the working directory — and with no `working_dir` that is `$HOME`,
        // which is essentially never where the script lives. With a
        // `working_dir` the same command is perfectly well defined (it is
        // how the acceptance task runs `uv run scripts/…`), so this
        // only refuses the combination that cannot be made to work.
        if self.working_dir.is_none()
            && let Some(d) = crate::interpreters::detect_from_task(&self.script_path, &self.args)
            && d.interpreter.kind != crate::interpreters::InterpreterKind::Direct
            && !d.script.is_absolute()
        {
            return Err(Error::InvalidTask(format!(
                "解释器 {} 后面的脚本是相对路径 {}，而任务没有工作目录：\
                 launchd 会在 $HOME 里找它。请给绝对路径，或者设置工作目录",
                self.script_path.display(),
                d.script.display()
            )));
        }
        if self.keep_alive && !matches!(self.trigger, Trigger::AtLogin | Trigger::Manual) {
            return Err(Error::InvalidTask(
                "keep_alive 表示“常驻，退出即重启”，只能和 --at-login 或 --manual 一起用；\
                 定时触发（--every / --daily / --weekly / --monthly）下 launchd 会在脚本退出后\
                 立刻重启它，定时规则就失效了"
                    .to_string(),
            ));
        }
        if self.timeout_secs == Some(0) {
            return Err(Error::InvalidTask("timeout_secs 必须 >= 1".to_string()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_reasonable_names() {
        for good in [
            "a",
            "0",
            "sync",
            "nightly-report-sync",
            "team.backup.nightly",
            "test.1e2f",
            &"a".repeat(MAX_NAME_LEN),
        ] {
            assert!(TaskName::new(good).is_ok(), "should accept {good:?}");
        }
    }

    /// M3 §3.1: a whole launchd label has to be a legal name, because an
    /// adopted task's name *is* its label.
    #[test]
    fn accepts_whole_launchd_labels() {
        for good in [
            "com.example.nightly-report-sync",
            "com.launchkeeper.sync",
            "Upper",
            "under_score",
            "org.Mozilla.Updater_2",
            "trailing.",
            "double..dot",
            &"a".repeat(128),
        ] {
            assert!(TaskName::new(good).is_ok(), "should accept {good:?}");
        }
    }

    #[test]
    fn rejects_bad_names() {
        for bad in [
            "",
            "-lead",
            ".lead",
            "_lead",
            "has space",
            "slash/path",
            "../escape",
            "中文",
            &"a".repeat(MAX_NAME_LEN + 1),
        ] {
            let err = TaskName::new(bad).unwrap_err();
            assert!(
                matches!(err, Error::InvalidName(_)),
                "should reject {bad:?}, got {err:?}"
            );
        }
    }

    #[test]
    fn an_adopted_task_keeps_its_own_label_and_plist() {
        let name = TaskName::new("com.example.sync").unwrap();
        let mut t = Task::new(name, "/usr/bin/true", Trigger::AtLogin);
        assert_eq!(t.label(), "com.launchkeeper.com.example.sync");
        assert!(!t.adopted);

        t.adopted = true;
        t.plist_path = Some(PathBuf::from("/x/sync.plist"));
        assert_eq!(t.label(), "com.example.sync");
    }

    #[test]
    fn label_uses_prefix() {
        assert_eq!(
            TaskName::new("sync").unwrap().label(),
            "com.launchkeeper.sync"
        );
    }

    #[test]
    fn trigger_json_shapes_match_the_contract() {
        let cases = [
            (Trigger::AtLogin, r#"{"kind":"at_login"}"#),
            (
                Trigger::Interval { seconds: 1800 },
                r#"{"kind":"interval","seconds":1800}"#,
            ),
            (
                Trigger::Calendar {
                    entries: vec![CalendarEntry::daily(21, 0)],
                },
                r#"{"kind":"calendar","entries":[{"minute":0,"hour":21,"weekday":null,"day":null}]}"#,
            ),
            (Trigger::Manual, r#"{"kind":"manual"}"#),
        ];
        for (trigger, json) in cases {
            assert_eq!(serde_json::to_string(&trigger).unwrap(), json);
            let back: Trigger = serde_json::from_str(json).unwrap();
            assert_eq!(back, trigger);
        }
    }

    #[test]
    fn trigger_validation() {
        assert!(Trigger::AtLogin.validate().is_ok());
        assert!(Trigger::Manual.validate().is_ok());
        assert!(Trigger::Interval { seconds: 1 }.validate().is_ok());
        assert!(Trigger::Interval { seconds: 0 }.validate().is_err());
        assert!(
            Trigger::Calendar {
                entries: Vec::new()
            }
            .validate()
            .is_err()
        );
        assert!(
            Trigger::Calendar {
                entries: vec![CalendarEntry::daily(21, 0)]
            }
            .validate()
            .is_ok()
        );
        for bad in [
            CalendarEntry {
                minute: 60,
                hour: 1,
                weekday: None,
                day: None,
            },
            CalendarEntry {
                minute: 0,
                hour: 24,
                weekday: None,
                day: None,
            },
            CalendarEntry {
                minute: 0,
                hour: 1,
                weekday: Some(8),
                day: None,
            },
            CalendarEntry {
                minute: 0,
                hour: 1,
                weekday: None,
                day: Some(0),
            },
            CalendarEntry {
                minute: 0,
                hour: 1,
                weekday: None,
                day: Some(32),
            },
        ] {
            let t = Trigger::Calendar { entries: vec![bad] };
            assert!(matches!(
                t.validate().unwrap_err(),
                Error::InvalidTrigger(_)
            ));
        }
        // launchd 允许 weekday 0 和 7 都表示周日
        for w in [0u8, 7] {
            let t = Trigger::Calendar {
                entries: vec![CalendarEntry {
                    minute: 0,
                    hour: 9,
                    weekday: Some(w),
                    day: None,
                }],
            };
            assert!(t.validate().is_ok());
        }
    }

    fn cal(entries: Vec<CalendarEntry>) -> Trigger {
        Trigger::Calendar { entries }
    }

    #[test]
    fn describe_in_chinese() {
        assert_eq!(Trigger::AtLogin.describe(), "登录时");
        assert_eq!(Trigger::Manual.describe(), "手动启停");
        assert_eq!(Trigger::Interval { seconds: 90 }.describe(), "每 90 秒");
        assert_eq!(Trigger::Interval { seconds: 1800 }.describe(), "每 30 分钟");
        assert_eq!(Trigger::Interval { seconds: 7200 }.describe(), "每 2 小时");
        assert_eq!(Trigger::Interval { seconds: 86400 }.describe(), "每 1 天");

        assert_eq!(
            cal(vec![CalendarEntry::daily(21, 0)]).describe(),
            "每天 21:00"
        );
        assert_eq!(
            cal(vec![
                CalendarEntry::daily(9, 0),
                CalendarEntry::daily(21, 30)
            ])
            .describe(),
            "每天 09:00、21:30"
        );
        assert_eq!(
            cal(vec![
                CalendarEntry {
                    minute: 30,
                    hour: 9,
                    weekday: Some(1),
                    day: None
                },
                CalendarEntry {
                    minute: 30,
                    hour: 9,
                    weekday: Some(5),
                    day: None
                },
            ])
            .describe(),
            "每周一、五 09:30"
        );
        assert_eq!(
            cal(vec![CalendarEntry {
                minute: 0,
                hour: 8,
                weekday: None,
                day: Some(1)
            }])
            .describe(),
            "每月 1 日 08:00"
        );
        assert_eq!(
            cal(vec![CalendarEntry {
                minute: 0,
                hour: 8,
                weekday: Some(7),
                day: None
            }])
            .describe(),
            "每周日 08:00"
        );
    }

    #[test]
    fn daily_dedups_non_consecutive_times() {
        assert_eq!(
            cal(vec![
                CalendarEntry::daily(21, 0),
                CalendarEntry::daily(9, 0),
                CalendarEntry::daily(21, 0),
            ])
            .describe(),
            "每天 09:00、21:00"
        );
    }

    #[test]
    fn keep_alive_only_with_at_login_or_manual() {
        let mut t = Task::new(
            TaskName::new("sync").unwrap(),
            "/usr/bin/true",
            Trigger::AtLogin,
        );
        t.keep_alive = true;
        assert!(t.validate().is_ok());

        t.trigger = Trigger::Manual;
        assert!(t.validate().is_ok(), "manual + keep_alive 就是服务型任务");

        for trigger in [
            Trigger::Interval { seconds: 60 },
            Trigger::Calendar {
                entries: vec![CalendarEntry::daily(21, 0)],
            },
        ] {
            t.trigger = trigger;
            let err = t.validate().unwrap_err();
            assert!(matches!(err, Error::InvalidTask(_)), "got {err:?}");
            assert!(err.to_string().contains("常驻"));
        }
    }

    #[test]
    fn manual_without_keep_alive_is_still_valid() {
        let t = Task::new(
            TaskName::new("bridge").unwrap(),
            "/usr/bin/true",
            Trigger::Manual,
        );
        assert!(!t.keep_alive);
        assert!(t.validate().is_ok());
    }

    #[test]
    fn only_manual_is_a_service() {
        let name = TaskName::new("x").unwrap();
        let service = Task::new(name.clone(), "/usr/bin/true", Trigger::Manual);
        assert!(service.is_service());

        for trigger in [
            Trigger::AtLogin,
            Trigger::Interval { seconds: 60 },
            Trigger::Calendar {
                entries: vec![CalendarEntry::daily(21, 0)],
            },
        ] {
            let t = Task::new(name.clone(), "/usr/bin/true", trigger);
            assert!(!t.is_service(), "{:?} 不是服务型任务", t.trigger);
        }
    }

    #[test]
    fn timeout_zero_is_rejected() {
        let mut t = Task::new(
            TaskName::new("sync").unwrap(),
            "/usr/bin/true",
            Trigger::AtLogin,
        );
        assert!(t.validate().is_ok());
        t.timeout_secs = Some(0);
        assert!(matches!(t.validate().unwrap_err(), Error::InvalidTask(_)));
        t.timeout_secs = Some(1);
        assert!(t.validate().is_ok());
    }

    #[test]
    fn task_validate_requires_absolute_script() {
        let name = TaskName::new("sync").unwrap();
        let mut t = Task::new(name, "relative.sh", Trigger::AtLogin);
        assert!(matches!(t.validate().unwrap_err(), Error::InvalidTask(_)));
        t.script_path = "/usr/bin/true".into();
        assert!(t.validate().is_ok());
    }

    /// An absolute `script_path` that is itself an interpreter hides a
    /// second path in `args`, and that one has to be resolvable too.
    #[test]
    fn a_relative_script_behind_an_interpreter_needs_a_working_dir() {
        let name = TaskName::new("sync").unwrap();
        let mut t = Task::new(name, "/usr/bin/python3", Trigger::AtLogin);
        t.args = vec!["scripts/sync.py".to_string()];
        let err = t.validate().unwrap_err();
        assert!(matches!(err, Error::InvalidTask(_)), "got {err:?}");
        assert!(err.to_string().contains("相对路径"), "{err}");

        // A working directory makes it well defined again — this is how the
        // acceptance task runs `uv run scripts/sync_cloud.py`.
        t.working_dir = Some(PathBuf::from("/p/report"));
        assert!(t.validate().is_ok());

        // An absolute one never needed the working directory.
        t.working_dir = None;
        t.args = vec!["/p/report/scripts/sync.py".to_string()];
        assert!(t.validate().is_ok());

        // And a plain script that merely *starts* in a relative-looking way
        // is not an interpreter invocation at all.
        let mut direct = Task::new(
            TaskName::new("direct").unwrap(),
            "/p/backup.sh",
            Trigger::AtLogin,
        );
        direct.args = vec!["relative/thing".to_string()];
        assert!(direct.validate().is_ok());
    }
}
