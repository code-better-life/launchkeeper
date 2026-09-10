//! Interpreter resolution shared by `add`, `set` and `show`
//! (`docs/M3-design.md` §1; the CLI side of it is item 1.9 there, "还没做"
//! until now). Thin glue over `launchkeeper_core::interpreters`: turns
//! `--interpreter`/auto-detection into one [`Interpreter`], and formats the
//! "运行方式" line printed by `add`/`set`/`show`.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use launchkeeper_core::interpreters::{self, Interpreter};
use launchkeeper_core::{InterpreterKind, Origin};

/// Resolves `--interpreter` (or auto-picks one) for `script`. `cwd` is the
/// resolved `--cwd`, if any — passed straight to [`interpreters::scan`],
/// which falls back to `script`'s own directory when it is `None`.
///
/// - `explicit == Some("direct")`: launchd execs `script` itself. Errors if
///   the script is not executable — that combination can never run.
/// - `explicit == Some(id_or_path)`: matched against a scanned candidate's
///   [`Interpreter::id`] first, so a known interpreter keeps its real
///   `kind`/`prefix_args`/`origin`; falling back to a bare, existing path
///   typed by hand (a [`Interpreter::custom`]).
/// - `explicit == None`: the scan's `recommended` candidate. If none is
///   recommended and the script is itself executable, falls back to running
///   it directly — this is what keeps a plain `--script /bin/echo` (no
///   extension, no shebang, but executable) working exactly as it did
///   before this feature existed. Otherwise, an error listing every
///   candidate found.
pub fn resolve_interpreter(
    script: &Path,
    cwd: Option<&Path>,
    explicit: Option<&str>,
) -> Result<Interpreter> {
    let path = launchkeeper_core::env::effective_path();
    let candidates = interpreters::scan(Some(script), cwd, &path, false);

    if let Some(explicit) = explicit {
        return resolve_explicit(script, explicit, &candidates);
    }

    if let Some(rec) = candidates.iter().find(|c| c.recommended) {
        return Ok(rec.clone());
    }
    if interpreters::is_executable(script) {
        return Ok(direct(script));
    }
    bail!(
        "无法为 {} 自动选择运行方式，请用 --interpreter 指定。\n{}",
        script.display(),
        format_candidates(&candidates)
    );
}

fn resolve_explicit(
    script: &Path,
    explicit: &str,
    candidates: &[Interpreter],
) -> Result<Interpreter> {
    if explicit == "direct" {
        if !interpreters::is_executable(script) {
            bail!(
                "--interpreter direct 要求脚本本身可执行: {}",
                script.display()
            );
        }
        return Ok(direct(script));
    }
    if let Some(found) = candidates.iter().find(|c| c.id() == explicit) {
        return Ok(found.clone());
    }
    let program = PathBuf::from(explicit);
    if program.exists() {
        return Ok(Interpreter::custom(program));
    }
    bail!(
        "--interpreter 既不是已知候选的 ID 也不是存在的路径: {explicit}\n{}",
        format_candidates(candidates)
    );
}

fn direct(script: &Path) -> Interpreter {
    Interpreter {
        kind: InterpreterKind::Direct,
        program: script.to_path_buf(),
        prefix_args: Vec::new(),
        version: None,
        origin: Origin::Custom,
        recommended: false,
        reason: None,
    }
}

/// A human-readable candidate listing for an error message: one
/// `- <id> (<label>)` line per candidate.
pub fn format_candidates(candidates: &[Interpreter]) -> String {
    if candidates.is_empty() {
        return "没有找到任何候选运行方式（用 `launchkeeper interpreters` 看看扫到了什么）。"
            .to_string();
    }
    let mut s = String::from("可用候选:");
    for c in candidates {
        s.push_str(&format!("\n  - {} ({})", c.id(), c.label()));
    }
    s
}

/// Fills in `--version`, unless `interp` is [`InterpreterKind::Direct`]
/// (which never has one — see [`Interpreter::detail`]). Used right before
/// printing, never during resolution itself: `resolve_interpreter` is called
/// with `with_versions: false` so `add`/`set` stay fast, and only the one
/// chosen program pays the `--version` cost.
pub fn with_version(mut interp: Interpreter) -> Interpreter {
    if interp.kind != InterpreterKind::Direct {
        interp.version = interpreters::version_of(&interp.program);
    }
    interp
}

/// The "运行方式" line: `<display_name> · <detail>`, plus `（<reason>）` when
/// `interp` carries one (only ever set on a freshly auto-picked candidate,
/// never on one reconstructed by [`interpreters::detect_from_task`]).
pub fn interpreter_line(interp: &Interpreter) -> String {
    let base = format!("{} · {}", interp.display_name(), interp.detail());
    match &interp.reason {
        Some(reason) => format!("{base}（{reason}）"),
        None => base,
    }
}

/// The "运行方式" line for an existing task, reconstructed from its stored
/// `script_path`/`args` via [`interpreters::detect_from_task`]. Used by
/// `show`, and by `add`/`set` in `--raw` mode where nothing was resolved up
/// front. `None` only when `detect_from_task` itself returns `None` (an
/// empty `script_path`, or a bare known interpreter with no script to run).
pub fn detect_display_line(script_path: &Path, args: &[String]) -> Option<String> {
    let detected = interpreters::detect_from_task(script_path, args)?;
    Some(interpreter_line(&with_version(detected.interpreter)))
}
