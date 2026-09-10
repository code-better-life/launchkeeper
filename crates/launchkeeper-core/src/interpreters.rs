//! Interpreter discovery and recommendation (`docs/M3-design.md` §1).
//!
//! A task's plist ultimately needs one program plus a list of arguments, but
//! that is a bad thing to make a user type: `uv run scripts/sync.py --once`
//! is four different decisions glued into one string. This module splits it
//! back apart — the *script* is what the user picked, the *interpreter* is how
//! it runs — and answers three questions:
//!
//! 1. [`scan`]: which interpreters exist on this machine and in this project,
//!    and which one should be the default for a given script;
//! 2. [`apply`]: given a script, its arguments and an interpreter, what does
//!    the task's `script_path` + `args` become;
//! 3. [`detect_from_task`]: the inverse, so an existing task opens with the
//!    right entry already selected.
//!
//! [`scan`] takes the `PATH` to search as a plain string rather than reading
//! the environment itself. Callers pass [`crate::env::effective_path`] (the
//! login shell's `PATH`, which is what the runner will hand the script);
//! tests pass a fake one built out of temporary directories.

use std::collections::HashMap;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// How long `<program> --version` is allowed to take before it is killed and
/// the version is reported as unknown.
///
/// The picker is opened from a form; a hung binary in `PATH` must cost the
/// user a visible pause at worst, never a wedged window.
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(2);

/// How many directories [`project_signals`] walks up before giving up.
pub const PROJECT_WALK_LIMIT: usize = 12;

/// What kind of program runs the script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum InterpreterKind {
    /// `python3` / `python`, including a project's `.venv/bin/python`.
    Python,
    /// `uv run <script>`.
    Uv,
    /// `node`.
    Node,
    /// `bun`.
    Bun,
    /// `deno run <script>`.
    Deno,
    /// `ruby`.
    Ruby,
    /// `zsh` / `bash` / `sh`.
    Shell,
    /// No interpreter at all: launchd executes the script itself, which
    /// requires it to be executable and to carry a shebang.
    Direct,
    /// A path the user typed in by hand.
    Custom,
}

/// Where an interpreter came from. Shown next to the version so that two
/// `python3`s are told apart by something other than their paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// `/usr/bin`, `/bin`, `/usr/sbin`, `/sbin` — shipped with macOS.
    System,
    /// `/opt/homebrew` or `/usr/local`, *and* that prefix really has a
    /// `bin/brew` in it — otherwise the path is just another `PATH` entry
    /// (`/usr/local/bin` is where plenty of non-Homebrew installers land).
    Homebrew,
    /// `<project>/.venv/bin`.
    ProjectVenv,
    /// `~/.local/bin/uv` itself: uv's own installer put it there.
    Uv,
    /// Anything else in `~/.local/bin` — a user-level install, not
    /// necessarily uv's doing (pipx, `pip install --user`, a hand-made
    /// symlink).
    UserLocal,
    /// Anywhere else on `PATH`.
    Path,
    /// Typed in by the user, or the script itself for [`InterpreterKind::Direct`].
    Custom,
}

impl Origin {
    /// A short Chinese label: `系统`, `Homebrew`, `项目 .venv`, `uv 安装`,
    /// `用户安装`, `PATH`, `自定义`.
    pub fn describe(&self) -> &'static str {
        match self {
            Origin::System => "系统",
            Origin::Homebrew => "Homebrew",
            Origin::ProjectVenv => "项目 .venv",
            Origin::Uv => "uv 安装",
            Origin::UserLocal => "用户安装",
            Origin::Path => "PATH",
            Origin::Custom => "自定义",
        }
    }
}

/// One way to run a script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interpreter {
    /// Which family this belongs to.
    pub kind: InterpreterKind,
    /// Absolute path of the program launchd will start. For
    /// [`InterpreterKind::Direct`] this is the script itself.
    pub program: PathBuf,
    /// Arguments that come before the script, e.g. `["run"]` for `uv`.
    pub prefix_args: Vec<String>,
    /// Version string as parsed out of `<program> --version`, when it was
    /// asked for and the program answered.
    pub version: Option<String>,
    /// Where the program lives.
    pub origin: Origin,
    /// True for the single entry [`scan`] chose as the default.
    pub recommended: bool,
    /// Why it was chosen, e.g. `按 .py 扩展名推荐；项目里没有 .venv 或 uv.lock`.
    /// Only ever set on the recommended entry.
    pub reason: Option<String>,
}

impl Interpreter {
    /// An interpreter the user typed a path for.
    pub fn custom(program: impl Into<PathBuf>) -> Interpreter {
        Interpreter {
            kind: InterpreterKind::Custom,
            program: program.into(),
            prefix_args: Vec::new(),
            version: None,
            origin: Origin::Custom,
            recommended: false,
            reason: None,
        }
    }

    /// Stable identity for a picker: the program path plus any prefix args,
    /// which is exactly what distinguishes `uv` from `uv run`.
    pub fn id(&self) -> String {
        if self.prefix_args.is_empty() {
            self.program.to_string_lossy().into_owned()
        } else {
            format!(
                "{} {}",
                self.program.to_string_lossy(),
                self.prefix_args.join(" ")
            )
        }
    }

    /// The short name shown in the closed dropdown: `python3`, `uv run`,
    /// `直接执行`.
    pub fn display_name(&self) -> String {
        if self.kind == InterpreterKind::Direct {
            return "直接执行".to_string();
        }
        let base = self
            .program
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.program.to_string_lossy().into_owned());
        if self.prefix_args.is_empty() {
            base
        } else {
            format!("{base} {}", self.prefix_args.join(" "))
        }
    }

    /// The secondary line in the picker: `3.9.6 · 系统 /usr/bin/python3`.
    pub fn detail(&self) -> String {
        if self.kind == InterpreterKind::Direct {
            return "脚本自带 shebang，交给 launchd 直接运行".to_string();
        }
        let place = format!("{} {}", self.origin.describe(), shorten_home(&self.program));
        match &self.version {
            Some(v) => format!("{v} · {place}"),
            None => place,
        }
    }

    /// One line for a list row: `python3 · 系统`.
    pub fn label(&self) -> String {
        if self.kind == InterpreterKind::Direct {
            return "直接执行".to_string();
        }
        format!("{} · {}", self.display_name(), self.origin.describe())
    }
}

/// What a task actually runs, taken apart again: the interpreter, the script
/// it runs, and the script's own arguments. Produced by [`detect_from_task`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Detected {
    /// The interpreter, with no version filled in (nothing was executed).
    pub interpreter: Interpreter,
    /// Flags that belong to the interpreter rather than to the script, i.e.
    /// the leading `-…` arguments between the interpreter's own
    /// [`Interpreter::prefix_args`] and the script: the `-u` of
    /// `python3 -u sync.py`. Kept apart so that a form or a `set` that only
    /// re-composes the command puts them back exactly where they were.
    pub interp_args: Vec<String>,
    /// The script itself.
    pub script: PathBuf,
    /// Arguments belonging to the script, not to the interpreter.
    pub args: Vec<String>,
}

/// The project-level facts that steer the recommendation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectSignals {
    /// The directory the signals were found in.
    pub root: Option<PathBuf>,
    /// `<root>/.venv/bin/python`, when it exists and is executable.
    pub venv_python: Option<PathBuf>,
    /// `<root>/pyproject.toml` exists.
    pub pyproject: bool,
    /// `<root>/uv.lock` exists.
    pub uv_lock: bool,
    /// `<root>/package.json` exists.
    pub package_json: bool,
}

impl ProjectSignals {
    /// True when nothing at all was found.
    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }
}

// ---------------------------------------------------------------------------
// scanning
// ---------------------------------------------------------------------------

struct Known {
    names: &'static [&'static str],
    kind: InterpreterKind,
    prefix: &'static [&'static str],
}

/// The programs we look for, in the order they are offered. `python3` before
/// `python` and `node` before `bun` is the whole of the "which one first"
/// policy for programs that are not the recommendation.
const KNOWN: &[Known] = &[
    Known {
        names: &["python3", "python"],
        kind: InterpreterKind::Python,
        prefix: &[],
    },
    Known {
        names: &["uv"],
        kind: InterpreterKind::Uv,
        prefix: &["run"],
    },
    Known {
        names: &["node"],
        kind: InterpreterKind::Node,
        prefix: &[],
    },
    Known {
        names: &["bun"],
        kind: InterpreterKind::Bun,
        prefix: &[],
    },
    Known {
        names: &["deno"],
        kind: InterpreterKind::Deno,
        prefix: &["run"],
    },
    Known {
        names: &["ruby"],
        kind: InterpreterKind::Ruby,
        prefix: &[],
    },
    Known {
        names: &["zsh", "bash", "sh"],
        kind: InterpreterKind::Shell,
        prefix: &[],
    },
];

/// Every interpreter worth offering for `script`, best first.
///
/// `script` decides the recommendation (by extension, then by shebang) and
/// whether a [`InterpreterKind::Direct`] entry exists at all; `project_dir` —
/// the task's working directory, when it has one — is where the walk for
/// `.venv` / `uv.lock` / `package.json` starts, falling back to the script's
/// own directory. `path` is a `:`-separated `PATH`; pass
/// [`crate::env::effective_path`].
///
/// `with_versions` runs `<program> --version` once per program (cached for
/// the life of the process, see [`version_of`]). It is the only part of this
/// function that starts a process, and the only part that can take a
/// noticeable amount of time.
///
/// A relative `script` is resolved against `project_dir` before anything
/// else looks at it — `is_executable`, `shebang` and the walk up from its
/// parent directory would otherwise silently resolve it against the calling
/// process's current directory, which is not where the task will run.
/// Without a `project_dir` there is nothing to resolve it against, and a
/// relative script simply yields no project signals and no 直接执行 entry.
///
/// The result is ordered: the recommended entry, then anything from the
/// project, then everything else in [`KNOWN`] order.
pub fn scan(
    script: Option<&Path>,
    project_dir: Option<&Path>,
    path: &str,
    with_versions: bool,
) -> Vec<Interpreter> {
    let resolved = script.map(|s| resolve_script(s, project_dir));
    let script = resolved.as_deref();
    let start = project_dir
        .map(PathBuf::from)
        .or_else(|| script.and_then(|s| s.parent()).map(PathBuf::from));
    let project = project_signals(start.as_deref());

    let mut out: Vec<Interpreter> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();

    let push = |out: &mut Vec<Interpreter>, seen: &mut Vec<PathBuf>, i: Interpreter| {
        let key = canonical(&i.program);
        if seen.contains(&key) {
            return;
        }
        seen.push(key);
        out.push(i);
    };

    if let Some(venv) = &project.venv_python {
        push(
            &mut out,
            &mut seen,
            Interpreter {
                kind: InterpreterKind::Python,
                program: venv.clone(),
                prefix_args: Vec::new(),
                version: None,
                origin: Origin::ProjectVenv,
                recommended: false,
                reason: None,
            },
        );
    }

    for known in KNOWN {
        for name in known.names {
            for dir in path.split(':').filter(|d| !d.is_empty()) {
                let candidate = Path::new(dir).join(name);
                if !is_executable(&candidate) {
                    continue;
                }
                push(
                    &mut out,
                    &mut seen,
                    Interpreter {
                        kind: known.kind,
                        program: candidate.clone(),
                        prefix_args: known.prefix.iter().map(|s| (*s).to_string()).collect(),
                        version: None,
                        origin: classify_origin(&candidate),
                        recommended: false,
                        reason: None,
                    },
                );
            }
        }
    }

    // "Let launchd exec the script" is only on the table when that would
    // actually work: the file has to be executable and carry a shebang.
    if let Some(s) = script
        && is_executable(s)
        && shebang(s).is_some()
    {
        // Through `push` like everything else: a script that *is* one of the
        // programs already listed must not show up a second time.
        push(
            &mut out,
            &mut seen,
            Interpreter {
                kind: InterpreterKind::Direct,
                program: s.to_path_buf(),
                prefix_args: Vec::new(),
                version: None,
                origin: Origin::Custom,
                recommended: false,
                reason: None,
            },
        );
    }

    recommend(&mut out, script, &project);

    if with_versions {
        for i in &mut out {
            if i.kind != InterpreterKind::Direct {
                i.version = version_of(&i.program);
            }
        }
    }

    // Stable sort: recommended, then the project's own, then KNOWN order.
    out.sort_by_key(|i| {
        if i.recommended {
            0
        } else if i.origin == Origin::ProjectVenv {
            1
        } else {
            2
        }
    });
    out
}

/// Walks up from `start` looking for the first directory that carries a
/// project marker (`.venv/bin/python`, `pyproject.toml`, `uv.lock`,
/// `package.json`), at most [`PROJECT_WALK_LIMIT`] levels.
///
/// The nearest directory wins: in a monorepo, the package next to the script
/// is the project, not the repository root.
///
/// A relative `start` is refused outright (an empty [`ProjectSignals`]):
/// walking it would resolve against this process's current directory, which
/// has nothing to do with where the task will run, and could report a
/// `.venv` that the task will never see.
pub fn project_signals(start: Option<&Path>) -> ProjectSignals {
    let mut signals = ProjectSignals::default();
    let Some(start) = start.filter(|s| s.is_absolute()) else {
        return signals;
    };
    let mut dir = start;
    for _ in 0..PROJECT_WALK_LIMIT {
        let venv = dir.join(".venv").join("bin").join("python");
        let venv3 = dir.join(".venv").join("bin").join("python3");
        let venv_python = if is_executable(&venv) {
            Some(venv)
        } else if is_executable(&venv3) {
            Some(venv3)
        } else {
            None
        };
        let pyproject = dir.join("pyproject.toml").is_file();
        let uv_lock = dir.join("uv.lock").is_file();
        let package_json = dir.join("package.json").is_file();
        if venv_python.is_some() || pyproject || uv_lock || package_json {
            signals.root = Some(dir.to_path_buf());
            signals.venv_python = venv_python;
            signals.pyproject = pyproject;
            signals.uv_lock = uv_lock;
            signals.package_json = package_json;
            return signals;
        }
        match dir.parent() {
            Some(p) if p != dir => dir = p,
            _ => break,
        }
    }
    signals
}

/// A relative `script` joined onto `project_dir` (the directory the task
/// will actually run in); anything else is returned unchanged. Purely
/// lexical — nothing is canonicalized and nothing has to exist.
fn resolve_script(script: &Path, project_dir: Option<&Path>) -> PathBuf {
    match project_dir {
        Some(dir) if script.is_relative() && dir.is_absolute() => dir.join(script),
        _ => script.to_path_buf(),
    }
}

/// Marks one entry of `cands` as the recommendation and gives it a reason.
fn recommend(cands: &mut [Interpreter], script: Option<&Path>, project: &ProjectSignals) {
    let Some(script) = script else {
        return;
    };
    let ext = script
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let sb = shebang(script);
    let direct_possible = is_executable(script) && sb.is_some();

    let by_ext = format!("按 .{ext} 扩展名推荐");
    let direct_reason = "脚本可执行且带 shebang，交给 launchd 直接运行".to_string();

    let picked: Option<(usize, String)> = match ext.as_str() {
        "py" => {
            if let Some(i) = find(cands, InterpreterKind::Python, Some(Origin::ProjectVenv)) {
                Some((i, "按 .py 扩展名推荐；项目里有 .venv".to_string()))
            } else if project.uv_lock
                && let Some(i) = find(cands, InterpreterKind::Uv, None)
            {
                Some((
                    i,
                    "按 .py 扩展名推荐；项目里有 uv.lock，用 uv run".to_string(),
                ))
            } else {
                find(cands, InterpreterKind::Python, None).map(|i| {
                    (
                        i,
                        "按 .py 扩展名推荐；项目里没有 .venv 或 uv.lock".to_string(),
                    )
                })
            }
        }
        "js" | "mjs" | "cjs" => first_of(
            cands,
            &[
                InterpreterKind::Node,
                InterpreterKind::Bun,
                InterpreterKind::Deno,
            ],
        )
        .map(|i| (i, by_ext.clone())),
        "ts" | "mts" | "cts" => first_of(
            cands,
            &[
                InterpreterKind::Bun,
                InterpreterKind::Deno,
                InterpreterKind::Node,
            ],
        )
        .map(|i| (i, by_ext.clone())),
        "rb" => find(cands, InterpreterKind::Ruby, None).map(|i| (i, by_ext.clone())),
        "sh" | "bash" | "zsh" => {
            if direct_possible {
                find(cands, InterpreterKind::Direct, None).map(|i| (i, direct_reason.clone()))
            } else if let Some(i) = shebang_match(cands, sb.as_deref()) {
                Some((i, "按 shebang 推荐".to_string()))
            } else {
                let want = if ext == "zsh" { "zsh" } else { "bash" };
                by_name(cands, want)
                    .or_else(|| find(cands, InterpreterKind::Shell, None))
                    .map(|i| (i, by_ext.clone()))
            }
        }
        _ => {
            if direct_possible {
                find(cands, InterpreterKind::Direct, None).map(|i| (i, direct_reason.clone()))
            } else {
                shebang_match(cands, sb.as_deref()).map(|i| (i, "按 shebang 推荐".to_string()))
            }
        }
    };

    if let Some((i, reason)) = picked {
        cands[i].recommended = true;
        cands[i].reason = Some(reason);
    }
}

fn find(cands: &[Interpreter], kind: InterpreterKind, origin: Option<Origin>) -> Option<usize> {
    cands
        .iter()
        .position(|c| c.kind == kind && origin.is_none_or(|o| c.origin == o))
}

fn first_of(cands: &[Interpreter], kinds: &[InterpreterKind]) -> Option<usize> {
    kinds.iter().find_map(|k| find(cands, *k, None))
}

fn by_name(cands: &[Interpreter], name: &str) -> Option<usize> {
    cands
        .iter()
        .position(|c| c.program.file_name().is_some_and(|f| f == name))
}

/// The candidate a script's shebang names, matched on the interpreter's file
/// name (`#!/usr/bin/env python3` and `#!/opt/homebrew/bin/python3` both mean
/// "python3", and the PATH copy we found is the one that will actually run).
fn shebang_match(cands: &[Interpreter], line: Option<&str>) -> Option<usize> {
    let line = line?;
    let mut tokens = line.split_whitespace();
    let first = tokens.next()?;
    // `#!/usr/bin/env python3` — the interpreter is what env is told to run.
    // `env` itself may come with flags (`-S`, `-i`) and with `NAME=value`
    // assignments before the program name; neither is the interpreter.
    // `env -u NAME prog` (a flag that takes a separate argument) would still
    // read `NAME` as the program — a real getopt is not worth it for a
    // shebang line, and the result is only ever a missing recommendation.
    let name = if Path::new(first).file_name().is_some_and(|f| f == "env") {
        tokens.find(|t| !t.starts_with('-') && !t.contains('='))?
    } else {
        first
    };
    let base = Path::new(name).file_name()?.to_str()?;
    by_name(cands, base)
}

// ---------------------------------------------------------------------------
// apply / detect
// ---------------------------------------------------------------------------

/// Turns "script + args + interpreter" into the `script_path` and `args` a
/// [`crate::Task`] stores.
///
/// [`InterpreterKind::Direct`] is the script itself; everything else is the
/// interpreter, its prefix args, `interp_args` (the interpreter's own flags,
/// e.g. the `-u` of `python3 -u sync.py`, normally
/// [`Detected::interp_args`] handed straight back), the script, and finally
/// the script's own arguments.
///
/// The frontend has a copy of this rule in `src/lib/interpreter.ts` (it
/// composes the same two fields without a round trip); the two are checked
/// against the same examples on both sides.
pub fn apply(
    task_script: &Path,
    task_args: &[String],
    interp: &Interpreter,
    interp_args: &[String],
) -> (PathBuf, Vec<String>) {
    if interp.kind == InterpreterKind::Direct {
        return (task_script.to_path_buf(), task_args.to_vec());
    }
    let mut args = interp.prefix_args.clone();
    args.extend(interp_args.iter().cloned());
    args.push(task_script.to_string_lossy().into_owned());
    args.extend(task_args.iter().cloned());
    (interp.program.clone(), args)
}

/// The inverse of [`apply`]: reads a stored task's `script_path` + `args`
/// back into an interpreter, a script and the script's own arguments.
///
/// Returns `None` when `script_path` is empty, or when it names a known
/// interpreter that was given no script to run — neither can be shown as
/// "this script, run this way".
pub fn detect_from_task(script_path: &Path, args: &[String]) -> Option<Detected> {
    let name = script_path.file_name()?.to_str()?;
    let known = KNOWN
        .iter()
        .find(|k| k.names.iter().any(|n| is_known_program(name, n)));
    let Some(known) = known else {
        // Not an interpreter we know: launchd runs this thing directly.
        return Some(Detected {
            interpreter: Interpreter {
                kind: InterpreterKind::Direct,
                program: script_path.to_path_buf(),
                prefix_args: Vec::new(),
                version: None,
                origin: Origin::Custom,
                recommended: false,
                reason: None,
            },
            interp_args: Vec::new(),
            script: script_path.to_path_buf(),
            args: args.to_vec(),
        });
    };

    // Prefix args are only really there when the stored command used them:
    // `uv run x.py` has them, a bare `uv x.py` (which is not a thing, but the
    // database can hold anything) does not.
    let mut rest = args;
    let mut prefix: Vec<String> = Vec::new();
    for want in known.prefix {
        match rest.first() {
            Some(first) if first == want => {
                prefix.push(first.clone());
                rest = &rest[1..];
            }
            _ => break,
        }
    }
    // Then the interpreter's own flags: everything leading that starts with
    // `-` belongs to the interpreter, not to the script (`python3 -u x.py`).
    // A lone `--` ends them and the next argument is the script.
    let mut interp_args: Vec<String> = Vec::new();
    while let Some(first) = rest.first() {
        if first == "--" {
            interp_args.push(first.clone());
            rest = &rest[1..];
            break;
        }
        if first.starts_with('-') && first.len() > 1 {
            interp_args.push(first.clone());
            rest = &rest[1..];
        } else {
            break;
        }
    }
    let (script, script_args) = rest.split_first()?;
    Some(Detected {
        interpreter: Interpreter {
            kind: known.kind,
            program: script_path.to_path_buf(),
            prefix_args: prefix,
            version: None,
            origin: classify_origin(script_path),
            recommended: false,
            reason: None,
        },
        interp_args,
        script: PathBuf::from(script),
        args: script_args.to_vec(),
    })
}

/// Whether the file name `name` is the known program `known`: either exactly
/// it, or it followed by a version (`python3.13`, `ruby2.7`, `node18`).
///
/// A version is digits and dots and nothing else, which is the whole point:
/// `node.js` and `python.py` are *scripts named after an interpreter*, and
/// the old "starts with `<known>.`" rule read them as `node` and `python`
/// running a script called `js` / `py`.
fn is_known_program(name: &str, known: &str) -> bool {
    let Some(rest) = name.strip_prefix(known) else {
        return false;
    };
    rest.is_empty()
        || (rest.chars().any(|c| c.is_ascii_digit())
            && rest.chars().all(|c| c.is_ascii_digit() || c == '.'))
}

// ---------------------------------------------------------------------------
// versions
// ---------------------------------------------------------------------------

fn version_cache() -> &'static Mutex<HashMap<PathBuf, Option<String>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Option<String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `<program> --version`, parsed down to the first dotted number, with a
/// [`VERSION_TIMEOUT`] and a process-wide cache keyed on the program path.
///
/// `None` when the program could not be started, took too long, or printed
/// nothing that looks like a version. The cache is never invalidated: an
/// upgrade mid-session is not worth re-running every interpreter on the
/// machine for, and the app is restarted often enough.
///
/// A `None` placeholder is written into the cache *before* the program is
/// started, so a second scan that arrives while a slow program is still
/// being probed answers "unknown version" immediately instead of starting
/// its own probe and waiting another [`VERSION_TIMEOUT`]. The real answer
/// replaces the placeholder as soon as it is in.
pub fn version_of(program: &Path) -> Option<String> {
    if let Ok(mut cache) = version_cache().lock() {
        if let Some(hit) = cache.get(program) {
            return hit.clone();
        }
        cache.insert(program.to_path_buf(), None);
    }
    let found = run_version(program);
    if let Ok(mut cache) = version_cache().lock() {
        cache.insert(program.to_path_buf(), found.clone());
    }
    found
}

/// One `<program> --version`, bounded by [`VERSION_TIMEOUT`] end to end.
///
/// The bound has to cover the *pipe*, not just the child: a wrapper script
/// that prints its version and leaves a background process holding the
/// inherited stdout exits immediately, and a plain `read_to_string` on that
/// pipe then blocks until the grandchild is done — forever, for a `sleep`
/// or a daemon. So the reading happens on a helper thread and the answer is
/// collected with `recv_timeout`.
///
/// The child is started in its own process group (`process_group(0)`) so
/// that a timeout can kill the whole group — the grandchild holding the pipe
/// is the thing that actually has to die, and it is not `child.kill()`'s to
/// kill.
fn run_version(program: &Path) -> Option<String> {
    let mut child = Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // The child leads its own group: killing `-pid` later reaches
        // everything it started, and nothing outside it.
        .process_group(0)
        .spawn()
        .ok()?;
    let pid = child.id();
    let mut stdout = child.stdout.take()?;
    let mut stderr = child.stderr.take()?;

    let (tx, rx) = mpsc::channel::<(String, String)>();
    std::thread::spawn(move || {
        // `--version` output is a line or two, so reading the two pipes one
        // after the other cannot deadlock on a full pipe buffer here; and if
        // a program does something stranger, the timeout below covers it.
        let mut out = String::new();
        let _ = stdout.read_to_string(&mut out);
        let mut err = String::new();
        let _ = stderr.read_to_string(&mut err);
        let _ = tx.send((out, err));
    });

    let collected = rx.recv_timeout(VERSION_TIMEOUT);
    if collected.is_err() {
        // Nothing came back in time. TERM the group, give it a moment, then
        // KILL; `child.kill()` afterwards covers the case where the group
        // kill did not land (e.g. the child already reaped itself).
        kill_group(pid, "-TERM");
        if rx.recv_timeout(KILL_GRACE).is_err() {
            kill_group(pid, "-KILL");
        }
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    let _ = child.wait();
    let (out, err) = collected.ok()?;
    // Some tools (older `sh`, `dash`) answer on stderr.
    parse_version(&out).or_else(|| parse_version(&err))
}

/// How long a timed-out probe's process group gets between `TERM` and
/// `KILL`.
const KILL_GRACE: Duration = Duration::from_millis(200);

/// `kill <signal> -- -<pid>`: the negative pid is the process group, which
/// is where a shim's background children live. Uses `/bin/kill` rather than
/// a `libc` dependency; failure is ignored because the only thing left to do
/// about it is what the caller already does — report no version.
fn kill_group(pid: u32, signal: &str) {
    let _ = Command::new("/bin/kill")
        .arg(signal)
        .arg("--")
        .arg(format!("-{pid}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// The first dotted number in `out`: `Python 3.9.6` → `3.9.6`, `v22.22.0` →
/// `22.22.0`, `GNU bash, version 3.2.57(1)-release` → `3.2.57`.
fn parse_version(out: &str) -> Option<String> {
    for line in out.lines() {
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if !bytes[i].is_ascii_digit() {
                i += 1;
                continue;
            }
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            let v = line[start..i].trim_end_matches('.');
            if v.contains('.') {
                return Some(v.to_string());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// small helpers
// ---------------------------------------------------------------------------

/// True for a regular file with any execute bit set.
pub fn is_executable(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(m) => m.is_file() && m.permissions().mode() & 0o111 != 0,
        Err(_) => false,
    }
}

/// The first line of `path` when it starts with `#!`, without the `#!` and
/// trimmed. `None` when the file has no shebang or cannot be read.
pub fn shebang(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = [0u8; 256];
    let n = file.read(&mut buf).ok()?;
    let head = &buf[..n];
    if !head.starts_with(b"#!") {
        return None;
    }
    let end = head.iter().position(|b| *b == b'\n').unwrap_or(head.len());
    let line = String::from_utf8_lossy(&head[2..end]).trim().to_string();
    if line.is_empty() { None } else { Some(line) }
}

/// Which [`Origin`] a program path belongs to, by prefix.
///
/// Two of the prefixes are checked against the machine rather than taken at
/// face value: `/opt/homebrew` and `/usr/local` are only *Homebrew* when
/// that prefix really has a `bin/brew` in it (`/usr/local/bin` in particular
/// is where all sorts of installers land on a machine with no Homebrew at
/// all), and `~/.local/bin` is only *uv 安装* for uv itself — everything
/// else there is just a user-level install.
pub fn classify_origin(program: &Path) -> Origin {
    let s = program.to_string_lossy();
    if s.contains("/.venv/bin/") {
        return Origin::ProjectVenv;
    }
    if s.starts_with("/usr/bin/")
        || s.starts_with("/bin/")
        || s.starts_with("/usr/sbin/")
        || s.starts_with("/sbin/")
    {
        return Origin::System;
    }
    for (prefix, brew) in [
        ("/opt/homebrew/", "/opt/homebrew/bin/brew"),
        ("/usr/local/", "/usr/local/bin/brew"),
    ] {
        if s.starts_with(prefix) {
            return if Path::new(brew).exists() {
                Origin::Homebrew
            } else {
                Origin::Path
            };
        }
    }
    if let Some(home) = dirs::home_dir() {
        let local_bin = home.join(".local").join("bin");
        if program.parent() == Some(local_bin.as_path()) {
            return if program.file_name().is_some_and(|f| f == "uv") {
                Origin::Uv
            } else {
                Origin::UserLocal
            };
        }
    }
    Origin::Path
}

/// `/Users/x/.local/bin/uv` → `~/.local/bin/uv`, for display only.
fn shorten_home(path: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

/// The path a program really is, for de-duplication. Falls back to the path
/// itself when it cannot be resolved.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Writes an executable shell script that prints `version_line` when run
    /// with `--version`, so a scan over a fake PATH sees a plausible program.
    fn fake_program(dir: &Path, name: &str, version_line: &str) -> PathBuf {
        fs::create_dir_all(dir).expect("mkdir");
        let p = dir.join(name);
        fs::write(&p, format!("#!/bin/sh\necho '{version_line}'\n")).expect("write");
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).expect("chmod");
        p
    }

    fn write_script(dir: &Path, name: &str, body: &str, executable: bool) -> PathBuf {
        fs::create_dir_all(dir).expect("mkdir");
        let p = dir.join(name);
        fs::write(&p, body).expect("write");
        let mode = if executable { 0o755 } else { 0o644 };
        fs::set_permissions(&p, fs::Permissions::from_mode(mode)).expect("chmod");
        p
    }

    /// A fake `PATH` with python3, uv, node and bash in it.
    fn fake_path(tmp: &TempDir) -> (String, PathBuf) {
        let bin = tmp.path().join("bin");
        fake_program(&bin, "python3", "FakePython 3.12.7");
        fake_program(&bin, "uv", "uv 0.8.4 (deadbeef 2026-01-01)");
        fake_program(&bin, "node", "v22.22.0");
        fake_program(&bin, "bash", "GNU bash, version 5.2.37(1)-release");
        fake_program(&bin, "ruby", "ruby 3.4.1 (2026-01-01 revision abc)");
        (bin.to_string_lossy().into_owned(), bin)
    }

    #[test]
    fn scan_finds_every_known_program_on_the_fake_path() {
        let tmp = TempDir::new().expect("tmp");
        let (path, bin) = fake_path(&tmp);
        let found = scan(None, None, &path, false);
        let names: Vec<String> = found.iter().map(|i| i.display_name()).collect();
        assert!(names.contains(&"python3".to_string()), "{names:?}");
        assert!(names.contains(&"uv run".to_string()), "{names:?}");
        assert!(names.contains(&"node".to_string()), "{names:?}");
        assert!(names.contains(&"bash".to_string()), "{names:?}");
        assert!(found.iter().all(|i| i.program.starts_with(&bin)));
        // Nothing is recommended without a script to recommend for.
        assert!(found.iter().all(|i| !i.recommended));
    }

    #[test]
    fn scan_ignores_non_executable_files_and_empty_path_entries() {
        let tmp = TempDir::new().expect("tmp");
        let bin = tmp.path().join("bin");
        write_script(&bin, "python3", "not executable", false);
        let path = format!("{}::{}", bin.display(), bin.display());
        assert!(scan(None, None, &path, false).is_empty());
    }

    #[test]
    fn scan_dedupes_by_canonical_path() {
        let tmp = TempDir::new().expect("tmp");
        let (path, bin) = fake_path(&tmp);
        let other = tmp.path().join("other");
        fs::create_dir_all(&other).expect("mkdir");
        std::os::unix::fs::symlink(bin.join("python3"), other.join("python3")).expect("symlink");
        let both = format!("{}:{}", other.display(), path);
        let pythons = scan(None, None, &both, false)
            .into_iter()
            .filter(|i| i.kind == InterpreterKind::Python)
            .count();
        assert_eq!(pythons, 1, "同一个二进制只应出现一次");
    }

    #[test]
    fn versions_are_parsed_from_the_program_itself() {
        let tmp = TempDir::new().expect("tmp");
        let (path, _) = fake_path(&tmp);
        let found = scan(None, None, &path, true);
        let by = |n: &str| {
            found
                .iter()
                .find(|i| i.display_name().starts_with(n))
                .and_then(|i| i.version.clone())
        };
        assert_eq!(by("python3"), Some("3.12.7".to_string()));
        assert_eq!(by("uv"), Some("0.8.4".to_string()));
        assert_eq!(by("node"), Some("22.22.0".to_string()));
        assert_eq!(by("bash"), Some("5.2.37".to_string()));
    }

    #[test]
    fn parse_version_handles_the_usual_shapes() {
        assert_eq!(parse_version("Python 3.9.6"), Some("3.9.6".into()));
        assert_eq!(parse_version("v22.22.0\n"), Some("22.22.0".into()));
        assert_eq!(
            parse_version("GNU bash, version 3.2.57(1)-release"),
            Some("3.2.57".into())
        );
        assert_eq!(
            parse_version("zsh 5.9 (arm64-apple-darwin24.0)"),
            Some("5.9".into())
        );
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("no numbers here"), None);
    }

    #[test]
    fn py_without_project_markers_recommends_system_python() {
        let tmp = TempDir::new().expect("tmp");
        let (path, bin) = fake_path(&tmp);
        let script = write_script(&tmp.path().join("proj"), "sync.py", "print(1)\n", false);
        let found = scan(Some(&script), None, &path, false);
        let rec = found.iter().find(|i| i.recommended).expect("有推荐");
        assert_eq!(rec.program, bin.join("python3"));
        assert_eq!(
            rec.reason.as_deref(),
            Some("按 .py 扩展名推荐；项目里没有 .venv 或 uv.lock")
        );
        assert_eq!(found[0].program, rec.program, "推荐项排在最前");
    }

    #[test]
    fn py_with_uv_lock_recommends_uv_run() {
        let tmp = TempDir::new().expect("tmp");
        let (path, bin) = fake_path(&tmp);
        let proj = tmp.path().join("proj");
        fs::create_dir_all(&proj).expect("mkdir");
        fs::write(proj.join("pyproject.toml"), "[project]\n").expect("write");
        fs::write(proj.join("uv.lock"), "").expect("write");
        let script = write_script(&proj, "sync.py", "print(1)\n", false);
        let rec = scan(Some(&script), None, &path, false)
            .into_iter()
            .find(|i| i.recommended)
            .expect("有推荐");
        assert_eq!(rec.kind, InterpreterKind::Uv);
        assert_eq!(rec.program, bin.join("uv"));
        assert_eq!(rec.prefix_args, vec!["run".to_string()]);
        assert!(rec.reason.as_deref().is_some_and(|r| r.contains("uv.lock")));
    }

    #[test]
    fn py_with_a_project_venv_beats_uv() {
        let tmp = TempDir::new().expect("tmp");
        let (path, _) = fake_path(&tmp);
        let proj = tmp.path().join("proj");
        fs::create_dir_all(&proj).expect("mkdir");
        fs::write(proj.join("uv.lock"), "").expect("write");
        let venv = fake_program(&proj.join(".venv").join("bin"), "python", "Python 3.13.2");
        let script = write_script(&proj, "sync.py", "print(1)\n", false);

        let found = scan(Some(&script), None, &path, false);
        let rec = found.iter().find(|i| i.recommended).expect("有推荐");
        assert_eq!(rec.program, venv);
        assert_eq!(rec.origin, Origin::ProjectVenv);
        assert_eq!(
            rec.reason.as_deref(),
            Some("按 .py 扩展名推荐；项目里有 .venv")
        );
    }

    #[test]
    fn the_project_walk_finds_markers_above_the_script() {
        let tmp = TempDir::new().expect("tmp");
        let proj = tmp.path().join("proj");
        let deep = proj.join("scripts").join("jobs");
        fs::create_dir_all(&deep).expect("mkdir");
        fs::write(proj.join("uv.lock"), "").expect("write");
        fs::write(proj.join("pyproject.toml"), "").expect("write");
        let signals = project_signals(Some(&deep));
        assert_eq!(signals.root.as_deref(), Some(proj.as_path()));
        assert!(signals.uv_lock && signals.pyproject);
        assert!(!signals.package_json);
    }

    #[test]
    fn project_dir_overrides_the_scripts_own_directory() {
        let tmp = TempDir::new().expect("tmp");
        let (path, _) = fake_path(&tmp);
        let proj = tmp.path().join("proj");
        let venv = fake_program(&proj.join(".venv").join("bin"), "python", "Python 3.13.2");
        // The script lives somewhere else entirely.
        let script = write_script(&tmp.path().join("elsewhere"), "run.py", "print(1)\n", false);

        let without = scan(Some(&script), None, &path, false);
        assert!(without.iter().all(|i| i.origin != Origin::ProjectVenv));

        let with = scan(Some(&script), Some(&proj), &path, false);
        let rec = with.iter().find(|i| i.recommended).expect("有推荐");
        assert_eq!(rec.program, venv);
    }

    #[test]
    fn js_recommends_node_and_rb_recommends_ruby() {
        let tmp = TempDir::new().expect("tmp");
        let (path, bin) = fake_path(&tmp);
        let dir = tmp.path().join("proj");
        for (name, program, reason) in [
            ("app.js", bin.join("node"), "按 .js 扩展名推荐"),
            ("task.rb", bin.join("ruby"), "按 .rb 扩展名推荐"),
        ] {
            let script = write_script(&dir, name, "// hi\n", false);
            let rec = scan(Some(&script), None, &path, false)
                .into_iter()
                .find(|i| i.recommended)
                .unwrap_or_else(|| panic!("{name} 应该有推荐"));
            assert_eq!(rec.program, program, "{name}");
            assert_eq!(rec.reason.as_deref(), Some(reason));
        }
    }

    #[test]
    fn an_executable_script_with_a_shebang_offers_and_recommends_direct() {
        let tmp = TempDir::new().expect("tmp");
        let (path, _) = fake_path(&tmp);
        let script = write_script(
            &tmp.path().join("proj"),
            "backup.sh",
            "#!/bin/zsh\necho hi\n",
            true,
        );
        let found = scan(Some(&script), None, &path, false);
        let rec = found.iter().find(|i| i.recommended).expect("有推荐");
        assert_eq!(rec.kind, InterpreterKind::Direct);
        assert_eq!(rec.program, script);
        assert_eq!(rec.display_name(), "直接执行");
        assert!(rec.reason.as_deref().is_some_and(|r| r.contains("shebang")));
    }

    #[test]
    fn an_env_shebang_with_flags_still_names_the_interpreter() {
        let tmp = TempDir::new().expect("tmp");
        let (path, bin) = fake_path(&tmp);
        // `env -S` (and `NAME=value` assignments) sit between `env` and the
        // program; neither is the interpreter.
        let script = write_script(
            &tmp.path().join("proj"),
            "job",
            "#!/usr/bin/env -S FOO=1 python3 -u\nprint(1)\n",
            false,
        );
        let rec = scan(Some(&script), None, &path, false)
            .into_iter()
            .find(|i| i.recommended)
            .expect("有推荐");
        assert_eq!(rec.program, bin.join("python3"));
        assert_eq!(rec.reason.as_deref(), Some("按 shebang 推荐"));
    }

    #[test]
    fn a_non_executable_shell_script_falls_back_to_the_shebangs_shell() {
        let tmp = TempDir::new().expect("tmp");
        let (path, bin) = fake_path(&tmp);
        let script = write_script(
            &tmp.path().join("proj"),
            "backup.sh",
            "#!/usr/bin/env bash\necho hi\n",
            false,
        );
        let found = scan(Some(&script), None, &path, false);
        assert!(
            found.iter().all(|i| i.kind != InterpreterKind::Direct),
            "不可执行的脚本不该出现「直接执行」"
        );
        let rec = found.iter().find(|i| i.recommended).expect("有推荐");
        assert_eq!(rec.program, bin.join("bash"));
        assert_eq!(rec.reason.as_deref(), Some("按 shebang 推荐"));
    }

    #[test]
    fn an_unknown_extension_without_a_shebang_has_no_recommendation() {
        let tmp = TempDir::new().expect("tmp");
        let (path, _) = fake_path(&tmp);
        let script = write_script(&tmp.path().join("proj"), "thing.xyz", "data\n", false);
        assert!(
            scan(Some(&script), None, &path, false)
                .iter()
                .all(|i| !i.recommended)
        );
    }

    #[test]
    fn apply_composes_and_detect_takes_it_apart_again() {
        let script = PathBuf::from("/p/sync.py");
        let args = vec!["--once".to_string()];
        let cases = [
            Interpreter {
                kind: InterpreterKind::Python,
                program: PathBuf::from("/usr/bin/python3"),
                prefix_args: Vec::new(),
                version: None,
                origin: Origin::System,
                recommended: false,
                reason: None,
            },
            Interpreter {
                kind: InterpreterKind::Uv,
                program: PathBuf::from("/Users/x/.local/bin/uv"),
                prefix_args: vec!["run".to_string()],
                version: None,
                origin: Origin::Uv,
                recommended: false,
                reason: None,
            },
        ];
        for interp in cases {
            let (path, composed) = apply(&script, &args, &interp, &[]);
            assert_eq!(path, interp.program);
            let back = detect_from_task(&path, &composed).expect("能还原");
            assert_eq!(back.script, script);
            assert_eq!(back.args, args);
            assert!(back.interp_args.is_empty());
            assert_eq!(back.interpreter.kind, interp.kind);
            assert_eq!(back.interpreter.prefix_args, interp.prefix_args);
            assert_eq!(back.interpreter.program, interp.program);
        }
    }

    #[test]
    fn interpreter_flags_survive_a_round_trip() {
        let python = Interpreter {
            kind: InterpreterKind::Python,
            program: PathBuf::from("/usr/bin/python3"),
            prefix_args: Vec::new(),
            version: None,
            origin: Origin::System,
            recommended: false,
            reason: None,
        };
        let script = PathBuf::from("/p/sync.py");
        let args = vec!["--once".to_string()];
        let flags = vec!["-u".to_string()];

        let (path, composed) = apply(&script, &args, &python, &flags);
        assert_eq!(
            composed,
            vec!["-u".to_string(), "/p/sync.py".into(), "--once".into()]
        );

        let back = detect_from_task(&path, &composed).expect("能还原");
        assert_eq!(back.interp_args, flags);
        assert_eq!(back.script, script);
        assert_eq!(back.args, args, "脚本自己的参数不该被当成解释器参数");

        // `--` ends the interpreter's flags; what follows is the script.
        let back = detect_from_task(
            Path::new("/usr/bin/python3"),
            &["-u".to_string(), "--".into(), "/p/-weird.py".into()],
        )
        .expect("能还原");
        assert_eq!(back.interp_args, vec!["-u".to_string(), "--".to_string()]);
        assert_eq!(back.script, PathBuf::from("/p/-weird.py"));

        // uv keeps its prefix arg in front of the flags.
        let uv = Interpreter {
            kind: InterpreterKind::Uv,
            program: PathBuf::from("/opt/x/bin/uv"),
            prefix_args: vec!["run".to_string()],
            version: None,
            origin: Origin::Path,
            recommended: false,
            reason: None,
        };
        let (_, composed) = apply(&script, &[], &uv, &["--frozen".to_string()]);
        assert_eq!(
            composed,
            vec!["run".to_string(), "--frozen".into(), "/p/sync.py".into()]
        );
        let back = detect_from_task(Path::new("/opt/x/bin/uv"), &composed).expect("能还原");
        assert_eq!(back.interpreter.prefix_args, vec!["run".to_string()]);
        assert_eq!(back.interp_args, vec!["--frozen".to_string()]);
        assert_eq!(back.script, script);
    }

    #[test]
    fn a_script_named_after_an_interpreter_is_not_one() {
        for name in ["/p/node.js", "/p/python.py", "/p/bash.sh", "/p/uv.py"] {
            let got = detect_from_task(Path::new(name), &["--flag".to_string()])
                .unwrap_or_else(|| panic!("{name} 应该还原成直接执行"));
            assert_eq!(got.interpreter.kind, InterpreterKind::Direct, "{name}");
            assert_eq!(got.script, PathBuf::from(name), "{name}");
            assert_eq!(got.args, vec!["--flag".to_string()], "{name}");
        }
        // A real version suffix still counts as the interpreter.
        for name in ["/usr/bin/python3.13", "/usr/bin/ruby2.7"] {
            let got = detect_from_task(Path::new(name), &["/p/a".to_string()])
                .unwrap_or_else(|| panic!("{name} 应该是解释器"));
            assert_ne!(got.interpreter.kind, InterpreterKind::Direct, "{name}");
            assert_eq!(got.script, PathBuf::from("/p/a"), "{name}");
        }
    }

    #[test]
    fn a_relative_script_is_resolved_against_the_project_dir() {
        let tmp = TempDir::new().expect("tmp");
        let (path, _) = fake_path(&tmp);
        let proj = tmp.path().join("proj");
        let venv = fake_program(&proj.join(".venv").join("bin"), "python", "Python 3.13.2");
        write_script(&proj.join("scripts"), "sync.py", "print(1)\n", false);

        let relative = Path::new("scripts/sync.py");
        let found = scan(Some(relative), Some(&proj), &path, false);
        let rec = found.iter().find(|i| i.recommended).expect("有推荐");
        assert_eq!(rec.program, venv, "相对路径应先拼到工作目录上再判断");

        // Without a working directory there is nothing to resolve against:
        // no project signals, and no 直接执行 entry either.
        let found = scan(Some(relative), None, &path, false);
        assert!(found.iter().all(|i| i.origin != Origin::ProjectVenv));
        assert!(found.iter().all(|i| i.kind != InterpreterKind::Direct));
    }

    #[test]
    fn project_signals_refuses_a_relative_start() {
        assert!(project_signals(Some(Path::new("proj/scripts"))).is_empty());
        assert!(project_signals(Some(Path::new(""))).is_empty());
    }

    #[test]
    fn an_executable_script_is_only_offered_once() {
        let tmp = TempDir::new().expect("tmp");
        let bin = tmp.path().join("bin");
        // A "script" that is also the bash on our fake PATH.
        let script = fake_program(&bin, "bash", "GNU bash, version 5.2.37(1)-release");
        let found = scan(Some(&script), None, &bin.to_string_lossy(), false);
        assert_eq!(
            found.len(),
            1,
            "同一个可执行文件不该既是解释器又是「直接执行」: {found:?}"
        );
    }

    #[test]
    fn apply_direct_leaves_the_script_alone() {
        let script = PathBuf::from("/p/backup.sh");
        let args = vec!["--dry-run".to_string()];
        let direct = Interpreter {
            kind: InterpreterKind::Direct,
            program: script.clone(),
            prefix_args: Vec::new(),
            version: None,
            origin: Origin::Custom,
            recommended: false,
            reason: None,
        };
        let (path, composed) = apply(&script, &args, &direct, &[]);
        assert_eq!(path, script);
        assert_eq!(composed, args);

        let back = detect_from_task(&path, &composed).expect("能还原");
        assert_eq!(back.interpreter.kind, InterpreterKind::Direct);
        assert_eq!(back.script, script);
        assert_eq!(back.args, args);
    }

    #[test]
    fn detect_understands_versioned_and_env_style_interpreters() {
        let back = detect_from_task(
            Path::new("/opt/homebrew/bin/python3.13"),
            &["/p/a.py".to_string()],
        )
        .expect("能还原");
        assert_eq!(back.interpreter.kind, InterpreterKind::Python);
        assert_eq!(back.interpreter.origin, Origin::Homebrew);
        assert_eq!(back.script, PathBuf::from("/p/a.py"));
        assert!(back.args.is_empty());
    }

    #[test]
    fn detect_returns_none_for_an_interpreter_with_no_script() {
        assert!(detect_from_task(Path::new("/usr/bin/python3"), &[]).is_none());
        assert!(detect_from_task(Path::new(""), &[]).is_none());
    }

    #[test]
    fn labels_and_details_read_like_the_design() {
        let i = Interpreter {
            kind: InterpreterKind::Python,
            program: PathBuf::from("/usr/bin/python3"),
            prefix_args: Vec::new(),
            version: Some("3.9.6".into()),
            origin: Origin::System,
            recommended: true,
            reason: None,
        };
        assert_eq!(i.display_name(), "python3");
        assert_eq!(i.detail(), "3.9.6 · 系统 /usr/bin/python3");
        assert_eq!(i.label(), "python3 · 系统");
        assert_eq!(i.id(), "/usr/bin/python3");

        let uv = Interpreter {
            kind: InterpreterKind::Uv,
            program: PathBuf::from("/opt/homebrew/bin/uv"),
            prefix_args: vec!["run".into()],
            version: Some("0.8.4".into()),
            origin: Origin::Homebrew,
            recommended: false,
            reason: None,
        };
        assert_eq!(uv.display_name(), "uv run");
        assert_eq!(uv.id(), "/opt/homebrew/bin/uv run");
    }

    #[test]
    fn origins_are_classified_by_prefix() {
        assert_eq!(
            classify_origin(Path::new("/usr/bin/python3")),
            Origin::System
        );
        assert_eq!(classify_origin(Path::new("/bin/zsh")), Origin::System);
        assert_eq!(
            classify_origin(Path::new("/p/proj/.venv/bin/python")),
            Origin::ProjectVenv
        );
        assert_eq!(
            classify_origin(Path::new("/opt/whatever/bin/x")),
            Origin::Path
        );
        if let Some(home) = dirs::home_dir() {
            let local_bin = home.join(".local").join("bin");
            // Only uv itself is "uv 安装"; the rest of ~/.local/bin is a
            // user install that uv may have had nothing to do with.
            assert_eq!(classify_origin(&local_bin.join("uv")), Origin::Uv);
            assert_eq!(
                classify_origin(&local_bin.join("python3")),
                Origin::UserLocal
            );
        }
    }

    #[test]
    fn a_homebrew_prefix_without_brew_in_it_is_just_path() {
        // The label follows the machine: these two prefixes only mean
        // Homebrew when that prefix actually has a brew in it.
        for (program, brew) in [
            ("/opt/homebrew/bin/node", "/opt/homebrew/bin/brew"),
            ("/usr/local/bin/node", "/usr/local/bin/brew"),
        ] {
            let want = if Path::new(brew).exists() {
                Origin::Homebrew
            } else {
                Origin::Path
            };
            assert_eq!(classify_origin(Path::new(program)), want, "{program}");
        }
    }

    /// The reason [`run_version`] reads on a helper thread instead of just
    /// waiting for the child: this shim exits immediately but leaves a
    /// background `sleep` holding the stdout it inherited. Waiting for the
    /// child succeeds at once and reading the pipe then blocks for 20
    /// seconds — the pipe only closes when the *grandchild* lets go.
    #[test]
    fn a_shim_that_leaks_stdout_to_a_background_child_still_times_out() {
        let tmp = TempDir::new().expect("tmp");
        let shim = write_script(
            tmp.path(),
            "slowpoke",
            "#!/bin/sh\necho 'slowpoke 1.2.3'\nsleep 20 &\nexit 0\n",
            true,
        );

        let started = std::time::Instant::now();
        assert_eq!(version_of(&shim), None, "超时的探测报告不出版本号");
        let elapsed = started.elapsed();
        assert!(
            elapsed < VERSION_TIMEOUT + Duration::from_millis(1500),
            "应该在超时后不久就返回，实际用了 {elapsed:?}"
        );

        // And the cache answers the second call without probing again.
        let started = std::time::Instant::now();
        assert_eq!(version_of(&shim), None);
        assert!(started.elapsed() < Duration::from_millis(200));
    }

    #[test]
    fn a_normal_program_is_probed_well_inside_the_timeout() {
        let tmp = TempDir::new().expect("tmp");
        let p = fake_program(tmp.path(), "quick", "quick 9.9.9");
        let started = std::time::Instant::now();
        assert_eq!(version_of(&p), Some("9.9.9".to_string()));
        assert!(started.elapsed() < VERSION_TIMEOUT);
    }

    #[test]
    fn shebang_is_read_only_from_real_shebang_lines() {
        let tmp = TempDir::new().expect("tmp");
        let dir = tmp.path();
        let with = write_script(dir, "a.sh", "#!/bin/zsh -e\necho\n", false);
        assert_eq!(shebang(&with).as_deref(), Some("/bin/zsh -e"));
        let without = write_script(dir, "b.sh", "echo\n", false);
        assert_eq!(shebang(&without), None);
        let empty = write_script(dir, "c.sh", "", false);
        assert_eq!(shebang(&empty), None);
    }
}
