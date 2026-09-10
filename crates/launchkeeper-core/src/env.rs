//! PATH handling.
//!
//! launchd hands a job a bare `PATH` of `/usr/bin:/bin:/usr/sbin:/sbin`, which
//! is almost never what a user's script expects. We capture the login shell's
//! `PATH` once and cache it in a file the runner can read without spawning a
//! shell on every run.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::paths;

/// Last-resort PATH when neither the cache nor a live capture works.
pub const FALLBACK_PATH: &str = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin";

/// Shell used when `$SHELL` is unset.
pub const DEFAULT_SHELL: &str = "/bin/zsh";

/// How long we let the login shell take before giving up.
pub const CAPTURE_TIMEOUT: Duration = Duration::from_secs(5);

/// Runs `$SHELL -l -c 'printf %s "$PATH"'` and returns the trimmed result.
///
/// The shell is started non-interactively (no `-i`) so that prompts and
/// interactive-only rc files cannot hang it, and it is killed after
/// [`CAPTURE_TIMEOUT`].
///
/// # Errors
/// [`Error::EnvCapture`] on spawn failure, timeout, a non-zero exit, or empty
/// output; the caller is expected to fall back to the cache or
/// [`FALLBACK_PATH`].
pub fn capture_login_shell_path() -> Result<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| DEFAULT_SHELL.to_string());
    let mut child = Command::new(&shell)
        .args(["-l", "-c", r#"printf %s "$PATH""#])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::EnvCapture(format!("无法启动 {shell}: {e}")))?;

    let deadline = Instant::now() + CAPTURE_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Error::EnvCapture(format!(
                        "{shell} 超过 {} 秒未返回",
                        CAPTURE_TIMEOUT.as_secs()
                    )));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(Error::EnvCapture(format!("等待 {shell} 失败: {e}"))),
        }
    };

    let mut stdout = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        pipe.read_to_string(&mut stdout)
            .map_err(|e| Error::EnvCapture(format!("读取 {shell} 输出失败: {e}")))?;
    }
    if !status.success() {
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        return Err(Error::EnvCapture(format!(
            "{shell} 退出码 {:?}: {}",
            status.code(),
            stderr.trim()
        )));
    }
    let path = stdout.trim().to_string();
    if path.is_empty() {
        return Err(Error::EnvCapture(format!("{shell} 返回了空 PATH")));
    }
    Ok(path)
}

/// Writes `path` to [`paths::path_cache_file`], creating the data directory.
///
/// # Errors
/// I/O errors.
pub fn write_path_cache(path: &str) -> Result<()> {
    let file = paths::path_cache_file()?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    }
    std::fs::write(&file, format!("{}\n", path.trim())).map_err(|e| Error::io(&file, e))
}

/// Reads the cached PATH. `Ok(None)` when there is no cache or it is blank.
///
/// # Errors
/// I/O errors other than "not found".
pub fn read_path_cache() -> Result<Option<String>> {
    let file = paths::path_cache_file()?;
    match std::fs::read_to_string(&file) {
        Ok(s) => {
            let s = s.trim().to_string();
            Ok(if s.is_empty() { None } else { Some(s) })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(&file, e)),
    }
}

/// Captures the login shell PATH and, on success, refreshes the cache.
/// Returns the captured value.
///
/// # Errors
/// Whatever [`capture_login_shell_path`] or [`write_path_cache`] returns.
pub fn refresh_path_cache() -> Result<String> {
    let path = capture_login_shell_path()?;
    write_path_cache(&path)?;
    Ok(path)
}

/// The PATH the runner should use, following the precedence from the design
/// doc: cache, then a live capture (written back to the cache), then
/// [`FALLBACK_PATH`]. A task's own `PATH` override is applied by the caller on
/// top of this.
pub fn effective_path() -> String {
    if let Ok(Some(cached)) = read_path_cache() {
        return cached;
    }
    refresh_path_cache().unwrap_or_else(|_| FALLBACK_PATH.to_string())
}
