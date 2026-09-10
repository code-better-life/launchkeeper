//! A thin wrapper around `/bin/launchctl`, restricted to the modern verbs.
//!
//! `load` / `unload` are never used: only `bootstrap`, `bootout`,
//! `kickstart` and `print`, all in the `gui/<uid>` domain.

use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use crate::error::{Error, Result};

/// Environment variable overriding the `launchctl` binary. Tests point it at
/// a fake; nothing else should set it.
pub const LAUNCHCTL_ENV: &str = "LAUNCHKEEPER_LAUNCHCTL";

const DEFAULT_LAUNCHCTL: &str = "/bin/launchctl";

fn launchctl_bin() -> String {
    match std::env::var_os(LAUNCHCTL_ENV) {
        Some(v) if !v.is_empty() => v.to_string_lossy().into_owned(),
        _ => DEFAULT_LAUNCHCTL.to_string(),
    }
}

/// What `launchctl print` says about a loaded job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobStatus {
    /// Pid, present only while the job is running.
    pub pid: Option<u32>,
    /// `last exit code`; `None` when the job has never exited.
    pub last_exit_status: Option<i32>,
    /// `state`, e.g. `running` or `waiting`.
    pub state: String,
}

/// The current user's uid, via `/usr/bin/id -u`. The successful value is
/// cached for the process lifetime; there is no fallback, because guessing a
/// uid would mean talking to the wrong launchd domain.
///
/// # Errors
/// [`Error::Launchctl`] (command `id -u`) when `id` cannot be run, exits
/// non-zero, or prints something that is not a number.
pub fn uid() -> Result<u32> {
    static UID: OnceLock<u32> = OnceLock::new();
    if let Some(v) = UID.get() {
        return Ok(*v);
    }
    let fail = |code: Option<i32>, msg: String| Error::Launchctl {
        command: "id -u".to_string(),
        code,
        stderr: msg,
    };
    let out = Command::new("/usr/bin/id")
        .arg("-u")
        .output()
        .map_err(|e| fail(None, format!("无法执行 /usr/bin/id: {e}")))?;
    if !out.status.success() {
        return Err(fail(
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let value: u32 = text
        .parse()
        .map_err(|_| fail(Some(0), format!("`id -u` 输出无法解析为 uid: {text:?}")))?;
    Ok(*UID.get_or_init(|| value))
}

/// `gui/<uid>`.
///
/// # Errors
/// Whatever [`uid`] returns.
pub fn domain() -> Result<String> {
    Ok(format!("gui/{}", uid()?))
}

/// `gui/<uid>/<label>`.
///
/// # Errors
/// Whatever [`uid`] returns.
pub fn service_target(label: &str) -> Result<String> {
    Ok(format!("{}/{label}", domain()?))
}

struct Output {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str]) -> Result<Output> {
    let bin = launchctl_bin();
    let out = Command::new(&bin)
        .args(args)
        .output()
        .map_err(|e| Error::io(bin.as_str(), e))?;
    Ok(Output {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

fn fail(args: &[&str], out: &Output, hint: Option<&str>) -> Error {
    let mut stderr = if out.stderr.trim().is_empty() {
        out.stdout.trim().to_string()
    } else {
        out.stderr.trim().to_string()
    };
    if let Some(h) = hint {
        if stderr.is_empty() {
            stderr = h.to_string();
        } else {
            stderr = format!("{stderr}（{h}）");
        }
    }
    Error::Launchctl {
        command: args.join(" "),
        code: out.code,
        stderr,
    }
}

fn mentions(out: &Output, needle: &str) -> bool {
    let n = needle.to_ascii_lowercase();
    out.stderr.to_ascii_lowercase().contains(&n) || out.stdout.to_ascii_lowercase().contains(&n)
}

/// `launchctl bootstrap gui/<uid> <plist>`.
///
/// # Errors
/// [`Error::Launchctl`]. When launchd reports EIO (exit 5 or
/// "Input/output error"), which means the job is already loaded, the error
/// carries a hint saying to `bootout` first.
pub fn bootstrap(plist: &Path) -> Result<()> {
    let domain = domain()?;
    let plist_str = plist.to_string_lossy().into_owned();
    let args = ["bootstrap", domain.as_str(), plist_str.as_str()];
    let out = run(&args)?;
    if out.code == Some(0) {
        return Ok(());
    }
    let hint = if out.code == Some(5) || mentions(&out, "input/output error") {
        Some("job 已加载，需先 bootout")
    } else {
        None
    };
    Err(fail(&args, &out, hint))
}

/// `launchctl bootout gui/<uid>/<label>`. A label that is not loaded is
/// treated as success.
///
/// # Errors
/// [`Error::Launchctl`] for any other failure.
pub fn bootout(label: &str) -> Result<()> {
    let target = service_target(label)?;
    let args = ["bootout", target.as_str()];
    let out = run(&args)?;
    if out.code == Some(0) {
        return Ok(());
    }
    // 未加载：launchctl 返回 3 (ESRCH) / "No such process" / "Could not find service"
    if means_absent(&out) {
        return Ok(());
    }
    Err(fail(&args, &out, None))
}

/// `launchctl kickstart gui/<uid>/<label>`: run the job now.
///
/// # Errors
/// [`Error::Launchctl`], notably when the job is not loaded.
pub fn kickstart(label: &str) -> Result<()> {
    let target = service_target(label)?;
    let args = ["kickstart", target.as_str()];
    let out = run(&args)?;
    if out.code == Some(0) {
        return Ok(());
    }
    Err(fail(&args, &out, None))
}

/// True when launchctl's answer means "there is nothing there", i.e. the
/// label is not loaded or the job has no running process. Both `bootout` and
/// [`kill`] treat that as success, because the goal state is already reached.
fn means_absent(out: &Output) -> bool {
    out.code == Some(3)
        || mentions(out, "no such process")
        || mentions(out, "could not find service")
        || mentions(out, "not find specified service")
}

/// `launchctl kill <signal> gui/<uid>/<label>`: send a signal to the job's
/// running process.
///
/// `signal` is a bare name as `launchctl` spells it (`"TERM"`, `"KILL"`,
/// also accepted with the `SIG` prefix or as a number). A job that is not
/// loaded, or loaded but not running, is treated as success: there is
/// nothing left to signal, which is exactly what the caller wanted.
///
/// # Errors
/// [`Error::Launchctl`] for any other failure.
pub fn kill(label: &str, signal: &str) -> Result<()> {
    let target = service_target(label)?;
    let args = ["kill", signal, target.as_str()];
    let out = run(&args)?;
    if out.code == Some(0) || means_absent(&out) {
        return Ok(());
    }
    Err(fail(&args, &out, None))
}

/// `launchctl print gui/<uid>/<label>`, parsed. `Ok(None)` means the job is
/// not loaded.
///
/// # Errors
/// [`Error::Launchctl`] when `print` fails for any other reason.
pub fn status(label: &str) -> Result<Option<JobStatus>> {
    let target = service_target(label)?;
    let args = ["print", target.as_str()];
    let out = run(&args)?;
    if out.code != Some(0) {
        if mentions(&out, "could not find service")
            || mentions(&out, "no such process")
            || out.code == Some(113)
        {
            return Ok(None);
        }
        return Err(fail(&args, &out, None));
    }
    Ok(Some(parse_print(&out.stdout)))
}

/// Parses the interesting lines out of `launchctl print` output.
pub(crate) fn parse_print(text: &str) -> JobStatus {
    let mut st = JobStatus {
        pid: None,
        last_exit_status: None,
        state: String::new(),
    };
    for line in text.lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "pid" => st.pid = value.parse().ok(),
            // 新版 launchctl 打印已经解码过的退出码
            "last exit code" => st.last_exit_status = value.parse().ok(),
            // 老版 macOS 打印的是 wait(2) 的原始状态字：高 8 位是退出码，
            // 低 7 位非零表示被信号杀死（此时没有退出码可言）。
            "last exit status" => st.last_exit_status = decode_wait_status(value),
            "state" => st.state = value.to_string(),
            _ => {}
        }
    }
    st
}

/// Decodes a raw `wait(2)` status word as printed by older `launchctl` for
/// `last exit status`. Returns `None` when the value is not a number or the
/// job was killed by a signal.
fn decode_wait_status(value: &str) -> Option<i32> {
    let raw: i32 = value.parse().ok()?;
    if raw < 0 {
        return None;
    }
    if raw & 0x7f != 0 {
        return None; // 被信号杀死
    }
    Some((raw >> 8) & 0xff)
}

/// `bootout` (ignoring "not loaded") followed by `bootstrap`. This is the
/// only supported way to apply a changed plist.
///
/// # Errors
/// [`Error::Launchctl`].
pub fn reload(plist: &Path, label: &str) -> Result<()> {
    bootout(label)?;
    wait_until_unloaded(label, UNLOAD_WAIT);
    bootstrap(plist)
}

/// How long [`reload`] waits for a booted-out job to disappear before it
/// bootstraps the new plist.
pub const UNLOAD_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// `bootout` of a job whose process is still running returns before the
/// job is actually gone: launchd tears it down asynchronously (it sends the
/// process SIGTERM first). An immediate `bootstrap` then fails with EIO,
/// "job already loaded". This polls `print` until the label is unknown, or
/// `max` elapses (then the bootstrap gets to report the real error).
fn wait_until_unloaded(label: &str, max: std::time::Duration) {
    let deadline = std::time::Instant::now() + max;
    loop {
        match status(label) {
            Ok(None) | Err(_) => return,
            Ok(Some(_)) if std::time::Instant::now() >= deadline => return,
            Ok(Some(_)) => std::thread::sleep(std::time::Duration::from_millis(150)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
gui/501/com.launchkeeper.sync = {
	active count = 1
	path = /Users/x/Library/LaunchAgents/com.launchkeeper.sync.plist
	state = running
	program = /usr/bin/true
	pid = 4242
	last exit code = 0
}";

    #[test]
    fn parses_print_output() {
        let st = parse_print(SAMPLE);
        assert_eq!(st.pid, Some(4242));
        assert_eq!(st.last_exit_status, Some(0));
        assert_eq!(st.state, "running");
    }

    #[test]
    fn parses_never_exited_and_waiting() {
        let st = parse_print("\tstate = waiting\n\tlast exit code = (never exited)\n");
        assert_eq!(st.pid, None);
        assert_eq!(st.last_exit_status, None);
        assert_eq!(st.state, "waiting");
    }

    fn out(code: i32, stderr: &str) -> Output {
        Output {
            code: Some(code),
            stdout: String::new(),
            stderr: stderr.to_string(),
        }
    }

    #[test]
    fn absent_answers_are_recognised() {
        // kill / bootout 对这些回答都当成功：目标状态已经达到了
        assert!(means_absent(&out(3, "")));
        assert!(means_absent(&out(1, "No such process")));
        assert!(means_absent(&out(
            1,
            "Could not find service \"x\" in domain"
        )));
        assert!(means_absent(&out(1, "Boot-out failed: 3: No such process")));
        // 真正的失败不该被吞掉
        assert!(!means_absent(&out(1, "Operation not permitted")));
        assert!(!means_absent(&out(5, "Input/output error")));
    }

    #[test]
    fn kill_parses_a_running_pid_out_of_print() {
        // stop 依赖 print 里的 pid 判断"还在不在"
        let st = parse_print("\tstate = running\n\tpid = 4242\n");
        assert_eq!(st.pid, Some(4242));
        let st = parse_print("\tstate = not running\n\tlast exit code = 0\n");
        assert_eq!(st.pid, None);
        assert_eq!(st.last_exit_status, Some(0));
    }

    #[test]
    fn domain_and_target_shape() {
        let uid = uid().unwrap();
        assert_eq!(domain().unwrap(), format!("gui/{uid}"));
        assert_eq!(
            service_target("com.launchkeeper.x").unwrap(),
            format!("gui/{uid}/com.launchkeeper.x")
        );
    }

    #[test]
    fn last_exit_code_is_taken_verbatim() {
        let st = parse_print("\tstate = waiting\n\tlast exit code = 3\n");
        assert_eq!(st.last_exit_status, Some(3));
    }

    #[test]
    fn last_exit_status_is_decoded_as_a_wait_status() {
        // 退出码 3 -> 3 << 8 = 768
        assert_eq!(
            parse_print("\tlast exit status = 768\n").last_exit_status,
            Some(3)
        );
        assert_eq!(
            parse_print("\tlast exit status = 0\n").last_exit_status,
            Some(0)
        );
        // exit 124 -> 124 << 8 = 31744
        assert_eq!(
            parse_print("\tlast exit status = 31744\n").last_exit_status,
            Some(124)
        );
        // SIGTERM (15) 杀死 -> 低 7 位非零 -> None
        assert_eq!(
            parse_print("\tlast exit status = 15\n").last_exit_status,
            None
        );
        assert_eq!(
            parse_print("\tlast exit status = (never exited)\n").last_exit_status,
            None
        );
    }
}
