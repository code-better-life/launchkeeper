//! Reading — and taking over — LaunchAgents that Launchkeeper did not write.
//!
//! Contract: `docs/M3-design.md` §3.2.
//!
//! Two things happen here and nowhere else: turning somebody else's plist
//! into an [`ExternalAgent`] (a read-only view, with an honest verdict on
//! whether Launchkeeper could manage it), and computing the [`AdoptionPlan`]
//! — the exact key-by-key diff that adoption would apply — so that no user
//! ever has to accept a rewrite of their own file sight unseen.
//!
//! Nothing in this module writes anything. [`crate::Service::adopt`] does the
//! writing, and only after copying the original to `<plist>.bak`.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use plist::{Dictionary, Value};

use crate::error::{Error, Result};
use crate::launchctl;
use crate::paths;
use crate::plist::{MANAGED_KEY, PlistOptions, build_plist};
use crate::task::{CalendarEntry, LABEL_PREFIX, Task, TaskName, Trigger};

/// Program-path prefixes that mean "this belongs to something else".
///
/// `/Library` catches `/Library/Application Support/...` helpers without
/// catching `~/Library/...`, which is where a user's own scripts routinely
/// live.
const FORBIDDEN_PROGRAM_PREFIXES: &[&str] = &[
    "/Applications",
    "/System",
    "/Library",
    "/opt/homebrew/Cellar",
    "/usr/local/Cellar",
];

/// Label prefixes owned by Apple and by Homebrew's service manager.
const FORBIDDEN_LABEL_PREFIXES: &[&str] = &["com.apple.", "homebrew.mxcl."];

/// launchd keys whose whole point is *when* (or *how often*) the job runs and
/// which Launchkeeper has no model for. Rewriting such a plist would silently
/// drop the only reason it exists.
///
/// `StartOnMount` (run whenever a filesystem is mounted) and `LaunchOnlyOnce`
/// (never run again for the life of this load) are here for the same reason
/// as `WatchPaths`: the generated plist would carry neither, so the job would
/// quietly become something else.
const UNSUPPORTED_KEYS: &[&str] = &[
    "WatchPaths",
    "QueueDirectories",
    "Sockets",
    "MachServices",
    "LaunchEvents",
    "inetdCompatibility",
    "StartOnMount",
    "LaunchOnlyOnce",
];

/// Whether Launchkeeper could take a LaunchAgent over, and if not, why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Adoptable {
    /// Adoption would work.
    Yes,
    /// Adoption is refused; the string is a human-readable reason.
    No(String),
}

impl Adoptable {
    /// True for [`Adoptable::Yes`].
    pub fn is_yes(&self) -> bool {
        matches!(self, Adoptable::Yes)
    }

    /// The refusal reason, or `None` when adoption is possible.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Adoptable::Yes => None,
            Adoptable::No(r) => Some(r),
        }
    }
}

/// A LaunchAgent plist Launchkeeper did not create, as far as it can be
/// understood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalAgent {
    /// The `Label` key, or the file stem when the plist has none.
    pub label: String,
    /// Absolute path of the plist file.
    pub path: PathBuf,
    /// The executable launchd would run, from `Program` or
    /// `ProgramArguments[0]`.
    pub program: Option<PathBuf>,
    /// Arguments after the program.
    pub args: Vec<String>,
    /// `WorkingDirectory`.
    pub working_dir: Option<PathBuf>,
    /// `EnvironmentVariables`.
    pub env: BTreeMap<String, String>,
    /// The trigger, when the plist's scheduling keys translate into
    /// Launchkeeper's model; `None` when they do not (in which case
    /// `adoptable` says so too).
    pub trigger: Option<Trigger>,
    /// `KeepAlive` in either of its shapes.
    pub keep_alive: bool,
    /// `StandardOutPath`, kept only to mention in the adopted task's
    /// description: Launchkeeper's own runner writes elsewhere.
    pub stdout_path: Option<PathBuf>,
    /// `StandardErrorPath`, same.
    pub stderr_path: Option<PathBuf>,
    /// launchd knows the label right now.
    pub loaded: bool,
    /// Pid, while the job is running.
    pub pid: Option<u32>,
    /// `last exit code` as launchd reports it.
    pub last_exit: Option<i32>,
    /// Whether Launchkeeper could take this over, and if not, why not.
    pub adoptable: Adoptable,
    /// The plist is already Launchkeeper's: it carries the
    /// [`MANAGED_KEY`] marker (an adopted task) or a
    /// `com.launchkeeper.*` label.
    pub managed_by_launchkeeper: bool,
}

impl ExternalAgent {
    /// The command line as a single string, for one-line display.
    pub fn command(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(p) = &self.program {
            parts.push(p.to_string_lossy().into_owned());
        }
        parts.extend(self.args.iter().cloned());
        parts.join(" ")
    }

    /// The trigger in Chinese, or `不支持的触发方式` when it did not
    /// translate.
    pub fn trigger_text(&self) -> String {
        match &self.trigger {
            Some(t) => t.describe(),
            None => "不支持的触发方式".to_string(),
        }
    }
}

/// `<plist>.bak`: where [`crate::Service::adopt`] copies the original before
/// rewriting it, and where [`crate::Service::unadopt`] restores it from.
///
/// The suffix is appended to the whole file name rather than replacing the
/// extension, so `com.x.plist` backs up to `com.x.plist.bak` and launchd —
/// which only ever reads `*.plist` — ignores it.
pub fn backup_path(plist: &Path) -> PathBuf {
    let mut name = plist.file_name().unwrap_or_default().to_os_string();
    name.push(".bak");
    plist.with_file_name(name)
}

fn as_string(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_string).map(str::to_string)
}

/// Parses one plist into an [`ExternalAgent`], including launchd's current
/// view of the label.
///
/// # Errors
/// [`Error::Io`] when the file cannot be read and [`Error::Plist`] when it is
/// not a plist dictionary. Everything else — a missing `Program`, an
/// untranslatable trigger, a label Apple owns — is reported through
/// [`ExternalAgent::adoptable`] rather than as an error, because the caller
/// still wants to *show* the agent.
pub fn parse_agent(path: &Path) -> Result<ExternalAgent> {
    let value = plist::Value::from_file(path)?;
    let dict = value
        .into_dictionary()
        .ok_or_else(|| Error::Adopt(format!("{} 的顶层不是字典", path.display())))?;
    Ok(agent_from_dict(path, &dict))
}

fn agent_from_dict(path: &Path, dict: &Dictionary) -> ExternalAgent {
    let label = as_string(dict.get("Label")).unwrap_or_else(|| {
        path.file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    });

    let (program, args) = program_and_args(dict);
    let working_dir = as_string(dict.get("WorkingDirectory")).map(PathBuf::from);
    let env = env_map(dict);
    let translated = translate_trigger(dict);
    let (trigger, keep_alive) = match &translated {
        Ok((t, k)) => (Some(t.clone()), *k),
        Err(_) => (None, false),
    };

    let managed_by_launchkeeper = label.starts_with(LABEL_PREFIX)
        || dict
            .get(MANAGED_KEY)
            .and_then(Value::as_boolean)
            .unwrap_or(false);

    let status = launchctl::status(&label).unwrap_or(None);

    let mut agent = ExternalAgent {
        label,
        path: path.to_path_buf(),
        program,
        args,
        working_dir,
        env,
        trigger,
        keep_alive,
        stdout_path: as_string(dict.get("StandardOutPath")).map(PathBuf::from),
        stderr_path: as_string(dict.get("StandardErrorPath")).map(PathBuf::from),
        loaded: status.is_some(),
        pid: status.as_ref().and_then(|s| s.pid),
        last_exit: status.as_ref().and_then(|s| s.last_exit_status),
        adoptable: Adoptable::Yes,
        managed_by_launchkeeper,
    };
    agent.adoptable = match refusal(&agent, dict, translated.err()) {
        Some(reason) => Adoptable::No(reason),
        None => Adoptable::Yes,
    };
    agent
}

fn program_and_args(dict: &Dictionary) -> (Option<PathBuf>, Vec<String>) {
    let argv: Vec<String> = dict
        .get("ProgramArguments")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_string)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let program = as_string(dict.get("Program"));
    match (program, argv.split_first()) {
        // launchd: `Program` is the executable, `ProgramArguments` is argv
        // (whose first element is conventionally the program name again).
        (Some(p), Some((_, rest))) => (Some(PathBuf::from(p)), rest.to_vec()),
        (Some(p), None) => (Some(PathBuf::from(p)), Vec::new()),
        (None, Some((first, rest))) => (Some(PathBuf::from(first)), rest.to_vec()),
        (None, None) => (None, Vec::new()),
    }
}

fn env_map(dict: &Dictionary) -> BTreeMap<String, String> {
    dict.get("EnvironmentVariables")
        .and_then(Value::as_dictionary)
        .map(|d| {
            d.iter()
                .filter_map(|(k, v)| v.as_string().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// Translates the plist's scheduling keys into a [`Trigger`] plus
/// `keep_alive`, or explains why they cannot be translated.
///
/// `RunAtLoad` only decides the answer when it is alone: a plist with both
/// `RunAtLoad` and `StartInterval` is an interval job that also fires at
/// login, and the interval is the part that must survive. The "also fires at
/// login" half is *not* representable — `Trigger` holds exactly one rule —
/// so it is dropped, and [`adoption_plan`] lists `RunAtLoad` among the keys
/// the rewrite removes so that the user sees it go (`docs/M3-design.md`
/// §3.6).
///
/// `KeepAlive` maps onto the single `keep_alive` flag, which the generated
/// plist writes back as `true` (an `AtLogin` task) or `{SuccessfulExit =
/// false}` (a `Manual` service) — see [`crate::plist::build_plist`]. Anything
/// with a different meaning, `{SuccessfulExit = true}` included, is refused
/// rather than inverted.
fn translate_trigger(dict: &Dictionary) -> std::result::Result<(Trigger, bool), String> {
    // These keys *are* the trigger, and there is no Trigger variant for any
    // of them — so there is no honest answer, not even `Manual`.
    if let Some(reason) = unsupported_key(dict) {
        return Err(reason);
    }
    let keep_alive = match dict.get("KeepAlive") {
        None => false,
        Some(Value::Boolean(b)) => *b,
        Some(Value::Dictionary(d)) => {
            let extra: Vec<&str> = d
                .keys()
                .map(String::as_str)
                .filter(|k| *k != "SuccessfulExit")
                .collect();
            if !extra.is_empty() {
                return Err(format!(
                    "KeepAlive 字典里有不支持的键: {}（只支持 SuccessfulExit）",
                    extra.join(", ")
                ));
            }
            // `{SuccessfulExit = false}` is "restart it unless it exited 0",
            // which is exactly what `keep_alive` means here.
            // `{SuccessfulExit = true}` is the opposite — restart it *only*
            // after a clean exit — and the rewrite would turn it into the
            // former, i.e. change what the job does.
            match d.get("SuccessfulExit").and_then(Value::as_boolean) {
                Some(false) => true,
                Some(true) => {
                    return Err("KeepAlive = {SuccessfulExit = true}（只在成功退出后重启）\
                         没有对应的模型，Launchkeeper 只支持 SuccessfulExit = false"
                        .to_string());
                }
                None => {
                    return Err(
                        "KeepAlive 字典里的 SuccessfulExit 缺失或不是布尔值（只支持 \
                         SuccessfulExit = false）"
                            .to_string(),
                    );
                }
            }
        }
        Some(_) => return Err("KeepAlive 既不是布尔值也不是字典".to_string()),
    };

    let trigger = if let Some(v) = dict.get("StartCalendarInterval") {
        Trigger::Calendar {
            entries: calendar_entries(v)?,
        }
    } else if let Some(v) = dict.get("StartInterval") {
        let secs = v
            .as_signed_integer()
            .ok_or_else(|| "StartInterval 不是整数".to_string())?;
        let secs = u32::try_from(secs).map_err(|_| format!("StartInterval 超出范围: {secs}"))?;
        Trigger::Interval { seconds: secs }
    } else if dict
        .get("RunAtLoad")
        .and_then(Value::as_boolean)
        .unwrap_or(false)
    {
        Trigger::AtLogin
    } else {
        Trigger::Manual
    };
    Ok((trigger, keep_alive))
}

/// The first [`UNSUPPORTED_KEYS`] entry the plist carries, as a refusal
/// reason.
fn unsupported_key(dict: &Dictionary) -> Option<String> {
    UNSUPPORTED_KEYS
        .iter()
        .find(|k| dict.contains_key(k))
        .map(|k| format!("plist 使用了 {k}，Launchkeeper 没有对应的模型（接管会丢掉它）"))
}

fn calendar_entries(v: &Value) -> std::result::Result<Vec<CalendarEntry>, String> {
    let dicts: Vec<&Dictionary> = match v {
        Value::Dictionary(d) => vec![d],
        Value::Array(a) => a
            .iter()
            .map(|x| {
                x.as_dictionary()
                    .ok_or_else(|| "StartCalendarInterval 数组里有非字典项".to_string())
            })
            .collect::<std::result::Result<_, _>>()?,
        _ => return Err("StartCalendarInterval 既不是字典也不是数组".to_string()),
    };
    let mut out = Vec::with_capacity(dicts.len());
    for d in dicts {
        if d.contains_key("Month") {
            return Err("StartCalendarInterval 里有 Month，Launchkeeper 只支持到“每月某日”".into());
        }
        let field = |k: &str| -> std::result::Result<Option<u8>, String> {
            match d.get(k) {
                None => Ok(None),
                Some(v) => {
                    let n = v
                        .as_signed_integer()
                        .ok_or_else(|| format!("StartCalendarInterval.{k} 不是整数"))?;
                    u8::try_from(n)
                        .map(Some)
                        .map_err(|_| format!("StartCalendarInterval.{k} 超出范围: {n}"))
                }
            }
        };
        // launchd treats an absent field as "every": no Hour means every
        // hour. Launchkeeper's CalendarEntry cannot say that, and quietly
        // pinning it to 00 would change when the job runs.
        let minute = field("Minute")?.ok_or_else(|| {
            "StartCalendarInterval 没有 Minute（表示每分钟触发），无法表示".to_string()
        })?;
        let hour = field("Hour")?.ok_or_else(|| {
            "StartCalendarInterval 没有 Hour（表示每小时触发），无法表示".to_string()
        })?;
        out.push(CalendarEntry {
            minute,
            hour,
            weekday: field("Weekday")?,
            day: field("Day")?,
        });
    }
    if out.is_empty() {
        return Err("StartCalendarInterval 是空的".to_string());
    }
    Ok(out)
}

/// The single verdict function: `None` means adoptable.
fn refusal(
    agent: &ExternalAgent,
    dict: &Dictionary,
    trigger_error: Option<String>,
) -> Option<String> {
    if agent.managed_by_launchkeeper {
        return Some("已经由 Launchkeeper 管理".to_string());
    }
    for prefix in FORBIDDEN_LABEL_PREFIXES {
        if agent.label.starts_with(prefix) {
            return Some(format!("label 以 {prefix} 开头，属于系统或 Homebrew"));
        }
    }
    if let Some(reason) = unsupported_key(dict) {
        return Some(reason);
    }
    if let Some(reason) = symlink_refusal(&agent.path) {
        return Some(reason);
    }
    // launchd's own "off" switch. The generated plist does not carry
    // `Disabled`, so adopting one of these would drop the key *and*
    // bootstrap the job — the user would find a job they had turned off
    // running again, because they asked Launchkeeper to manage it.
    if dict
        .get("Disabled")
        .and_then(Value::as_boolean)
        .unwrap_or(false)
    {
        return Some("plist 里 Disabled = true，这个 job 是停用状态；请先启用它再接管".to_string());
    }
    let Some(program) = &agent.program else {
        return Some("plist 里既没有 Program 也没有 ProgramArguments".to_string());
    };
    if !program.is_absolute() {
        return Some(format!("程序路径不是绝对路径: {}", program.display()));
    }
    if let Some(reason) = forbidden_program(program) {
        return Some(reason);
    }
    match paths::launch_agents_dir() {
        Ok(dir) if same_dir(agent.path.parent(), &dir) => {}
        Ok(dir) => {
            return Some(format!(
                "plist 不在 {}（只接管用户自己的 LaunchAgents）",
                dir.display()
            ));
        }
        Err(e) => return Some(format!("无法确定 LaunchAgents 目录: {e}")),
    }
    if let Some(e) = trigger_error {
        return Some(e);
    }
    // The last word belongs to the model itself: whatever survives the checks
    // above still has to be a task `Task::validate` accepts, or adoption
    // would fail halfway through.
    task_for(agent).err()
}

/// A refusal when `path` is itself a symbolic link.
///
/// Adoption rewrites the file it is pointed at, and writing through a symlink
/// rewrites whatever is at the *other* end — a file outside
/// `~/Library/LaunchAgents`, quite possibly one under version control in
/// somebody's dotfiles repo. The `.bak` would land next to the link, not next
/// to the file that actually changed, so even the undo would be misleading.
fn symlink_refusal(path: &Path) -> Option<String> {
    let md = std::fs::symlink_metadata(path).ok()?;
    if !md.file_type().is_symlink() {
        return None;
    }
    let target = std::fs::read_link(path)
        .map(|t| t.display().to_string())
        .unwrap_or_else(|_| "（无法读取链接目标）".to_string());
    Some(format!(
        "plist 是一个符号链接，指向 {target}；接管会重写链接指向的那个文件，\
         备份却会落在链接旁边。请直接把真实文件放进 LaunchAgents 目录"
    ))
}

/// Whether `parent` is the same directory as `dir`.
///
/// Compared literally first and through [`Path::canonicalize`] second: on
/// macOS `/tmp` is a symlink to `/private/tmp` and `$TMPDIR` lives under
/// `/var/folders/...` (itself `/private/var/...`), so a plist that really *is*
/// in the configured directory can easily arrive spelled differently — and
/// refusing it with "不在 …目录" would be a lie. When either side cannot be
/// canonicalized (a path that no longer exists, a permission error), the raw
/// comparison is the answer rather than an error.
fn same_dir(parent: Option<&Path>, dir: &Path) -> bool {
    let Some(parent) = parent else {
        return false;
    };
    if parent == dir {
        return true;
    }
    match (parent.canonicalize(), dir.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn forbidden_program(program: &Path) -> Option<String> {
    let text = program.to_string_lossy();
    for prefix in FORBIDDEN_PROGRAM_PREFIXES {
        if text == *prefix || text.starts_with(&format!("{prefix}/")) {
            return Some(format!("程序在 {prefix} 下，属于系统或别的软件包"));
        }
    }
    let in_bundle = program.components().any(|c| match c {
        Component::Normal(s) => s.to_string_lossy().ends_with(".app"),
        _ => false,
    });
    if in_bundle {
        return Some(format!(
            "程序在 app bundle 里: {}（应由那个 app 自己管理）",
            program.display()
        ));
    }
    None
}

/// The [`Task`] adoption would store for `agent`.
///
/// The task's name **is** the label: an adopted plist keeps the label it had,
/// and the runner looks its row up by the name it is handed on the command
/// line, so the two have to be the same string.
///
/// The error is a human-readable reason, which is also what
/// [`ExternalAgent::adoptable`] reports.
fn task_for(agent: &ExternalAgent) -> std::result::Result<Task, String> {
    let name = TaskName::new(&agent.label).map_err(|e| e.to_string())?;
    let program = agent
        .program
        .clone()
        .ok_or_else(|| "没有程序路径".to_string())?;
    let trigger = agent
        .trigger
        .clone()
        .ok_or_else(|| "触发方式无法翻译".to_string())?;
    let mut task = Task::new(name, program, trigger);
    task.display_name = agent.label.clone();
    task.description = Some(adoption_note(agent));
    task.args = agent.args.clone();
    task.working_dir = agent.working_dir.clone();
    task.env = agent.env.clone();
    task.keep_alive = agent.keep_alive;
    task.adopted = true;
    task.plist_path = Some(agent.path.clone());
    task.validate().map_err(|e| e.to_string())?;
    Ok(task)
}

/// The description an adopted task starts with: where it came from and, when
/// the original redirected its output somewhere, where those files used to
/// be — Launchkeeper's runner captures output per run instead, so the old
/// paths stop growing and the user should be told once.
fn adoption_note(agent: &ExternalAgent) -> String {
    let mut note = format!("接管自 {}", agent.path.display());
    let mut logs: Vec<String> = Vec::new();
    for p in [&agent.stdout_path, &agent.stderr_path]
        .into_iter()
        .flatten()
    {
        let s = p.to_string_lossy().into_owned();
        if !logs.contains(&s) {
            logs.push(s);
        }
    }
    if !logs.is_empty() {
        note.push_str(&format!("；原日志在 {}（不再写入）", logs.join("、")));
    }
    note
}

/// The change adoption would make to one plist, key by key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptionPlan {
    /// The launchd label, which adoption does **not** change.
    pub label: String,
    /// The plist that would be rewritten.
    pub path: PathBuf,
    /// Where the original bytes would be copied first.
    pub backup_path: PathBuf,
    /// The task that would be stored in the database.
    pub task: Task,
    /// Keys the rewrite drops, and their current values. A key whose value
    /// merely *changes* appears here with its old value and in `added` with
    /// the new one.
    pub removed: Vec<(String, String)>,
    /// Keys the rewrite adds or changes, with their new values.
    pub added: Vec<(String, String)>,
    /// Keys that come through untouched, with their (identical) value.
    pub kept: Vec<(String, String)>,
}

/// Computes the diff adoption would apply to `agent`'s plist.
///
/// # Errors
/// [`Error::Adopt`] when the agent is not adoptable (the message is
/// [`ExternalAgent::adoptable`]'s reason), plus path errors.
pub fn adoption_plan(agent: &ExternalAgent, runner: &Path) -> Result<AdoptionPlan> {
    if let Adoptable::No(reason) = &agent.adoptable {
        return Err(Error::Adopt(format!("{}: {reason}", agent.label)));
    }
    let task = task_for(agent).map_err(Error::Adopt)?;
    let runner_log = paths::runner_log_path(&task.name)?;
    let data_dir = paths::data_dir()?;
    let after = build_plist(
        &task,
        &PlistOptions {
            runner_path: runner,
            runner_log: &runner_log,
            data_dir: &data_dir,
        },
    );
    let before = plist::Value::from_file(&agent.path)?
        .into_dictionary()
        .ok_or_else(|| Error::Adopt(format!("{} 的顶层不是字典", agent.path.display())))?;

    let mut removed = Vec::new();
    let mut added = Vec::new();
    let mut kept = Vec::new();
    let mut keys: Vec<String> = before.keys().cloned().collect();
    for k in after.keys() {
        if !before.contains_key(k) {
            keys.push(k.clone());
        }
    }
    keys.sort();
    for k in keys {
        match (before.get(&k), after.get(&k)) {
            (Some(old), None) => removed.push((k, value_text(old))),
            (None, Some(new)) => added.push((k, value_text(new))),
            (Some(old), Some(new)) if old == new => kept.push((k, value_text(new))),
            (Some(old), Some(new)) => {
                removed.push((k.clone(), value_text(old)));
                added.push((k, value_text(new)));
            }
            (None, None) => {}
        }
    }

    Ok(AdoptionPlan {
        label: agent.label.clone(),
        path: agent.path.clone(),
        backup_path: backup_path(&agent.path),
        task,
        removed,
        added,
        kept,
    })
}

/// One plist value as a single line, for the diff.
pub fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Real(r) => r.to_string(),
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(value_text).collect::<Vec<_>>().join(", ")
        ),
        Value::Dictionary(d) => format!(
            "{{{}}}",
            d.iter()
                .map(|(k, v)| format!("{k}={}", value_text(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => format!("{other:?}"),
    }
}

/// Every LaunchAgent in `dir` that Launchkeeper did not create itself,
/// sorted by label.
///
/// Plists named `com.launchkeeper.*` are skipped: those are ordinary
/// Launchkeeper tasks and belong in the normal list. Adopted plists — which
/// keep their original label and carry the [`MANAGED_KEY`] marker — *are*
/// listed, flagged [`ExternalAgent::managed_by_launchkeeper`], because from
/// this directory's point of view they are still somebody else's file that
/// Launchkeeper happens to drive.
///
/// A file that does not parse is reported as an unadoptable agent rather
/// than swallowed: a plist launchd cannot read is worth seeing.
///
/// # Errors
/// [`Error::Io`] when `dir` cannot be listed. A missing directory yields an
/// empty list.
pub fn list_external(dir: &Path) -> Result<Vec<ExternalAgent>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::io(dir, e)),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(dir, e))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".plist") || name.starts_with(LABEL_PREFIX) {
            continue;
        }
        if !path.is_file() {
            continue;
        }
        out.push(match parse_agent(&path) {
            Ok(a) => a,
            Err(e) => unreadable(&path, &e.to_string()),
        });
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    Ok(out)
}

fn unreadable(path: &Path, reason: &str) -> ExternalAgent {
    ExternalAgent {
        label: path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        path: path.to_path_buf(),
        program: None,
        args: Vec::new(),
        working_dir: None,
        env: BTreeMap::new(),
        trigger: None,
        keep_alive: false,
        stdout_path: None,
        stderr_path: None,
        loaded: false,
        pid: None,
        last_exit: None,
        adoptable: Adoptable::No(format!("plist 无法读取: {reason}")),
        managed_by_launchkeeper: false,
    }
}

/// The agent in `dir` whose label is `label`.
///
/// Looks the label up rather than assuming the file is called
/// `<label>.plist`: launchd matches on the `Label` key, and hand-written
/// plists are routinely named something shorter.
///
/// # Errors
/// [`Error::AgentNotFound`], plus whatever [`list_external`] returns.
pub fn find(dir: &Path, label: &str) -> Result<ExternalAgent> {
    list_external(dir)?
        .into_iter()
        .find(|a| a.label == label)
        .ok_or_else(|| Error::AgentNotFound(label.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(pairs: &[(&str, Value)]) -> Dictionary {
        let mut d = Dictionary::new();
        for (k, v) in pairs {
            d.insert((*k).to_string(), v.clone());
        }
        d
    }

    fn argv(items: &[&str]) -> Value {
        Value::Array(items.iter().map(|s| Value::String((*s).into())).collect())
    }

    #[test]
    fn backup_appends_to_the_whole_file_name() {
        assert_eq!(
            backup_path(Path::new("/a/com.x.y.plist")),
            PathBuf::from("/a/com.x.y.plist.bak")
        );
    }

    #[test]
    fn program_beats_argv_zero() {
        let (p, a) = program_and_args(&dict(&[
            ("Program", Value::String("/bin/sh".into())),
            ("ProgramArguments", argv(&["sh", "-c", "echo"])),
        ]));
        assert_eq!(p, Some(PathBuf::from("/bin/sh")));
        assert_eq!(a, vec!["-c", "echo"]);

        let (p, a) = program_and_args(&dict(&[("ProgramArguments", argv(&["/bin/echo", "hi"]))]));
        assert_eq!(p, Some(PathBuf::from("/bin/echo")));
        assert_eq!(a, vec!["hi"]);

        let (p, a) = program_and_args(&dict(&[("Program", Value::String("/bin/true".into()))]));
        assert_eq!(p, Some(PathBuf::from("/bin/true")));
        assert!(a.is_empty());

        let (p, _) = program_and_args(&Dictionary::new());
        assert_eq!(p, None);
    }

    #[test]
    fn interval_calendar_at_login_and_manual() {
        let (t, k) = translate_trigger(&dict(&[("StartInterval", Value::Integer(1800.into()))]))
            .expect("interval");
        assert_eq!(t, Trigger::Interval { seconds: 1800 });
        assert!(!k);

        let (t, _) = translate_trigger(&dict(&[("RunAtLoad", Value::Boolean(true))])).unwrap();
        assert_eq!(t, Trigger::AtLogin);

        let (t, _) = translate_trigger(&Dictionary::new()).unwrap();
        assert_eq!(t, Trigger::Manual);

        // RunAtLoad only decides when it is alone.
        let (t, _) = translate_trigger(&dict(&[
            ("RunAtLoad", Value::Boolean(true)),
            ("StartInterval", Value::Integer(60.into())),
        ]))
        .unwrap();
        assert_eq!(t, Trigger::Interval { seconds: 60 });
    }

    #[test]
    fn calendar_dict_and_array() {
        let one = dict(&[(
            "StartCalendarInterval",
            Value::Dictionary(dict(&[
                ("Hour", Value::Integer(21.into())),
                ("Minute", Value::Integer(0.into())),
            ])),
        )]);
        let (t, _) = translate_trigger(&one).unwrap();
        assert_eq!(
            t,
            Trigger::Calendar {
                entries: vec![CalendarEntry::daily(21, 0)]
            }
        );

        let many = dict(&[(
            "StartCalendarInterval",
            Value::Array(vec![
                Value::Dictionary(dict(&[
                    ("Hour", Value::Integer(9.into())),
                    ("Minute", Value::Integer(30.into())),
                    ("Weekday", Value::Integer(1.into())),
                ])),
                Value::Dictionary(dict(&[
                    ("Hour", Value::Integer(8.into())),
                    ("Minute", Value::Integer(0.into())),
                    ("Day", Value::Integer(1.into())),
                ])),
            ]),
        )]);
        let (t, _) = translate_trigger(&many).unwrap();
        let Trigger::Calendar { entries } = t else {
            panic!("应当是日历触发")
        };
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].weekday, Some(1));
        assert_eq!(entries[1].day, Some(1));
    }

    #[test]
    fn calendar_rejects_month_and_missing_fields() {
        for bad in [
            dict(&[(
                "StartCalendarInterval",
                Value::Dictionary(dict(&[
                    ("Month", Value::Integer(3.into())),
                    ("Hour", Value::Integer(1.into())),
                    ("Minute", Value::Integer(0.into())),
                ])),
            )]),
            // No Hour means "every hour", which the model cannot say.
            dict(&[(
                "StartCalendarInterval",
                Value::Dictionary(dict(&[("Minute", Value::Integer(0.into()))])),
            )]),
            dict(&[(
                "StartCalendarInterval",
                Value::Dictionary(dict(&[("Hour", Value::Integer(3.into()))])),
            )]),
        ] {
            assert!(translate_trigger(&bad).is_err(), "应当拒绝: {bad:?}");
        }
    }

    #[test]
    fn keep_alive_shapes() {
        let (_, k) = translate_trigger(&dict(&[("KeepAlive", Value::Boolean(true))])).unwrap();
        assert!(k);
        let (_, k) = translate_trigger(&dict(&[(
            "KeepAlive",
            Value::Dictionary(dict(&[("SuccessfulExit", Value::Boolean(false))])),
        )]))
        .unwrap();
        assert!(k);
        let err = translate_trigger(&dict(&[(
            "KeepAlive",
            Value::Dictionary(dict(&[("Crashed", Value::Boolean(true))])),
        )]))
        .unwrap_err();
        assert!(err.contains("Crashed"), "{err}");
    }

    /// `{SuccessfulExit = true}` means the opposite of what `keep_alive`
    /// writes back, so it is refused instead of quietly inverted. An empty
    /// dictionary — `KeepAlive = {}`, which launchd reads as plain `true` —
    /// is refused for the same reason: guessing is how the job silently
    /// changes behaviour.
    #[test]
    fn keep_alive_successful_exit_true_is_refused_not_inverted() {
        let err = translate_trigger(&dict(&[(
            "KeepAlive",
            Value::Dictionary(dict(&[("SuccessfulExit", Value::Boolean(true))])),
        )]))
        .unwrap_err();
        assert!(err.contains("SuccessfulExit"), "{err}");

        let err = translate_trigger(&dict(&[(
            "KeepAlive",
            Value::Dictionary(Dictionary::new()),
        )]))
        .unwrap_err();
        assert!(err.contains("SuccessfulExit"), "{err}");
    }

    #[test]
    fn watch_paths_and_friends_have_no_trigger_at_all() {
        for key in [
            "WatchPaths",
            "QueueDirectories",
            "Sockets",
            "MachServices",
            "LaunchEvents",
            "inetdCompatibility",
            "StartOnMount",
            "LaunchOnlyOnce",
        ] {
            let d = dict(&[(key, argv(&["/tmp/inbox"]))]);
            let err = translate_trigger(&d).unwrap_err();
            assert!(err.contains(key), "{err}");
        }
    }

    /// A minimal, adoptable plist body, so the tests below only differ in the
    /// one key they are about.
    fn plist_xml(extra: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
             \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\"><dict>\n\
             <key>Label</key><string>com.example.unit.refusal</string>\n\
             <key>ProgramArguments</key><array><string>/bin/echo</string></array>\n\
             {extra}</dict></plist>\n"
        )
    }

    /// Writing through a symlink would rewrite a file somewhere else
    /// entirely, so the refusal names where the link points.
    #[test]
    fn a_symlinked_plist_is_refused_and_names_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.plist");
        std::fs::write(&real, plist_xml("")).unwrap();
        let link = dir.path().join("link.plist");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let agent = parse_agent(&link).expect("符号链接本身仍然要能解析");
        let reason = agent.adoptable.reason().expect("应当拒绝");
        assert!(reason.contains("符号链接"), "{reason}");
        assert!(reason.contains("real.plist"), "{reason}");
    }

    /// `Disabled = true` is launchd's own off switch, and the generated plist
    /// has no key for it: adopting would turn the job back on.
    #[test]
    fn a_disabled_job_is_refused_until_it_is_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("off.plist");
        std::fs::write(&path, plist_xml("<key>Disabled</key><true/>\n")).unwrap();
        let reason = parse_agent(&path)
            .unwrap()
            .adoptable
            .reason()
            .expect("应当拒绝")
            .to_string();
        assert!(reason.contains("Disabled"), "{reason}");

        // `Disabled = false` says nothing and must not refuse on its own —
        // this one only fails the "not in ~/Library/LaunchAgents" check.
        let path = dir.path().join("on.plist");
        std::fs::write(&path, plist_xml("<key>Disabled</key><false/>\n")).unwrap();
        let reason = parse_agent(&path)
            .unwrap()
            .adoptable
            .reason()
            .unwrap_or_default()
            .to_string();
        assert!(!reason.contains("Disabled"), "{reason}");
    }

    /// `/tmp` and `/private/tmp` are the same directory on macOS; a plist in
    /// one spelling must not be refused for "not being in" the other.
    #[test]
    fn the_directory_check_sees_through_symlinked_spellings() {
        assert!(same_dir(Some(Path::new("/tmp")), Path::new("/private/tmp")));
        assert!(same_dir(Some(Path::new("/private/tmp")), Path::new("/tmp")));
        // Neither side exists: the raw comparison is the answer.
        assert!(same_dir(
            Some(Path::new("/no/such/dir")),
            Path::new("/no/such/dir")
        ));
        assert!(!same_dir(
            Some(Path::new("/no/such/dir")),
            Path::new("/other/missing")
        ));
        assert!(!same_dir(None, Path::new("/tmp")));
    }

    #[test]
    fn forbidden_programs_are_named() {
        for p in [
            "/Applications/Foo.app/Contents/MacOS/foo",
            "/System/Library/x",
            "/Library/Helper/h",
            "/opt/homebrew/Cellar/x/1/bin/x",
            "/usr/local/Cellar/x/1/bin/x",
            "/Users/me/Foo.app/Contents/MacOS/foo",
        ] {
            assert!(forbidden_program(Path::new(p)).is_some(), "应当拒绝 {p}");
        }
        for p in [
            "/Users/me/bin/x.sh",
            "/usr/bin/true",
            "/opt/homebrew/bin/uv",
        ] {
            assert!(forbidden_program(Path::new(p)).is_none(), "不该拒绝 {p}");
        }
        // A directory that merely *starts with the same letters* is fine.
        assert!(forbidden_program(Path::new("/Libraryish/x")).is_none());
    }

    #[test]
    fn value_text_flattens_nested_shapes() {
        assert_eq!(value_text(&Value::Boolean(true)), "true");
        assert_eq!(value_text(&argv(&["a", "b"])), "[a, b]");
        assert_eq!(
            value_text(&Value::Dictionary(dict(&[(
                "SuccessfulExit",
                Value::Boolean(false)
            )]))),
            "{SuccessfulExit=false}"
        );
    }
}
