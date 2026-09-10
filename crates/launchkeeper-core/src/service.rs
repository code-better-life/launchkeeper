//! The layer that ties database, plist and launchctl together.
//!
//! Both the temporary CLI and the future Tauri app drive Launchkeeper through
//! this type, so that "write db, write plist, reload launchd" happens in one
//! order everywhere.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::agents::{self, AdoptionPlan};
use crate::env;
use crate::error::{Error, Result};
use crate::launchctl;
use crate::paths;
use crate::plist::{PlistOptions, write_plist};
use crate::plist_gen::write_plist_adopting;
use crate::store::Store;
use crate::task::{Task, TaskName};

/// How long [`Service::stop`] waits for SIGTERM to work before escalating to
/// SIGKILL. Comfortably longer than the runner's own 5 s child grace period,
/// so that the runner gets to finish its bookkeeping first.
pub const STOP_GRACE: Duration = Duration::from_secs(8);

/// How often [`Service::stop`] re-asks launchd whether the job is gone.
const STOP_POLL: Duration = Duration::from_millis(200);

/// High-level operations on tasks.
pub struct Service {
    store: Store,
    runner_path: Option<PathBuf>,
}

impl Service {
    /// Builds a service over `store`, using `runner_path` as the program in
    /// every generated plist.
    pub fn new(store: Store, runner_path: PathBuf) -> Service {
        Service {
            store,
            runner_path: Some(runner_path),
        }
    }

    /// Builds a service that has no runner binary. Everything that does not
    /// write a plist works; anything that does fails with [`Error::NoRunner`].
    /// Callers that only read (list, runs, logs, status) or only talk to
    /// launchd (disable, remove) use this so that a missing runner binary is
    /// not a hard error.
    pub fn without_runner(store: Store) -> Service {
        Service {
            store,
            runner_path: None,
        }
    }

    /// The underlying store, for reads the service does not wrap.
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// The runner binary baked into generated plists.
    ///
    /// # Errors
    /// [`Error::NoRunner`] when the service was built without one.
    pub fn runner_path(&self) -> Result<&Path> {
        self.runner_path.as_deref().ok_or(Error::NoRunner)
    }

    fn require_task(&self, name: &TaskName) -> Result<Task> {
        self.store
            .get_task(name)?
            .ok_or_else(|| Error::TaskNotFound(name.to_string()))
    }

    /// Creates the log directories `task` needs and answers the two paths
    /// that go into its [`PlistOptions`].
    fn prepare_plist_inputs(&self, task: &Task) -> Result<(PathBuf, PathBuf)> {
        let log_dir = paths::task_log_dir(&task.name)?;
        std::fs::create_dir_all(&log_dir).map_err(|e| Error::io(&log_dir, e))?;
        let runner_log = paths::runner_log_path(&task.name)?;
        if let Some(parent) = runner_log.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        Ok((runner_log, paths::data_dir()?))
    }

    fn write_plist_for(&self, task: &Task) -> Result<PathBuf> {
        let (runner_log, data_dir) = self.prepare_plist_inputs(task)?;
        let dest = paths::plist_path(task)?;
        let opts = PlistOptions {
            runner_path: self.runner_path()?,
            runner_log: &runner_log,
            data_dir: &data_dir,
        };
        write_plist(task, &opts, &dest)?;
        Ok(dest)
    }

    /// Adds a task to the database. launchd is not touched; call
    /// [`Service::enable`] for that.
    ///
    /// # Errors
    /// [`Error::TaskExists`], validation and database errors.
    pub fn add_task(&self, t: Task) -> Result<()> {
        self.store.insert_task(&t)
    }

    /// Updates a task. If the task is currently enabled, the plist is
    /// rewritten and launchd reloaded *first*, and the database row is only
    /// written once launchd accepted the new job: a failed reload therefore
    /// leaves the database exactly as it was, instead of leaving a row that
    /// describes a schedule launchd never got.
    ///
    /// The plist is rewritten and reloaded even when its content did not
    /// change (e.g. only the description was edited). Comparing plists to skip
    /// the reload would be an optimisation with its own failure mode; a reload
    /// of an unchanged job is harmless.
    ///
    /// # Errors
    /// [`Error::TaskNotFound`], validation, database and launchctl errors.
    /// A launchctl failure is reported with a note that the database was left
    /// untouched.
    pub fn update_task(&self, t: Task) -> Result<()> {
        t.validate()?;
        if self.store.get_task(&t.name)?.is_none() {
            return Err(Error::TaskNotFound(t.name.to_string()));
        }
        if self.is_enabled(&t.name)? {
            let dest = self.write_plist_for(&t)?;
            launchctl::reload(&dest, &t.label()).map_err(|e| {
                Error::InvalidTask(format!(
                    "重新加载 launchd job 失败，数据库未改动（任务保持原样）: {e}"
                ))
            })?;
        }
        self.store.update_task(&t)
    }

    /// Removes a task entirely: bootout if loaded, delete the plist, delete
    /// the database row (which cascades to runs), delete the log directory.
    ///
    /// # Errors
    /// launchctl, database and I/O errors. Missing files are not errors.
    pub fn remove_task(&self, name: &TaskName) -> Result<()> {
        // The label and the plist path both depend on whether the task was
        // adopted, so read the row before touching launchd. A row that is
        // already gone still gets the default label booted out, which is the
        // pre-M3 behaviour.
        let task = self.store.get_task(name)?;
        let label = task.as_ref().map_or_else(|| name.label(), Task::label);
        // bootout 对没加载的 label 本来就返回 Ok，不必先 print 一次。
        launchctl::bootout(&label)?;
        let plist = match &task {
            Some(t) => paths::plist_path(t)?,
            None => paths::own_plist_path(name)?,
        };
        if plist.exists() {
            std::fs::remove_file(&plist).map_err(|e| Error::io(&plist, e))?;
        }
        self.store.delete_task(name)?;
        let runner_log = paths::runner_log_path(name)?;
        match std::fs::remove_file(&runner_log) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io(&runner_log, e)),
        }
        let log_dir = paths::task_log_dir(name)?;
        match std::fs::remove_dir_all(&log_dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io(&log_dir, e)),
        }
        Ok(())
    }

    /// Writes the plist and (re)loads it into launchd. Also refreshes the
    /// cached login-shell PATH; a failure there is ignored, since the runner
    /// has its own fallbacks.
    ///
    /// # Errors
    /// [`Error::TaskNotFound`], I/O and launchctl errors.
    pub fn enable(&self, name: &TaskName) -> Result<()> {
        let task = self.require_task(name)?;
        let dest = self.write_plist_for(&task)?;
        launchctl::reload(&dest, &task.label())?;
        let _ = env::refresh_path_cache();
        Ok(())
    }

    /// Boots the job out of launchd and deletes its plist. The database row
    /// and the logs are kept.
    ///
    /// # Errors
    /// launchctl and I/O errors.
    pub fn disable(&self, name: &TaskName) -> Result<()> {
        let task = self.require_task(name)?;
        launchctl::bootout(&task.label())?;
        let plist = paths::plist_path(&task)?;
        if plist.exists() {
            std::fs::remove_file(&plist).map_err(|e| Error::io(&plist, e))?;
        }
        Ok(())
    }

    /// launchd's view of a task, or `None` when it is not enabled.
    ///
    /// "Enabled" means the plist exists *and* launchd knows the label, so
    /// answering that question already costs one `launchctl print`. Callers
    /// that also want the pid or the last exit status — the app's task list
    /// does — should use this instead of [`Service::is_enabled`] followed by
    /// a second [`launchctl::status`] call for the same label.
    ///
    /// It takes no `&self`: nothing here touches the store, and the app needs
    /// to make this call *outside* the mutex that guards the service.
    ///
    /// # Errors
    /// launchctl and path errors.
    pub fn enabled_status(task: &Task) -> Result<Option<launchctl::JobStatus>> {
        if !paths::plist_path(task)?.exists() {
            return Ok(None);
        }
        launchctl::status(&task.label())
    }

    /// True when the plist exists *and* launchd knows about the label.
    ///
    /// # Errors
    /// launchctl and path errors.
    pub fn is_enabled(&self, name: &TaskName) -> Result<bool> {
        // A name with no row is not enabled: where its plist would live now
        // depends on a `Task` that does not exist, and "no task" is a
        // perfectly good answer to "is it loaded".
        match self.store.get_task(name)? {
            Some(t) => Ok(Service::enabled_status(&t)?.is_some()),
            None => Ok(false),
        }
    }

    /// Asks launchd to run the job now.
    ///
    /// For a service task ([`Task::is_service`]) this is exactly
    /// [`Service::start`]: "run it now" and "start the service" are the same
    /// request, and the caller should not have to know which kind of task it
    /// is holding.
    ///
    /// # Errors
    /// [`Error::NotEnabled`] when a non-service job is not loaded; launchctl
    /// errors.
    pub fn run_now(&self, name: &TaskName) -> Result<()> {
        let task = self.require_task(name)?;
        if task.is_service() {
            return self.start(name);
        }
        let label = task.label();
        if launchctl::status(&label)?.is_none() {
            return Err(Error::NotEnabled(name.to_string()));
        }
        launchctl::kickstart(&label)
    }

    /// Starts a service: loads the job into launchd if it is not loaded yet,
    /// then `kickstart`s it.
    ///
    /// Enabling first is deliberate — a service the user asks to start should
    /// start, not fail with "not enabled" — and it is also why this needs a
    /// runner path when the task was not enabled before.
    ///
    /// # Errors
    /// [`Error::TaskNotFound`], [`Error::NoRunner`], launchctl and I/O errors.
    pub fn start(&self, name: &TaskName) -> Result<()> {
        if !self.is_enabled(name)? {
            self.enable(name)?;
        }
        launchctl::kickstart(&self.require_task(name)?.label())
    }

    /// Stops a running service.
    ///
    /// Sends SIGTERM through `launchctl kill`, which the runner catches and
    /// forwards to the script before exiting 0 — and an exit code of 0 is
    /// exactly what keeps `KeepAlive = {SuccessfulExit = false}` from
    /// restarting the job (measured, `docs/M2.5-design.md` §3.1). The job
    /// stays loaded, so it can be started again without rewriting the plist.
    ///
    /// A job that is not loaded, or loaded but not running, is a no-op. If
    /// the process is still there after [`STOP_GRACE`], it gets SIGKILL.
    ///
    /// # Errors
    /// launchctl errors.
    pub fn stop(&self, name: &TaskName) -> Result<()> {
        let label = self.require_task(name)?.label();
        let Some(status) = launchctl::status(&label)? else {
            return Ok(()); // 没加载，本来就没在跑
        };
        if status.pid.is_none() {
            return Ok(()); // 加载了但没在跑
        }
        launchctl::kill(&label, "TERM")?;

        let deadline = Instant::now() + STOP_GRACE;
        while Instant::now() < deadline {
            std::thread::sleep(STOP_POLL);
            match launchctl::status(&label)? {
                None => return Ok(()),
                Some(s) if s.pid.is_none() => return Ok(()),
                Some(_) => {}
            }
        }
        launchctl::kill(&label, "KILL")
    }

    // ---- M3 §3.2: LaunchAgents Launchkeeper did not write ----------------

    /// The diff [`Service::adopt`] would apply to `label`'s plist.
    ///
    /// Read-only: nothing is written, nothing is loaded. The UI shows this
    /// before asking for confirmation, and the CLI's `adopt --dry-run`
    /// prints it.
    ///
    /// # Errors
    /// [`Error::AgentNotFound`], [`Error::Adopt`] when the agent is not
    /// adoptable, [`Error::NoRunner`], plus I/O and plist errors.
    pub fn adoption_plan(&self, label: &str) -> Result<AdoptionPlan> {
        let dir = paths::launch_agents_dir()?;
        let agent = agents::find(&dir, label)?;
        agents::adoption_plan(&agent, self.runner_path()?)
    }

    /// Takes a LaunchAgent somebody else wrote under management, in place.
    ///
    /// The label does not change — that is the whole point: whatever else on
    /// the machine refers to this job by label keeps working, and the job
    /// keeps its identity in `launchctl print`. What changes is that
    /// `ProgramArguments` now points at the Launchkeeper runner, so runs,
    /// exit codes and per-run logs start being recorded like any other task.
    ///
    /// In order: the original bytes are copied to `<plist>.bak`, the task row
    /// is inserted (`adopted = true`, remembering the plist path), the plist
    /// is rewritten with the `LaunchkeeperManaged` marker, and launchd is
    /// reloaded. Any failure after the copy rolls the earlier steps back, so
    /// a half-adopted job is not a state the user can end up in.
    ///
    /// If the rollback itself cannot put the original file back, the `.bak`
    /// is **kept** and its path is part of the returned error: the one thing
    /// worse than a failed adoption is a failed adoption that also deleted
    /// the only copy of the user's plist.
    ///
    /// Refuses when `<plist>.bak` already exists: that file is the only copy
    /// of somebody's original, and overwriting it to make room for a second
    /// backup would destroy exactly what the backup is for.
    ///
    /// # Errors
    /// [`Error::AgentNotFound`], [`Error::Adopt`], [`Error::TaskExists`],
    /// [`Error::NoRunner`], launchctl, database and I/O errors.
    pub fn adopt(&self, label: &str) -> Result<Task> {
        self.adopt_with(label, true)
    }

    /// [`Service::adopt`], with a say over the last step.
    ///
    /// `reload = false` writes the plist and the database row but leaves
    /// launchd alone: the job that is loaded right now keeps running under
    /// the *old* definition until the next login (or until the task is
    /// enabled by hand). That is the honest meaning of the 接管确认框's
    /// unticked 「接管后立即重新加载」 — the file has changed, launchd's
    /// in-memory copy has not — and it is the option for a job that must not
    /// be interrupted at this moment.
    ///
    /// # Errors
    /// The same as [`Service::adopt`].
    pub fn adopt_with(&self, label: &str, reload: bool) -> Result<Task> {
        let plan = self.adoption_plan(label)?;
        if plan.backup_path.exists() {
            return Err(Error::Adopt(format!(
                "备份文件已存在: {} —— 它是原始 plist 的唯一副本，接管不会覆盖它。\
                 请先确认它的内容（上一次接管可能没有撤销干净），处理掉再重试",
                plan.backup_path.display()
            )));
        }
        if self.store.get_task(&plan.task.name)?.is_some() {
            return Err(Error::TaskExists(plan.task.name.to_string()));
        }

        std::fs::copy(&plan.path, &plan.backup_path)
            .map_err(|e| Error::io(&plan.backup_path, e))?;

        // From here on every failure restores what it found.
        //
        // `restore_plist` says whether this file has already been rewritten.
        // When it has not, the `.bak` is a pure duplicate of an untouched
        // file and can go. When it has, the `.bak` is the *only* copy of the
        // user's original, so it is consumed by a restore that is known to
        // have landed — and kept, with its path in the error, when the
        // restore fails. Deleting it on a restore whose outcome we did not
        // check is how originals are lost for good.
        // Answers with the error to report and whether the user's own file is
        // back where it belongs — the caller below needs the second half
        // before it hands launchd a path again.
        let rollback = |restore_plist: bool, drop_row: bool, cause: Error| -> (Error, bool) {
            if drop_row {
                let _ = self.store.delete_task(&plan.task.name);
            }
            if !restore_plist {
                let _ = std::fs::remove_file(&plan.backup_path);
                return (cause, true);
            }
            match restore_backup(&plan.backup_path, &plan.path) {
                Ok(()) => (cause, true),
                Err(e) => (
                    Error::Adopt(format!(
                        "{cause}；回滚时无法把原 plist 还原到 {}: {e}。\
                         原文件的备份保留在 {}，请手动把它移回去（`mv` 即可），不要删掉它",
                        plan.path.display(),
                        plan.backup_path.display()
                    )),
                    false,
                ),
            }
        };

        if let Err(e) = self.store.insert_task(&plan.task) {
            return Err(rollback(false, false, e).0);
        }

        let (runner_log, data_dir) = match self.prepare_plist_inputs(&plan.task) {
            Ok(v) => v,
            Err(e) => return Err(rollback(false, true, e).0),
        };
        let opts = PlistOptions {
            runner_path: self.runner_path()?,
            runner_log: &runner_log,
            data_dir: &data_dir,
        };
        if let Err(e) = write_plist_adopting(&plan.task, &opts, &plan.path) {
            return Err(rollback(true, true, e).0);
        }

        if reload && let Err(e) = launchctl::reload(&plan.path, &plan.label) {
            let (e, restored) = rollback(true, true, e);
            // The failed reload booted the old job out first, so put the
            // original definition back — but only when the file at that path
            // *is* the original again. Bootstrapping the rewritten plist
            // whose database row was just deleted would be worse than
            // leaving the job unloaded.
            if restored {
                let _ = launchctl::reload(&plan.path, &plan.label);
            }
            return Err(e);
        }
        let _ = env::refresh_path_cache();
        Ok(plan.task)
    }

    /// Undoes [`Service::adopt`]: puts the original plist back, byte for
    /// byte, and forgets the task.
    ///
    /// The `.bak` is renamed (not rewritten) over the managed plist, so the
    /// file that comes back is the one that was there — same bytes, same
    /// formatting, comments and all. Run logs on disk are deliberately kept;
    /// only the database row goes, along with the run history that cascades
    /// off it.
    ///
    /// # Errors
    /// [`Error::TaskNotFound`], [`Error::Adopt`] when the task was not
    /// adopted or its backup is missing, launchctl, database and I/O errors.
    pub fn unadopt(&self, name: &TaskName) -> Result<()> {
        let task = self.require_task(name)?;
        if !task.adopted {
            return Err(Error::Adopt(format!(
                "{name} 不是接管来的任务，撤销接管对它没有意义；要删除请用 remove"
            )));
        }
        let plist = paths::plist_path(&task)?;
        let backup = agents::backup_path(&plist);
        if !backup.exists() {
            return Err(Error::Adopt(format!(
                "找不到备份文件 {}，无法按字节还原原始 plist",
                backup.display()
            )));
        }
        let label = task.label();
        launchctl::bootout(&label)?;
        std::fs::rename(&backup, &plist).map_err(|e| Error::io(&plist, e))?;
        self.store.delete_task(name)?;
        launchctl::reload(&plist, &label).map_err(|e| {
            Error::Adopt(format!(
                "原 plist 已按字节还原到 {}，数据库行也已删除，但重新加载失败: {e}",
                plist.display()
            ))
        })
    }

    /// Loads a LaunchAgent Launchkeeper does not manage into launchd.
    ///
    /// A label launchd already knows is left alone: "enable" is a state, not
    /// a verb, and this job belongs to somebody else. Reloading it would
    /// bootout a *running* third-party job — a mail agent, a sync daemon —
    /// and start it again, which is not what a user who clicked a switch that
    /// was already on asked for. So the answer is `Ok(())` and launchd is not
    /// touched.
    ///
    /// Otherwise `reload` (bootout, then bootstrap): launchd answers a
    /// `bootstrap` of an already-loaded job with EIO, and the plist itself is
    /// never touched either way.
    ///
    /// # Errors
    /// launchctl errors.
    pub fn external_enable(&self, label: &str, path: &Path) -> Result<()> {
        if launchctl::status(label)?.is_some() {
            return Ok(());
        }
        launchctl::reload(path, label)
    }

    /// Boots an unmanaged LaunchAgent out of launchd, leaving its plist file
    /// exactly where it is (so it comes back at the next login).
    ///
    /// # Errors
    /// launchctl errors.
    pub fn external_disable(&self, label: &str) -> Result<()> {
        launchctl::bootout(label)
    }

    /// `launchctl kickstart` on an unmanaged label.
    ///
    /// No run is recorded: without the runner in `ProgramArguments` there is
    /// nothing to record it. Adopt the agent to get run history.
    ///
    /// # Errors
    /// launchctl errors, notably when the job is not loaded.
    pub fn external_kickstart(&self, label: &str) -> Result<()> {
        launchctl::kickstart(label)
    }

    // ---- M4 §1: AI task insight ------------------------------------------

    /// The stored explanation of a task, or `None` when it has never been
    /// explained. A plain read — no network, no key needed.
    ///
    /// # Errors
    /// [`Error::TaskNotFound`] and database errors.
    pub fn insight(&self, name: &TaskName) -> Result<Option<crate::ai::Insight>> {
        self.require_task(name)?;
        self.store.get_insight(name)
    }

    /// Explains a task with the configured model and stores the answer.
    ///
    /// With `refresh = false` a stored insight is returned as it is and no
    /// request is made: an explanation costs money and several seconds, and
    /// opening the tab again is not a reason to pay either. `refresh = true`
    /// always calls the model and overwrites.
    ///
    /// What gets sent is decided in [`crate::ai::build_prompt`], not here:
    /// this method's job is to gather the task, its newest
    /// [`crate::ai::RUNS_IN_PROMPT`] runs and the tail of the newest run's
    /// captured output, and to write the answer back. In particular the
    /// environment *values* never leave the machine — see
    /// [`crate::ai::redact_env`].
    ///
    /// # Errors
    /// [`Error::TaskNotFound`], [`Error::AiNoKey`], [`Error::AiHttp`],
    /// [`Error::Ai`], and database errors.
    pub fn explain_task(
        &self,
        name: &TaskName,
        cfg: &crate::ai::AiConfig,
        lang: crate::ai::Lang,
        refresh: bool,
    ) -> Result<crate::ai::Insight> {
        let task = self.require_task(name)?;
        if !refresh && let Some(existing) = self.store.get_insight(name)? {
            return Ok(existing);
        }
        let Some(api_key) = crate::ai::get_api_key()? else {
            return Err(Error::AiNoKey);
        };
        let runs = self.store.list_runs(name, crate::ai::RUNS_IN_PROMPT)?;
        let log_tail = runs.first().map(log_tail_of).unwrap_or_default();
        let input = crate::ai::ExplainInput {
            task,
            runs,
            log_tail,
            lang,
        };
        let insight = crate::ai::explain(cfg, &api_key, &input)?;
        self.store.upsert_insight(&insight)?;
        Ok(insight)
    }

    /// [`Service::stop`] followed by [`Service::start`].
    ///
    /// Not `kickstart -k`: that would kill the runner without giving it the
    /// chance to shut the script down, and the run row would never get a
    /// `finished_at`.
    ///
    /// # Errors
    /// Whatever `stop` and `start` return.
    pub fn restart(&self, name: &TaskName) -> Result<()> {
        self.stop(name)?;
        self.start(name)
    }
}

/// The tail of one run's captured output, stdout then stderr, each labelled
/// and each bounded at [`crate::ai::MAX_LOG_BYTES`].
///
/// Reading is lossy on purpose: a script's output is arbitrary bytes, and a
/// stray non-UTF-8 sequence must not be the reason 解读 fails. A missing or
/// unreadable file contributes nothing rather than an error — the run row can
/// outlive its log files (`prune_runs` deletes those, `unadopt` keeps them),
/// and an insight built from half the evidence beats no insight at all.
fn log_tail_of(run: &crate::store::Run) -> String {
    let half = crate::ai::MAX_LOG_BYTES / 2;
    let read = |p: &Path| -> Option<String> {
        if p.as_os_str().is_empty() {
            return None;
        }
        let bytes = std::fs::read(p).ok()?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let trimmed = crate::ai::truncate_tail(&text, half);
        (!trimmed.trim().is_empty()).then_some(trimmed)
    };
    let mut out = String::new();
    if let Some(s) = read(&run.stdout_path) {
        out.push_str("--- stdout ---\n");
        out.push_str(&s);
        if !s.ends_with('\n') {
            out.push('\n');
        }
    }
    if let Some(s) = read(&run.stderr_path) {
        out.push_str("--- stderr ---\n");
        out.push_str(&s);
    }
    out
}

/// Puts `backup`'s bytes back at `plist`, consuming the backup only when that
/// worked.
///
/// `rename` first, because it is atomic and keeps the file's own bytes and
/// permissions; a `copy` + `remove` fallback covers the cases `rename` cannot
/// do (the two paths on different filesystems, most plausibly a
/// `LAUNCHKEEPER_LAUNCH_AGENTS_DIR` pointed somewhere exotic). The backup is
/// removed only after the copy has landed, and a failure to remove it — the
/// bytes are already safely back — is not worth failing the restore over.
///
/// # Errors
/// The `rename` error when the `copy` fallback also fails, so that the
/// message describes the first, more interesting failure.
fn restore_backup(backup: &Path, plist: &Path) -> std::io::Result<()> {
    let Err(rename_err) = std::fs::rename(backup, plist) else {
        return Ok(());
    };
    std::fs::copy(backup, plist).map_err(|_| rename_err)?;
    let _ = std::fs::remove_file(backup);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn restore_backup_moves_the_original_back_and_takes_the_backup_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let plist = dir.path().join("job.plist");
        let backup = dir.path().join("job.plist.bak");
        std::fs::write(&plist, "rewritten").unwrap();
        std::fs::write(&backup, "original").unwrap();

        restore_backup(&backup, &plist).unwrap();
        assert_eq!(std::fs::read_to_string(&plist).unwrap(), "original");
        assert!(!backup.exists(), ".bak 应当被移回去，不再留着");
    }

    /// The failure the rollback in [`Service::adopt_with`] must survive: the
    /// restore cannot land. The backup has to still be there afterwards —
    /// it is the only copy of the user's file.
    #[test]
    fn a_restore_that_cannot_land_keeps_the_backup() {
        let dir = tempfile::tempdir().unwrap();
        let plist = dir.path().join("job.plist");
        let backup = dir.path().join("job.plist.bak");
        std::fs::write(&plist, "rewritten").unwrap();
        std::fs::write(&backup, "original").unwrap();
        // Read-only file in a read-only directory: `rename` cannot replace
        // the entry and `copy` cannot open the destination for writing.
        std::fs::set_permissions(&plist, std::fs::Permissions::from_mode(0o444)).unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();

        let err = restore_backup(&backup, &plist).unwrap_err();

        // Put the permissions back before asserting, so a failing assert
        // still leaves a removable temp directory behind.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(backup.exists(), "还原失败时必须保留备份: {err}");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "original");
    }
}
