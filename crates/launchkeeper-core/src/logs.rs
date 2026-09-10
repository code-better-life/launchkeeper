//! Run log files: `<task_log_dir>/<YYYYMMDD-HHMMSS>.out` and `.err`.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, Utc};

use crate::error::{Error, Result};
use crate::paths;
use crate::store::Run;
use crate::task::TaskName;

/// How many runs of a task are kept before older ones are pruned.
pub const DEFAULT_KEEP_RUNS: usize = 50;

/// Stem format: local time, second resolution.
fn stem(started_at: DateTime<Utc>) -> String {
    started_at
        .with_timezone(&Local)
        .format("%Y%m%d-%H%M%S")
        .to_string()
}

/// The freshly created log files of one run, with the paths that were
/// actually claimed.
#[derive(Debug)]
pub struct RunLogs {
    /// Path of the stdout file.
    pub stdout_path: PathBuf,
    /// Opened, exclusively created stdout file.
    pub stdout: File,
    /// Path of the stderr file.
    pub stderr_path: PathBuf,
    /// Opened, exclusively created stderr file.
    pub stderr: File,
}

/// Creates both log files of a run under the task's log directory, which is
/// created if missing.
///
/// The stem is local time with second resolution; both files are opened with
/// `create_new`, so two runners starting in the same second cannot end up
/// writing to the same file — the loser retries with `-1`, `-2`, ....
///
/// # Errors
/// I/O errors while creating the directory or the files.
pub fn create_run_logs(name: &TaskName, started_at: DateTime<Utc>) -> Result<RunLogs> {
    let dir = paths::task_log_dir(name)?;
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    create_in_dir(&dir, started_at)
}

/// Same as [`create_run_logs`] but against an explicit, already existing
/// directory.
///
/// # Errors
/// I/O errors while creating the files.
pub fn create_in_dir(dir: &Path, started_at: DateTime<Utc>) -> Result<RunLogs> {
    let base = stem(started_at);
    let mut suffix = 0u32;
    loop {
        let s = if suffix == 0 {
            base.clone()
        } else {
            format!("{base}-{suffix}")
        };
        let out_path = dir.join(format!("{s}.out"));
        let err_path = dir.join(format!("{s}.err"));
        match create_new(&out_path) {
            Ok(stdout) => match create_new(&err_path) {
                Ok(stderr) => {
                    return Ok(RunLogs {
                        stdout_path: out_path,
                        stdout,
                        stderr_path: err_path,
                        stderr,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // `.out` 抢到了但 `.err` 没抢到：把刚建的 `.out` 让出去。
                    drop(stdout);
                    let _ = std::fs::remove_file(&out_path);
                }
                Err(e) => return Err(Error::io(&err_path, e)),
            },
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(Error::io(&out_path, e)),
        }
        suffix = suffix
            .checked_add(1)
            .ok_or_else(|| Error::io(dir, std::io::Error::other("日志文件名后缀用尽")))?;
    }
}

fn create_new(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

/// The stdout/stderr paths a run *would* get, without creating anything.
///
/// Kept for tests and for callers that only need to predict the layout; the
/// runner uses [`create_run_logs`] instead, because checking-then-creating
/// races with a concurrent runner.
pub fn paths_in_dir(dir: &Path, started_at: DateTime<Utc>) -> (PathBuf, PathBuf) {
    let base = stem(started_at);
    let mut suffix = 0u32;
    loop {
        let s = if suffix == 0 {
            base.clone()
        } else {
            format!("{base}-{suffix}")
        };
        let out = dir.join(format!("{s}.out"));
        let err = dir.join(format!("{s}.err"));
        if !out.exists() && !err.exists() {
            return (out, err);
        }
        suffix += 1;
    }
}

/// Deletes both log files of a run; missing files are ignored.
pub fn remove_run_logs(run: &Run) {
    for p in [&run.stdout_path, &run.stderr_path] {
        if p.as_os_str().is_empty() {
            continue;
        }
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::store::{Run, RunId, TriggerKind};

    fn at() -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0).unwrap()
    }

    #[test]
    fn stem_is_local_time() {
        let expected = at()
            .with_timezone(&Local)
            .format("%Y%m%d-%H%M%S")
            .to_string();
        assert_eq!(stem(at()), expected);
        assert_eq!(expected.len(), 15);
    }

    #[test]
    fn same_second_collisions_get_suffixes() {
        let dir = tempfile::tempdir().unwrap();
        let base = stem(at());

        let (out, err) = paths_in_dir(dir.path(), at());
        assert_eq!(out.file_name().unwrap(), format!("{base}.out").as_str());
        assert_eq!(err.file_name().unwrap(), format!("{base}.err").as_str());

        std::fs::write(&out, "").unwrap();
        let (out1, err1) = paths_in_dir(dir.path(), at());
        assert_eq!(out1.file_name().unwrap(), format!("{base}-1.out").as_str());
        assert_eq!(err1.file_name().unwrap(), format!("{base}-1.err").as_str());

        // 只有 .err 存在也算冲突
        std::fs::write(&err1, "").unwrap();
        let (out2, _) = paths_in_dir(dir.path(), at());
        assert_eq!(out2.file_name().unwrap(), format!("{base}-2.out").as_str());
    }

    #[test]
    fn create_in_dir_creates_both_files_and_avoids_collisions() {
        let dir = tempfile::tempdir().unwrap();
        let base = stem(at());

        let a = create_in_dir(dir.path(), at()).unwrap();
        assert_eq!(
            a.stdout_path.file_name().unwrap(),
            format!("{base}.out").as_str()
        );
        assert!(a.stdout_path.exists() && a.stderr_path.exists());

        let b = create_in_dir(dir.path(), at()).unwrap();
        assert_eq!(
            b.stdout_path.file_name().unwrap(),
            format!("{base}-1.out").as_str()
        );
        assert_eq!(
            b.stderr_path.file_name().unwrap(),
            format!("{base}-1.err").as_str()
        );

        // 只有 .err 存在也算冲突
        std::fs::write(dir.path().join(format!("{base}-2.err")), "").unwrap();
        let c = create_in_dir(dir.path(), at()).unwrap();
        assert_eq!(
            c.stdout_path.file_name().unwrap(),
            format!("{base}-3.out").as_str()
        );
        assert!(!dir.path().join(format!("{base}-2.out")).exists());
    }

    #[test]
    fn remove_run_logs_ignores_missing() {
        let dir = tempfile::tempdir().unwrap();
        let (out, err) = paths_in_dir(dir.path(), at());
        std::fs::write(&out, "hi").unwrap();
        let run = Run {
            id: RunId(1),
            task_name: crate::task::TaskName::new("a").unwrap(),
            started_at: at(),
            finished_at: None,
            exit_code: None,
            pid: None,
            stdout_path: out.clone(),
            stderr_path: err.clone(),
            trigger_kind: TriggerKind::Manual,
            stop_reason: None,
        };
        remove_run_logs(&run);
        assert!(!out.exists());
        assert!(!err.exists());
        remove_run_logs(&run); // 再来一次也不 panic
    }

    #[test]
    fn default_keep_is_fifty() {
        assert_eq!(DEFAULT_KEEP_RUNS, 50);
    }
}
