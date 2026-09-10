//! JSON shapes printed by `--json`. Kept separate from `main.rs` so the
//! wire contract is easy to scan and to keep stable for scripts/AI agents.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use launchkeeper_core::interpreters::{self, Interpreter};
use launchkeeper_core::{
    AdoptionPlan, ExternalAgent, Insight, InterpreterKind, JobStatus, Origin, Run, StopReason,
    Task, Trigger,
};
use serde::Serialize;

/// Full JSON view of a task, as returned by `show`, `list`, `set`, and every
/// mutating command's `{"ok": true, "task": ...}` envelope.
#[derive(Debug, Serialize)]
pub struct TaskJson {
    pub name: String,
    pub display_name: String,
    pub description: Option<String>,
    pub script_path: PathBuf,
    pub args: Vec<String>,
    /// The real script `interpreters::apply`/`detect_from_task` decomposed
    /// `script_path` + `args` into — for a `--raw` task this can differ a
    /// lot from `script_path` (e.g. `uv` vs `scripts/sync_cloud.py`).
    /// Falls back to `script_path`/`args` verbatim when `detect_from_task`
    /// can't parse them (an empty `script_path`, or a bare known
    /// interpreter given no script to run).
    pub script: PathBuf,
    pub script_args: Vec<String>,
    /// `python3 · 系统` / `uv run · uv 安装` / `直接执行`, from
    /// `interpreters::detect_from_task`. `None` only when that returns
    /// `None` (see `script` above).
    pub interpreter_label: Option<String>,
    pub working_dir: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
    /// Structured form of [`Trigger`], e.g. `{"kind":"calendar","entries":[...]}`.
    pub trigger: Trigger,
    /// The same human-readable string `show`/`list` print, e.g. `每天 21:00`.
    pub trigger_text: String,
    pub keep_alive: bool,
    /// True for a `{"kind":"manual"}` task, i.e. one driven by
    /// `start`/`stop`/`restart` rather than by a schedule.
    pub is_service: bool,
    pub timeout_secs: Option<u32>,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub notify_on_fail: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Database says the task should be loaded (a plist exists and launchd
    /// knows the label). See [`launchkeeper_core::Service::is_enabled`].
    pub enabled: bool,
    /// launchd actually reports the label as loaded right now.
    pub loaded: bool,
    pub pid: Option<u32>,
    /// Seconds since the currently running run started; `null` unless launchd
    /// reports a pid *and* the newest run has not finished.
    pub uptime_secs: Option<i64>,
    pub last_run: Option<RunJson>,
}

/// How long the run that is still in flight has been going, in seconds.
/// `None` when launchd reports no pid or the newest run already finished.
pub fn uptime_secs(job: Option<&JobStatus>, last_run: Option<&Run>) -> Option<i64> {
    job?.pid?;
    let run = last_run?;
    if run.finished_at.is_some() {
        return None;
    }
    Some((Utc::now() - run.started_at).num_seconds().max(0))
}

impl TaskJson {
    pub fn from_task(
        t: &Task,
        enabled: bool,
        job: Option<&JobStatus>,
        last_run: Option<&Run>,
    ) -> TaskJson {
        let detected = interpreters::detect_from_task(&t.script_path, &t.args);
        let (script, script_args, interpreter_label) = match &detected {
            Some(d) => (
                d.script.clone(),
                d.args.clone(),
                Some(d.interpreter.label()),
            ),
            None => (t.script_path.clone(), t.args.clone(), None),
        };
        TaskJson {
            name: t.name.as_str().to_string(),
            display_name: t.display_name.clone(),
            description: t.description.clone(),
            script_path: t.script_path.clone(),
            args: t.args.clone(),
            script,
            script_args,
            interpreter_label,
            working_dir: t.working_dir.clone(),
            env: t.env.clone(),
            trigger: t.trigger.clone(),
            trigger_text: t.trigger.describe(),
            keep_alive: t.keep_alive,
            is_service: t.is_service(),
            timeout_secs: t.timeout_secs,
            tags: t.tags.clone(),
            favorite: t.favorite,
            notify_on_fail: t.notify_on_fail,
            created_at: t.created_at,
            updated_at: t.updated_at,
            enabled,
            loaded: job.is_some(),
            pid: job.and_then(|j| j.pid),
            uptime_secs: uptime_secs(job, last_run),
            last_run: last_run.map(RunJson::from_run),
        }
    }
}

/// JSON view of one recorded run, used by `runs --json`, `show --json`
/// (`last_run`) and `logs --json` (`run_id` refers to the same id space).
#[derive(Debug, Serialize)]
pub struct RunJson {
    pub id: i64,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub exit_code: Option<i32>,
    /// `None` while the run has not finished yet.
    pub duration_ms: Option<i64>,
    pub trigger_kind: &'static str,
    /// Why the run ended: `"exited"`, `"stopped"` (a stop signal) or
    /// `"timeout"`. `null` while running and on rows written before schema v2.
    pub stop_reason: Option<&'static str>,
    pub running: bool,
    pub stdout_path: PathBuf,
    pub stderr_path: PathBuf,
}

impl RunJson {
    pub fn from_run(r: &Run) -> RunJson {
        RunJson {
            id: r.id.0,
            started_at: r.started_at,
            finished_at: r.finished_at,
            exit_code: r.exit_code,
            duration_ms: r
                .finished_at
                .map(|f| (f - r.started_at).num_milliseconds().max(0)),
            trigger_kind: r.trigger_kind.as_str(),
            stop_reason: r.stop_reason.map(StopReason::as_str),
            running: r.finished_at.is_none(),
            stdout_path: r.stdout_path.clone(),
            stderr_path: r.stderr_path.clone(),
        }
    }
}

/// One row of `status --json`.
#[derive(Debug, Serialize)]
pub struct StatusJson {
    pub name: String,
    pub enabled: bool,
    pub loaded: bool,
    /// True for a `{"kind":"manual"}` task.
    pub is_service: bool,
    pub pid: Option<u32>,
    /// Seconds the current run has been going; `null` when nothing is
    /// running. Mostly interesting for services, where it is the service's
    /// uptime.
    pub uptime_secs: Option<i64>,
    pub state: Option<String>,
    pub last_run: Option<RunJson>,
}

/// `logs --json` output.
#[derive(Debug, Serialize)]
pub struct LogsJson {
    pub run_id: i64,
    pub stream: &'static str,
    pub path: PathBuf,
    pub content: String,
    pub truncated: bool,
}

/// The envelope every mutating command prints in `--json` mode.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ResultJson {
    Ok { ok: bool, task: Box<TaskJson> },
    Err { ok: bool, error: String },
}

impl ResultJson {
    pub fn ok(task: TaskJson) -> ResultJson {
        ResultJson::Ok {
            ok: true,
            task: Box::new(task),
        }
    }

    pub fn err(message: impl std::fmt::Display) -> ResultJson {
        ResultJson::Err {
            ok: false,
            error: message.to_string(),
        }
    }
}

/// `config data-dir` (no path given) output: a plain read, no `ok` envelope,
/// same convention as `show`.
#[derive(Debug, Serialize)]
pub struct ConfigDataDirJson {
    pub data_dir: PathBuf,
    /// `"env"` | `"config_file"` | `"default"` — see
    /// [`launchkeeper_core::paths::DataDirSource::as_str`].
    pub source: &'static str,
    pub config_file: PathBuf,
}

/// `config data-dir <path>` (writing) output: the usual `ok`/`error`
/// mutation envelope.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ConfigDataDirResultJson {
    Ok {
        ok: bool,
        data_dir: PathBuf,
        config_file: PathBuf,
        /// Present only when the write, though successful, will have no
        /// effect in this environment because `LAUNCHKEEPER_DATA_DIR` is
        /// set and outranks the file. The human output says the same thing
        /// in prose; a script or an AI agent should not have to parse that
        /// to notice.
        #[serde(skip_serializing_if = "Option::is_none")]
        warning: Option<String>,
    },
    Err {
        ok: bool,
        error: String,
    },
}

impl ConfigDataDirResultJson {
    pub fn ok(
        data_dir: PathBuf,
        config_file: PathBuf,
        warning: Option<String>,
    ) -> ConfigDataDirResultJson {
        ConfigDataDirResultJson::Ok {
            ok: true,
            data_dir,
            config_file,
            warning,
        }
    }

    pub fn err(message: impl std::fmt::Display) -> ConfigDataDirResultJson {
        ConfigDataDirResultJson::Err {
            ok: false,
            error: message.to_string(),
        }
    }
}

/// One candidate from `interpreters --json`: the raw [`Interpreter`] fields
/// plus the two derived strings ([`Interpreter::id`]/[`Interpreter::label`])
/// so a script or AI agent never has to reimplement that formatting itself.
#[derive(Debug, Serialize)]
pub struct InterpreterJson {
    pub id: String,
    pub kind: InterpreterKind,
    pub program: PathBuf,
    pub prefix_args: Vec<String>,
    pub version: Option<String>,
    pub origin: Origin,
    pub recommended: bool,
    pub reason: Option<String>,
    pub label: String,
}

impl InterpreterJson {
    pub fn from_interpreter(i: &Interpreter) -> InterpreterJson {
        InterpreterJson {
            id: i.id(),
            kind: i.kind,
            program: i.program.clone(),
            prefix_args: i.prefix_args.clone(),
            version: i.version.clone(),
            origin: i.origin,
            recommended: i.recommended,
            reason: i.reason.clone(),
            label: i.label(),
        }
    }
}

/// One LaunchAgent Launchkeeper did not create, as printed by `agents`.
///
/// A read, so no `ok` envelope — same convention as `show`/`list`.
#[derive(Debug, Serialize)]
pub struct AgentJson {
    /// The launchd label. This is also what `adopt`, and the external
    /// fallback of `enable`/`disable`/`run`, take as their argument.
    pub label: String,
    pub path: PathBuf,
    pub program: Option<PathBuf>,
    pub args: Vec<String>,
    /// `program` and `args` joined for display.
    pub command: String,
    pub working_dir: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
    /// The trigger translated into Launchkeeper's model; `null` when the
    /// plist's scheduling keys have no equivalent (`adoptable` is then false
    /// and `reason` says so).
    pub trigger: Option<Trigger>,
    /// The same human string `agents` prints, e.g. `每天 21:00`.
    pub trigger_text: String,
    pub keep_alive: bool,
    pub loaded: bool,
    pub pid: Option<u32>,
    pub last_exit: Option<i32>,
    /// True when `adopt <label>` would work.
    pub adoptable: bool,
    /// Why not, when `adoptable` is false; `null` otherwise.
    pub reason: Option<String>,
    /// The plist is already driven by Launchkeeper (an adopted task, or one
    /// of its own `com.launchkeeper.*` labels).
    pub managed_by_launchkeeper: bool,
}

impl AgentJson {
    pub fn from_agent(a: &ExternalAgent) -> AgentJson {
        AgentJson {
            label: a.label.clone(),
            path: a.path.clone(),
            program: a.program.clone(),
            args: a.args.clone(),
            command: a.command(),
            working_dir: a.working_dir.clone(),
            env: a.env.clone(),
            trigger: a.trigger.clone(),
            trigger_text: a.trigger_text(),
            keep_alive: a.keep_alive,
            loaded: a.loaded,
            pid: a.pid,
            last_exit: a.last_exit,
            adoptable: a.adoptable.is_yes(),
            reason: a.adoptable.reason().map(str::to_string),
            managed_by_launchkeeper: a.managed_by_launchkeeper,
        }
    }
}

/// One plist key in an [`AdoptPlanJson`] diff group.
#[derive(Debug, Serialize)]
pub struct PlistChangeJson {
    pub key: String,
    /// The value as a single line; nested arrays/dicts are flattened.
    pub value: String,
}

fn changes(pairs: &[(String, String)]) -> Vec<PlistChangeJson> {
    pairs
        .iter()
        .map(|(key, value)| PlistChangeJson {
            key: key.clone(),
            value: value.clone(),
        })
        .collect()
}

/// What `adopt --dry-run` prints: the exact key-by-key rewrite adoption
/// would apply, plus the task it would store.
#[derive(Debug, Serialize)]
pub struct AdoptPlanJson {
    /// The label, which adoption does not change.
    pub label: String,
    pub path: PathBuf,
    /// Where the original bytes get copied first.
    pub backup_path: PathBuf,
    /// Keys the rewrite drops, with their current values. A key whose value
    /// merely changes appears here with the old value and in `added` with
    /// the new one.
    pub removed: Vec<PlistChangeJson>,
    /// Keys the rewrite adds or changes, with their new values.
    pub added: Vec<PlistChangeJson>,
    /// Keys that come through untouched.
    pub kept: Vec<PlistChangeJson>,
    /// The task row adoption would insert.
    pub task: TaskJson,
}

impl AdoptPlanJson {
    pub fn new(plan: &AdoptionPlan, task: TaskJson) -> AdoptPlanJson {
        AdoptPlanJson {
            label: plan.label.clone(),
            path: plan.path.clone(),
            backup_path: plan.backup_path.clone(),
            removed: changes(&plan.removed),
            added: changes(&plan.added),
            kept: changes(&plan.kept),
            task,
        }
    }
}

/// `adopt --dry-run --json`: the usual `ok` envelope around a plan rather
/// than a task, because nothing was mutated.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum AdoptPlanResultJson {
    Ok {
        ok: bool,
        dry_run: bool,
        plan: Box<AdoptPlanJson>,
    },
    Err {
        ok: bool,
        error: String,
    },
}

impl AdoptPlanResultJson {
    pub fn ok(plan: AdoptPlanJson) -> AdoptPlanResultJson {
        AdoptPlanResultJson::Ok {
            ok: true,
            dry_run: true,
            plan: Box::new(plan),
        }
    }

    pub fn err(message: impl std::fmt::Display) -> AdoptPlanResultJson {
        AdoptPlanResultJson::Err {
            ok: false,
            error: message.to_string(),
        }
    }
}

/// The mutation envelope for `enable`/`disable`/`run` when the argument
/// turned out to be an *external* label rather than a task: same `ok` shape,
/// but carrying an `agent` instead of a `task`.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum AgentResultJson {
    Ok { ok: bool, agent: Box<AgentJson> },
    Err { ok: bool, error: String },
}

impl AgentResultJson {
    pub fn ok(agent: AgentJson) -> AgentResultJson {
        AgentResultJson::Ok {
            ok: true,
            agent: Box::new(agent),
        }
    }

    pub fn err(message: impl std::fmt::Display) -> AgentResultJson {
        AgentResultJson::Err {
            ok: false,
            error: message.to_string(),
        }
    }
}

// ---- M4 §1: AI task insight ----------------------------------------------

/// `explain --json`: one stored (or freshly generated) explanation. A plain
/// read shape, no `ok` envelope — same convention as `show`.
///
/// `cached` says whether this call reused what was already stored (`true`) or
/// actually paid for a model round trip (`false`). `prompt_hash` identifies
/// the exact prompt that produced `content`, so a caller can tell two
/// explanations of the same task apart without diffing their prose.
#[derive(Debug, Serialize)]
pub struct InsightJson {
    pub task: String,
    pub created_at: DateTime<Utc>,
    pub model: String,
    pub prompt_hash: String,
    pub content: String,
    pub cached: bool,
}

impl InsightJson {
    pub fn from_insight(i: &Insight, cached: bool) -> InsightJson {
        InsightJson {
            task: i.task_name.as_str().to_string(),
            created_at: i.created_at,
            model: i.model.clone(),
            prompt_hash: i.prompt_hash.clone(),
            content: i.content.clone(),
            cached,
        }
    }
}

/// `explain --json`'s error half. `explain` is a read in the "no state
/// changes on failure" sense but a mutation in the "this costs money and
/// writes a row" sense, so a failure still has to be machine-readable —
/// hence an untagged enum with the plain object on success rather than a
/// bare object.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum InsightResultJson {
    Ok(Box<InsightJson>),
    Err { ok: bool, error: String },
}

impl InsightResultJson {
    pub fn ok(insight: InsightJson) -> InsightResultJson {
        InsightResultJson::Ok(Box::new(insight))
    }

    pub fn err(message: impl std::fmt::Display) -> InsightResultJson {
        InsightResultJson::Err {
            ok: false,
            error: message.to_string(),
        }
    }
}

/// `ai config` (no flags) output: the non-secret configuration plus whether
/// a key can be found and where it came from. The key itself is never in any
/// JSON this CLI prints.
#[derive(Debug, Serialize)]
pub struct AiConfigJson {
    /// `"anthropic"` | `"openai_compatible"`.
    pub provider: &'static str,
    /// What the user set, or `null` when the provider default applies.
    pub base_url: Option<String>,
    /// The URL actually used, provider default filled in.
    pub effective_base_url: String,
    /// The endpoint one completion request goes to.
    pub endpoint: String,
    pub model: String,
    /// Where `ai.json` lives.
    pub config_file: PathBuf,
    /// True when a key is available from either source.
    pub has_api_key: bool,
    /// `"env"` | `"file"` | `null` — which source produced it.
    pub api_key_source: Option<&'static str>,
}

/// `ai config --provider/...` and `ai key set|clear`: the usual mutation
/// envelope, carrying the config as it now stands.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum AiConfigResultJson {
    Ok { ok: bool, config: Box<AiConfigJson> },
    Err { ok: bool, error: String },
}

impl AiConfigResultJson {
    pub fn ok(config: AiConfigJson) -> AiConfigResultJson {
        AiConfigResultJson::Ok {
            ok: true,
            config: Box::new(config),
        }
    }

    pub fn err(message: impl std::fmt::Display) -> AiConfigResultJson {
        AiConfigResultJson::Err {
            ok: false,
            error: message.to_string(),
        }
    }
}

/// `ai test`: did the configured endpoint answer, and with what.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum AiTestResultJson {
    Ok {
        ok: bool,
        /// The endpoint that answered.
        endpoint: String,
        model: String,
        /// The model's one-line reply, verbatim.
        reply: String,
    },
    Err {
        ok: bool,
        error: String,
    },
}

impl AiTestResultJson {
    pub fn ok(endpoint: String, model: String, reply: String) -> AiTestResultJson {
        AiTestResultJson::Ok {
            ok: true,
            endpoint,
            model,
            reply,
        }
    }

    pub fn err(message: impl std::fmt::Display) -> AiTestResultJson {
        AiTestResultJson::Err {
            ok: false,
            error: message.to_string(),
        }
    }
}
