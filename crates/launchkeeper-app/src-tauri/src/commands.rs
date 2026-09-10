//! Every `#[tauri::command]`, plus the view and input types the frontend
//! sees. The contract is `docs/M2-design.md` §3.2 and §3.3; the generated
//! `src/lib/bindings.ts` is the machine-readable copy of it.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use specta::Type;
// `specta-typescript` refuses to export 64-bit integers unless each one opts
// in, because `number` loses precision above 2^53. Every value marked here is
// a SQLite row id or a file size — both are bounded well below that in
// practice, and `number` keeps the frontend free of `BigInt` ceremony.
use specta_typescript::Number;
use tauri::{AppHandle, Manager};

use launchkeeper_core::{
    Interpreter, InterpreterKind, Origin, Run, RunId, Service, StopReason, Task, TaskName, Trigger,
    TriggerKind, agents, env, interpreters, launchctl, paths,
};

use crate::error::{AppError, AppResult};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// §3.3 view and input types
// ---------------------------------------------------------------------------

/// The status dot in the task list.
///
/// For a scheduled task, decided exactly as `docs/M2-design.md` §3.3 spells
/// out, in this order: not enabled → `Disabled`; newest run still running →
/// `Running`; no run at all → `Never`; exit code 0 → `Ok`; anything else →
/// `Failed`.
///
/// A service ([`launchkeeper_core::Task::is_service`]) is decided from
/// launchd's answer instead, because "is it up right now?" is the only
/// question that matters for one: not enabled → `Disabled`; launchd reports a
/// pid → `Running`; otherwise `Stopped`, unless the last run ended badly and
/// `keep_alive` is on — a crash loop — which is `Failed`.
///
/// There is deliberately no `Restarting`: a crash-looping service is
/// `Failed`, and the row's own text (exit code, stop reason, last-run time)
/// says what is happening. One more colour of dot would not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Last run succeeded.
    Ok,
    /// Last run failed, timed out or was killed.
    Failed,
    /// A run is in flight.
    Running,
    /// Enabled but never run.
    Never,
    /// Not loaded into launchd.
    Disabled,
    /// A service that is loaded but not running right now.
    Stopped,
}

/// One environment variable. A list of these rather than a map, because
/// TypeScript records lose ordering and make an editable form awkward.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct EnvVar {
    /// Variable name.
    pub key: String,
    /// Variable value.
    pub value: String,
}

/// A run's row id, as it crosses the bridge.
///
/// A named newtype rather than a bare `i64` because `#[specta(type = ...)]`
/// only applies to struct fields, not to command arguments — and `read_log`
/// needs one. It also makes `RunView.id` and `read_log`'s parameter provably
/// the same thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct RunHandle(#[specta(type = Number)] pub i64);

impl From<RunId> for RunHandle {
    fn from(id: RunId) -> RunHandle {
        RunHandle(id.0)
    }
}

/// One recorded execution, as the frontend sees it. Timestamps are RFC 3339
/// strings so the frontend can hand them straight to `Date`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RunView {
    /// Row id; the handle `read_log` takes.
    pub id: RunHandle,
    /// RFC 3339, UTC.
    pub started_at: String,
    /// RFC 3339, UTC. `None` while the run is in flight.
    pub finished_at: Option<String>,
    /// Exit code. `None` with `finished_at` set means killed by a signal.
    pub exit_code: Option<i32>,
    /// Wall-clock duration, `None` while running.
    #[specta(type = Option<Number>)]
    pub duration_ms: Option<i64>,
    /// launchd-triggered or user-triggered.
    pub trigger_kind: TriggerKind,
    /// Why the run ended. `None` while it is still going, and on rows a
    /// pre-M2.5 runner wrote.
    pub stop_reason: Option<StopReason>,
    /// True while `finished_at` is unset.
    pub running: bool,
}

impl From<&Run> for RunView {
    fn from(r: &Run) -> RunView {
        RunView {
            id: r.id.into(),
            started_at: rfc3339(r.started_at),
            finished_at: r.finished_at.map(rfc3339),
            exit_code: r.exit_code,
            duration_ms: r
                .finished_at
                .map(|f| (f - r.started_at).num_milliseconds().max(0)),
            trigger_kind: r.trigger_kind,
            stop_reason: r.stop_reason,
            running: r.finished_at.is_none(),
        }
    }
}

/// A task as the list renders it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TaskView {
    /// Unique name (also the launchd label suffix).
    pub name: String,
    /// Human-facing name.
    pub display_name: String,
    /// Optional longer description.
    pub description: Option<String>,
    /// Structured trigger, for the editor.
    pub trigger: Trigger,
    /// `Trigger::describe()`, for the list row.
    pub trigger_text: String,
    /// How the task runs, as one line: `python3 · 系统`, `uv run · uv 安装`,
    /// `直接执行`. Derived from the stored `script_path` + `args` by
    /// [`launchkeeper_core::interpreters::detect_from_task`], so it costs
    /// nothing but string work and needs no filesystem access — the picker's
    /// full scan happens only when the form is open.
    pub interpreter_label: Option<String>,
    /// [`Task::is_service`]: a `Manual` task, driven by start/stop rather
    /// than by a schedule. The list row and the detail header switch their
    /// whole set of controls on this.
    pub is_service: bool,
    /// launchd `KeepAlive`. On a service it means "restart after a crash,
    /// stay down after a manual stop", and it is what makes `Failed` mean
    /// "crash-looping" rather than "ended badly once".
    pub keep_alive: bool,
    /// Plist present *and* launchd knows the label.
    pub enabled: bool,
    /// Pid of the running job, when launchd reports one.
    pub loaded_pid: Option<u32>,
    /// Seconds since the run that is still in flight started. `None` unless
    /// launchd reports a pid *and* the newest run has no `finished_at` — for
    /// a service, that is its uptime.
    #[specta(type = Option<Number>)]
    pub uptime_secs: Option<i64>,
    /// Newest run, when there is one.
    pub last_run: Option<RunView>,
    /// Free-form tags.
    pub tags: Vec<String>,
    /// Pinned to the menu bar.
    pub favorite: bool,
    /// Post a notification when a run fails ([`TaskInput::notify_on_fail`]).
    /// The list row never shows this directly, but [`crate::notify`] needs
    /// it on the same value the scan loop already produces, rather than a
    /// second database read per task per scan.
    pub notify_on_fail: bool,
    /// The status dot.
    pub status: TaskStatus,
}

/// Which section of the 运行方式 picker an entry belongs to
/// (`docs/M3-design.md` §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum InterpreterGroup {
    /// 推荐 — the one entry `scan` picked, and only that one.
    Recommended,
    /// 项目内 — a `.venv` belonging to the script's own project.
    Project,
    /// PATH 里的其他解释器 — everything else, including 直接执行.
    Path,
}

/// One row of the 运行方式 picker.
///
/// The strings are all built on the Rust side ([`Interpreter::display_name`],
/// [`Interpreter::detail`]) so that the picker, the list row's
/// `interpreter_label` and the CLI cannot describe the same interpreter three
/// different ways.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct InterpreterView {
    /// Stable identity for the `<select>`-alike: the program plus its prefix
    /// args ([`Interpreter::id`]).
    pub id: String,
    /// Which family this is; the frontend needs it to know that `direct`
    /// composes differently from everything else.
    pub kind: InterpreterKind,
    /// Absolute path of the program (the script itself, for `direct`).
    pub program: String,
    /// Arguments that precede the script, e.g. `["run"]`.
    pub prefix_args: Vec<String>,
    /// Parsed `--version` output, when the program answered in time.
    pub version: Option<String>,
    /// Where it came from.
    pub origin: Origin,
    /// The bold part of a row: `python3`, `uv run`, `直接执行`.
    pub label: String,
    /// The muted part: `3.9.6 · 系统 /usr/bin/python3`.
    pub detail: String,
    /// True for the single recommended entry.
    pub recommended: bool,
    /// Why it is recommended; rendered as the one-line note under the
    /// dropdown. Only set on the recommended entry.
    pub reason: Option<String>,
    /// Which section it renders under.
    pub group: InterpreterGroup,
}

impl From<&Interpreter> for InterpreterView {
    fn from(i: &Interpreter) -> InterpreterView {
        InterpreterView {
            id: i.id(),
            kind: i.kind,
            program: i.program.to_string_lossy().into_owned(),
            prefix_args: i.prefix_args.clone(),
            version: i.version.clone(),
            origin: i.origin,
            label: i.display_name(),
            detail: i.detail(),
            recommended: i.recommended,
            reason: i.reason.clone(),
            group: if i.recommended {
                InterpreterGroup::Recommended
            } else if i.origin == Origin::ProjectVenv {
                InterpreterGroup::Project
            } else {
                InterpreterGroup::Path
            },
        }
    }
}

/// Everything the editor form can set. Used both as `create_task` /
/// `update_task` input and, inside [`TaskDetail`], as the form's initial
/// value — so the two never drift apart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TaskInput {
    /// Unique name; `^[a-z0-9][a-z0-9.-]{0,63}$`.
    pub name: String,
    /// Human-facing name.
    pub display_name: String,
    /// Optional longer description.
    pub description: Option<String>,
    /// Absolute path to the script or executable.
    pub script_path: String,
    /// Arguments passed to it.
    pub args: Vec<String>,
    /// Working directory; `$HOME` when unset.
    pub working_dir: Option<String>,
    /// Extra environment variables.
    pub env: Vec<EnvVar>,
    /// When it runs.
    pub trigger: Trigger,
    /// launchd `KeepAlive`; only valid with [`Trigger::AtLogin`] or
    /// [`Trigger::Manual`]. `Task::validate` enforces that, so an invalid
    /// combination comes back as an error from `create_task`/`update_task`
    /// rather than being silently dropped.
    pub keep_alive: bool,
    /// Per-run wall-clock limit in seconds.
    pub timeout_secs: Option<u32>,
    /// Free-form tags.
    pub tags: Vec<String>,
    /// Pinned to the menu bar.
    pub favorite: bool,
    /// Post a notification when a run fails.
    pub notify_on_fail: bool,
}

/// The detail pane's payload: the editable form value, the two timestamps
/// that are not editable, and the same [`TaskView`] the list holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TaskDetail {
    /// The editable fields.
    pub task: TaskInput,
    /// RFC 3339, UTC.
    pub created_at: String,
    /// RFC 3339, UTC.
    pub updated_at: String,
    /// Derived, read-only view.
    pub view: TaskView,
    /// This task came from somebody else's LaunchAgent (M3 §3.1). The detail
    /// pane says so, and offers 撤销接管.
    pub adopted: bool,
    /// The plist this task lives in when it was adopted — its original file,
    /// which adoption did not move. `None` for tasks Launchkeeper created,
    /// whose plist is derived from the name.
    pub plist_path: Option<String>,
}

/// Which of a run's two captured streams to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    /// The script's standard output.
    Stdout,
    /// The script's standard error.
    Stderr,
}

/// The tail of one log file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct LogChunk {
    /// The tail, lossily decoded as UTF-8.
    pub content: String,
    /// Size of the whole file on disk.
    #[specta(type = Number)]
    pub total_bytes: i64,
    /// True when `content` is only the tail of a larger file.
    pub truncated: bool,
}

// ---------------------------------------------------------------------------
// conversions
// ---------------------------------------------------------------------------

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn name_of(s: &str) -> AppResult<TaskName> {
    Ok(TaskName::new(s)?)
}

impl TaskInput {
    /// Builds a [`Task`] from the form. `existing` is the row being edited,
    /// when there is one: its `created_at` is preserved, and so is the
    /// adoption bookkeeping (M3 §3.1) — the form has no field for either,
    /// and losing `adopted`/`plist_path` on an ordinary edit would move an
    /// adopted job's plist out from under launchd.
    fn into_task(self, existing: Option<&Task>) -> AppResult<Task> {
        let now = Utc::now();
        let mut env = BTreeMap::new();
        for v in self.env {
            env.insert(v.key, v.value);
        }
        let task = Task {
            name: name_of(&self.name)?,
            display_name: self.display_name,
            description: self.description,
            script_path: PathBuf::from(self.script_path),
            args: self.args,
            working_dir: self.working_dir.map(PathBuf::from),
            env,
            trigger: self.trigger,
            keep_alive: self.keep_alive,
            timeout_secs: self.timeout_secs,
            tags: self.tags,
            favorite: self.favorite,
            notify_on_fail: self.notify_on_fail,
            created_at: existing.map_or(now, |t| t.created_at),
            updated_at: now,
            adopted: existing.is_some_and(|t| t.adopted),
            plist_path: existing.and_then(|t| t.plist_path.clone()),
        };
        task.validate()?;
        Ok(task)
    }
}

impl From<&Task> for TaskInput {
    fn from(t: &Task) -> TaskInput {
        TaskInput {
            name: t.name.as_str().to_string(),
            display_name: t.display_name.clone(),
            description: t.description.clone(),
            script_path: t.script_path.to_string_lossy().into_owned(),
            args: t.args.clone(),
            working_dir: t
                .working_dir
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            env: t
                .env
                .iter()
                .map(|(k, v)| EnvVar {
                    key: k.clone(),
                    value: v.clone(),
                })
                .collect(),
            trigger: t.trigger.clone(),
            keep_alive: t.keep_alive,
            timeout_secs: t.timeout_secs,
            tags: t.tags.clone(),
            favorite: t.favorite,
            notify_on_fail: t.notify_on_fail,
        }
    }
}

/// Builds the derived view of one task from data already in hand.
///
/// `status` is launchd's answer for this task (`None` = not enabled); it is
/// passed in rather than fetched here so that the caller decides when to pay
/// for the `launchctl print`, and pays for it exactly once (M1).
fn assemble(
    task: &Task,
    status: Option<&launchctl::JobStatus>,
    last_run: Option<&Run>,
) -> TaskView {
    build_view(task, status.is_some(), status.and_then(|s| s.pid), last_run)
}

/// The one place a [`TaskView`] is built, so that the launchctl-backed path
/// ([`assemble`]) and the database-only startup path ([`seed_views`]) cannot
/// drift apart.
fn build_view(
    task: &Task,
    enabled: bool,
    loaded_pid: Option<u32>,
    last_run: Option<&Run>,
) -> TaskView {
    TaskView {
        name: task.name.as_str().to_string(),
        display_name: task.display_name.clone(),
        description: task.description.clone(),
        trigger: task.trigger.clone(),
        trigger_text: task.trigger.describe(),
        interpreter_label: interpreters::detect_from_task(&task.script_path, &task.args)
            .map(|d| d.interpreter.label()),
        is_service: task.is_service(),
        keep_alive: task.keep_alive,
        enabled,
        loaded_pid,
        uptime_secs: uptime_secs(loaded_pid, last_run),
        last_run: last_run.map(RunView::from),
        tags: task.tags.clone(),
        favorite: task.favorite,
        notify_on_fail: task.notify_on_fail,
        status: status_of(task, enabled, loaded_pid, last_run),
    }
}

/// How long the run that is still in flight has been going, in seconds.
///
/// Both halves are required: launchd has to report a pid (the process really
/// is there) *and* the newest run must have no `finished_at` (the runner has
/// not written its epitaph yet). Either alone would show an uptime for a job
/// that is not running. Same rule as the CLI's `status --json`.
fn uptime_secs(loaded_pid: Option<u32>, last_run: Option<&Run>) -> Option<i64> {
    loaded_pid?;
    let run = last_run?;
    if run.finished_at.is_some() {
        return None;
    }
    Some((Utc::now() - run.started_at).num_seconds().max(0))
}

/// Builds the derived view of one task. One `launchctl print` and one
/// database read per task, which is why the list is refreshed on a timer
/// rather than per keystroke (§3.4).
pub fn view_of(service: &Service, task: &Task) -> AppResult<TaskView> {
    let status = Service::enabled_status(task)?;
    let last_run = service.store().last_run(&task.name)?;
    Ok(assemble(task, status.as_ref(), last_run.as_ref()))
}

fn status_of(
    task: &Task,
    enabled: bool,
    loaded_pid: Option<u32>,
    last_run: Option<&Run>,
) -> TaskStatus {
    if !enabled {
        return TaskStatus::Disabled;
    }
    if task.is_service() {
        // launchd's pid is the truth for a service, not the run row: a runner
        // that died without writing `finished_at` leaves a row that claims to
        // be running forever, and a service the user can see is gone must not
        // show as up.
        if loaded_pid.is_some() {
            return TaskStatus::Running;
        }
        if task.keep_alive && last_run.is_some_and(ended_badly) {
            // KeepAlive will bring it back, so this is a crash loop rather
            // than a stopped service.
            return TaskStatus::Failed;
        }
        return TaskStatus::Stopped;
    }
    match last_run {
        None => TaskStatus::Never,
        Some(r) if r.finished_at.is_none() => TaskStatus::Running,
        Some(r) if r.exit_code == Some(0) => TaskStatus::Ok,
        Some(_) => TaskStatus::Failed,
    }
}

/// Whether a finished run is one launchd would restart a `KeepAlive` service
/// for.
///
/// A run that was asked to stop is never "bad", whatever the script's own
/// exit code turned out to be: the user pressed 停止, and `KeepAlive =
/// {SuccessfulExit = false}` keeps the job down because the *runner* exited 0
/// (`docs/M2.5-design.md` §3.1). A run with no `finished_at` is not judged at
/// all — that is the fraction of a second between the process disappearing
/// and the runner writing its row, and flashing 崩溃 there would be a lie.
fn ended_badly(r: &Run) -> bool {
    r.finished_at.is_some() && r.exit_code != Some(0) && r.stop_reason != Some(StopReason::Stopped)
}

fn view_by_name(service: &Service, name: &TaskName) -> AppResult<TaskView> {
    let task = service
        .store()
        .get_task(name)?
        .ok_or_else(|| AppError::new(format!("任务不存在: {name}")))?;
    view_of(service, &task)
}

/// Collects the view of every task, sorted by display name. Shared by
/// `list_tasks` and the background scan of §3.4.
///
/// The service mutex is held only for the database reads and is released
/// before the `launchctl print` calls: a scan of N tasks otherwise blocks
/// every command for N launchctl round trips (H1).
pub fn all_views(state: &AppState) -> AppResult<Vec<TaskView>> {
    let rows = {
        let service = state.service();
        let tasks = service.store().list_tasks()?;
        let mut rows = Vec::with_capacity(tasks.len());
        for task in tasks {
            let last_run = service.store().last_run(&task.name)?;
            rows.push((task, last_run));
        }
        rows
    };
    let mut out = Vec::with_capacity(rows.len());
    for (task, last_run) in rows {
        let status = Service::enabled_status(&task)?;
        out.push(assemble(&task, status.as_ref(), last_run.as_ref()));
    }
    out.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    Ok(out)
}

/// The same list, built from the database alone — no `launchctl` at all.
///
/// Used once, to seed the tray at startup (M7): a cold start must not pay for
/// one `launchctl print` per task before the window appears. `enabled` falls
/// back to "the plist file exists", which is a `stat` rather than an IPC round
/// trip and is right in every case but a job the user booted out by hand; the
/// first scan-loop iteration corrects it a moment later.
pub fn seed_views(state: &AppState) -> AppResult<Vec<TaskView>> {
    let service = state.service();
    let mut out = Vec::new();
    for task in service.store().list_tasks()? {
        let last_run = service.store().last_run(&task.name)?;
        let enabled = paths::plist_path(&task)?.exists();
        // No pid without launchctl, so a running service seeds as `Stopped`
        // and the first scan corrects it a moment later.
        out.push(build_view(&task, enabled, None, last_run.as_ref()));
    }
    out.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    Ok(out)
}

// ---------------------------------------------------------------------------
// §3.2 commands
// ---------------------------------------------------------------------------
//
// Every command is `async` and does its work inside `spawn_blocking`.
//
// A synchronous `#[tauri::command]` runs on the main thread, so a command
// that waits on the service mutex — which the background scan of §3.4 holds
// while it reads the database — freezes the window. `async` moves the command
// onto Tauri's async runtime, and `spawn_blocking` moves the blocking part
// (SQLite, launchctl, the filesystem) off that runtime's worker threads.
//
// They take `AppHandle` rather than `State` because the closure handed to
// `spawn_blocking` must be `'static`, which a borrowed `State<'_, _>` is not.
// Neither type crosses the IPC bridge, so the generated TypeScript is
// unchanged either way.

/// Runs `f` on the blocking pool.
async fn blocking<T, F>(f: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(f).await {
        Ok(r) => r,
        Err(e) => Err(AppError::new(format!("后台任务失败: {e}"))),
    }
}

/// Every task with its derived status.
#[tauri::command]
#[specta::specta]
pub async fn list_tasks(app: AppHandle) -> AppResult<Vec<TaskView>> {
    blocking(move || all_views(&app.state::<AppState>())).await
}

/// One task, with the editable form value and its timestamps.
#[tauri::command]
#[specta::specta]
pub async fn get_task(app: AppHandle, name: String) -> AppResult<TaskDetail> {
    blocking(move || {
        let state = app.state::<AppState>();
        let n = name_of(&name)?;
        let task = {
            let service = state.service();
            service
                .store()
                .get_task(&n)?
                .ok_or_else(|| AppError::new(format!("任务不存在: {name}")))?
        };
        let view = view_of(&state.service(), &task)?;
        Ok(TaskDetail {
            task: TaskInput::from(&task),
            created_at: rfc3339(task.created_at),
            updated_at: rfc3339(task.updated_at),
            view,
            adopted: task.adopted,
            plist_path: task
                .plist_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
        })
    })
    .await
}

/// Creates a task in the database. launchd is untouched until `set_enabled`.
#[tauri::command]
#[specta::specta]
pub async fn create_task(app: AppHandle, input: TaskInput) -> AppResult<TaskView> {
    blocking(move || {
        let state = app.state::<AppState>();
        let task = input.into_task(None)?;
        let service = state.service();
        service.add_task(task.clone())?;
        view_of(&service, &task)
    })
    .await
}

/// Updates a task. When it is enabled, core rewrites the plist and reloads
/// launchd *before* touching the database, so a rejected schedule leaves
/// everything as it was.
#[tauri::command]
#[specta::specta]
pub async fn update_task(app: AppHandle, name: String, input: TaskInput) -> AppResult<TaskView> {
    blocking(move || {
        let state = app.state::<AppState>();
        let n = name_of(&name)?;
        if input.name != name {
            return Err(AppError::new(format!(
                "不支持重命名任务（{name} -> {}）：请新建一个任务",
                input.name
            )));
        }
        let service = state.service();
        let existing = service
            .store()
            .get_task(&n)?
            .ok_or_else(|| AppError::new(format!("任务不存在: {name}")))?;
        let task = input.into_task(Some(&existing))?;
        service.update_task(task.clone())?;
        view_of(&service, &task)
    })
    .await
}

/// Removes the task, its plist, its launchd job, its rows and its logs.
#[tauri::command]
#[specta::specta]
pub async fn delete_task(app: AppHandle, name: String) -> AppResult<()> {
    blocking(move || {
        let state = app.state::<AppState>();
        let service = state.service();
        service.remove_task(&name_of(&name)?)?;
        Ok(())
    })
    .await
}

/// Loads the task into launchd (writing its plist) or boots it out.
#[tauri::command]
#[specta::specta]
pub async fn set_enabled(app: AppHandle, name: String, enabled: bool) -> AppResult<TaskView> {
    blocking(move || {
        let state = app.state::<AppState>();
        let service = state.service();
        let n = name_of(&name)?;
        if enabled {
            service.enable(&n)?;
        } else {
            service.disable(&n)?;
        }
        view_by_name(&service, &n)
    })
    .await
}

/// Runs the task now.
///
/// An enabled task is kickstarted through launchd, so the run happens in the
/// same environment a scheduled run would. A disabled task has no launchd
/// job to kickstart, so the runner is spawned directly with `--manual`; the
/// run is not waited on, and its own logging records it.
///
/// For a service, "run it now" is "start it" — [`start_service`] — because
/// there is nothing else it could mean.
#[tauri::command]
#[specta::specta]
pub async fn run_now(app: AppHandle, name: String) -> AppResult<()> {
    blocking(move || run_now_inner(&app.state::<AppState>(), &name)).await
}

/// The body of [`run_now`], reachable without going through the async
/// wrapper so that the menu bar (`tray.rs`) runs a favourite through exactly
/// the same path the UI does.
///
/// # Errors
/// Unknown task, launchd failures, or a missing runner binary.
pub fn run_now_inner(state: &AppState, name: &str) -> AppResult<()> {
    let service = state.service();
    let n = name_of(name)?;
    let Some(task) = service.store().get_task(&n)? else {
        return Err(AppError::new(format!("任务不存在: {name}")));
    };
    if task.is_service() {
        // `Service::start` loads the job first when it is not loaded, so a
        // disabled service starts rather than failing — and it must not go
        // down the "spawn the runner directly" path below, which would run
        // the script outside launchd with no way to stop it again.
        service.start(&n)?;
        return Ok(());
    }
    if service.is_enabled(&n)? {
        service.run_now(&n)?;
        return Ok(());
    }
    let runner = state.runner_path()?;
    let child = std::process::Command::new(runner)
        .arg("run")
        .arg(n.as_str())
        .arg("--manual")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| AppError::new(format!("启动 runner 失败（{}）: {e}", runner.display())))?;
    reap(child);
    Ok(())
}

/// Waits on a spawned child in the background.
///
/// The app never cares about the exit status, but a child that is never
/// waited on stays a zombie for as long as the app runs, and a long session
/// with a few "立即运行" clicks a day would accumulate them (M2).
fn reap(mut child: std::process::Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

/// The three service verbs of `docs/M2.5-design.md` §5, so that the commands
/// below and the menu bar share one body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    /// Load the job if needed, then `kickstart` it.
    Start,
    /// SIGTERM through launchd; the job stays loaded.
    Stop,
    /// [`Lifecycle::Stop`] then [`Lifecycle::Start`].
    Restart,
}

/// Applies one lifecycle verb to a task.
///
/// # Errors
/// Unknown task, launchd failures, or a missing runner binary.
pub fn lifecycle(state: &AppState, name: &str, what: Lifecycle) -> AppResult<TaskView> {
    let service = state.service();
    let n = name_of(name)?;
    if service.store().get_task(&n)?.is_none() {
        return Err(AppError::new(format!("任务不存在: {name}")));
    }
    match what {
        Lifecycle::Start => service.start(&n)?,
        Lifecycle::Stop => service.stop(&n)?,
        Lifecycle::Restart => service.restart(&n)?,
    }
    // Re-read: `start` may have enabled the task on the way, so `enabled`,
    // the pid and the status are all different from what the caller holds.
    view_by_name(&service, &n)
}

/// Whether `name` is a service that launchd reports as running right now.
///
/// The menu bar asks this to decide whether its item means 启动 or 停止. An
/// unknown or non-service task is simply "not a running service"; the caller
/// then falls back to [`run_now_inner`], which handles both.
///
/// # Errors
/// Invalid task names, database and launchctl errors.
pub fn running_service(state: &AppState, name: &str) -> AppResult<bool> {
    let n = name_of(name)?;
    // The lock is dropped before the `launchctl print`, as everywhere else.
    let task = state.service().store().get_task(&n)?;
    let Some(task) = task else {
        return Ok(false);
    };
    if !task.is_service() {
        return Ok(false);
    }
    Ok(Service::enabled_status(&task)?
        .and_then(|s| s.pid)
        .is_some())
}

/// Starts a service, loading it into launchd first if it is not loaded yet.
///
/// Blocking for as long as `launchctl` takes, which is why — like every
/// command here — the work happens off the main thread.
#[tauri::command]
#[specta::specta]
pub async fn start_service(app: AppHandle, name: String) -> AppResult<TaskView> {
    blocking(move || lifecycle(&app.state::<AppState>(), &name, Lifecycle::Start)).await
}

/// Stops a running service and leaves the job loaded, so that
/// [`start_service`] can bring it back without rewriting the plist.
///
/// This one can block for up to [`launchkeeper_core::service::STOP_GRACE`]
/// waiting for the process to go away — harmless on the blocking pool, and
/// the reason this is an `async` command rather than a synchronous one.
#[tauri::command]
#[specta::specta]
pub async fn stop_service(app: AppHandle, name: String) -> AppResult<TaskView> {
    blocking(move || lifecycle(&app.state::<AppState>(), &name, Lifecycle::Stop)).await
}

/// Stops a service and starts it again.
#[tauri::command]
#[specta::specta]
pub async fn restart_service(app: AppHandle, name: String) -> AppResult<TaskView> {
    blocking(move || lifecycle(&app.state::<AppState>(), &name, Lifecycle::Restart)).await
}

/// The newest `limit` runs of a task, newest first.
#[tauri::command]
#[specta::specta]
pub async fn list_runs(app: AppHandle, name: String, limit: u32) -> AppResult<Vec<RunView>> {
    blocking(move || {
        let state = app.state::<AppState>();
        let service = state.service();
        let runs = service
            .store()
            .list_runs(&name_of(&name)?, limit as usize)?;
        Ok(runs.iter().map(RunView::from).collect())
    })
    .await
}

/// Upper bound on what one [`read_log`] call returns, and what
/// `tail_bytes == 0` means.
///
/// The log tail is polled once a second while a run is in flight (§3.4), and
/// the whole chunk crosses the IPC bridge as a JSON string every time; an
/// unbounded read of a runaway script's log would push the app over on
/// memory long before the user learned anything from it.
pub const MAX_TAIL_BYTES: u64 = 4 * 1024 * 1024;

/// The last `tail_bytes` bytes of one of a run's two log files.
///
/// `tail_bytes == 0`, or anything above it, means [`MAX_TAIL_BYTES`]. The
/// read is byte-oriented and then decoded lossily, so a cut in the middle of
/// a multi-byte character yields a replacement character instead of an error.
#[tauri::command]
#[specta::specta]
pub async fn read_log(
    app: AppHandle,
    run_id: RunHandle,
    stream: LogStream,
    tail_bytes: u32,
) -> AppResult<LogChunk> {
    blocking(move || {
        use std::io::{Read, Seek, SeekFrom};

        let state = app.state::<AppState>();
        let run = {
            let service = state.service();
            service
                .store()
                .get_run(RunId(run_id.0))?
                .ok_or_else(|| AppError::new(format!("运行记录不存在: {}", run_id.0)))?
        };
        let path = match stream {
            LogStream::Stdout => &run.stdout_path,
            LogStream::Stderr => &run.stderr_path,
        };
        let mut file = match std::fs::File::open(path) {
            Ok(f) => f,
            // The runner creates both files up front, but a pruned or
            // manually deleted log should read as empty rather than as an
            // error.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(LogChunk {
                    content: String::new(),
                    total_bytes: 0,
                    truncated: false,
                });
            }
            Err(e) => {
                return Err(AppError::new(format!(
                    "读取日志失败（{}）: {e}",
                    path.display()
                )));
            }
        };
        let total = file.metadata()?.len();
        let want = tail_window(tail_bytes);
        let truncated = total > want;
        if truncated {
            file.seek(SeekFrom::Start(total - want))?;
        }
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        Ok(LogChunk {
            content: String::from_utf8_lossy(&buf).into_owned(),
            // Log files are bounded by disk, not by 2^53; `i64` is only here
            // because TypeScript has no `u64`.
            total_bytes: i64::try_from(total).unwrap_or(i64::MAX),
            truncated,
        })
    })
    .await
}

/// How many trailing bytes a `tail_bytes` argument actually asks for.
fn tail_window(tail_bytes: u32) -> u64 {
    match u64::from(tail_bytes) {
        0 => MAX_TAIL_BYTES,
        n => n.min(MAX_TAIL_BYTES),
    }
}

/// Every interpreter worth offering for `script`, best first
/// (`docs/M3-design.md` §1).
///
/// `script` is the script the form currently points at — it decides the
/// recommendation and whether 直接执行 is on the list at all — and
/// `project_dir` is the form's working directory, where the walk for `.venv`
/// / `uv.lock` / `package.json` starts. Both may be absent while the user is
/// still typing; the result is then just "what is installed on this machine".
///
/// The `PATH` searched is [`env::effective_path`], i.e. the login shell's,
/// which is the one the runner will hand the script — not the bare
/// `/usr/bin:/bin:…` launchd gives this process.
///
/// Versions are collected (one `--version` per program, cached for the life
/// of the process), which is why this runs on the blocking pool like every
/// other command.
#[tauri::command]
#[specta::specta]
pub async fn scan_interpreters(
    script: Option<String>,
    project_dir: Option<String>,
) -> AppResult<Vec<InterpreterView>> {
    blocking(move || {
        let script = script.filter(|s| !s.trim().is_empty()).map(PathBuf::from);
        let dir = project_dir
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from);
        let found = interpreters::scan(
            script.as_deref(),
            dir.as_deref(),
            &env::effective_path(),
            true,
        );
        Ok(found.iter().map(InterpreterView::from).collect())
    })
    .await
}

/// What the form needs to know about a script path before it will save it.
///
/// Two questions, one round trip, asked once on save rather than on every
/// keystroke: does it exist (a path that is merely mistyped becomes a task
/// that fails silently at 21:00), and is it executable (with no interpreter
/// selected, launchd has to `exec` the file itself — if it cannot, the job
/// never starts and the only trace is a launchd error nobody reads).
#[tauri::command]
#[specta::specta]
pub async fn check_script(path: String) -> AppResult<ScriptCheck> {
    blocking(move || {
        let p = PathBuf::from(path);
        Ok(ScriptCheck {
            exists: p.exists(),
            executable: interpreters::is_executable(&p),
        })
    })
    .await
}

/// The answer to [`check_script`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ScriptCheck {
    /// The path resolves to something.
    pub exists: bool,
    /// A regular file with an execute bit — i.e. something launchd could
    /// start on its own. Note that this is deliberately *not* "executable
    /// and carrying a shebang", which is what `scan_interpreters` requires
    /// before offering 直接执行: a compiled binary has no shebang and runs
    /// perfectly well, and the CLI has always accepted one
    /// (`--script /bin/echo`).
    pub executable: bool,
}

/// Absolute path of the directory holding a task's per-run logs. The UI uses
/// it for "reveal in Finder".
#[tauri::command]
#[specta::specta]
pub async fn task_log_dir(name: String) -> AppResult<String> {
    blocking(move || {
        let dir = paths::task_log_dir(&name_of(&name)?)?;
        Ok(dir.to_string_lossy().into_owned())
    })
    .await
}

/// Reveals `path` in Finder.
///
/// `open -R` rather than the opener plugin: the plugin reveals a file only
/// when it exists, and the UI also wants to point at a log directory that a
/// task has not written to yet — `open -R` on a directory selects it in its
/// parent, which is the behaviour the "在 Finder 中显示" menu item promises.
///
/// The path must be absolute, and it is passed after `--`, so that nothing
/// the frontend hands over can be read by `open` as an option (M3).
#[tauri::command]
#[specta::specta]
pub async fn reveal_in_finder(path: String) -> AppResult<()> {
    blocking(move || {
        let p = PathBuf::from(&path);
        if !p.is_absolute() || path.starts_with('-') {
            return Err(AppError::new(format!("拒绝显示非绝对路径: {path}")));
        }
        let child = std::process::Command::new("/usr/bin/open")
            .arg("-R")
            .arg("--")
            .arg(&p)
            .stdin(std::process::Stdio::null())
            .spawn()
            .map_err(|e| AppError::new(format!("在 Finder 中显示失败（{path}）: {e}")))?;
        reap(child);
        Ok(())
    })
    .await
}

// ---------------------------------------------------------------------------
// M3 §3.4: LaunchAgents Launchkeeper did not write
// ---------------------------------------------------------------------------
//
// Everything below belongs to 原地接管 (`docs/M3-design.md` §3). The core
// module `agents` does the reading and `Service::adopt` / `unadopt` the
// writing; this section only turns them into the six commands the list's
// 其他 LaunchAgents group and the 接管 dialog call.
//
// None of it runs on the background scan of §3.4. Listing external agents
// means reading and parsing every file in `~/Library/LaunchAgents` plus one
// `launchctl print` each — filesystem work that has no business happening
// twice a second — so the frontend asks for it on demand: at startup, after
// an adoption or an undo, and when 刷新 is pressed.

/// `/Users/x/Library/LaunchAgents/a.plist` → `~/Library/LaunchAgents/a.plist`.
///
/// Display only, and only for paths that really are under `$HOME`. The same
/// abbreviation core applies to interpreter paths, so a path the user sees in
/// two places reads the same way in both.
fn shorten_home(p: &std::path::Path) -> String {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
        && let Ok(rest) = p.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    p.display().to_string()
}

fn shorten_opt(p: Option<&PathBuf>) -> Option<String> {
    p.map(|p| shorten_home(p))
}

/// One LaunchAgent Launchkeeper did not write, as the list and the read-only
/// detail pane render it.
///
/// Every string is composed here rather than in the frontend, for the reason
/// [`InterpreterView`] gives: the CLI's `agents` table and this pane must not
/// describe the same plist two different ways.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ExternalAgentView {
    /// The launchd label — the identity adoption deliberately preserves. It
    /// is also this view's key: the commands below all take it.
    pub label: String,
    /// The plist file, with `$HOME` abbreviated to `~`.
    pub path: String,
    /// The same file, absolute — what 在 Finder 中显示 has to be given, since
    /// `reveal_in_finder` (rightly) refuses anything that is not an absolute
    /// path and cannot expand a `~` the frontend wrote.
    pub full_path: String,
    /// Program plus arguments as one line.
    pub command: String,
    /// The trigger in Chinese, or 不支持的触发方式 when the plist's
    /// scheduling keys do not translate into Launchkeeper's model.
    pub trigger_text: String,
    /// The structured trigger when the plist's scheduling keys translate
    /// into Launchkeeper's model; `None` otherwise. The frontend describes
    /// it in the UI language (`lib/describeTrigger.ts`) instead of showing
    /// the Chinese `trigger_text`.
    pub trigger: Option<Trigger>,
    /// `WorkingDirectory`, abbreviated.
    pub working_dir: Option<String>,
    /// `StandardOutPath`, abbreviated. Shown as 日志 in the read-only detail:
    /// adoption stops using it, so the user should know where it was.
    pub stdout_path: Option<String>,
    /// `StandardErrorPath`, abbreviated.
    pub stderr_path: Option<String>,
    /// `KeepAlive` in either of its shapes.
    pub keep_alive: bool,
    /// launchd knows this label right now.
    pub loaded: bool,
    /// Pid, while the job has a process.
    pub pid: Option<u32>,
    /// The exit code launchd remembers for it.
    pub last_exit: Option<i32>,
    /// Adoption would work. `false` greys the row out.
    pub adoptable: bool,
    /// Why not, when `adoptable` is false — the row's tooltip.
    pub adopt_reason: Option<String>,
    /// Already Launchkeeper's: an adopted task's plist, or a
    /// `com.launchkeeper.*` label. Such rows are listed (core keeps them so
    /// that the flag has a value at all) but never offered for adoption.
    pub managed: bool,
}

impl From<&launchkeeper_core::ExternalAgent> for ExternalAgentView {
    fn from(a: &launchkeeper_core::ExternalAgent) -> ExternalAgentView {
        ExternalAgentView {
            label: a.label.clone(),
            path: shorten_home(&a.path),
            full_path: a.path.to_string_lossy().into_owned(),
            command: a.command(),
            trigger_text: a.trigger_text(),
            trigger: a.trigger.clone(),
            working_dir: shorten_opt(a.working_dir.as_ref()),
            stdout_path: shorten_opt(a.stdout_path.as_ref()),
            stderr_path: shorten_opt(a.stderr_path.as_ref()),
            keep_alive: a.keep_alive,
            loaded: a.loaded,
            pid: a.pid,
            last_exit: a.last_exit,
            adoptable: a.adoptable.is_yes(),
            adopt_reason: a.adoptable.reason().map(str::to_string),
            managed: a.managed_by_launchkeeper,
        }
    }
}

/// One line of the adoption diff: a plist key and its value, flattened to a
/// single string by [`launchkeeper_core::agents::value_text`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DiffEntry {
    /// The plist key.
    pub key: String,
    /// Its value, on one line.
    pub value: String,
}

fn diff_entries(pairs: &[(String, String)]) -> Vec<DiffEntry> {
    pairs
        .iter()
        .map(|(key, value)| DiffEntry {
            key: key.clone(),
            value: value.clone(),
        })
        .collect()
}

/// What adoption would do to one plist, key by key — the dialog's whole
/// content, so that nobody accepts a rewrite of their own file unseen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AdoptionPlanView {
    /// The label, which adoption does not change.
    pub label: String,
    /// The task name the adopted job would get (equal to the label).
    pub name: String,
    /// The plist that would be rewritten, abbreviated.
    pub path: String,
    /// Where the original bytes go first, abbreviated.
    pub backup_path: String,
    /// Keys the rewrite drops, with their current values.
    pub removed: Vec<DiffEntry>,
    /// Keys it adds or changes, with their new values.
    pub added: Vec<DiffEntry>,
    /// Keys that come through untouched.
    pub kept: Vec<DiffEntry>,
}

impl From<&launchkeeper_core::AdoptionPlan> for AdoptionPlanView {
    fn from(p: &launchkeeper_core::AdoptionPlan) -> AdoptionPlanView {
        AdoptionPlanView {
            label: p.label.clone(),
            name: p.task.name.as_str().to_string(),
            path: shorten_home(&p.path),
            backup_path: shorten_home(&p.backup_path),
            removed: diff_entries(&p.removed),
            added: diff_entries(&p.added),
            kept: diff_entries(&p.kept),
        }
    }
}

/// Looks one agent up by label in `~/Library/LaunchAgents`.
fn find_agent(label: &str) -> AppResult<launchkeeper_core::ExternalAgent> {
    let dir = paths::launch_agents_dir()?;
    Ok(agents::find(&dir, label)?)
}

/// Every LaunchAgent in `~/Library/LaunchAgents` that Launchkeeper did not
/// create, sorted by label.
///
/// Deliberately *not* part of the background scan: see the note at the top of
/// this section.
#[tauri::command]
#[specta::specta]
pub async fn list_external_agents() -> AppResult<Vec<ExternalAgentView>> {
    blocking(move || {
        let dir = paths::launch_agents_dir()?;
        let found = agents::list_external(&dir)?;
        Ok(found.iter().map(ExternalAgentView::from).collect())
    })
    .await
}

/// The diff [`adopt_agent`] would apply. Read-only: nothing is written and
/// nothing is loaded, which is what makes it safe to fetch just to show it.
#[tauri::command]
#[specta::specta]
pub async fn adoption_plan(app: AppHandle, label: String) -> AppResult<AdoptionPlanView> {
    blocking(move || {
        let state = app.state::<AppState>();
        let service = state.service();
        let plan = service.adoption_plan(&label)?;
        Ok(AdoptionPlanView::from(&plan))
    })
    .await
}

/// Takes an external LaunchAgent under management, in place.
///
/// `reload = false` rewrites the plist and records the task but leaves
/// launchd's loaded copy alone — the unticked 「接管后立即重新加载」 — so a
/// job that must not be interrupted right now keeps running under its old
/// definition until the next login.
///
/// Returns the new task's view, so the caller can select it in the list
/// without a round trip.
#[tauri::command]
#[specta::specta]
pub async fn adopt_agent(app: AppHandle, label: String, reload: bool) -> AppResult<TaskView> {
    blocking(move || {
        let state = app.state::<AppState>();
        let service = state.service();
        let task = service.adopt_with(&label, reload)?;
        view_of(&service, &task)
    })
    .await
}

/// Undoes an adoption: the original plist comes back byte for byte from its
/// `.bak` and the task row goes. Log files on disk are kept.
#[tauri::command]
#[specta::specta]
pub async fn unadopt_task(app: AppHandle, name: String) -> AppResult<()> {
    blocking(move || {
        let state = app.state::<AppState>();
        let service = state.service();
        service.unadopt(&name_of(&name)?)?;
        Ok(())
    })
    .await
}

/// Loads an unmanaged LaunchAgent into launchd, or boots it out. Its plist is
/// never touched — this is the one thing Launchkeeper does to somebody else's
/// file without changing it.
#[tauri::command]
#[specta::specta]
pub async fn external_set_enabled(
    app: AppHandle,
    label: String,
    enabled: bool,
) -> AppResult<ExternalAgentView> {
    blocking(move || {
        let state = app.state::<AppState>();
        let agent = find_agent(&label)?;
        {
            let service = state.service();
            if enabled {
                service.external_enable(&agent.label, &agent.path)?;
            } else {
                service.external_disable(&agent.label)?;
            }
        }
        // Re-read rather than patching the view in hand: `loaded`, the pid and
        // the last exit code all come from launchd, and only launchd knows
        // what they are now.
        Ok(ExternalAgentView::from(&find_agent(&label)?))
    })
    .await
}

/// `launchctl kickstart` on an unmanaged label.
///
/// No run is recorded — without the runner in `ProgramArguments` there is
/// nothing to record it, which is precisely what adoption fixes.
///
/// The label is looked up in `~/Library/LaunchAgents` first, exactly as
/// [`external_set_enabled`] does: this command reaches launchd with a string
/// the frontend chose, and "kickstart whatever happens to answer to that
/// name" is not a thing to do to a machine's launchd. A label with no plist
/// of ours behind it is [`launchkeeper_core::Error::AgentNotFound`], not a
/// launchctl error the user has to decode.
#[tauri::command]
#[specta::specta]
pub async fn external_run(app: AppHandle, label: String) -> AppResult<()> {
    blocking(move || {
        let agent = find_agent(&label)?;
        let state = app.state::<AppState>();
        let service = state.service();
        service.external_kickstart(&agent.label)?;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use launchkeeper_core::{CalendarEntry, Store};

    fn sample_task() -> Task {
        let mut t = Task::new(
            TaskName::new("sample").expect("name"),
            "/usr/bin/true",
            Trigger::Calendar {
                entries: vec![CalendarEntry::daily(21, 0)],
            },
        );
        t.env.insert("FOO".into(), "bar".into());
        t.tags = vec!["work".into()];
        t
    }

    #[test]
    fn task_input_round_trips() {
        let task = sample_task();
        let input = TaskInput::from(&task);
        let back = input.into_task(Some(&task)).expect("valid round trip");
        assert_eq!(back.name, task.name);
        assert_eq!(back.env, task.env);
        assert_eq!(back.trigger, task.trigger);
        assert_eq!(back.tags, task.tags);
        assert_eq!(back.created_at, task.created_at);
    }

    #[test]
    fn the_view_labels_how_the_task_runs() {
        let mut task = sample_task();
        // A plain executable: launchd runs it itself.
        assert_eq!(
            build_view(&task, true, None, None)
                .interpreter_label
                .as_deref(),
            Some("直接执行")
        );

        task.script_path = PathBuf::from("/opt/homebrew/bin/uv");
        task.args = vec!["run".into(), "/p/sync.py".into(), "--once".into()];
        assert_eq!(
            build_view(&task, true, None, None)
                .interpreter_label
                .as_deref(),
            Some("uv run · Homebrew")
        );

        // An interpreter with nothing to run cannot be described, and the
        // label is simply absent rather than wrong.
        task.args = Vec::new();
        task.script_path = PathBuf::from("/usr/bin/python3");
        assert_eq!(build_view(&task, true, None, None).interpreter_label, None);
    }

    #[test]
    fn the_interpreter_view_groups_and_describes_an_entry() {
        let recommended = Interpreter {
            kind: InterpreterKind::Python,
            program: PathBuf::from("/usr/bin/python3"),
            prefix_args: Vec::new(),
            version: Some("3.9.6".into()),
            origin: Origin::System,
            recommended: true,
            reason: Some("按 .py 扩展名推荐".into()),
        };
        let view = InterpreterView::from(&recommended);
        assert_eq!(view.id, "/usr/bin/python3");
        assert_eq!(view.label, "python3");
        assert_eq!(view.detail, "3.9.6 · 系统 /usr/bin/python3");
        assert_eq!(view.group, InterpreterGroup::Recommended);

        let venv = Interpreter {
            origin: Origin::ProjectVenv,
            program: PathBuf::from("/p/.venv/bin/python"),
            recommended: false,
            reason: None,
            ..recommended.clone()
        };
        assert_eq!(
            InterpreterView::from(&venv).group,
            InterpreterGroup::Project
        );

        let other = Interpreter {
            origin: Origin::Homebrew,
            program: PathBuf::from("/opt/homebrew/bin/python3"),
            ..venv
        };
        assert_eq!(InterpreterView::from(&other).group, InterpreterGroup::Path);
    }

    #[test]
    fn tail_window_is_clamped_and_zero_means_the_cap() {
        assert_eq!(tail_window(0), MAX_TAIL_BYTES);
        assert_eq!(tail_window(1024), 1024);
        assert_eq!(tail_window(u32::MAX), MAX_TAIL_BYTES);
    }

    /// A `Manual` task, i.e. `Task::is_service()`.
    fn service_task(keep_alive: bool) -> Task {
        let mut t = Task::new(
            TaskName::new("bridge").expect("name"),
            "/usr/bin/yes",
            Trigger::Manual,
        );
        t.keep_alive = keep_alive;
        t
    }

    /// Opens a store, records one run of `task` and hands back both, so the
    /// status tests can work with real rows rather than hand-built ones.
    fn store_with_run(task: &Task) -> (Store, RunId) {
        let store = Store::open_in_memory().expect("store");
        store.insert_task(task).expect("insert");
        let id = store
            .start_run(
                &task.name,
                TriggerKind::Manual,
                Utc::now(),
                std::path::Path::new("/tmp/out"),
                std::path::Path::new("/tmp/err"),
            )
            .expect("start");
        (store, id)
    }

    #[test]
    fn status_follows_the_table() {
        let task = sample_task();
        assert_eq!(status_of(&task, false, None, None), TaskStatus::Disabled);
        assert_eq!(status_of(&task, true, None, None), TaskStatus::Never);

        let (store, id) = store_with_run(&task);

        let running = store.get_run(id).expect("get").expect("row");
        assert_eq!(
            status_of(&task, true, None, Some(&running)),
            TaskStatus::Running
        );
        assert!(RunView::from(&running).running);
        assert!(RunView::from(&running).duration_ms.is_none());
        assert_eq!(RunView::from(&running).stop_reason, None);

        store
            .finish_run(id, Utc::now(), Some(0), Some(StopReason::Exited))
            .expect("finish ok");
        let ok = store.get_run(id).expect("get").expect("row");
        assert_eq!(status_of(&task, true, None, Some(&ok)), TaskStatus::Ok);
        assert!(RunView::from(&ok).duration_ms.is_some());
        assert_eq!(
            RunView::from(&ok).stop_reason,
            Some(StopReason::Exited),
            "stop_reason 要原样送到前端"
        );

        store
            .finish_run(id, Utc::now(), Some(1), Some(StopReason::Exited))
            .expect("finish fail");
        let failed = store.get_run(id).expect("get").expect("row");
        assert_eq!(
            status_of(&task, true, None, Some(&failed)),
            TaskStatus::Failed
        );
    }

    #[test]
    fn a_service_is_running_when_launchd_has_a_pid() {
        let task = service_task(true);
        let (store, id) = store_with_run(&task);
        let run = store.get_run(id).expect("get").expect("row");

        assert_eq!(
            status_of(&task, true, Some(4242), Some(&run)),
            TaskStatus::Running
        );
        // 没 pid 就不是运行中，哪怕那条 run 还没写 finished_at（runner 死了）
        assert_eq!(
            status_of(&task, true, None, Some(&run)),
            TaskStatus::Stopped
        );
        assert_eq!(
            status_of(&task, false, Some(4242), None),
            TaskStatus::Disabled
        );
    }

    #[test]
    fn a_stopped_service_is_stopped_not_failed() {
        let task = service_task(true);
        let (store, id) = store_with_run(&task);
        // 手动停止：runner 转发 SIGTERM，子进程被信号打死 -> exit_code 为空
        store
            .finish_run(id, Utc::now(), None, Some(StopReason::Stopped))
            .expect("finish");
        let run = store.get_run(id).expect("get").expect("row");
        assert_eq!(
            status_of(&task, true, None, Some(&run)),
            TaskStatus::Stopped
        );
    }

    #[test]
    fn a_crash_looping_service_is_failed_only_with_keep_alive() {
        let with = service_task(true);
        let (store, id) = store_with_run(&with);
        store
            .finish_run(id, Utc::now(), Some(1), Some(StopReason::Exited))
            .expect("finish");
        let run = store.get_run(id).expect("get").expect("row");

        assert_eq!(status_of(&with, true, None, Some(&run)), TaskStatus::Failed);
        // 没勾 keep_alive 的服务退出就是退出，launchd 不会再拉起来，
        // 那不是崩溃循环，只是停了。
        let without = service_task(false);
        assert_eq!(
            status_of(&without, true, None, Some(&run)),
            TaskStatus::Stopped
        );
    }

    #[test]
    fn a_service_that_never_ran_is_stopped() {
        let task = service_task(true);
        assert_eq!(status_of(&task, true, None, None), TaskStatus::Stopped);
    }

    #[test]
    fn uptime_needs_both_a_pid_and_an_unfinished_run() {
        let task = service_task(true);
        let (store, id) = store_with_run(&task);
        let running = store.get_run(id).expect("get").expect("row");

        assert!(uptime_secs(Some(9), Some(&running)).is_some_and(|s| s >= 0));
        assert_eq!(
            uptime_secs(None, Some(&running)),
            None,
            "没 pid 就没运行时长"
        );
        assert_eq!(uptime_secs(Some(9), None), None);

        store
            .finish_run(id, Utc::now(), Some(0), Some(StopReason::Exited))
            .expect("finish");
        let done = store.get_run(id).expect("get").expect("row");
        assert_eq!(
            uptime_secs(Some(9), Some(&done)),
            None,
            "已结束的 run 不该报运行时长"
        );
    }

    // ---- M3 §3.4: 其他 LaunchAgents -------------------------------------

    fn external(adoptable: launchkeeper_core::Adoptable) -> launchkeeper_core::ExternalAgent {
        launchkeeper_core::ExternalAgent {
            label: "com.example.backup-notes".into(),
            path: PathBuf::from("/tmp/agents/com.example.backup-notes.plist"),
            program: Some(PathBuf::from("/bin/zsh")),
            args: vec!["/Users/demo/scripts/backup-notes.sh".into()],
            working_dir: Some(PathBuf::from("/Users/demo/Notes")),
            env: BTreeMap::new(),
            trigger: Some(Trigger::Calendar {
                entries: vec![CalendarEntry::daily(3, 0)],
            }),
            keep_alive: false,
            stdout_path: Some(PathBuf::from("/tmp/backup-notes.log")),
            stderr_path: None,
            loaded: true,
            pid: Some(321),
            last_exit: Some(0),
            adoptable,
            managed_by_launchkeeper: false,
        }
    }

    #[test]
    fn the_external_view_composes_its_own_strings() {
        let view = ExternalAgentView::from(&external(launchkeeper_core::Adoptable::Yes));
        assert_eq!(view.label, "com.example.backup-notes");
        assert_eq!(
            view.full_path, "/tmp/agents/com.example.backup-notes.plist",
            "在 Finder 中显示要的是绝对路径，不是缩写过的那个"
        );
        assert_eq!(view.command, "/bin/zsh /Users/demo/scripts/backup-notes.sh");
        assert_eq!(view.trigger_text, "每天 03:00");
        assert!(view.adoptable);
        assert_eq!(view.adopt_reason, None);
        assert!(view.loaded);
        assert_eq!(view.pid, Some(321));

        // 不可接管的行灰掉，理由原样送到 tooltip
        let no = launchkeeper_core::Adoptable::No("第三方应用安装".into());
        let view = ExternalAgentView::from(&external(no));
        assert!(!view.adoptable);
        assert_eq!(view.adopt_reason.as_deref(), Some("第三方应用安装"));
    }

    #[test]
    fn home_is_abbreviated_for_display() {
        let home = PathBuf::from(std::env::var_os("HOME").expect("HOME"));
        assert_eq!(
            shorten_home(&home.join("Library/LaunchAgents/a.plist")),
            "~/Library/LaunchAgents/a.plist"
        );
        assert_eq!(shorten_home(std::path::Path::new("/etc/x")), "/etc/x");
    }

    #[test]
    fn the_plan_view_keeps_the_three_diff_groups_apart() {
        let mut task = sample_task();
        task.adopted = true;
        let plan = launchkeeper_core::AdoptionPlan {
            label: "com.example.backup-notes".into(),
            path: PathBuf::from("/tmp/a.plist"),
            backup_path: PathBuf::from("/tmp/a.plist.bak"),
            task,
            removed: vec![("ProgramArguments".into(), "/bin/zsh x.sh".into())],
            added: vec![
                ("ProgramArguments".into(), "runner run sample".into()),
                ("LaunchkeeperManaged".into(), "true".into()),
            ],
            kept: vec![("Label".into(), "com.example.backup-notes".into())],
        };
        let view = AdoptionPlanView::from(&plan);
        assert_eq!(view.name, "sample", "接管后的任务名就是 plan 里的那个");
        assert_eq!(view.backup_path, "/tmp/a.plist.bak");
        assert_eq!(view.removed.len(), 1);
        assert_eq!(view.added.len(), 2);
        assert_eq!(view.added[1].key, "LaunchkeeperManaged");
        assert_eq!(view.kept[0].value, "com.example.backup-notes");
    }

    #[test]
    fn the_view_carries_the_service_flags() {
        let task = service_task(true);
        let view = build_view(&task, true, Some(77), None);
        assert!(view.is_service);
        assert!(view.keep_alive);
        assert_eq!(view.loaded_pid, Some(77));
        assert_eq!(view.trigger_text, "手动启停");

        let scheduled = build_view(&sample_task(), true, None, None);
        assert!(!scheduled.is_service);
        assert!(!scheduled.keep_alive);
    }
}
