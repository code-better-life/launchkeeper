//! Filesystem layout. Everything is derived from `dirs`; nothing is hard-coded.
//!
//! Two environment variables override the defaults, for tests and the CLI:
//! `LAUNCHKEEPER_DATA_DIR` and `LAUNCHKEEPER_LAUNCH_AGENTS_DIR`.
//!
//! [`data_dir`] has a third, lower-precedence source below the env var: the
//! one-line config file at [`config_data_dir_file`], for machines where
//! exporting `LAUNCHKEEPER_DATA_DIR` from the shell rc is inconvenient. See
//! [`data_dir_source`] for the full precedence order and how to tell which
//! source produced the answer.

use std::path::PathBuf;

use crate::error::{Error, Result};
use crate::task::{Task, TaskName};

/// Environment variable overriding [`data_dir`].
pub const DATA_DIR_ENV: &str = "LAUNCHKEEPER_DATA_DIR";
/// Environment variable overriding [`runner_log_dir`].
pub const RUNNER_LOG_DIR_ENV: &str = "LAUNCHKEEPER_RUNNER_LOG_DIR";
/// Environment variable overriding [`launch_agents_dir`].
pub const LAUNCH_AGENTS_DIR_ENV: &str = "LAUNCHKEEPER_LAUNCH_AGENTS_DIR";
/// Environment variable overriding the base of [`config_dir`] (the XDG
/// convention: when set and non-empty, `$XDG_CONFIG_HOME/launchkeeper`
/// replaces `~/.config/launchkeeper`).
pub const XDG_CONFIG_HOME_ENV: &str = "XDG_CONFIG_HOME";

fn home() -> Result<PathBuf> {
    dirs::home_dir().ok_or(Error::NoHomeDir)
}

/// Which of [`data_dir`]'s sources produced its answer, highest precedence
/// first. See [`data_dir_source`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataDirSource {
    /// `$LAUNCHKEEPER_DATA_DIR`.
    Env,
    /// [`config_data_dir_file`].
    ConfigFile,
    /// `~/Library/Application Support/Launchkeeper/`.
    Default,
}

impl DataDirSource {
    /// A short, stable, lowercase label — used as the CLI's `--json` value
    /// for `config data-dir`.
    pub fn as_str(self) -> &'static str {
        match self {
            DataDirSource::Env => "env",
            DataDirSource::ConfigFile => "config_file",
            DataDirSource::Default => "default",
        }
    }
}

/// `<data_dir>/bin/launchkeeper-runner`: where the runner binary is installed
/// so that plists never point at an external or otherwise TCC-protected
/// volume (launchd cannot even `exec` a binary living there).
///
/// # Errors
/// [`Error::NoHomeDir`].
pub fn installed_runner_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("bin").join("launchkeeper-runner"))
}

/// `~/Library/Application Support/Launchkeeper/`, or wherever
/// [`data_dir_source`] resolves to.
pub fn data_dir() -> Result<PathBuf> {
    Ok(data_dir_source()?.0)
}

/// Resolves the data directory and reports which source produced it, in
/// precedence order:
///
/// 1. `$LAUNCHKEEPER_DATA_DIR`, if set and non-empty.
/// 2. [`config_data_dir_file`] (`~/.config/launchkeeper/data-dir`, a one-line
///    path, trimmed) if it exists and is non-blank.
/// 3. The default, `~/Library/Application Support/Launchkeeper/`.
///
/// # Errors
/// [`Error::NoHomeDir`] when falling through to the default. An I/O error
/// reading the config file (anything other than "does not exist") is treated
/// the same as "absent" and falls through to the next source instead of
/// propagating — this file is a convenience, and a glitch reading it must
/// never make the CLI unusable.
pub fn data_dir_source() -> Result<(PathBuf, DataDirSource)> {
    if let Some(v) = std::env::var_os(DATA_DIR_ENV)
        && !v.is_empty()
    {
        return Ok((PathBuf::from(v), DataDirSource::Env));
    }
    if let Some(p) = read_config_data_dir() {
        return Ok((p, DataDirSource::ConfigFile));
    }
    Ok((
        home()?
            .join("Library")
            .join("Application Support")
            .join("Launchkeeper"),
        DataDirSource::Default,
    ))
}

/// `~/.config/launchkeeper/`, or `$XDG_CONFIG_HOME/launchkeeper/` when that
/// variable is set and non-empty.
pub fn config_dir() -> Result<PathBuf> {
    if let Some(v) = std::env::var_os(XDG_CONFIG_HOME_ENV)
        && !v.is_empty()
    {
        return Ok(PathBuf::from(v).join("launchkeeper"));
    }
    Ok(home()?.join(".config").join("launchkeeper"))
}

/// `<config_dir>/data-dir`: an optional one-line text file naming the data
/// directory. See [`data_dir_source`].
pub fn config_data_dir_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("data-dir"))
}

/// Reads and trims [`config_data_dir_file`]. `None` when it is missing,
/// blank, or its path cannot even be resolved (no home dir) — every failure
/// mode falls through to the next precedence level rather than erroring.
fn read_config_data_dir() -> Option<PathBuf> {
    let file = config_data_dir_file().ok()?;
    let raw = std::fs::read_to_string(&file).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

/// `<data_dir>/launchkeeper.db`.
pub fn db_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("launchkeeper.db"))
}

/// `<data_dir>/logs/`.
pub fn logs_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("logs"))
}

/// `<logs_dir>/<name>/`.
pub fn task_log_dir(name: &TaskName) -> Result<PathBuf> {
    Ok(logs_dir()?.join(name.as_str()))
}

/// `<data_dir>/login-shell-path`: one line of plain text.
pub fn path_cache_file() -> Result<PathBuf> {
    Ok(data_dir()?.join("login-shell-path"))
}

/// `~/Library/Logs/Launchkeeper/`, or `$LAUNCHKEEPER_RUNNER_LOG_DIR`.
///
/// This is where launchd itself writes the runner's stdout/stderr
/// (`StandardOutPath`). It deliberately does **not** follow `data_dir`:
/// launchd cannot open a file on an external (TCC-protected) volume and
/// would fail the job with `EX_CONFIG`, so this stays on the boot disk.
/// These files are tiny.
///
/// # Errors
/// [`Error::NoHomeDir`].
pub fn runner_log_dir() -> Result<PathBuf> {
    if let Some(v) = std::env::var_os(RUNNER_LOG_DIR_ENV)
        && !v.is_empty()
    {
        return Ok(PathBuf::from(v));
    }
    Ok(home()?.join("Library").join("Logs").join("Launchkeeper"))
}

/// `<runner_log_dir>/<name>.log`.
///
/// # Errors
/// [`Error::NoHomeDir`].
pub fn runner_log_path(name: &TaskName) -> Result<PathBuf> {
    Ok(runner_log_dir()?.join(format!("{}.log", name.as_str())))
}

/// `~/Library/LaunchAgents/`, or `$LAUNCHKEEPER_LAUNCH_AGENTS_DIR`.
pub fn launch_agents_dir() -> Result<PathBuf> {
    if let Some(v) = std::env::var_os(LAUNCH_AGENTS_DIR_ENV)
        && !v.is_empty()
    {
        return Ok(PathBuf::from(v));
    }
    Ok(home()?.join("Library").join("LaunchAgents"))
}

/// Where `task`'s plist lives.
///
/// For a task Launchkeeper created, that is
/// `<launch_agents_dir>/com.launchkeeper.<name>.plist`. For a task
/// [adopted](crate::Task::adopted) in place it is the file that was already
/// there — [`Task::plist_path`](crate::Task::plist_path) — which keeps its
/// original name, and whose label is the task name verbatim.
///
/// # Errors
/// [`Error::NoHomeDir`].
pub fn plist_path(task: &Task) -> Result<PathBuf> {
    if task.adopted
        && let Some(p) = &task.plist_path
    {
        return Ok(p.clone());
    }
    Ok(launch_agents_dir()?.join(format!("{}.plist", task.label())))
}

/// `<launch_agents_dir>/com.launchkeeper.<name>.plist`: where a task
/// Launchkeeper creates itself gets its plist, before any adoption is
/// involved. [`plist_path`] is what callers holding a [`Task`] want.
///
/// # Errors
/// [`Error::NoHomeDir`].
pub fn own_plist_path(name: &TaskName) -> Result<PathBuf> {
    Ok(launch_agents_dir()?.join(format!("{}.plist", name.label())))
}
