//! SQLite storage for task metadata and run history.
//!
//! launchd owns "is this job loaded"; this database owns everything launchd
//! has no place for: descriptions, tags, run history, log paths.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::types::Value;
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Row, TransactionBehavior, params, params_from_iter,
};
use serde::{Deserialize, Serialize};

use crate::ai::Insight;
use crate::error::{Error, Result};
use crate::task::{Task, TaskName, Trigger};

/// How long a connection waits for a competing writer before giving up.
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Migration scripts, applied in order. Index + 1 is the target
/// `PRAGMA user_version`. Adding a file under `migrations/` is not enough:
/// it has to be listed here too, and only ever appended to.
const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init.sql", include_str!("../migrations/0001_init.sql")),
    (
        "0002_stop_reason.sql",
        include_str!("../migrations/0002_stop_reason.sql"),
    ),
    (
        "0003_adopted.sql",
        include_str!("../migrations/0003_adopted.sql"),
    ),
    (
        "0004_ai_insights.sql",
        include_str!("../migrations/0004_ai_insights.sql"),
    ),
];

/// Primary key of a row in `runs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunId(pub i64);

/// Why a run happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    /// launchd started it.
    Scheduled,
    /// The user asked for it.
    Manual,
}

impl TriggerKind {
    /// The string stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            TriggerKind::Scheduled => "scheduled",
            TriggerKind::Manual => "manual",
        }
    }

    /// Parses the database representation.
    ///
    /// # Errors
    /// [`Error::Corrupt`] for anything else.
    pub fn parse(s: &str) -> Result<TriggerKind> {
        match s {
            "scheduled" => Ok(TriggerKind::Scheduled),
            "manual" => Ok(TriggerKind::Manual),
            other => Err(Error::Corrupt(format!("未知 trigger_kind: {other:?}"))),
        }
    }
}

/// Why a run ended. Stored in `runs.stop_reason`; `None` on rows a v1 runner
/// wrote and on runs that have not finished yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The script finished on its own, for whatever exit code.
    Exited,
    /// The runner was asked to stop (SIGTERM / SIGINT, e.g. `launchctl kill
    /// TERM` behind [`crate::Service::stop`]) and terminated the script.
    Stopped,
    /// The run exceeded `timeout_secs` and the script was killed.
    Timeout,
}

impl StopReason {
    /// The string stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            StopReason::Exited => "exited",
            StopReason::Stopped => "stopped",
            StopReason::Timeout => "timeout",
        }
    }

    /// A short Chinese label for lists and run history.
    pub fn describe(self) -> &'static str {
        match self {
            StopReason::Exited => "正常结束",
            StopReason::Stopped => "手动停止",
            StopReason::Timeout => "超时",
        }
    }

    /// Parses the database representation.
    ///
    /// # Errors
    /// [`Error::Corrupt`] for anything else.
    pub fn parse(s: &str) -> Result<StopReason> {
        match s {
            "exited" => Ok(StopReason::Exited),
            "stopped" => Ok(StopReason::Stopped),
            "timeout" => Ok(StopReason::Timeout),
            other => Err(Error::Corrupt(format!("未知 stop_reason: {other:?}"))),
        }
    }
}

/// One recorded execution of a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// Row id.
    pub id: RunId,
    /// Task this run belongs to.
    pub task_name: TaskName,
    /// When the runner started the script.
    pub started_at: DateTime<Utc>,
    /// When it finished; `None` while still running.
    pub finished_at: Option<DateTime<Utc>>,
    /// Exit code; `None` with a `finished_at` set means killed by a signal.
    pub exit_code: Option<i32>,
    /// Pid of the user script.
    pub pid: Option<u32>,
    /// Absolute path of the captured stdout file.
    pub stdout_path: PathBuf,
    /// Absolute path of the captured stderr file.
    pub stderr_path: PathBuf,
    /// Why the run happened.
    pub trigger_kind: TriggerKind,
    /// Why the run ended; `None` while it is still running, and on rows
    /// written before schema v2.
    pub stop_reason: Option<StopReason>,
}

impl Run {
    /// True when the run finished with exit code 0.
    pub fn succeeded(&self) -> bool {
        self.finished_at.is_some() && self.exit_code == Some(0)
    }
}

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse_ts(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| Error::Corrupt(format!("时间戳 {s:?} 无法解析: {e}")))
}

const TASK_COLUMNS: &str = "name, display_name, description, script_path, args, working_dir, env, \
     \"trigger\", keep_alive, timeout_secs, tags, favorite, notify_on_fail, created_at, updated_at, \
     adopted, plist_path";

const RUN_COLUMNS: &str = "id, task_name, started_at, finished_at, exit_code, pid, stdout_path, stderr_path, trigger_kind, stop_reason";

/// The database handle. Cheap to keep around for the process lifetime.
pub struct Store {
    conn: Connection,
}

/// Puts the database into WAL mode, tolerating the one way that legitimately
/// fails: `PRAGMA journal_mode=WAL` needs an exclusive lock and, unlike an
/// ordinary statement, does **not** go through the busy handler — so two
/// processes opening the same fresh database at the same moment (the CLI and
/// the runner, say) would have one of them fail outright.
///
/// The journal mode is a property of the file, not of the connection, so
/// losing that race is harmless as long as someone wins it: the mode is read
/// first and only set when it is not WAL already, and a `SQLITE_BUSY` from
/// the attempt is ignored rather than propagated.
fn set_wal(conn: &Connection) -> Result<()> {
    let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    if mode.eq_ignore_ascii_case("wal") {
        return Ok(());
    }
    match conn.pragma_update(None, "journal_mode", "WAL") {
        Ok(()) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::DatabaseBusy
                || e.code == rusqlite::ErrorCode::DatabaseLocked =>
        {
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

impl Store {
    /// Opens (creating parent directories as needed) and migrates the database
    /// at `path`, enabling WAL and foreign keys.
    ///
    /// # Errors
    /// I/O or SQLite errors.
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let conn = Connection::open(path)?;
        // 多个进程（CLI、runner、未来的 app）会同时开这个库，明确给一个等锁上限，
        // 不依赖 rusqlite 的默认值。
        conn.busy_timeout(BUSY_TIMEOUT)?;
        set_wal(&conn)?;
        Store::finish_open(conn)
    }

    /// Opens the database at `path` **read-only**: no directory is created,
    /// no pragma that writes is set, no migration is run, and `busy_timeout`
    /// (rather than the module's five-second [`BUSY_TIMEOUT`]) bounds the
    /// wait for a competing writer.
    ///
    /// This exists for callers that must not have side effects and must not
    /// block for long — shell completion above all: a TAB that creates a
    /// data directory, converts a database to WAL, takes a migration's
    /// `BEGIN IMMEDIATE` lock or freezes the terminal for five seconds is a
    /// worse bug than a missing completion.
    ///
    /// # Errors
    /// SQLite errors, including "unable to open database file" when `path`
    /// does not exist (read-only never creates it), and [`Error::Corrupt`]
    /// when the file's schema is older than this build expects — migrating
    /// it is exactly what this constructor promises not to do.
    pub fn open_read_only(path: &Path, busy_timeout: std::time::Duration) -> Result<Store> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(busy_timeout)?;
        let store = Store { conn };
        let have = store.schema_version()?;
        let want = MIGRATIONS.len() as i64;
        if have < want {
            return Err(Error::Corrupt(format!(
                "数据库 schema 版本是 {have}，这个版本期望 {want}；\
                 只读打开不会迁移，请先用普通方式打开一次"
            )));
        }
        Ok(store)
    }

    /// An in-memory database, migrated the same way. For tests.
    ///
    /// # Errors
    /// SQLite errors.
    pub fn open_in_memory() -> Result<Store> {
        Store::finish_open(Connection::open_in_memory()?)
    }

    fn finish_open(conn: Connection) -> Result<Store> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    /// Applies every migration whose index is beyond `PRAGMA user_version`.
    /// Idempotent, and safe against a second process opening the same file at
    /// the same time: each migration runs inside its own `BEGIN IMMEDIATE`
    /// transaction that re-reads `user_version` after taking the write lock,
    /// so the loser of the race sees the winner's version and skips.
    fn migrate(&mut self) -> Result<()> {
        for (i, (name, sql)) in MIGRATIONS.iter().enumerate() {
            let version = i as i64 + 1;
            let tx = self
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let current: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            if version <= current {
                tx.rollback()?;
                continue;
            }
            tx.execute_batch(sql)
                .map_err(|e| Error::Corrupt(format!("迁移 {name} 失败: {e}")))?;
            // pragma_update 不接受参数绑定，version 是本地常量，不是外部输入。
            tx.execute_batch(&format!("PRAGMA user_version = {version}"))?;
            tx.commit()?;
        }
        Ok(())
    }

    /// The schema version currently applied.
    ///
    /// # Errors
    /// SQLite errors.
    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    /// Escape hatch for callers that need raw SQL (the CLI's `status`).
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    // ---- tasks -----------------------------------------------------------

    /// Inserts a new task.
    ///
    /// # Errors
    /// [`Error::TaskExists`] when the name is taken; validation errors from
    /// [`Task::validate`].
    pub fn insert_task(&self, t: &Task) -> Result<()> {
        t.validate()?;
        if self.get_task(&t.name)?.is_some() {
            return Err(Error::TaskExists(t.name.to_string()));
        }
        let sql = format!(
            "INSERT INTO tasks ({TASK_COLUMNS}) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)"
        );
        self.conn.execute(&sql, params_from_iter(task_params(t)?))?;
        Ok(())
    }

    /// Overwrites an existing task (matched by name).
    ///
    /// # Errors
    /// [`Error::TaskNotFound`] when the task does not exist.
    pub fn update_task(&self, t: &Task) -> Result<()> {
        t.validate()?;
        let sql = "UPDATE tasks SET display_name=?2, description=?3, script_path=?4, args=?5, \
             working_dir=?6, env=?7, \"trigger\"=?8, keep_alive=?9, timeout_secs=?10, tags=?11, \
             favorite=?12, notify_on_fail=?13, created_at=?14, updated_at=?15, adopted=?16, \
             plist_path=?17 WHERE name=?1";
        let n = self.conn.execute(sql, params_from_iter(task_params(t)?))?;
        if n == 0 {
            return Err(Error::TaskNotFound(t.name.to_string()));
        }
        Ok(())
    }

    /// Deletes a task and, by cascade, its run rows. Deleting a missing task
    /// is not an error.
    ///
    /// # Errors
    /// SQLite errors.
    pub fn delete_task(&self, name: &TaskName) -> Result<()> {
        self.conn
            .execute("DELETE FROM tasks WHERE name=?1", params![name.as_str()])?;
        Ok(())
    }

    /// Looks a task up by name.
    ///
    /// # Errors
    /// SQLite or decoding errors.
    pub fn get_task(&self, name: &TaskName) -> Result<Option<Task>> {
        let sql = format!("SELECT {TASK_COLUMNS} FROM tasks WHERE name=?1");
        let row = self
            .conn
            .query_row(&sql, params![name.as_str()], |r| Ok(row_to_task(r)))
            .optional()?;
        row.transpose()
    }

    /// Every task, ordered by `display_name`.
    ///
    /// # Errors
    /// SQLite or decoding errors.
    pub fn list_tasks(&self) -> Result<Vec<Task>> {
        let sql = format!("SELECT {TASK_COLUMNS} FROM tasks ORDER BY display_name, name");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |r| Ok(row_to_task(r)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }

    // ---- runs ------------------------------------------------------------

    /// Records the start of a run and returns its id.
    ///
    /// # Errors
    /// SQLite errors, including a foreign-key violation for an unknown task.
    pub fn start_run(
        &self,
        name: &TaskName,
        kind: TriggerKind,
        started_at: DateTime<Utc>,
        stdout_path: &Path,
        stderr_path: &Path,
    ) -> Result<RunId> {
        self.conn.execute(
            "INSERT INTO runs (task_name, started_at, stdout_path, stderr_path, trigger_kind) \
             VALUES (?1,?2,?3,?4,?5)",
            params![
                name.as_str(),
                ts(started_at),
                stdout_path.to_string_lossy(),
                stderr_path.to_string_lossy(),
                kind.as_str(),
            ],
        )?;
        Ok(RunId(self.conn.last_insert_rowid()))
    }

    /// Records the end of a run. `exit_code` is `None` when the process was
    /// killed by a signal; `stop_reason` says which of the three ways the run
    /// ended (see [`StopReason`]).
    ///
    /// # Errors
    /// [`Error::RunNotFound`] when the id is unknown.
    pub fn finish_run(
        &self,
        id: RunId,
        finished_at: DateTime<Utc>,
        exit_code: Option<i32>,
        stop_reason: Option<StopReason>,
    ) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE runs SET finished_at=?2, exit_code=?3, stop_reason=?4 WHERE id=?1",
            params![
                id.0,
                ts(finished_at),
                exit_code,
                stop_reason.map(StopReason::as_str)
            ],
        )?;
        if n == 0 {
            return Err(Error::RunNotFound(id.0));
        }
        Ok(())
    }

    /// Stores the pid of the running script.
    ///
    /// # Errors
    /// [`Error::RunNotFound`] when the id is unknown.
    pub fn set_run_pid(&self, id: RunId, pid: u32) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE runs SET pid=?2 WHERE id=?1",
            params![id.0, i64::from(pid)],
        )?;
        if n == 0 {
            return Err(Error::RunNotFound(id.0));
        }
        Ok(())
    }

    /// The most recent `limit` runs of a task, newest first.
    ///
    /// # Errors
    /// SQLite or decoding errors.
    pub fn list_runs(&self, name: &TaskName, limit: usize) -> Result<Vec<Run>> {
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE task_name=?1 ORDER BY started_at DESC, id DESC LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![name.as_str(), limit as i64], |r| Ok(row_to_run(r)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }

    /// One run by its row id.
    ///
    /// # Errors
    /// SQLite or decoding errors.
    pub fn get_run(&self, id: RunId) -> Result<Option<Run>> {
        let sql = format!("SELECT {RUN_COLUMNS} FROM runs WHERE id=?1");
        let mut stmt = self.conn.prepare(&sql)?;
        let row = stmt
            .query_row(params![id.0], |r| Ok(row_to_run(r)))
            .optional()?;
        row.transpose()
    }

    /// The newest run of a task, if any.
    ///
    /// # Errors
    /// SQLite or decoding errors.
    pub fn last_run(&self, name: &TaskName) -> Result<Option<Run>> {
        Ok(self.list_runs(name, 1)?.into_iter().next())
    }

    /// Deletes every run of `name` beyond the newest `keep` and returns the
    /// deleted rows so the caller can remove their log files.
    ///
    /// # Errors
    /// SQLite or decoding errors.
    pub fn prune_runs(&self, name: &TaskName, keep: usize) -> Result<Vec<Run>> {
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE task_name=?1 \
             ORDER BY started_at DESC, id DESC LIMIT -1 OFFSET ?2"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![name.as_str(), keep as i64], |r| Ok(row_to_run(r)))?;
        let mut doomed = Vec::new();
        for r in rows {
            doomed.push(r??);
        }
        drop(stmt);
        if doomed.is_empty() {
            return Ok(doomed);
        }
        let ids: Vec<String> = doomed.iter().map(|r| r.id.0.to_string()).collect();
        // id 是我们刚从库里读出来的整数，拼进 SQL 里没有注入面。
        self.conn.execute_batch(&format!(
            "BEGIN IMMEDIATE; DELETE FROM runs WHERE id IN ({}); COMMIT;",
            ids.join(",")
        ))?;
        Ok(doomed)
    }

    // ---- AI insights (M4 §1) ---------------------------------------------

    /// The stored explanation of a task, or `None` when it has never been
    /// explained.
    ///
    /// # Errors
    /// SQLite or decoding errors.
    pub fn get_insight(&self, name: &TaskName) -> Result<Option<Insight>> {
        let row = self
            .conn
            .query_row(
                "SELECT task_name, created_at, model, prompt_hash, content \
                 FROM ai_insights WHERE task_name=?1",
                params![name.as_str()],
                |r| Ok(row_to_insight(r)),
            )
            .optional()?;
        row.transpose()
    }

    /// Stores an explanation, replacing the task's previous one.
    ///
    /// One row per task by construction (`task_name` is the primary key), so
    /// re-explaining overwrites rather than accumulating: the UI shows "the"
    /// insight and its timestamp, not a history nobody asked for.
    ///
    /// # Errors
    /// SQLite errors, including a foreign-key violation when the task does
    /// not exist.
    pub fn upsert_insight(&self, insight: &Insight) -> Result<()> {
        self.conn.execute(
            "INSERT INTO ai_insights (task_name, created_at, model, prompt_hash, content) \
             VALUES (?1,?2,?3,?4,?5) \
             ON CONFLICT(task_name) DO UPDATE SET \
               created_at=excluded.created_at, model=excluded.model, \
               prompt_hash=excluded.prompt_hash, content=excluded.content",
            params![
                insight.task_name.as_str(),
                ts(insight.created_at),
                insight.model,
                insight.prompt_hash,
                insight.content,
            ],
        )?;
        Ok(())
    }

    /// Deletes a task's stored explanation. Deleting a missing one is not an
    /// error. (Deleting the *task* takes its insight along by cascade; this
    /// is for forgetting a stale answer without deleting anything else.)
    ///
    /// # Errors
    /// SQLite errors.
    pub fn delete_insight(&self, name: &TaskName) -> Result<()> {
        self.conn.execute(
            "DELETE FROM ai_insights WHERE task_name=?1",
            params![name.as_str()],
        )?;
        Ok(())
    }
}

fn row_to_insight(r: &Row<'_>) -> Result<Insight> {
    let task_name: String = r.get(0)?;
    let created_at: String = r.get(1)?;
    Ok(Insight {
        task_name: TaskName::new(&task_name)?,
        created_at: parse_ts(&created_at)?,
        model: r.get(2)?,
        prompt_hash: r.get(3)?,
        content: r.get(4)?,
    })
}

fn text(s: impl Into<String>) -> Value {
    Value::Text(s.into())
}

fn opt_text(s: Option<String>) -> Value {
    s.map_or(Value::Null, Value::Text)
}

fn boolean(b: bool) -> Value {
    Value::Integer(i64::from(b))
}

fn task_params(t: &Task) -> Result<Vec<Value>> {
    Ok(vec![
        text(t.name.as_str()),
        text(t.display_name.clone()),
        opt_text(t.description.clone()),
        text(t.script_path.to_string_lossy().into_owned()),
        text(serde_json::to_string(&t.args)?),
        opt_text(
            t.working_dir
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
        ),
        text(serde_json::to_string(&t.env)?),
        text(serde_json::to_string(&t.trigger)?),
        boolean(t.keep_alive),
        t.timeout_secs
            .map_or(Value::Null, |v| Value::Integer(i64::from(v))),
        text(serde_json::to_string(&t.tags)?),
        boolean(t.favorite),
        boolean(t.notify_on_fail),
        text(ts(t.created_at)),
        text(ts(t.updated_at)),
        boolean(t.adopted),
        opt_text(
            t.plist_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
        ),
    ])
}

fn row_to_task(r: &Row<'_>) -> Result<Task> {
    let name: String = r.get(0)?;
    let args: String = r.get(4)?;
    let working_dir: Option<String> = r.get(5)?;
    let env: String = r.get(6)?;
    let trigger: String = r.get(7)?;
    let timeout_secs: Option<i64> = r.get(9)?;
    let tags: String = r.get(10)?;
    let created_at: String = r.get(13)?;
    let updated_at: String = r.get(14)?;
    let plist_path: Option<String> = r.get(16)?;
    Ok(Task {
        name: TaskName::new(&name)?,
        display_name: r.get(1)?,
        description: r.get(2)?,
        script_path: PathBuf::from(r.get::<_, String>(3)?),
        args: serde_json::from_str::<Vec<String>>(&args)?,
        working_dir: working_dir.map(PathBuf::from),
        env: serde_json::from_str::<BTreeMap<String, String>>(&env)?,
        trigger: serde_json::from_str::<Trigger>(&trigger)?,
        keep_alive: r.get(8)?,
        timeout_secs: timeout_secs.map(|v| v.clamp(0, i64::from(u32::MAX)) as u32),
        tags: serde_json::from_str::<Vec<String>>(&tags)?,
        favorite: r.get(11)?,
        notify_on_fail: r.get(12)?,
        created_at: parse_ts(&created_at)?,
        updated_at: parse_ts(&updated_at)?,
        adopted: r.get(15)?,
        plist_path: plist_path.map(PathBuf::from),
    })
}

fn row_to_run(r: &Row<'_>) -> Result<Run> {
    let task_name: String = r.get(1)?;
    let started_at: String = r.get(2)?;
    let finished_at: Option<String> = r.get(3)?;
    let pid: Option<i64> = r.get(5)?;
    let kind: String = r.get(8)?;
    let stop_reason: Option<String> = r.get(9)?;
    Ok(Run {
        id: RunId(r.get(0)?),
        task_name: TaskName::new(&task_name)?,
        started_at: parse_ts(&started_at)?,
        finished_at: finished_at.as_deref().map(parse_ts).transpose()?,
        exit_code: r.get(4)?,
        pid: pid.map(|p| p as u32),
        stdout_path: PathBuf::from(r.get::<_, Option<String>>(6)?.unwrap_or_default()),
        stderr_path: PathBuf::from(r.get::<_, Option<String>>(7)?.unwrap_or_default()),
        trigger_kind: TriggerKind::parse(&kind)?,
        stop_reason: stop_reason.as_deref().map(StopReason::parse).transpose()?,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::TimeZone;

    use super::*;
    use crate::task::{CalendarEntry, Trigger};

    fn name(s: &str) -> TaskName {
        TaskName::new(s).unwrap()
    }

    fn sample_task(n: &str) -> Task {
        let mut t = Task::new(name(n), "/usr/bin/true", Trigger::AtLogin);
        t.display_name = format!("任务 {n}");
        t.description = Some("描述".into());
        t.args = vec!["--flag".into(), "值".into()];
        t.working_dir = Some(PathBuf::from("/tmp"));
        t.env.insert("FOO".into(), "bar".into());
        t.tags = vec!["a".into(), "b".into()];
        t.keep_alive = true; // 与默认的 AtLogin 触发配套
        t.timeout_secs = Some(1800);
        t.favorite = true;
        t.notify_on_fail = false;
        t
    }

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    fn insight(task: &str, content: &str) -> Insight {
        Insight {
            task_name: name(task),
            created_at: at(0),
            model: "claude-sonnet-5".into(),
            prompt_hash: "a".repeat(64),
            content: content.into(),
        }
    }

    #[test]
    fn migrations_are_idempotent_and_versioned() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("nested").join("launchkeeper.db");
        {
            let s = Store::open(&db).unwrap();
            assert_eq!(s.schema_version().unwrap(), MIGRATIONS.len() as i64);
            s.insert_task(&sample_task("a")).unwrap();
        }
        let s = Store::open(&db).unwrap();
        assert_eq!(s.schema_version().unwrap(), MIGRATIONS.len() as i64);
        assert!(s.get_task(&name("a")).unwrap().is_some());
    }

    #[test]
    fn task_round_trips_every_field() {
        let s = Store::open_in_memory().unwrap();
        let mut t = sample_task("sync");
        t.keep_alive = false; // 日历触发不能配 keep_alive
        t.trigger = Trigger::Calendar {
            entries: vec![CalendarEntry::daily(21, 0)],
        };
        t.created_at = at(0);
        t.updated_at = at(5);
        s.insert_task(&t).unwrap();
        let back = s.get_task(&name("sync")).unwrap().unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn insert_twice_is_task_exists() {
        let s = Store::open_in_memory().unwrap();
        s.insert_task(&sample_task("a")).unwrap();
        assert!(matches!(
            s.insert_task(&sample_task("a")).unwrap_err(),
            Error::TaskExists(_)
        ));
    }

    #[test]
    fn update_missing_is_task_not_found() {
        let s = Store::open_in_memory().unwrap();
        assert!(matches!(
            s.update_task(&sample_task("ghost")).unwrap_err(),
            Error::TaskNotFound(_)
        ));
    }

    #[test]
    fn update_changes_fields() {
        let s = Store::open_in_memory().unwrap();
        let mut t = sample_task("a");
        t.created_at = at(0);
        t.updated_at = at(0);
        s.insert_task(&t).unwrap();
        t.display_name = "改过了".into();
        t.keep_alive = false;
        t.trigger = Trigger::Interval { seconds: 300 };
        t.timeout_secs = None;
        t.updated_at = at(10);
        s.update_task(&t).unwrap();
        assert_eq!(s.get_task(&name("a")).unwrap().unwrap(), t);
    }

    #[test]
    fn list_tasks_sorted_by_display_name() {
        let s = Store::open_in_memory().unwrap();
        for (n, disp) in [("c", "AAA"), ("a", "ZZZ"), ("b", "MMM")] {
            let mut t = sample_task(n);
            t.display_name = disp.into();
            s.insert_task(&t).unwrap();
        }
        let names: Vec<String> = s
            .list_tasks()
            .unwrap()
            .iter()
            .map(|t| t.display_name.clone())
            .collect();
        assert_eq!(names, vec!["AAA", "MMM", "ZZZ"]);
    }

    #[test]
    fn delete_task_cascades_to_runs_and_tolerates_missing() {
        let s = Store::open_in_memory().unwrap();
        s.insert_task(&sample_task("a")).unwrap();
        s.start_run(
            &name("a"),
            TriggerKind::Scheduled,
            at(0),
            Path::new("/tmp/a.out"),
            Path::new("/tmp/a.err"),
        )
        .unwrap();
        s.delete_task(&name("a")).unwrap();
        assert!(s.get_task(&name("a")).unwrap().is_none());
        assert!(s.list_runs(&name("a"), 10).unwrap().is_empty());
        s.delete_task(&name("a")).unwrap(); // 再删一次不报错
    }

    #[test]
    fn run_lifecycle() {
        let s = Store::open_in_memory().unwrap();
        s.insert_task(&sample_task("a")).unwrap();
        let id = s
            .start_run(
                &name("a"),
                TriggerKind::Manual,
                at(0),
                Path::new("/tmp/a.out"),
                Path::new("/tmp/a.err"),
            )
            .unwrap();
        let run = s.last_run(&name("a")).unwrap().unwrap();
        assert_eq!(run.id, id);
        assert_eq!(run.trigger_kind, TriggerKind::Manual);
        assert_eq!(run.started_at, at(0));
        assert_eq!(run.finished_at, None);
        assert_eq!(run.pid, None);
        assert_eq!(run.stdout_path, PathBuf::from("/tmp/a.out"));
        assert_eq!(run.stop_reason, None);
        assert!(!run.succeeded());

        s.set_run_pid(id, 4242).unwrap();
        s.finish_run(id, at(3), Some(0), Some(StopReason::Exited))
            .unwrap();
        let run = s.last_run(&name("a")).unwrap().unwrap();
        assert_eq!(run.pid, Some(4242));
        assert_eq!(run.finished_at, Some(at(3)));
        assert_eq!(run.exit_code, Some(0));
        assert_eq!(run.stop_reason, Some(StopReason::Exited));
        assert!(run.succeeded());

        // 信号杀死：finished_at 有值但 exit_code 为 None
        let id2 = s
            .start_run(
                &name("a"),
                TriggerKind::Scheduled,
                at(10),
                Path::new("/tmp/b.out"),
                Path::new("/tmp/b.err"),
            )
            .unwrap();
        s.finish_run(id2, at(11), None, Some(StopReason::Stopped))
            .unwrap();
        let run = s.last_run(&name("a")).unwrap().unwrap();
        assert_eq!(run.id, id2);
        assert_eq!(run.exit_code, None);
        assert_eq!(run.stop_reason, Some(StopReason::Stopped));
        assert!(run.finished_at.is_some());
        assert!(!run.succeeded());
    }

    #[test]
    fn stop_reason_strings() {
        for (r, s) in [
            (StopReason::Exited, "exited"),
            (StopReason::Stopped, "stopped"),
            (StopReason::Timeout, "timeout"),
        ] {
            assert_eq!(r.as_str(), s);
            assert_eq!(StopReason::parse(s).unwrap(), r);
            assert_eq!(serde_json::to_string(&r).unwrap(), format!("\"{s}\""));
        }
        assert_eq!(StopReason::Stopped.describe(), "手动停止");
        assert!(matches!(
            StopReason::parse("nope").unwrap_err(),
            Error::Corrupt(_)
        ));
    }

    /// The old v1 `CREATE TABLE` text, verbatim, so this test still describes
    /// a real v1 database once `migrations/0001_init.sql` grows a comment.
    const V1_SCHEMA: &str = "\
CREATE TABLE tasks (
  name TEXT PRIMARY KEY, display_name TEXT NOT NULL, description TEXT,
  script_path TEXT NOT NULL, args TEXT NOT NULL DEFAULT '[]', working_dir TEXT,
  env TEXT NOT NULL DEFAULT '{}', \"trigger\" TEXT NOT NULL,
  keep_alive INTEGER NOT NULL DEFAULT 0, timeout_secs INTEGER,
  tags TEXT NOT NULL DEFAULT '[]', favorite INTEGER NOT NULL DEFAULT 0,
  notify_on_fail INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE TABLE runs (
  id INTEGER PRIMARY KEY,
  task_name TEXT NOT NULL REFERENCES tasks(name) ON DELETE CASCADE,
  started_at TEXT NOT NULL, finished_at TEXT, exit_code INTEGER, pid INTEGER,
  stdout_path TEXT, stderr_path TEXT, trigger_kind TEXT NOT NULL
);
CREATE INDEX runs_task_started ON runs(task_name, started_at DESC);
PRAGMA user_version = 1;
";

    #[test]
    fn v1_database_migrates_forward_keeping_its_rows() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("launchkeeper.db");

        // 手工造一个 v1 库：老 schema、user_version = 1、一条已完成的 run
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute_batch(V1_SCHEMA).unwrap();
            conn.execute(
                "INSERT INTO tasks (name, display_name, script_path, \"trigger\", created_at, updated_at) \
                 VALUES ('legacy','legacy','/usr/bin/true','{\"kind\":\"at_login\"}',?1,?1)",
                params![ts(at(0))],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO runs (task_name, started_at, finished_at, exit_code, stdout_path, stderr_path, trigger_kind) \
                 VALUES ('legacy',?1,?2,0,'/tmp/l.out','/tmp/l.err','scheduled')",
                params![ts(at(0)), ts(at(5))],
            )
            .unwrap();
        }

        let s = Store::open(&db).unwrap();
        assert_eq!(s.schema_version().unwrap(), MIGRATIONS.len() as i64);
        assert_eq!(s.schema_version().unwrap(), 4);

        // v3 的两列在老行上取到默认值：不是接管来的任务
        let legacy = s.get_task(&name("legacy")).unwrap().unwrap();
        assert!(!legacy.adopted);
        assert_eq!(legacy.plist_path, None);
        assert_eq!(legacy.label(), "com.launchkeeper.legacy");

        // 老行读得出来，stop_reason 为 NULL
        let run = s.last_run(&name("legacy")).unwrap().unwrap();
        assert_eq!(run.exit_code, Some(0));
        assert_eq!(run.stop_reason, None);
        assert!(run.succeeded());

        // 新写入能带上 stop_reason
        s.finish_run(run.id, at(6), Some(1), Some(StopReason::Timeout))
            .unwrap();
        let run = s.last_run(&name("legacy")).unwrap().unwrap();
        assert_eq!(run.stop_reason, Some(StopReason::Timeout));

        // v4 的表在老库上也建了出来，老任务立刻能存解读
        s.upsert_insight(&insight("legacy", "第一版")).unwrap();
        assert_eq!(
            s.get_insight(&name("legacy")).unwrap().unwrap().content,
            "第一版"
        );

        // 再开一次不会重复跑迁移
        let s = Store::open(&db).unwrap();
        assert_eq!(s.schema_version().unwrap(), 4);
        assert!(s.get_task(&name("legacy")).unwrap().is_some());
    }

    #[test]
    fn unknown_run_id_is_run_not_found() {
        let s = Store::open_in_memory().unwrap();
        assert!(matches!(
            s.finish_run(RunId(99), at(0), Some(0), None).unwrap_err(),
            Error::RunNotFound(99)
        ));
        assert!(matches!(
            s.set_run_pid(RunId(99), 1).unwrap_err(),
            Error::RunNotFound(99)
        ));
    }

    #[test]
    fn run_for_unknown_task_violates_foreign_key() {
        let s = Store::open_in_memory().unwrap();
        assert!(
            s.start_run(
                &name("ghost"),
                TriggerKind::Scheduled,
                at(0),
                Path::new("/tmp/x.out"),
                Path::new("/tmp/x.err"),
            )
            .is_err()
        );
    }

    #[test]
    fn list_runs_newest_first_and_limited() {
        let s = Store::open_in_memory().unwrap();
        s.insert_task(&sample_task("a")).unwrap();
        for i in 0..5 {
            s.start_run(
                &name("a"),
                TriggerKind::Scheduled,
                at(i * 60),
                Path::new("/tmp/x.out"),
                Path::new("/tmp/x.err"),
            )
            .unwrap();
        }
        let runs = s.list_runs(&name("a"), 3).unwrap();
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].started_at, at(240));
        assert_eq!(runs[2].started_at, at(120));
    }

    #[test]
    fn prune_keeps_newest_and_returns_deleted_rows() {
        let s = Store::open_in_memory().unwrap();
        s.insert_task(&sample_task("a")).unwrap();
        s.insert_task(&sample_task("b")).unwrap();
        for i in 0..6 {
            s.start_run(
                &name("a"),
                TriggerKind::Scheduled,
                at(i * 60),
                &PathBuf::from(format!("/tmp/a-{i}.out")),
                &PathBuf::from(format!("/tmp/a-{i}.err")),
            )
            .unwrap();
        }
        s.start_run(
            &name("b"),
            TriggerKind::Scheduled,
            at(0),
            Path::new("/tmp/b.out"),
            Path::new("/tmp/b.err"),
        )
        .unwrap();

        let deleted = s.prune_runs(&name("a"), 2).unwrap();
        assert_eq!(deleted.len(), 4);
        // 删掉的是最旧的四条，按新到旧返回
        assert_eq!(deleted[0].started_at, at(180));
        assert_eq!(deleted[3].started_at, at(0));
        assert_eq!(deleted[3].stdout_path, PathBuf::from("/tmp/a-0.out"));

        let left = s.list_runs(&name("a"), 100).unwrap();
        assert_eq!(left.len(), 2);
        assert_eq!(left[0].started_at, at(300));
        assert_eq!(left[1].started_at, at(240));
        // 其他任务不受影响
        assert_eq!(s.list_runs(&name("b"), 100).unwrap().len(), 1);
        // 再 prune 一次没得删
        assert!(s.prune_runs(&name("a"), 2).unwrap().is_empty());
        // keep 大于总数
        assert!(s.prune_runs(&name("a"), 50).unwrap().is_empty());
        // keep 为 0 全删
        assert_eq!(s.prune_runs(&name("a"), 0).unwrap().len(), 2);
    }

    #[test]
    fn two_stores_open_the_same_fresh_database_concurrently() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("launchkeeper.db");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let db = db.clone();
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    let s = Store::open(&db).expect("并发打开同一个新库应当成功");
                    assert_eq!(s.schema_version().unwrap(), MIGRATIONS.len() as i64);
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn open_read_only_reads_without_creating_or_migrating_anything() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope").join("launchkeeper.db");
        let quick = std::time::Duration::from_millis(300);

        // A database that does not exist stays that way — no directory, no
        // file, no journal.
        assert!(Store::open_read_only(&missing, quick).is_err());
        assert!(!missing.parent().unwrap().exists());

        // A real one reads back what a writer put in.
        let db = dir.path().join("launchkeeper.db");
        {
            let s = Store::open(&db).unwrap();
            s.insert_task(&sample_task("a")).unwrap();
        }
        let ro = Store::open_read_only(&db, quick).unwrap();
        let names: Vec<String> = ro
            .list_tasks()
            .unwrap()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        assert_eq!(names, vec!["a".to_string()]);

        // And it really is read-only.
        assert!(ro.insert_task(&sample_task("b")).is_err());
    }

    #[test]
    fn open_read_only_gives_up_quickly_on_a_database_it_cannot_read() {
        let dir = tempfile::tempdir().unwrap();
        let garbage = dir.path().join("garbage.db");
        std::fs::write(&garbage, b"this is not a sqlite file at all").unwrap();
        let started = std::time::Instant::now();
        assert!(Store::open_read_only(&garbage, std::time::Duration::from_millis(300)).is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn insight_round_trips_and_overwrites_instead_of_accumulating() {
        let s = Store::open_in_memory().unwrap();
        s.insert_task(&sample_task("a")).unwrap();
        assert_eq!(s.get_insight(&name("a")).unwrap(), None);

        let first = insight("a", "第一次解读");
        s.upsert_insight(&first).unwrap();
        assert_eq!(s.get_insight(&name("a")).unwrap().unwrap(), first);

        // 再解读一次是覆盖，不是追加：一个任务只有一条
        let second = Insight {
            created_at: at(600),
            model: "gpt-4o-mini".into(),
            prompt_hash: "b".repeat(64),
            content: "第二次解读".into(),
            ..first.clone()
        };
        s.upsert_insight(&second).unwrap();
        assert_eq!(s.get_insight(&name("a")).unwrap().unwrap(), second);
        let n: i64 = s
            .conn()
            .query_row("SELECT COUNT(*) FROM ai_insights", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);

        // 单独删掉解读不动任务本身
        s.delete_insight(&name("a")).unwrap();
        assert_eq!(s.get_insight(&name("a")).unwrap(), None);
        assert!(s.get_task(&name("a")).unwrap().is_some());
        s.delete_insight(&name("a")).unwrap(); // 再删一次不报错
    }

    #[test]
    fn deleting_a_task_cascades_to_its_insight() {
        let s = Store::open_in_memory().unwrap();
        s.insert_task(&sample_task("a")).unwrap();
        s.insert_task(&sample_task("b")).unwrap();
        s.upsert_insight(&insight("a", "A 的解读")).unwrap();
        s.upsert_insight(&insight("b", "B 的解读")).unwrap();

        s.delete_task(&name("a")).unwrap();
        assert_eq!(s.get_insight(&name("a")).unwrap(), None);
        // 别的任务的解读不受影响
        assert!(s.get_insight(&name("b")).unwrap().is_some());
    }

    #[test]
    fn an_insight_for_an_unknown_task_violates_the_foreign_key() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.upsert_insight(&insight("ghost", "无主的解读")).is_err());
    }

    #[test]
    fn trigger_kind_strings() {
        assert_eq!(TriggerKind::Scheduled.as_str(), "scheduled");
        assert_eq!(TriggerKind::Manual.as_str(), "manual");
        assert_eq!(TriggerKind::parse("manual").unwrap(), TriggerKind::Manual);
        assert!(matches!(
            TriggerKind::parse("nope").unwrap_err(),
            Error::Corrupt(_)
        ));
    }
}
