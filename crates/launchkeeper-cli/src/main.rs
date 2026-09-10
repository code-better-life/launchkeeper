//! `launchkeeper`: the first-class, script- and AI-agent-friendly
//! command-line interface to `launchkeeper-core` (the crate itself keeps its
//! original name, `launchkeeper-cli`; only the `[[bin]]` was renamed). It is
//! a full citizen alongside the Tauri app, not a scaffold that gets thrown
//! away once the GUI exists — every mutating and read command has a stable
//! `--json` shape documented in `docs/CLI.md` for non-interactive callers.
//!
//! Design contract: `docs/M1-design.md` §10, `docs/CLI.md`.

mod completions;
mod docs;
mod fmt;
mod interp;
mod json;
mod trigger_args;

use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use clap_complete::engine::ArgValueCompleter;
use launchkeeper_core::interpreters;
use launchkeeper_core::plist::{PlistOptions, build_plist};
use launchkeeper_core::{
    Error, ExternalAgent, Interpreter, JobStatus, Run, Service, Store, Task, TaskName, agents, ai,
    launchctl, paths,
};

use completions::{CompletionShell, complete_task_name};
use json::{
    AdoptPlanJson, AdoptPlanResultJson, AgentJson, AgentResultJson, AiConfigJson,
    AiConfigResultJson, AiTestResultJson, ConfigDataDirJson, ConfigDataDirResultJson, InsightJson,
    InsightResultJson, InterpreterJson, LogsJson, ResultJson, RunJson, StatusJson, TaskJson,
};
use trigger_args::{TriggerArgs, trigger_from_args};

/// Visualize and drive Launchkeeper tasks from the command line.
#[derive(Debug, Parser)]
#[command(name = "launchkeeper", version)]
struct Cli {
    /// Path to the `launchkeeper-runner` binary. Overrides
    /// `LAUNCHKEEPER_RUNNER` and the sibling-binary lookup.
    #[arg(long, global = true)]
    runner: Option<PathBuf>,

    /// Overrides where Launchkeeper keeps its database and logs
    /// (`LAUNCHKEEPER_DATA_DIR`). See `config data-dir` for a persistent,
    /// non-env-var way to set this.
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,

    /// Print machine-readable JSON instead of human text. See `docs/CLI.md`
    /// for the exact shape of each command. No-op for `plist`, which always
    /// prints raw XML.
    #[arg(long, short = 'j', global = true)]
    json: bool,

    #[command(subcommand)]
    command: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Registers a new task.
    Add(AddArgs),
    /// Edits fields of an existing task without re-adding it. Reloads
    /// launchd automatically if the task is enabled.
    Set(SetArgs),
    /// Changes an existing task's trigger.
    #[command(alias = "trigger")]
    SetTrigger(SetTriggerArgs),
    /// Lists every task.
    #[command(alias = "ls")]
    List,
    /// Shows one task in full, plus its recent runs.
    Show {
        /// Task name.
        #[arg(add = ArgValueCompleter::new(complete_task_name))]
        name: String,
    },
    /// Loads the task into launchd.
    #[command(alias = "on")]
    Enable {
        /// Task name.
        #[arg(add = ArgValueCompleter::new(complete_task_name))]
        name: String,
    },
    /// Unloads the task from launchd (keeps the database row and logs).
    ///
    /// Also accepts the label of a LaunchAgent Launchkeeper does not manage,
    /// which is unloaded from launchd without its plist being touched. When
    /// that agent is one Launchkeeper would refuse to adopt — i.e. it belongs
    /// to some other piece of software — this asks first (`--yes` skips).
    #[command(alias = "off")]
    Disable {
        /// Task name, or the label of an external LaunchAgent.
        #[arg(add = ArgValueCompleter::new(complete_task_name))]
        name: String,
        /// Skip the confirmation prompt for a foreign LaunchAgent.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Deletes a task entirely: launchd job, plist, database row, logs.
    #[command(alias = "rm")]
    Remove {
        /// Task name.
        #[arg(add = ArgValueCompleter::new(complete_task_name))]
        name: String,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
    /// Runs a task right now.
    Run {
        /// Task name.
        #[arg(add = ArgValueCompleter::new(complete_task_name))]
        name: String,
        /// Skip launchd: exec the runner directly (`run <name> --manual`).
        #[arg(long)]
        direct: bool,
    },
    /// Starts a service task (`--manual`), loading it into launchd first if
    /// it is not loaded yet. Equivalent to `run` for other triggers.
    Start {
        /// Task name.
        #[arg(add = ArgValueCompleter::new(complete_task_name))]
        name: String,
    },
    /// Stops a running task by sending SIGTERM through launchd. The job stays
    /// loaded, so `start` can bring it back without re-enabling.
    Stop {
        /// Task name.
        #[arg(add = ArgValueCompleter::new(complete_task_name))]
        name: String,
    },
    /// `stop` followed by `start`.
    Restart {
        /// Task name.
        #[arg(add = ArgValueCompleter::new(complete_task_name))]
        name: String,
    },
    /// Prints a run's captured output.
    #[command(alias = "log")]
    Logs {
        /// Task name.
        #[arg(add = ArgValueCompleter::new(complete_task_name))]
        name: String,
        /// 1 = newest, 2 = second-newest, ...
        #[arg(long, default_value_t = 1)]
        last: usize,
        /// Print stderr instead of stdout.
        #[arg(long)]
        err: bool,
        /// Only show the last N lines of the log content. Works in both
        /// human and `--json` mode; sets `"truncated": true` in JSON when it
        /// actually cuts content.
        #[arg(long)]
        tail: Option<usize>,
    },
    /// Lists recent runs of a task.
    Runs {
        /// Task name.
        #[arg(add = ArgValueCompleter::new(complete_task_name))]
        name: String,
        /// How many runs to show.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Prints the plist that would be generated for a task.
    Plist {
        /// Task name.
        #[arg(add = ArgValueCompleter::new(complete_task_name))]
        name: String,
    },
    /// Summarizes every task's launchd state.
    #[command(alias = "st")]
    Status,
    /// Lists the LaunchAgents in `~/Library/LaunchAgents` that Launchkeeper
    /// did not create, with a verdict on whether each could be adopted.
    Agents,
    /// Takes a hand-written LaunchAgent under management, in place: the label
    /// and the file stay where they are, but the runner is put in front of
    /// the script so runs, exit codes and logs get recorded.
    Adopt {
        /// The launchd label, as printed by `agents`.
        label: String,
        /// Print the plist diff and stop; nothing is written.
        #[arg(long)]
        dry_run: bool,
        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Undoes `adopt`: restores the original plist from `<plist>.bak`, byte
    /// for byte, and forgets the task. Log files are kept.
    Unadopt {
        /// The adopted task's name, which is its launchd label.
        #[arg(add = ArgValueCompleter::new(complete_task_name))]
        name: String,
        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Lists interpreters (python3, uv run, node, ...) found on `PATH` and,
    /// with `--script`, recommends one — the same discovery/recommendation
    /// logic `add`/`set` use to auto-pick. Never touches the database.
    Interpreters {
        /// Recommend for this script (by extension, then shebang). Must
        /// exist. Omit to just list everything found on `PATH`, nothing
        /// recommended.
        #[arg(long)]
        script: Option<PathBuf>,
        /// Where to look for project markers (`.venv`, `pyproject.toml`,
        /// `uv.lock`, `package.json`). Defaults to `--script`'s own
        /// directory.
        #[arg(long)]
        cwd: Option<PathBuf>,
    },
    /// Prints a static shell completion script to stdout. See `docs/CLI.md`
    /// for the install line for each shell, and for the alternative, dynamic
    /// `COMPLETE=<shell>` activation that also completes task names.
    Completions {
        /// Which shell to generate the script for.
        shell: CompletionShell,
    },
    /// Asks the configured model what a task does, whether it is healthy and
    /// why it last failed. Prints the stored explanation if there is one;
    /// `--refresh` always calls the model again.
    Explain {
        /// Task name.
        #[arg(add = ArgValueCompleter::new(complete_task_name))]
        name: String,
        /// Ignore the stored explanation and generate a new one.
        #[arg(long, short = 'r')]
        refresh: bool,
        /// Language for the answer. Defaults to `zh-CN`.
        #[arg(long, default_value = "zh-CN")]
        lang: String,
    },
    /// Prints the complete CLI reference (`docs/CLI.md`), embedded in this
    /// binary at compile time. Meant to be piped into a pager, a file, or an
    /// AI assistant's context; `--json` has no effect, the output is
    /// Markdown either way.
    Docs,
    /// Prints a short, copy-pasteable prompt that tells an AI assistant that
    /// Launchkeeper is installed and how to drive it — starting with
    /// `launchkeeper docs`. `--json` has no effect.
    #[command(name = "ai-prompt")]
    AiPrompt {
        /// `zh-CN` or `en`. Defaults to `LC_ALL`/`LANG`, and to `zh-CN` when
        /// neither is set.
        #[arg(long)]
        lang: Option<String>,
    },
    /// AI provider configuration and the API key. See `explain`.
    #[command(subcommand)]
    Ai(AiCmd),
    /// CLI-local configuration (not task configuration) — currently just
    /// where the data directory is.
    #[command(subcommand)]
    Config(ConfigCmd),
}

/// `launchkeeper ai <cmd>`.
#[derive(Debug, Subcommand)]
enum AiCmd {
    /// Prints, or sets, the non-secret AI configuration (`<data_dir>/ai.json`,
    /// shared with the desktop app). Never prints the API key.
    Config {
        /// `anthropic` or `openai_compatible`.
        #[arg(long)]
        provider: Option<String>,
        /// Endpoint root, e.g. `http://127.0.0.1:11434/v1` for a local
        /// Ollama. Pass an empty string to go back to the provider default.
        #[arg(long)]
        base_url: Option<String>,
        /// Model id, passed to the provider verbatim.
        #[arg(long)]
        model: Option<String>,
    },
    /// The API key, in the macOS key file.
    #[command(subcommand)]
    Key(AiKeyCmd),
    /// Sends a one-line ping to the configured endpoint and prints what came
    /// back. Costs a handful of tokens.
    Test,
}

/// `launchkeeper ai key <cmd>`.
#[derive(Debug, Subcommand)]
enum AiKeyCmd {
    /// Reads a key from stdin (or prompts on a tty) and stores it in the
    /// macOS key file. Deliberately takes no argument: a key on the command
    /// line ends up in shell history, in `ps` output and in any process
    /// listing on the machine.
    Set,
    /// Deletes the key file entry. Does not touch
    /// `LAUNCHKEEPER_AI_API_KEY`, which is an environment variable this
    /// process cannot unset for your shell.
    Clear,
}

/// `launchkeeper config <cmd>`.
#[derive(Debug, Subcommand)]
enum ConfigCmd {
    /// Prints, or persistently sets, the data directory
    /// (`~/.config/launchkeeper/data-dir`, below `--data-dir` and
    /// `LAUNCHKEEPER_DATA_DIR` in precedence — see `docs/CLI.md`).
    DataDir {
        /// New data directory to remember. Omit to print the currently
        /// resolved directory and which source produced it.
        path: Option<PathBuf>,
    },
}

#[derive(Debug, clap::Args)]
struct AddArgs {
    /// Task name (`^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$`). A whole launchd
    /// label is legal, which is what `adopt` relies on.
    name: String,
    /// Path to the script the runner should start. Combined with an
    /// interpreter (auto-detected by extension/shebang, or `--interpreter`)
    /// unless `--raw` is given, in which case it's used verbatim, exactly as
    /// it would have been before interpreter support existed.
    #[arg(long)]
    script: PathBuf,
    /// One argument to pass to the script (not to the interpreter — see
    /// `interpreters`). Repeatable.
    #[arg(long = "arg")]
    args: Vec<String>,
    /// Explicit interpreter: an `id` from `launchkeeper interpreters`
    /// (e.g. `/usr/bin/python3` or `/Users/x/.local/bin/uv run`), a bare
    /// program path, or `direct` to skip interpreters and let launchd exec
    /// the script itself (requires the script to be executable). Overrides
    /// auto-detection. Conflicts with `--raw`.
    #[arg(long, conflicts_with = "raw")]
    interpreter: Option<String>,
    /// Skips interpreter resolution entirely: `--script`/`--arg` are stored
    /// exactly as given, the way `add` worked before this feature existed
    /// (e.g. `--script ~/.local/bin/uv --arg run --arg scripts/sync.py`).
    /// Conflicts with `--interpreter`.
    #[arg(long, conflicts_with = "interpreter")]
    raw: bool,
    /// Working directory. Defaults to `$HOME` if left unset. Also where
    /// interpreter auto-detection looks for project markers (`.venv`,
    /// `uv.lock`, `package.json`) when not `--raw`.
    #[arg(long)]
    cwd: Option<PathBuf>,
    /// `KEY=VALUE` environment override. Repeatable.
    #[arg(long = "env", value_parser = parse_env_kv)]
    env: Vec<(String, String)>,
    #[command(flatten)]
    trigger: TriggerArgs,
    /// Human-facing name shown in listings. Defaults to `name`.
    #[arg(long)]
    display_name: Option<String>,
    /// Optional longer description.
    #[arg(long)]
    description: Option<String>,
    /// A tag for filtering. Repeatable.
    #[arg(long = "tag")]
    tags: Vec<String>,
    /// Sets launchd `KeepAlive` (only valid with `--at-login` or
    /// `--manual`). On a `--manual` service it means "restart after a crash,
    /// but stay down after a manual stop".
    #[arg(long)]
    keep_alive: bool,
    /// Kill the script after this long, e.g. `30m`, `2h`, `90s`. The run is
    /// recorded with exit code 124.
    #[arg(long)]
    timeout: Option<String>,
}

#[derive(Debug, clap::Args)]
struct SetTriggerArgs {
    /// Task name.
    #[arg(add = ArgValueCompleter::new(complete_task_name))]
    name: String,
    #[command(flatten)]
    trigger: TriggerArgs,
}

#[derive(Debug, clap::Args)]
struct SetArgs {
    /// Task name.
    #[arg(add = ArgValueCompleter::new(complete_task_name))]
    name: String,
    /// New human-facing display name.
    #[arg(long)]
    display_name: Option<String>,
    /// New description.
    #[arg(long)]
    description: Option<String>,
    /// New script path (same resolution and interpreter combination as
    /// `add`'s `--script`, unless `--raw` is given).
    #[arg(long)]
    script: Option<PathBuf>,
    /// New argument to pass to the script. Repeatable; if any `--arg` is
    /// given, it REPLACES the full argument list (old arguments are cleared
    /// first). Omit entirely to keep the script's current arguments — this
    /// still applies when only `--interpreter` or a new `--script` changes,
    /// not just when nothing about the run configuration changes.
    #[arg(long = "arg")]
    args: Vec<String>,
    /// Explicit interpreter, same grammar as `add --interpreter`. Given
    /// alone (no `--script`), re-composes the task's current script under
    /// the new interpreter. Conflicts with `--raw`.
    #[arg(long, conflicts_with = "raw")]
    interpreter: Option<String>,
    /// Skips interpreter resolution for this edit: any `--script`/`--arg`
    /// given are stored verbatim, like `add --raw`. Conflicts with
    /// `--interpreter`.
    #[arg(long, conflicts_with = "interpreter")]
    raw: bool,
    /// New working directory.
    #[arg(long)]
    cwd: Option<PathBuf>,
    /// `KEY=VALUE` to set/overwrite in the environment. Repeatable, merges
    /// into the existing map.
    #[arg(long = "env", value_parser = parse_env_kv)]
    env: Vec<(String, String)>,
    /// Environment key to remove. Repeatable.
    #[arg(long = "unset-env")]
    unset_env: Vec<String>,
    /// New tag. Repeatable; if any `--tag` is given, it REPLACES the full
    /// tag list, same as `--arg`.
    #[arg(long = "tag")]
    tags: Vec<String>,
    /// New timeout, e.g. `30m`, `2h`, `90s`. Conflicts with `--no-timeout`.
    #[arg(long, conflicts_with = "no_timeout")]
    timeout: Option<String>,
    /// Clears the timeout.
    #[arg(long, conflicts_with = "timeout")]
    no_timeout: bool,
    /// Pins the task to the menu bar.
    #[arg(long, conflicts_with = "no_favorite")]
    favorite: bool,
    /// Unpins the task from the menu bar.
    #[arg(long, conflicts_with = "favorite")]
    no_favorite: bool,
    /// Enables the failure notification.
    #[arg(long, conflicts_with = "no_notify")]
    notify: bool,
    /// Disables the failure notification.
    #[arg(long, conflicts_with = "notify")]
    no_notify: bool,
    /// Sets launchd `KeepAlive` (only valid with `--at-login` or
    /// `--manual`). On a `--manual` service it means "restart after a crash,
    /// but stay down after a manual stop".
    #[arg(long, conflicts_with = "no_keep_alive")]
    keep_alive: bool,
    /// Clears `KeepAlive`.
    #[arg(long, conflicts_with = "keep_alive")]
    no_keep_alive: bool,
}

fn parse_env_kv(s: &str) -> std::result::Result<(String, String), String> {
    let (k, v) = s
        .split_once('=')
        .ok_or_else(|| format!("--env 格式错误: {s:?}，期望 KEY=VALUE"))?;
    if k.is_empty() {
        return Err(format!("--env 键不能为空: {s:?}"));
    }
    Ok((k.to_string(), v.to_string()))
}

fn main() {
    // Must run before anything else touches stdin/stdout: if `COMPLETE` is
    // set in the environment, this prints completion candidates and exits
    // the process right here, never reaching the rest of `main`.
    completions::hook_dynamic_completion();

    if let Err(e) = try_main() {
        eprintln!("错误: {e:#}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<()> {
    let cli = Cli::parse();

    // Must happen before any core call that touches paths::data_dir().
    if let Some(dir) = &cli.data_dir {
        // SAFETY: single-threaded at this point, before any core path lookup.
        unsafe { std::env::set_var(paths::DATA_DIR_ENV, dir) };
    }

    // Neither prints/writes a task, so both bypass the store entirely —
    // `completions` doesn't touch the data dir at all, and `config` only
    // reads/writes the tiny data-dir config file, not the database.
    if let Cmd::Completions { shell } = &cli.command {
        completions::print_static_script(*shell);
        return Ok(());
    }
    // Two compile-time constants. Neither reads the database, the data
    // directory or `~/Library/LaunchAgents`, so both work on a machine where
    // Launchkeeper has never been run — which is exactly the machine an
    // assistant runs `docs` on before it does anything else.
    match &cli.command {
        Cmd::Docs => {
            print!("{}", docs::CLI_REFERENCE);
            return Ok(());
        }
        Cmd::AiPrompt { lang } => {
            let lang = lang
                .as_deref()
                .map_or_else(docs::default_lang, ai::Lang::from_tag);
            print!("{}", docs::prompt(lang));
            return Ok(());
        }
        _ => {}
    }
    if let Cmd::Config(cmd) = cli.command {
        return cmd_config(cmd, cli.json);
    }
    // `ai` reads and writes `<data_dir>/ai.json` and the key file; none of
    // its subcommands opens the database. Same early-out as `config`, so
    // `ai key set` works before a single task exists.
    if let Cmd::Ai(cmd) = cli.command {
        return cmd_ai(cmd, cli.json);
    }
    // Pure filesystem/PATH discovery, no task involved: same early-out as
    // `completions`/`config` above, so this works without a data dir ever
    // having been initialized.
    if let Cmd::Interpreters { script, cwd } = &cli.command {
        return cmd_interpreters(script.as_deref(), cwd.as_deref(), cli.json);
    }

    let db_path = paths::db_path().context("确定数据库路径失败")?;
    let store = Store::open(&db_path).context("打开数据库失败")?;
    // The runner is resolved lazily: only the commands that actually bake a
    // path into a plist (or exec it) need one, so `list`/`show`/`runs`/
    // `logs`/`status`/`disable`/`remove` keep working on a machine where the
    // runner binary is missing.
    let service = if needs_runner(&cli.command) {
        Service::new(store, resolve_runner(cli.runner.as_deref())?)
    } else {
        Service::without_runner(store)
    };

    let json = cli.json;
    match cli.command {
        Cmd::Add(args) => cmd_add(&service, args, json),
        Cmd::Set(args) => cmd_set(&service, args, json),
        Cmd::SetTrigger(args) => cmd_set_trigger(&service, args, json),
        Cmd::List => cmd_list(&service, json),
        Cmd::Show { name } => cmd_show(&service, &name, json),
        Cmd::Enable { name } => cmd_enable(&service, &name, json),
        Cmd::Disable { name, yes } => cmd_disable(&service, &name, yes, json),
        Cmd::Remove { name, yes } => cmd_remove(&service, &name, yes, json),
        Cmd::Run { name, direct } => cmd_run(&service, &name, direct, json),
        Cmd::Start { name } => cmd_lifecycle(&service, &name, Lifecycle::Start, json),
        Cmd::Stop { name } => cmd_lifecycle(&service, &name, Lifecycle::Stop, json),
        Cmd::Restart { name } => cmd_lifecycle(&service, &name, Lifecycle::Restart, json),
        Cmd::Logs {
            name,
            last,
            err,
            tail,
        } => cmd_logs(&service, &name, last, err, tail, json),
        Cmd::Runs { name, limit } => cmd_runs(&service, &name, limit, json),
        Cmd::Plist { name } => cmd_plist(&service, &name),
        Cmd::Status => cmd_status(&service, json),
        Cmd::Agents => cmd_agents(json),
        Cmd::Adopt {
            label,
            dry_run,
            yes,
        } => cmd_adopt(&service, &label, dry_run, yes, json),
        Cmd::Unadopt { name, yes } => cmd_unadopt(&service, &name, yes, json),
        Cmd::Explain {
            name,
            refresh,
            lang,
        } => cmd_explain(&service, &name, refresh, &lang, json),
        // Handled above, before the store was even opened.
        Cmd::Completions { .. }
        | Cmd::Config(_)
        | Cmd::Ai(_)
        | Cmd::Interpreters { .. }
        | Cmd::Docs
        | Cmd::AiPrompt { .. } => {
            unreachable!()
        }
    }
}

/// Which subcommands actually need a runner binary: the ones that write a
/// plist (`add` refreshes nothing but validates against the same path,
/// `enable`, `plist`, `set-trigger`, `set`) or exec it (`run`). Everything
/// else must keep working on a machine where no runner is installed.
fn needs_runner(cmd: &Cmd) -> bool {
    matches!(
        cmd,
        Cmd::Add(_)
            | Cmd::Set(_)
            | Cmd::SetTrigger(_)
            | Cmd::Enable { .. }
            | Cmd::Plist { .. }
            | Cmd::Run { .. }
            // start (and restart, which ends in a start) enables a task that
            // is not loaded yet, and that writes a plist.
            | Cmd::Start { .. }
            | Cmd::Restart { .. }
            // adopt rewrites the adopted plist to point at the runner, so it
            // has to know where the runner is — even for `--dry-run`, whose
            // whole output is the diff that path would produce.
            | Cmd::Adopt { .. }
    )
}

/// `--runner` > `LAUNCHKEEPER_RUNNER` > sibling of the current binary.
fn resolve_runner(explicit: Option<&Path>) -> Result<PathBuf> {
    let source = resolve_runner_source(explicit)?;
    launchkeeper_core::runner_install::install(&source)
        .with_context(|| format!("安装 runner 失败: {}", source.display()))
}

/// File name of the runner binary wherever it is installed.
const RUNNER_NAME: &str = "launchkeeper-runner";

/// Where the runner binary comes from: `--runner` > `LAUNCHKEEPER_RUNNER` >
/// next to this executable > `<exe dir>/../libexec/`.
///
/// The last two cover every shipped layout: Homebrew's Formula puts both
/// binaries in the same `bin/` of a Cellar keg (`current_exe` resolves the
/// `/opt/homebrew/bin` symlink, so the sibling is found through it), and a
/// keg that would rather keep the helper off `PATH` puts it in `libexec/`.
/// The app bundle is the same shape as the first: both binaries sit in
/// `Launchkeeper.app/Contents/MacOS/`.
fn resolve_runner_source(explicit: Option<&Path>) -> Result<PathBuf> {
    let exe = std::env::current_exe().context("无法确定当前可执行文件路径")?;
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("当前可执行文件没有父目录: {}", exe.display()))?;
    let env = std::env::var_os("LAUNCHKEEPER_RUNNER").map(PathBuf::from);
    locate_runner_source(explicit, env.as_deref(), dir).ok_or_else(|| {
        anyhow::anyhow!(
            "找不到 {RUNNER_NAME}：请用 --runner 指定路径，或设置 LAUNCHKEEPER_RUNNER，\
             或把它放在 {} 旁边（尝试的路径: {}）",
            dir.display(),
            runner_candidates(dir)
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("、")
        )
    })
}

/// The probe order of [`resolve_runner_source`] with its two environmental
/// inputs passed in, so that it can be tested against temporary directories
/// instead of against whatever this machine happens to have installed.
///
/// `explicit` and `env` are returned verbatim without being probed: an
/// explicitly named path that does not exist must produce the installer's
/// "no such file" error naming *that* path, not a lookup failure listing
/// paths the caller never asked for. An empty `env` counts as unset, so that
/// `LAUNCHKEEPER_RUNNER= launchkeeper …` falls back instead of erroring.
fn locate_runner_source(
    explicit: Option<&Path>,
    env: Option<&Path>,
    exe_dir: &Path,
) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return Some(p.to_path_buf());
    }
    if let Some(p) = env
        && !p.as_os_str().is_empty()
    {
        return Some(p.to_path_buf());
    }
    runner_candidates(exe_dir).into_iter().find(|p| p.exists())
}

/// The installed-layout paths probed, in order, when nothing was named
/// explicitly.
fn runner_candidates(exe_dir: &Path) -> Vec<PathBuf> {
    vec![
        exe_dir.join(RUNNER_NAME),
        exe_dir.join("..").join("libexec").join(RUNNER_NAME),
    ]
}

fn task_name(s: &str) -> Result<TaskName> {
    TaskName::new(s).map_err(anyhow::Error::from)
}

/// Makes `p` absolute without resolving symlinks: joining the cwd when
/// needed, then removing `.` and `..` lexically. Canonicalizing would rewrite
/// a deliberately chosen symlink path (e.g. a stable `~/bin/task` pointing at
/// a versioned script) into whatever it happens to point at today, and the
/// plist would then outlive the link.
fn absolute_path(p: &Path, must_exist: bool) -> Result<PathBuf> {
    if must_exist && !p.exists() {
        bail!("路径不存在: {}", p.display());
    }
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().context("获取当前目录失败")?.join(p)
    };
    Ok(lexical_normalize(&joined))
}

/// Drops `.` components and resolves `..` against the preceding component,
/// purely on the path string.
fn lexical_normalize(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                // 只有普通目录名才能被 ".." 抵消；根和前缀要保留。
                let pops = matches!(out.components().next_back(), Some(Component::Normal(_)));
                if pops {
                    out.pop();
                } else if out.as_os_str().is_empty() {
                    out.push(c.as_os_str());
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// Builds the task `add` would create, plus the [`Interpreter`] that was
/// resolved for it (with `--version` filled in, ready to print) — `None`
/// when `--raw` skipped resolution entirely.
fn build_task(name: TaskName, args: &AddArgs) -> Result<(Task, Option<Interpreter>)> {
    let trigger = trigger_from_args(&args.trigger).map_err(|e| anyhow::anyhow!(e))?;
    let script_input = absolute_path(&args.script, true)
        .with_context(|| format!("--script 无法解析: {}", args.script.display()))?;
    let cwd = match &args.cwd {
        Some(cwd) => Some(absolute_path(cwd, false)?),
        None => None,
    };

    let (script_path, task_args, resolved) = if args.raw {
        (script_input, args.args.clone(), None)
    } else {
        let interp = interp::resolve_interpreter(
            &script_input,
            cwd.as_deref(),
            args.interpreter.as_deref(),
        )?;
        // No interpreter flags here: `add` composes a command from scratch,
        // and `--script`/`--arg` have no way to express one. (`set` below
        // does carry them over, because there they may already exist.)
        let (script_path, task_args) = interpreters::apply(&script_input, &args.args, &interp, &[]);
        (script_path, task_args, Some(interp::with_version(interp)))
    };

    let mut task = Task::new(name.clone(), script_path, trigger);
    task.display_name = args
        .display_name
        .clone()
        .unwrap_or_else(|| name.as_str().to_string());
    task.description = args.description.clone();
    task.args = task_args;
    task.working_dir = cwd;
    task.env = args.env.iter().cloned().collect::<BTreeMap<_, _>>();
    task.tags = args.tags.clone();
    task.keep_alive = args.keep_alive;
    task.timeout_secs = match &args.timeout {
        Some(t) => Some(trigger_args::parse_duration(t).map_err(|e| anyhow::anyhow!(e))?),
        None => None,
    };
    task.validate().context("任务校验失败")?;
    Ok((task, resolved))
}

/// Prints `v` as pretty JSON on stdout.
fn print_json<T: serde::Serialize>(v: &T) {
    match serde_json::to_string_pretty(v) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("JSON 序列化失败: {e}"),
    }
}

/// Builds the JSON view of `task`, filling in launchd state and last-run
/// info the same way `show`/`list`/`status` already compute it.
fn build_task_json(service: &Service, task: &Task) -> Result<TaskJson> {
    let enabled = service.is_enabled(&task.name).unwrap_or(false);
    let job: Option<JobStatus> = launchctl::status(&task.label()).unwrap_or(None);
    let last_run: Option<Run> = service.store().last_run(&task.name)?;
    Ok(TaskJson::from_task(
        task,
        enabled,
        job.as_ref(),
        last_run.as_ref(),
    ))
}

/// Shared tail of every mutating command: on success, print JSON (if `json`)
/// or call `on_success` for the human message; on error, print the JSON
/// error envelope and exit(1) (if `json`), or propagate the error as before.
fn emit_mutation_result(
    json: bool,
    result: Result<TaskJson>,
    on_success: impl FnOnce(&TaskJson),
) -> Result<()> {
    match result {
        Ok(t) => {
            if json {
                print_json(&ResultJson::ok(t));
            } else {
                on_success(&t);
            }
            Ok(())
        }
        Err(e) => {
            if json {
                print_json(&ResultJson::err(format!("{e:#}")));
                std::process::exit(1);
            }
            Err(e)
        }
    }
}

fn cmd_add(service: &Service, args: AddArgs, json: bool) -> Result<()> {
    let result = (|| -> Result<(TaskJson, Option<Interpreter>)> {
        let name = task_name(&args.name)?;
        let (task, resolved) = build_task(name.clone(), &args)?;
        service.add_task(task).context("添加任务失败")?;
        let t = service
            .store()
            .get_task(&name)?
            .context("刚添加的任务读取失败")?;
        Ok((build_task_json(service, &t)?, resolved))
    })();
    match result {
        Ok((t, resolved)) => {
            if json {
                print_json(&ResultJson::ok(t));
            } else {
                println!("已添加任务 {}", t.name);
                if let Some(interp) = &resolved {
                    println!("运行方式: {}", interp::interpreter_line(interp));
                }
            }
            Ok(())
        }
        Err(e) => {
            if json {
                print_json(&ResultJson::err(format!("{e:#}")));
                std::process::exit(1);
            }
            Err(e)
        }
    }
}

fn cmd_set(service: &Service, args: SetArgs, json: bool) -> Result<()> {
    let result = (|| -> Result<(TaskJson, Option<Interpreter>)> {
        let name = task_name(&args.name)?;
        let mut task = service
            .store()
            .get_task(&name)?
            .ok_or_else(|| anyhow::anyhow!("任务不存在: {}", args.name))?;
        let resolved = apply_set(&mut task, &args)?;
        service.update_task(task.clone()).context("更新任务失败")?;
        Ok((build_task_json(service, &task)?, resolved))
    })();
    match result {
        Ok((t, resolved)) => {
            if json {
                print_json(&ResultJson::ok(t));
            } else {
                println!("已更新 {}", t.name);
                if let Some(interp) = &resolved {
                    println!("运行方式: {}", interp::interpreter_line(interp));
                }
            }
            Ok(())
        }
        Err(e) => {
            if json {
                print_json(&ResultJson::err(format!("{e:#}")));
                std::process::exit(1);
            }
            Err(e)
        }
    }
}

/// Applies the optional-field edits of `set` onto `task`, then validates it.
/// Only flags that were actually given change anything. Returns the
/// resolved [`Interpreter`] (version filled in) when `--script`/`--arg`/
/// `--interpreter` actually went through interpreter resolution — `None`
/// when nothing about the run configuration changed, or `--raw` was given.
fn apply_set(task: &mut Task, args: &SetArgs) -> Result<Option<Interpreter>> {
    if let Some(v) = &args.display_name {
        task.display_name = v.clone();
    }
    if let Some(v) = &args.description {
        task.description = Some(v.clone());
    }

    let cwd = match &args.cwd {
        Some(v) => Some(absolute_path(v, false)?),
        None => None,
    };
    if let Some(cwd) = &cwd {
        task.working_dir = Some(cwd.clone());
    }

    let mut resolved: Option<Interpreter> = None;
    if args.raw {
        if let Some(v) = &args.script {
            task.script_path = absolute_path(v, true)
                .with_context(|| format!("--script 无法解析: {}", v.display()))?;
        }
        if !args.args.is_empty() {
            task.args = args.args.clone();
        }
    } else if args.script.is_some() || !args.args.is_empty() || args.interpreter.is_some() {
        // Any of these touches the run configuration: re-derive the script,
        // its own arguments and the interpreter, falling back to what the
        // task currently runs for whichever of the three was not given.
        let current = interpreters::detect_from_task(&task.script_path, &task.args);
        let script = match &args.script {
            Some(v) => absolute_path(v, true)
                .with_context(|| format!("--script 无法解析: {}", v.display()))?,
            None => current
                .as_ref()
                .map(|d| d.script.clone())
                .ok_or_else(|| anyhow::anyhow!("无法从当前任务判断脚本路径，请提供 --script"))?,
        };
        let script_args = if !args.args.is_empty() {
            args.args.clone()
        } else {
            current.as_ref().map(|d| d.args.clone()).unwrap_or_default()
        };
        // Project-signal scanning: the new --cwd if given, else the task's
        // existing working_dir (scan falls back to the script's own
        // directory either way when this is None).
        let scan_cwd = cwd.clone().or_else(|| task.working_dir.clone());
        // Flags that belong to the interpreter (`python3 -u x.py`) are not
        // expressible on the command line either, but they may already be in
        // the task — keep them, unless the interpreter itself is being
        // replaced, in which case its predecessor's flags mean nothing.
        let keep_interp_args = args.interpreter.is_none();
        let interp_args = match (&current, keep_interp_args) {
            (Some(d), true) => d.interp_args.clone(),
            _ => Vec::new(),
        };
        let interp = if args.script.is_some() || args.interpreter.is_some() {
            interp::resolve_interpreter(&script, scan_cwd.as_deref(), args.interpreter.as_deref())?
        } else if let Some(d) = current {
            // Only --arg changed: keep whatever interpreter is already
            // running this script instead of re-scanning the machine.
            d.interpreter
        } else {
            interp::resolve_interpreter(&script, scan_cwd.as_deref(), None)?
        };
        let (script_path, task_args) =
            interpreters::apply(&script, &script_args, &interp, &interp_args);
        task.script_path = script_path;
        task.args = task_args;
        resolved = Some(interp::with_version(interp));
    }

    for (k, v) in &args.env {
        task.env.insert(k.clone(), v.clone());
    }
    for k in &args.unset_env {
        task.env.remove(k);
    }
    if !args.tags.is_empty() {
        task.tags = args.tags.clone();
    }
    if args.no_timeout {
        task.timeout_secs = None;
    } else if let Some(t) = &args.timeout {
        task.timeout_secs = Some(trigger_args::parse_duration(t).map_err(|e| anyhow::anyhow!(e))?);
    }
    if args.favorite {
        task.favorite = true;
    }
    if args.no_favorite {
        task.favorite = false;
    }
    if args.notify {
        task.notify_on_fail = true;
    }
    if args.no_notify {
        task.notify_on_fail = false;
    }
    if args.keep_alive {
        task.keep_alive = true;
    }
    if args.no_keep_alive {
        task.keep_alive = false;
    }
    task.updated_at = chrono::Utc::now();
    task.validate().context("任务校验失败")?;
    Ok(resolved)
}

fn cmd_set_trigger(service: &Service, args: SetTriggerArgs, json: bool) -> Result<()> {
    let result = (|| -> Result<TaskJson> {
        let name = task_name(&args.name)?;
        let trigger = trigger_from_args(&args.trigger).map_err(|e| anyhow::anyhow!(e))?;
        let mut task = service
            .store()
            .get_task(&name)?
            .ok_or_else(|| anyhow::anyhow!("任务不存在: {}", args.name))?;
        task.trigger = trigger;
        task.validate().context("触发规则校验失败")?;
        task.updated_at = chrono::Utc::now();
        service
            .update_task(task.clone())
            .context("更新触发规则失败")?;
        build_task_json(service, &task)
    })();
    emit_mutation_result(json, result, |t| println!("已更新 {} 的触发规则", t.name))
}

fn cmd_list(service: &Service, json: bool) -> Result<()> {
    let tasks = service.store().list_tasks().context("读取任务列表失败")?;
    if json {
        let mut arr = Vec::with_capacity(tasks.len());
        for t in &tasks {
            arr.push(build_task_json(service, t)?);
        }
        print_json(&arr);
        return Ok(());
    }
    let mut rows = Vec::new();
    for t in &tasks {
        let enabled = service.is_enabled(&t.name).unwrap_or(false);
        let last = service.store().last_run(&t.name)?;
        let last_str = match last {
            Some(r) => format!(
                "{} (exit {})",
                fmt::local_time(r.started_at),
                fmt::exit_code(&r)
            ),
            None => "-".to_string(),
        };
        rows.push(vec![
            t.name.as_str().to_string(),
            t.trigger.describe(),
            if enabled {
                "✓".to_string()
            } else {
                "–".to_string()
            },
            last_str,
        ]);
    }
    print!(
        "{}",
        fmt::table(&["NAME", "TRIGGER", "ENABLED", "LAST RUN"], &rows)
    );
    Ok(())
}

fn cmd_show(service: &Service, name: &str, json: bool) -> Result<()> {
    let n = task_name(name)?;
    let t = service
        .store()
        .get_task(&n)?
        .ok_or_else(|| anyhow::anyhow!("任务不存在: {name}"))?;
    if json {
        print_json(&build_task_json(service, &t)?);
        return Ok(());
    }
    let enabled = service.is_enabled(&n).unwrap_or(false);

    println!("name:            {}", t.name);
    println!("display_name:    {}", t.display_name);
    println!(
        "description:     {}",
        t.description.as_deref().unwrap_or("-")
    );
    println!("script_path:     {}", t.script_path.display());
    println!("args:            {}", t.args.join(" "));
    if let Some(line) = interp::detect_display_line(&t.script_path, &t.args) {
        println!("运行方式:        {line}");
    }
    println!(
        "working_dir:     {}",
        t.working_dir
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!("env:");
    for (k, v) in &t.env {
        println!("  {k}={v}");
    }
    println!("trigger:         {}", t.trigger.describe());
    println!("keep_alive:      {}", t.keep_alive);
    println!(
        "timeout:         {}",
        t.timeout_secs
            .map(|s| format!("{s} 秒"))
            .unwrap_or_else(|| "-".to_string())
    );
    println!("tags:            {}", t.tags.join(", "));
    println!("favorite:        {}", t.favorite);
    println!("notify_on_fail:  {}", t.notify_on_fail);
    println!("enabled:         {}", if enabled { "✓" } else { "–" });
    println!("created_at:      {}", fmt::local_time(t.created_at));
    println!("updated_at:      {}", fmt::local_time(t.updated_at));

    let runs = service.store().list_runs(&n, 5)?;
    println!("\nlast {} run(s):", runs.len());
    let rows: Vec<Vec<String>> = runs
        .iter()
        .map(|r| {
            vec![
                r.id.0.to_string(),
                fmt::local_time(r.started_at),
                fmt::duration(r),
                fmt::exit_code(r),
                r.trigger_kind.as_str().to_string(),
                fmt::stop_reason(r),
            ]
        })
        .collect();
    print!(
        "{}",
        fmt::table(
            &["ID", "STARTED", "DURATION", "EXIT", "KIND", "STOP"],
            &rows
        )
    );
    Ok(())
}

fn cmd_enable(service: &Service, name: &str, json: bool) -> Result<()> {
    if let Some(agent) = external_target(service, name)? {
        let result = (|| -> Result<AgentJson> {
            service
                .external_enable(&agent.label, &agent.path)
                .with_context(|| format!("加载 {} 失败", agent.label))?;
            reread_agent(&agent.label)
        })();
        return emit_agent_result(json, result, |a| {
            println!(
                "已加载外部 LaunchAgent {}（未接管，plist 没有被改动）",
                a.label
            );
        });
    }
    let result = (|| -> Result<TaskJson> {
        let n = task_name(name)?;
        service.enable(&n).context("启用任务失败")?;
        let t = service
            .store()
            .get_task(&n)?
            .ok_or_else(|| anyhow::anyhow!("任务不存在: {name}"))?;
        build_task_json(service, &t)
    })();
    emit_mutation_result(json, result, |t| println!("已启用 {}", t.name))
}

/// The plist path of `name`, when `name` is a task Launchkeeper adopted.
///
/// An adopted task drives the user's *own* file, at the path they put it
/// (`docs/M3-design.md` §3.1), so every command that says "the plist" is
/// talking about a file the user wrote by hand. `disable` and `remove` delete
/// it; a prompt that does not say so is asking about the wrong thing. Errors
/// answer `None`: this only ever adds a sentence, and a task that cannot be
/// read here will fail loudly a moment later anyway.
fn adopted_plist(service: &Service, name: &str) -> Option<PathBuf> {
    let n = task_name(name).ok()?;
    let task = service.store().get_task(&n).ok().flatten()?;
    if !task.adopted {
        return None;
    }
    paths::plist_path(&task).ok()
}

/// Whether turning `agent` off is turning off *somebody else's* software.
///
/// "Not adoptable" is the honest proxy for "belongs to another product": the
/// verdict that stops `adopt` — an Apple/Homebrew label prefix, a program
/// inside an `.app` bundle or under `/usr`, a `WatchPaths`/`MachServices`
/// job — is exactly the verdict that says this plist was installed by an
/// updater, a sync client or a background helper rather than written by the
/// user. A plist Launchkeeper itself wrote but has no row for is excluded:
/// it is refused for a different reason and is not someone else's.
fn is_foreign_agent(agent: &ExternalAgent) -> bool {
    !agent.managed_by_launchkeeper && agent.adoptable.reason().is_some()
}

/// The warning `off <label>` prints before unloading a foreign LaunchAgent.
fn foreign_disable_warning(agent: &ExternalAgent) -> String {
    format!(
        "警告：{label} 是别的软件自己装的 LaunchAgent（自动更新、云同步、后台助手一类），\
         Launchkeeper 并没有接管它。\n\
         不可接管的原因：{reason}\n\
         禁用它很可能让那个软件不再自动更新 / 不再同步，甚至直接不能用。\n\
         plist 文件不会被改动（还在 {path}），只是从 launchd 卸载；\
         想恢复就执行 `launchkeeper on {label}`。",
        label = agent.label,
        reason = agent.adoptable.reason().unwrap_or("不可接管"),
        path = agent.path.display(),
    )
}

fn cmd_disable(service: &Service, name: &str, yes: bool, json: bool) -> Result<()> {
    if let Some(agent) = external_target(service, name)? {
        if is_foreign_agent(&agent) && !yes {
            // `--json` is for non-interactive callers, so it never blocks on
            // stdin — same contract as `remove` and `adopt`.
            if json {
                let result: Result<AgentJson> = Err(anyhow::anyhow!(
                    "{} 不是 Launchkeeper 管理的任务，而是别的软件的 LaunchAgent（{}）；\
                     --json 模式下禁用它必须加 --yes",
                    agent.label,
                    agent.adoptable.reason().unwrap_or("不可接管"),
                ));
                return emit_agent_result(json, result, |_| {});
            }
            println!("{}", foreign_disable_warning(&agent));
            print!("确定要禁用 {} 吗？[y/N] ", agent.label);
            std::io::stdout().flush().ok();
            let mut answer = String::new();
            let _ = std::io::stdin().read_line(&mut answer);
            let answer = answer.trim().to_ascii_lowercase();
            if answer != "y" && answer != "yes" {
                println!("已取消");
                return Ok(());
            }
        }
        let result = (|| -> Result<AgentJson> {
            service
                .external_disable(&agent.label)
                .with_context(|| format!("卸载 {} 失败", agent.label))?;
            reread_agent(&agent.label)
        })();
        return emit_agent_result(json, result, |a| {
            println!(
                "已从 launchd 卸载 {}（plist 还在 {}，下次登录会再加载）",
                a.label,
                a.path.display()
            );
        });
    }
    // Read before the plist is gone: `disable` deletes it, and for an adopted
    // task that file is the user's own.
    let adopted = adopted_plist(service, name);
    let result = (|| -> Result<TaskJson> {
        let n = task_name(name)?;
        service.disable(&n).context("停用任务失败")?;
        let t = service
            .store()
            .get_task(&n)?
            .ok_or_else(|| anyhow::anyhow!("任务不存在: {name}"))?;
        build_task_json(service, &t)
    })();
    emit_mutation_result(json, result, |t| {
        println!("已停用 {}", t.name);
        if let Some(plist) = &adopted {
            println!(
                "注意：这是接管来的任务，{} 已被删除；原件的备份还在 {}.bak，\
                 `unadopt {}` 可以把它放回去",
                plist.display(),
                plist.display(),
                t.name
            );
        }
    })
}

fn cmd_remove(service: &Service, name: &str, yes: bool, json: bool) -> Result<()> {
    if !yes {
        if json {
            let result: Result<TaskJson> =
                Err(anyhow::anyhow!("--json 模式下删除任务必须加 --yes"));
            return emit_mutation_result(json, result, |_| {});
        }
        if let Some(plist) = adopted_plist(service, name) {
            println!(
                "{name} 是接管来的任务：删除会同时删掉你自己的 {}（原件的备份 {}.bak 会留下）。\n\
                 想把原来的 LaunchAgent 原样还回去，请用 `unadopt {name}` 而不是 remove。",
                plist.display(),
                plist.display()
            );
        }
        print!("确定要删除任务 {name} 吗？此操作不可恢复 [y/N] ");
        std::io::stdout().flush().ok();
        let mut answer = String::new();
        let _ = std::io::stdin().read_line(&mut answer);
        let answer = answer.trim().to_ascii_lowercase();
        if answer != "y" && answer != "yes" {
            println!("已取消");
            return Ok(());
        }
    }
    let result = (|| -> Result<TaskJson> {
        let n = task_name(name)?;
        let task = service
            .store()
            .get_task(&n)?
            .ok_or_else(|| anyhow::anyhow!("任务不存在: {name}"))?;
        let captured = build_task_json(service, &task)?;
        service.remove_task(&n).context("删除任务失败")?;
        Ok(captured)
    })();
    emit_mutation_result(json, result, |t| println!("已删除 {}", t.name))
}

fn cmd_run(service: &Service, name: &str, direct: bool, json: bool) -> Result<()> {
    if !direct && let Some(agent) = external_target(service, name)? {
        let result = (|| -> Result<AgentJson> {
            service
                .external_kickstart(&agent.label)
                .with_context(|| format!("kickstart {} 失败", agent.label))?;
            reread_agent(&agent.label)
        })();
        return emit_agent_result(json, result, |a| {
            println!(
                "已触发外部 LaunchAgent {}（未接管，不会有运行记录；`adopt {}` 之后才有）",
                a.label, a.label
            );
        });
    }
    if direct {
        let outcome = (|| -> Result<(TaskJson, i32)> {
            let n = task_name(name)?;
            let runner = service.runner_path()?.to_path_buf();
            let status = Command::new(&runner)
                .arg("run")
                .arg(name)
                .arg("--manual")
                .status()
                .with_context(|| format!("执行 runner 失败: {}", runner.display()))?;
            let code = status.code().unwrap_or(1);
            let t = service
                .store()
                .get_task(&n)?
                .ok_or_else(|| anyhow::anyhow!("任务不存在: {name}"))?;
            let tj = build_task_json(service, &t)?;
            Ok((tj, code))
        })();

        return match outcome {
            Ok((t, code)) => {
                if json {
                    if code == 0 {
                        print_json(&ResultJson::ok(t));
                        Ok(())
                    } else {
                        print_json(&ResultJson::err(format!("脚本以退出码 {code} 结束")));
                        std::process::exit(1);
                    }
                } else {
                    println!("已直接运行 {}", t.name);
                    if let Some(run) = &t.last_run {
                        println!(
                            "日志: stdout={} stderr={}",
                            run.stdout_path.display(),
                            run.stderr_path.display()
                        );
                    }
                    if code != 0 {
                        std::process::exit(code);
                    }
                    Ok(())
                }
            }
            Err(e) => {
                if json {
                    print_json(&ResultJson::err(format!("{e:#}")));
                    std::process::exit(1);
                }
                Err(e)
            }
        };
    }

    let result = (|| -> Result<TaskJson> {
        let n = task_name(name)?;
        match service.run_now(&n) {
            Ok(()) => {}
            Err(Error::NotEnabled(_)) => {
                bail!(
                    "任务 {name} 未启用，无法通过 launchd 运行；用 --direct 直接跑，或先 `enable {name}`"
                );
            }
            Err(e) => return Err(e.into()),
        }
        let t = service
            .store()
            .get_task(&n)?
            .ok_or_else(|| anyhow::anyhow!("任务不存在: {name}"))?;
        build_task_json(service, &t)
    })();
    emit_mutation_result(json, result, |t| println!("已触发 {}", t.name))
}

/// The three service verbs, so `start`/`stop`/`restart` share one body.
#[derive(Debug, Clone, Copy)]
enum Lifecycle {
    Start,
    Stop,
    Restart,
}

impl Lifecycle {
    fn past_tense(self) -> &'static str {
        match self {
            Lifecycle::Start => "已启动",
            Lifecycle::Stop => "已停止",
            Lifecycle::Restart => "已重启",
        }
    }
}

fn cmd_lifecycle(service: &Service, name: &str, what: Lifecycle, json: bool) -> Result<()> {
    let result = (|| -> Result<TaskJson> {
        let n = task_name(name)?;
        if service.store().get_task(&n)?.is_none() {
            bail!("任务不存在: {name}");
        }
        match what {
            Lifecycle::Start => service.start(&n).context("启动任务失败")?,
            Lifecycle::Stop => service.stop(&n).context("停止任务失败")?,
            Lifecycle::Restart => service.restart(&n).context("重启任务失败")?,
        }
        // 重新读一次：start 可能顺手把任务 enable 了，enabled/loaded/pid 都变了
        let t = service
            .store()
            .get_task(&n)?
            .ok_or_else(|| anyhow::anyhow!("任务不存在: {name}"))?;
        build_task_json(service, &t)
    })();
    emit_mutation_result(json, result, |t| {
        print!("{} {}", what.past_tense(), t.name);
        match t.pid {
            Some(pid) => println!("（pid {pid}）"),
            None => println!(),
        }
    })
}

/// Keeps only the last `n` lines of `content`. Returns the (possibly
/// unchanged) content and whether it was actually cut.
fn tail_lines(content: &str, n: usize) -> (String, bool) {
    if n == 0 {
        return (String::new(), !content.is_empty());
    }
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= n {
        return (content.to_string(), false);
    }
    let kept = &lines[lines.len() - n..];
    let mut s = kept.join("\n");
    if content.ends_with('\n') {
        s.push('\n');
    }
    (s, true)
}

fn cmd_logs(
    service: &Service,
    name: &str,
    last: usize,
    err: bool,
    tail: Option<usize>,
    json: bool,
) -> Result<()> {
    let n = task_name(name)?;
    if last == 0 {
        bail!("--last 必须 >= 1");
    }
    let runs = service.store().list_runs(&n, last)?;
    let run = runs
        .get(last - 1)
        .ok_or_else(|| anyhow::anyhow!("任务 {name} 没有第 {last} 近的运行记录"))?;
    if run.finished_at.is_none() && !json {
        eprintln!("运行 #{} 仍在进行中，以下是目前已捕获的输出", run.id.0);
    }
    let path = if err {
        run.stderr_path.clone()
    } else {
        run.stdout_path.clone()
    };
    let bytes =
        std::fs::read(&path).with_context(|| format!("读取日志失败: {}", path.display()))?;

    if tail.is_none() && !json {
        std::io::stdout().write_all(&bytes).ok();
        return Ok(());
    }

    let content = String::from_utf8_lossy(&bytes).into_owned();
    let (shown, truncated) = match tail {
        Some(n) => tail_lines(&content, n),
        None => (content, false),
    };
    if json {
        let stream = if err { "stderr" } else { "stdout" };
        print_json(&LogsJson {
            run_id: run.id.0,
            stream,
            path,
            content: shown,
            truncated,
        });
    } else {
        print!("{shown}");
    }
    Ok(())
}

fn cmd_runs(service: &Service, name: &str, limit: usize, json: bool) -> Result<()> {
    let n = task_name(name)?;
    let runs = service.store().list_runs(&n, limit)?;
    if json {
        let arr: Vec<RunJson> = runs.iter().map(RunJson::from_run).collect();
        print_json(&arr);
        return Ok(());
    }
    let rows: Vec<Vec<String>> = runs
        .iter()
        .map(|r| {
            vec![
                r.id.0.to_string(),
                fmt::local_time(r.started_at),
                fmt::duration(r),
                fmt::exit_code(r),
                r.trigger_kind.as_str().to_string(),
                fmt::stop_reason(r),
            ]
        })
        .collect();
    print!(
        "{}",
        fmt::table(
            &["ID", "STARTED", "DURATION", "EXIT", "KIND", "STOP"],
            &rows
        )
    );
    Ok(())
}

fn cmd_plist(service: &Service, name: &str) -> Result<()> {
    let n = task_name(name)?;
    let task = service
        .store()
        .get_task(&n)?
        .ok_or_else(|| anyhow::anyhow!("任务不存在: {name}"))?;
    let runner_log = paths::runner_log_path(&n).context("确定 runner 日志路径失败")?;
    let data_dir = launchkeeper_core::paths::data_dir()?;
    let opts = PlistOptions {
        runner_path: service.runner_path()?,
        runner_log: &runner_log,
        data_dir: &data_dir,
    };
    let dict = build_plist(&task, &opts);
    let stdout = std::io::stdout();
    plist::to_writer_xml(stdout.lock(), &plist::Value::Dictionary(dict))
        .context("序列化 plist 失败")?;
    println!();
    Ok(())
}

fn cmd_status(service: &Service, json: bool) -> Result<()> {
    let tasks = service.store().list_tasks().context("读取任务列表失败")?;
    if json {
        let mut arr = Vec::with_capacity(tasks.len());
        for t in &tasks {
            let enabled = service.is_enabled(&t.name).unwrap_or(false);
            let job = launchctl::status(&t.label()).unwrap_or(None);
            let last_run = service.store().last_run(&t.name)?;
            arr.push(StatusJson {
                name: t.name.as_str().to_string(),
                enabled,
                loaded: job.is_some(),
                is_service: t.is_service(),
                pid: job.as_ref().and_then(|j| j.pid),
                uptime_secs: json::uptime_secs(job.as_ref(), last_run.as_ref()),
                state: job.as_ref().map(|j| j.state.clone()),
                last_run: last_run.as_ref().map(RunJson::from_run),
            });
        }
        print_json(&arr);
        return Ok(());
    }
    let mut rows = Vec::new();
    for t in &tasks {
        let enabled = service.is_enabled(&t.name).unwrap_or(false);
        let job = launchctl::status(&t.label()).unwrap_or(None);
        let (loaded, pid, state) = match &job {
            Some(j) => (
                "✓".to_string(),
                j.pid
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                j.state.clone(),
            ),
            None => ("–".to_string(), "-".to_string(), "-".to_string()),
        };
        let last = service.store().last_run(&t.name)?;
        let uptime = json::uptime_secs(job.as_ref(), last.as_ref())
            .map(fmt::uptime)
            .unwrap_or_else(|| "-".to_string());
        let last_str = match last {
            Some(r) => format!("exit {}", fmt::exit_code(&r)),
            None => "-".to_string(),
        };
        rows.push(vec![
            t.name.as_str().to_string(),
            if t.is_service() {
                "服务".to_string()
            } else {
                "定时".to_string()
            },
            if enabled {
                "✓".to_string()
            } else {
                "–".to_string()
            },
            loaded,
            pid,
            uptime,
            state,
            last_str,
        ]);
    }
    print!(
        "{}",
        fmt::table(
            &[
                "NAME",
                "KIND",
                "ENABLED",
                "LOADED",
                "PID",
                "UPTIME",
                "STATE",
                "LAST RESULT"
            ],
            &rows
        )
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// M3 §3.3: LaunchAgents Launchkeeper did not write
// ---------------------------------------------------------------------------

/// Resolves `name` to a LaunchAgent Launchkeeper does not manage, when — and
/// only when — no task by that name exists.
///
/// This is what lets `on`/`off`/`run` take an external label. A task always
/// wins: an adopted task's name *is* its label, so it is found here as a task
/// and goes down the normal path, plist rewrite and run history included.
fn external_target(service: &Service, name: &str) -> Result<Option<ExternalAgent>> {
    if let Ok(n) = TaskName::new(name)
        && service.store().get_task(&n)?.is_some()
    {
        return Ok(None);
    }
    let dir = paths::launch_agents_dir()?;
    match agents::find(&dir, name) {
        Ok(a) => Ok(Some(a)),
        Err(Error::AgentNotFound(_)) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// The `emit_mutation_result` of the external path: same `ok` envelope,
/// carrying an `agent` instead of a `task`.
fn emit_agent_result(
    json: bool,
    result: Result<AgentJson>,
    on_success: impl FnOnce(&AgentJson),
) -> Result<()> {
    match result {
        Ok(a) => {
            if json {
                print_json(&AgentResultJson::ok(a));
            } else {
                on_success(&a);
            }
            Ok(())
        }
        Err(e) => {
            if json {
                print_json(&AgentResultJson::err(format!("{e:#}")));
                std::process::exit(1);
            }
            Err(e)
        }
    }
}

/// Re-reads `label` from disk so the printed agent shows the state *after*
/// the launchctl call, not before it.
fn reread_agent(label: &str) -> Result<AgentJson> {
    let dir = paths::launch_agents_dir()?;
    Ok(AgentJson::from_agent(&agents::find(&dir, label)?))
}

fn cmd_agents(json: bool) -> Result<()> {
    let dir = paths::launch_agents_dir().context("确定 LaunchAgents 目录失败")?;
    let found = agents::list_external(&dir).context("扫描 LaunchAgents 失败")?;
    if json {
        let arr: Vec<AgentJson> = found.iter().map(AgentJson::from_agent).collect();
        print_json(&arr);
        return Ok(());
    }
    let rows: Vec<Vec<String>> = found
        .iter()
        .map(|a| {
            vec![
                a.label.clone(),
                a.trigger_text(),
                if a.loaded { "✓".into() } else { "–".into() },
                a.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
                match a.adoptable.reason() {
                    None => "可接管".to_string(),
                    Some(r) => format!("否：{r}"),
                },
            ]
        })
        .collect();
    print!(
        "{}",
        fmt::table(&["LABEL", "TRIGGER", "LOADED", "PID", "ADOPTABLE"], &rows)
    );
    if !found.is_empty() {
        println!(
            "\n目录: {}（`adopt <LABEL>` 接管，`adopt <LABEL> --dry-run` 先看 plist 会怎么改）",
            dir.display()
        );
    }
    Ok(())
}

/// Prints the plist diff adoption would apply, in human form.
fn print_plan(plan: &launchkeeper_core::AdoptionPlan) {
    println!("接管 {}", plan.label);
    println!("  plist:  {}", plan.path.display());
    println!("  备份到: {}", plan.backup_path.display());
    println!("  任务名: {}（= label，不变）", plan.task.name);
    println!("  触发:   {}", plan.task.trigger.describe());
    let group = |title: &str, pairs: &[(String, String)], mark: char| {
        if pairs.is_empty() {
            return;
        }
        println!("  {title}");
        for (k, v) in pairs {
            println!("    {mark} {k} = {v}");
        }
    };
    group("移除:", &plan.removed, '-');
    group("新增:", &plan.added, '+');
    group("保持:", &plan.kept, ' ');
}

fn cmd_adopt(service: &Service, label: &str, dry_run: bool, yes: bool, json: bool) -> Result<()> {
    if dry_run {
        let result = (|| -> Result<(launchkeeper_core::AdoptionPlan, AdoptPlanJson)> {
            let plan = service.adoption_plan(label)?;
            let task = build_task_json(service, &plan.task)?;
            let view = AdoptPlanJson::new(&plan, task);
            Ok((plan, view))
        })();
        return match result {
            Ok((plan, view)) => {
                if json {
                    print_json(&AdoptPlanResultJson::ok(view));
                } else {
                    print_plan(&plan);
                    println!("\n（--dry-run，什么都没写）");
                }
                Ok(())
            }
            Err(e) => {
                if json {
                    print_json(&AdoptPlanResultJson::err(format!("{e:#}")));
                    std::process::exit(1);
                }
                Err(e)
            }
        };
    }

    // Show what is about to happen — the whole point of having a plan type is
    // that nobody finds out afterwards which keys the rewrite dropped. `-y`
    // skips the *question*, not the answer to "what did it just do to my
    // file"; only `--json` (whose caller reads structured output, and can ask
    // for the plan itself with `--dry-run --json`) prints nothing here.
    if !json {
        print_plan(&service.adoption_plan(label)?);
        if yes {
            println!();
        }
    }

    if !yes {
        if json {
            let result: Result<TaskJson> = Err(anyhow::anyhow!("--json 模式下接管必须加 --yes"));
            return emit_mutation_result(json, result, |_| {});
        }
        print!("\n确定要接管 {label} 吗？原文件会先备份为 .bak [y/N] ");
        std::io::stdout().flush().ok();
        let mut answer = String::new();
        let _ = std::io::stdin().read_line(&mut answer);
        let answer = answer.trim().to_ascii_lowercase();
        if answer != "y" && answer != "yes" {
            println!("已取消");
            return Ok(());
        }
    }

    let result = (|| -> Result<TaskJson> {
        let task = service.adopt(label).context("接管失败")?;
        let stored = service
            .store()
            .get_task(&task.name)?
            .context("刚接管的任务读取失败")?;
        build_task_json(service, &stored)
    })();
    emit_mutation_result(json, result, |t| {
        println!("已接管 {}（label 不变）", t.name);
        println!("撤销: launchkeeper unadopt {}", t.name);
    })
}

fn cmd_unadopt(service: &Service, name: &str, yes: bool, json: bool) -> Result<()> {
    if !yes {
        if json {
            let result: Result<TaskJson> =
                Err(anyhow::anyhow!("--json 模式下撤销接管必须加 --yes"));
            return emit_mutation_result(json, result, |_| {});
        }
        if let Some(plist) = adopted_plist(service, name) {
            println!(
                "{} 会按字节还原成接管前的样子，之后它就不再归 Launchkeeper 管了。",
                plist.display()
            );
        }
        print!(
            "确定要撤销接管 {name} 吗？在 Launchkeeper 里对它做过的改动（触发、环境变量、\
             超时等）会一并丢弃，运行历史也会删除（日志文件保留）[y/N] "
        );
        std::io::stdout().flush().ok();
        let mut answer = String::new();
        let _ = std::io::stdin().read_line(&mut answer);
        let answer = answer.trim().to_ascii_lowercase();
        if answer != "y" && answer != "yes" {
            println!("已取消");
            return Ok(());
        }
    }
    let result = (|| -> Result<TaskJson> {
        let n = task_name(name)?;
        let task = service
            .store()
            .get_task(&n)?
            .ok_or_else(|| anyhow::anyhow!("任务不存在: {name}"))?;
        // Snapshot before it disappears, like `remove` does.
        let captured = build_task_json(service, &task)?;
        service.unadopt(&n).context("撤销接管失败")?;
        Ok(captured)
    })();
    emit_mutation_result(json, result, |t| {
        println!("已撤销接管 {}，原 plist 已还原", t.name);
    })
}

/// `launchkeeper interpreters`. Never opens the store — same convention as
/// `completions`/`config`.
fn cmd_interpreters(script: Option<&Path>, cwd: Option<&Path>, json: bool) -> Result<()> {
    let script_abs = match script {
        Some(s) => Some(
            absolute_path(s, true)
                .with_context(|| format!("--script 无法解析: {}", s.display()))?,
        ),
        None => None,
    };
    let cwd_abs = match cwd {
        Some(c) => Some(absolute_path(c, false)?),
        None => None,
    };
    let path = launchkeeper_core::env::effective_path();
    // `with_versions: true` — this command exists specifically to be looked
    // at (or scraped by an agent), so the one-time `--version` cost per
    // candidate is worth it, unlike the fast scan `add`/`set` do.
    let candidates = interpreters::scan(script_abs.as_deref(), cwd_abs.as_deref(), &path, true);

    if json {
        let arr: Vec<InterpreterJson> = candidates
            .iter()
            .map(InterpreterJson::from_interpreter)
            .collect();
        print_json(&arr);
        return Ok(());
    }

    let rows: Vec<Vec<String>> = candidates
        .iter()
        .map(|c| {
            vec![
                c.id(),
                c.display_name(),
                c.version.clone().unwrap_or_else(|| "-".to_string()),
                c.origin.describe().to_string(),
                match (c.recommended, &c.reason) {
                    (true, Some(r)) => format!("推荐：{r}"),
                    (true, None) => "推荐".to_string(),
                    (false, _) => "-".to_string(),
                },
            ]
        })
        .collect();
    print!(
        "{}",
        fmt::table(&["ID", "程序", "版本", "来源", "推荐/理由"], &rows)
    );
    Ok(())
}

fn cmd_config(cmd: ConfigCmd, json: bool) -> Result<()> {
    match cmd {
        ConfigCmd::DataDir { path: None } => cmd_config_data_dir_show(json),
        ConfigCmd::DataDir { path: Some(p) } => cmd_config_data_dir_set(&p, json),
    }
}

fn cmd_config_data_dir_show(json: bool) -> Result<()> {
    let (data_dir, source) = paths::data_dir_source().context("确定数据目录失败")?;
    let config_file = paths::config_data_dir_file().context("确定配置文件路径失败")?;
    if json {
        print_json(&ConfigDataDirJson {
            data_dir,
            source: source.as_str(),
            config_file,
        });
        return Ok(());
    }
    println!("data_dir: {}", data_dir.display());
    let source_label = match source {
        paths::DataDirSource::Env => format!("环境变量 {}", paths::DATA_DIR_ENV),
        paths::DataDirSource::ConfigFile => format!("配置文件 {}", config_file.display()),
        paths::DataDirSource::Default => "默认路径".to_string(),
    };
    println!("来源:      {source_label}");
    if source != paths::DataDirSource::ConfigFile {
        println!(
            "（用 `launchkeeper config data-dir <path>` 写入 {} 来改变默认值；\
             --data-dir 和 {} 环境变量优先级更高）",
            config_file.display(),
            paths::DATA_DIR_ENV
        );
    }
    Ok(())
}

fn cmd_config_data_dir_set(path: &Path, json: bool) -> Result<()> {
    let result = (|| -> Result<(PathBuf, PathBuf)> {
        // 不要求存在：常见用法是先写好这个文件，再去建那个目录（或者让它在第一次
        // 用到时被 Store::open / logs 之类的调用自动建出来）。
        let abs = absolute_path(path, false)?;
        let file = paths::config_data_dir_file().context("确定配置文件路径失败")?;
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("创建目录失败: {}", dir.display()))?;
        }
        // Written as bytes, not through `Display`: a path is bytes on Unix
        // and `display()` replaces anything that is not valid UTF-8 with
        // U+FFFD, which would silently store a *different* directory.
        // `data_dir_source` reads this file back with `read_to_string`, so a
        // non-UTF-8 path still will not survive the round trip — but at
        // least it is not corrupted on the way in.
        let mut bytes = abs.as_os_str().as_bytes().to_vec();
        bytes.push(b'\n');
        std::fs::write(&file, bytes)
            .with_context(|| format!("写入配置文件失败: {}", file.display()))?;
        Ok((abs, file))
    })();
    match result {
        Ok((data_dir, config_file)) => {
            // The write succeeded but will not take effect while the env var
            // is set: it outranks the file (see `paths::data_dir_source`).
            let shadowed = std::env::var_os(paths::DATA_DIR_ENV).is_some_and(|v| !v.is_empty());
            let warning = shadowed.then(|| {
                format!(
                    "环境变量 {} 仍然设置着，优先级高于这个文件，取消它之后这个文件才会生效",
                    paths::DATA_DIR_ENV
                )
            });
            if json {
                print_json(&ConfigDataDirResultJson::ok(
                    data_dir.clone(),
                    config_file.clone(),
                    warning,
                ));
            } else {
                println!("已写入 {}", config_file.display());
                println!("新的 data_dir: {}", data_dir.display());
                if let Some(w) = warning {
                    println!("注意: {w}。");
                }
            }
            Ok(())
        }
        Err(e) => {
            if json {
                print_json(&ConfigDataDirResultJson::err(format!("{e:#}")));
                std::process::exit(1);
            }
            Err(e)
        }
    }
}

// ---- M4 §1: AI task insight ----------------------------------------------

/// Builds the JSON view of the current AI configuration. Never includes the
/// key itself — only whether one was found and which of the two sources it
/// came from.
fn ai_config_json(cfg: &ai::AiConfig) -> Result<AiConfigJson> {
    let env_key = std::env::var(ai::API_KEY_ENV)
        .ok()
        .is_some_and(|v| !v.trim().is_empty());
    // A key file read can be slow or (in a locked/headless session) fail;
    // that is not a reason for `ai config` to fail, so a failure reads as
    // "no key" rather than propagating.
    let has_key = env_key || ai::get_api_key().unwrap_or(None).is_some();
    Ok(AiConfigJson {
        provider: cfg.provider.as_str(),
        base_url: cfg.base_url.clone(),
        effective_base_url: cfg.effective_base_url(),
        endpoint: cfg.endpoint(),
        model: cfg.model.clone(),
        config_file: ai::config_path()?,
        has_api_key: has_key,
        api_key_source: match (has_key, env_key) {
            (false, _) => None,
            (true, true) => Some("env"),
            (true, false) => Some("file"),
        },
    })
}

fn cmd_ai(cmd: AiCmd, json: bool) -> Result<()> {
    match cmd {
        AiCmd::Config {
            provider,
            base_url,
            model,
        } => cmd_ai_config(
            provider.as_deref(),
            base_url.as_deref(),
            model.as_deref(),
            json,
        ),
        AiCmd::Key(AiKeyCmd::Set) => cmd_ai_key_set(json),
        AiCmd::Key(AiKeyCmd::Clear) => cmd_ai_key_clear(json),
        AiCmd::Test => cmd_ai_test(json),
    }
}

fn cmd_ai_config(
    provider: Option<&str>,
    base_url: Option<&str>,
    model: Option<&str>,
    json: bool,
) -> Result<()> {
    let writing = provider.is_some() || base_url.is_some() || model.is_some();
    let result = (|| -> Result<AiConfigJson> {
        let mut cfg = ai::load_config().context("读取 ai.json 失败")?;
        if let Some(p) = provider {
            cfg.provider = ai::AiProvider::parse(p)?;
        }
        if let Some(u) = base_url {
            // An empty string is how you say "go back to the provider
            // default" without a separate `--no-base-url` flag.
            cfg.base_url = (!u.trim().is_empty()).then(|| u.trim().to_string());
        }
        if let Some(m) = model {
            let m = m.trim();
            if m.is_empty() {
                bail!("--model 不能为空");
            }
            cfg.model = m.to_string();
        }
        if writing {
            ai::save_config(&cfg).context("写入 ai.json 失败")?;
        }
        ai_config_json(&cfg)
    })();

    match result {
        Ok(c) => {
            if json {
                if writing {
                    print_json(&AiConfigResultJson::ok(c));
                } else {
                    print_json(&c);
                }
            } else {
                if writing {
                    println!("已写入 {}", c.config_file.display());
                }
                println!("provider:  {}", c.provider);
                println!("base_url:  {}", c.effective_base_url);
                println!("model:     {}", c.model);
                println!("endpoint:  {}", c.endpoint);
                println!(
                    "api_key:   {}",
                    match c.api_key_source {
                        Some("env") => format!("已配置（环境变量 {}）", ai::API_KEY_ENV),
                        Some(_) => "已配置（密钥文件）".to_string(),
                        None => format!(
                            "未配置 —— `launchkeeper ai key set`，或设置 {}",
                            ai::API_KEY_ENV
                        ),
                    }
                );
            }
            Ok(())
        }
        Err(e) => {
            if json {
                print_json(&AiConfigResultJson::err(format!("{e:#}")));
                std::process::exit(1);
            }
            Err(e)
        }
    }
}

/// Reads the key without ever putting it on a command line.
///
/// From a pipe (`echo $KEY | launchkeeper ai key set`) it is one line of
/// stdin; from a terminal it is a prompt. Either way the key never appears in
/// argv, so it cannot be read out of shell history or another user's `ps`.
///
/// The terminal read is deliberately *not* silenced: doing that portably
/// means either a `termios` dependency or `unsafe`, and a key echoed into a
/// scrollback the user controls is a smaller problem than either. The prompt
/// says so, so nobody is surprised.
fn read_secret_from_stdin() -> Result<String> {
    use std::io::IsTerminal;
    let tty = std::io::stdin().is_terminal();
    if tty {
        eprint!("粘贴 API Key（回车结束，注意屏幕会回显）: ");
        std::io::stderr().flush().ok();
    }
    let mut line = String::new();
    let n = std::io::stdin()
        .read_line(&mut line)
        .context("读取 API Key 失败")?;
    if tty {
        eprintln!();
    }
    if n == 0 {
        bail!("没有从标准输入读到任何内容");
    }
    let key = line.trim().to_string();
    if key.is_empty() {
        bail!("API Key 不能为空");
    }
    Ok(key)
}

fn cmd_ai_key_set(json: bool) -> Result<()> {
    let result = (|| -> Result<AiConfigJson> {
        let key = read_secret_from_stdin()?;
        ai::set_api_key(&key)?;
        ai_config_json(&ai::load_config()?)
    })();
    match result {
        Ok(c) => {
            if json {
                print_json(&AiConfigResultJson::ok(c));
            } else {
                println!(
                    "已存入密钥文件 {}（权限 0600）",
                    ai::key_path()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                );
                if std::env::var_os(ai::API_KEY_ENV).is_some_and(|v| !v.is_empty()) {
                    println!(
                        "注意: 环境变量 {} 仍然设置着，读取时它的优先级更高。",
                        ai::API_KEY_ENV
                    );
                }
            }
            Ok(())
        }
        Err(e) => {
            if json {
                print_json(&AiConfigResultJson::err(format!("{e:#}")));
                std::process::exit(1);
            }
            Err(e)
        }
    }
}

fn cmd_ai_key_clear(json: bool) -> Result<()> {
    let result = (|| -> Result<AiConfigJson> {
        ai::clear_api_key()?;
        ai_config_json(&ai::load_config()?)
    })();
    match result {
        Ok(c) => {
            if json {
                print_json(&AiConfigResultJson::ok(c));
            } else {
                println!("已删除密钥文件条目");
                if c.api_key_source == Some("env") {
                    println!(
                        "注意: 环境变量 {} 还在，`explain` 仍然能拿到 Key；\
                         要彻底停用请在 shell 里 unset 它。",
                        ai::API_KEY_ENV
                    );
                }
            }
            Ok(())
        }
        Err(e) => {
            if json {
                print_json(&AiConfigResultJson::err(format!("{e:#}")));
                std::process::exit(1);
            }
            Err(e)
        }
    }
}

fn cmd_ai_test(json: bool) -> Result<()> {
    let result = (|| -> Result<(ai::AiConfig, String)> {
        let cfg = ai::load_config().context("读取 ai.json 失败")?;
        let key = ai::get_api_key()?.ok_or(Error::AiNoKey)?;
        let reply = ai::test_connection(&cfg, &key)?;
        Ok((cfg, reply))
    })();
    match result {
        Ok((cfg, reply)) => {
            if json {
                print_json(&AiTestResultJson::ok(cfg.endpoint(), cfg.model, reply));
            } else {
                println!("{} 可用，模型 {} 回复: {reply}", cfg.endpoint(), cfg.model);
            }
            Ok(())
        }
        Err(e) => {
            if json {
                print_json(&AiTestResultJson::err(format!("{e:#}")));
                std::process::exit(1);
            }
            Err(e)
        }
    }
}

fn cmd_explain(service: &Service, name: &str, refresh: bool, lang: &str, json: bool) -> Result<()> {
    let result = (|| -> Result<(InsightJson, bool)> {
        let n = task_name(name)?;
        let cfg = ai::load_config().context("读取 ai.json 失败")?;
        // Whether this call will pay for a model round trip is decided the
        // same way `Service::explain_task` decides it, and asked *before* the
        // call so that human mode can say "正在解读..." only when something
        // is actually about to take several seconds.
        let cached = !refresh && service.insight(&n)?.is_some();
        if !cached && !json {
            eprintln!("正在用 {} 解读 {name}……", cfg.model);
        }
        let insight = service.explain_task(&n, &cfg, ai::Lang::from_tag(lang), refresh)?;
        Ok((InsightJson::from_insight(&insight, cached), cached))
    })();
    match result {
        Ok((i, cached)) => {
            if json {
                print_json(&InsightResultJson::ok(i));
            } else {
                println!("任务:   {}", i.task);
                println!("模型:   {}", i.model);
                println!(
                    "生成于: {}{}",
                    fmt::local_time(i.created_at),
                    if cached {
                        "（已缓存，--refresh 重新解读）"
                    } else {
                        ""
                    }
                );
                println!();
                println!("{}", i.content);
            }
            Ok(())
        }
        Err(e) => {
            if json {
                print_json(&InsightResultJson::err(format!("{e:#}")));
                std::process::exit(1);
            }
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, b"runner").expect("write");
    }

    #[test]
    fn explicit_wins_and_is_not_probed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ghost = dir.path().join("nowhere").join(RUNNER_NAME);
        touch(&dir.path().join(RUNNER_NAME));
        assert_eq!(
            locate_runner_source(Some(&ghost), None, dir.path()),
            Some(ghost)
        );
    }

    #[test]
    fn the_env_var_beats_the_sibling_but_an_empty_one_does_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sibling = dir.path().join(RUNNER_NAME);
        touch(&sibling);
        let from_env = dir.path().join("elsewhere").join(RUNNER_NAME);
        assert_eq!(
            locate_runner_source(None, Some(&from_env), dir.path()),
            Some(from_env)
        );
        assert_eq!(
            locate_runner_source(None, Some(Path::new("")), dir.path()),
            Some(sibling)
        );
    }

    /// Homebrew's Formula, and the app bundle's `Contents/MacOS/`: the CLI
    /// and the runner are installed side by side.
    #[test]
    fn a_sibling_of_the_cli_is_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bin = dir.path().join("bin");
        let sibling = bin.join(RUNNER_NAME);
        touch(&sibling);
        assert_eq!(locate_runner_source(None, None, &bin), Some(sibling));
    }

    /// A keg that keeps the helper off `PATH`: `bin/launchkeeper` next to
    /// `libexec/launchkeeper-runner`.
    #[test]
    fn a_libexec_neighbour_of_the_cli_is_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).expect("mkdir");
        let helper = dir.path().join("libexec").join(RUNNER_NAME);
        touch(&helper);
        let found = locate_runner_source(None, None, &bin).expect("found");
        assert_eq!(
            found.canonicalize().expect("canonicalize"),
            helper.canonicalize().expect("canonicalize")
        );
    }

    #[test]
    fn nothing_installed_anywhere_is_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(locate_runner_source(None, None, dir.path()), None);
    }
}
