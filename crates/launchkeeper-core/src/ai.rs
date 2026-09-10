//! AI task insight (PRD M4 item 1, `docs/M4-design.md` §1).
//!
//! Everything about talking to a model lives here: the non-secret
//! configuration (`<data_dir>/ai.json`, shared by the CLI and the app), the
//! API key (`<data_dir>/ai-key`, with an environment override), the prompt that
//! describes a task to a model, and the two provider wire formats
//! (Anthropic Messages and OpenAI chat completions).
//!
//! Three rules shape this module:
//!
//! - **Environment variable *values* never leave the machine.** The prompt
//!   lists the variable *names* a task sets and nothing else — a task whose
//!   environment carries `AWS_SECRET_ACCESS_KEY` or a database URL must not
//!   have it read out to a third party because the user clicked 解读. See
//!   [`redact_env`].
//! - **The log tail is bounded.** [`MAX_LOG_BYTES`] of it, cut on a character
//!   boundary and from the *end* (the interesting part of a failing log is
//!   its last lines, not its first).
//! - **The key is never an argument.** [`explain`] takes it as a parameter
//!   from [`get_api_key`]; nothing in this crate puts it on a command line,
//!   in a plist, or in the database.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, SubsecRound, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::paths;
use crate::store::{Run, StopReason};
use crate::task::{Task, TaskName};

/// Environment variable that overrides the keychain, in both directions:
/// when it is set and non-empty, [`get_api_key`] answers with it and the
/// keychain is not touched at all.
///
/// This is what the tests use (a keychain read on macOS can pop an
/// authorization dialog, which no test may ever depend on) and what a
/// headless or CI invocation of the CLI uses.
pub const API_KEY_ENV: &str = "LAUNCHKEEPER_AI_API_KEY";

/// How much of the newest run's log is sent. 16 KiB is comfortably more than
/// a stack trace and comfortably less than a chatty rsync transcript.
pub const MAX_LOG_BYTES: usize = 16 * 1024;

/// How many runs of history the prompt describes.
pub const RUNS_IN_PROMPT: usize = 10;

/// Request timeout for one model call.
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(60);

/// Upper bound on the model's answer.
const MAX_TOKENS: u32 = 1500;

/// The Anthropic API version header every request carries.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Default base URL for [`AiProvider::Anthropic`].
pub const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";

/// Default base URL for [`AiProvider::OpenAiCompatible`].
pub const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

/// Default model: the current Sonnet, which is fast and cheap enough that
/// explaining a task is not something the user has to think about the cost
/// of.
pub const DEFAULT_MODEL: &str = "claude-sonnet-5";

/// Which wire format to speak.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum AiProvider {
    /// Anthropic's Messages API (`POST <base>/v1/messages`).
    #[default]
    Anthropic,
    /// Anything that speaks OpenAI chat completions
    /// (`POST <base>/chat/completions`): OpenAI itself, Ollama, vLLM,
    /// OpenRouter, a corporate gateway.
    ///
    /// Renamed explicitly: `rename_all = "snake_case"` would spell this
    /// `open_ai_compatible`, and the wire value has to match what
    /// [`AiProvider::as_str`] writes into `ai.json`, `--json` and the CLI's
    /// `--provider`.
    #[serde(rename = "openai_compatible")]
    OpenAiCompatible,
}

impl AiProvider {
    /// The stable string used in `ai.json`, in `--json` output and as the
    /// CLI's `--provider` value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            AiProvider::Anthropic => "anthropic",
            AiProvider::OpenAiCompatible => "openai_compatible",
        }
    }

    /// Parses [`AiProvider::as_str`], plus the friendlier spellings a person
    /// types (`openai`, `openai-compatible`).
    ///
    /// # Errors
    /// [`Error::Ai`] for anything else.
    pub fn parse(s: &str) -> Result<AiProvider> {
        match s.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "anthropic" => Ok(AiProvider::Anthropic),
            "openai" | "openai_compatible" => Ok(AiProvider::OpenAiCompatible),
            other => Err(Error::Ai(format!(
                "未知的 AI 提供方 {other:?}，可选: anthropic / openai_compatible"
            ))),
        }
    }

    /// The base URL to use when [`AiConfig::base_url`] is not set.
    #[must_use]
    pub fn default_base_url(self) -> &'static str {
        match self {
            AiProvider::Anthropic => ANTHROPIC_BASE_URL,
            AiProvider::OpenAiCompatible => OPENAI_BASE_URL,
        }
    }
}

impl fmt::Display for AiProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The non-secret half of the AI configuration: `<data_dir>/ai.json`.
///
/// Deliberately *not* in the app's `settings.json`: the CLI has no business
/// reading a file whose other half is about tray icons and notification
/// checkboxes, and the app has no business owning a setting the CLI must be
/// able to change. One small file in the shared data directory, with the same
/// `LAUNCHKEEPER_DATA_DIR` override everything else follows.
///
/// The API key is **not** here — see [`get_api_key`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(default)]
pub struct AiConfig {
    /// Which wire format the endpoint speaks.
    pub provider: AiProvider,
    /// Endpoint root. `None` means [`AiProvider::default_base_url`]. A
    /// trailing slash is tolerated.
    pub base_url: Option<String>,
    /// Model id, passed through verbatim.
    pub model: String,
}

impl Default for AiConfig {
    fn default() -> AiConfig {
        AiConfig {
            provider: AiProvider::Anthropic,
            base_url: None,
            model: DEFAULT_MODEL.to_string(),
        }
    }
}

impl AiConfig {
    /// The base URL actually used, without its trailing slash.
    #[must_use]
    pub fn effective_base_url(&self) -> String {
        let raw = self
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| self.provider.default_base_url());
        raw.trim_end_matches('/').to_string()
    }

    /// The URL one completion request goes to.
    #[must_use]
    pub fn endpoint(&self) -> String {
        let base = self.effective_base_url();
        match self.provider {
            AiProvider::Anthropic => format!("{base}/v1/messages"),
            AiProvider::OpenAiCompatible => format!("{base}/chat/completions"),
        }
    }
}

/// `<data_dir>/ai.json`.
///
/// # Errors
/// [`Error::NoHomeDir`] when the data directory cannot be resolved.
pub fn config_path() -> Result<PathBuf> {
    Ok(paths::data_dir()?.join("ai.json"))
}

/// Reads `<data_dir>/ai.json`.
///
/// A missing file is [`AiConfig::default`], not an error — that is every
/// installation until the user first picks a provider. A file that exists but
/// does not parse falls back to the default with a line on stderr, the same
/// rule the app's `settings.json` follows: a hand-edited config must not be
/// able to make 解读 impossible to reach.
///
/// # Errors
/// Read failures that are not "file does not exist", and
/// [`Error::NoHomeDir`].
pub fn load_config() -> Result<AiConfig> {
    let path = config_path()?;
    match std::fs::read_to_string(&path) {
        Ok(raw) => Ok(serde_json::from_str(&raw).unwrap_or_else(|e| {
            eprintln!("ai.json 解析失败（{}），使用默认配置: {e}", path.display());
            AiConfig::default()
        })),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AiConfig::default()),
        Err(e) => Err(Error::io(&path, e)),
    }
}

/// Writes `<data_dir>/ai.json` atomically (temp file in the same directory,
/// `sync_all`, `rename`), so a crash mid-write leaves the old file rather
/// than half of a new one.
///
/// # Errors
/// I/O errors, and [`Error::NoHomeDir`].
pub fn save_config(cfg: &AiConfig) -> Result<()> {
    let path = config_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    }
    let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    let json = serde_json::to_string_pretty(cfg)?;
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp).map_err(|e| Error::io(&tmp, e))?;
        f.write_all(json.as_bytes())
            .map_err(|e| Error::io(&tmp, e))?;
        f.sync_all().map_err(|e| Error::io(&tmp, e))?;
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(Error::io(&path, e));
    }
    Ok(())
}

// ---- the API key ---------------------------------------------------------

/// `<data_dir>/ai-key`: the API key, one line, mode 0600.
///
/// The macOS keychain was the first design, but an unsigned binary changes
/// identity on every rebuild and the keychain then wants a consent dialog
/// (`errSecInteractionNotAllowed` when it cannot show one). Until the app
/// is signed a private file in the user-controlled data dir is the reliable
/// store; the env override below remains the way tests and CI supply a key.
pub fn key_path() -> Result<PathBuf> {
    Ok(paths::data_dir()?.join("ai-key"))
}

/// The API key: `$LAUNCHKEEPER_AI_API_KEY` if set and non-empty, otherwise
/// the contents of [`key_path`], otherwise `None`.
///
/// The environment variable wins on purpose: it is a read-side override for
/// this process and never written anywhere.
///
/// # Errors
/// [`Error::Io`] when the key file exists but cannot be read. A *missing*
/// file is `Ok(None)`, not an error.
pub fn get_api_key() -> Result<Option<String>> {
    if let Ok(v) = std::env::var(API_KEY_ENV)
        && !v.trim().is_empty()
    {
        return Ok(Some(v.trim().to_string()));
    }
    let path = key_path()?;
    match std::fs::read_to_string(&path) {
        Ok(k) if k.trim().is_empty() => Ok(None),
        Ok(k) => Ok(Some(k.trim().to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(&path, e)),
    }
}

/// Stores the API key in [`key_path`] with mode 0600 (written to a sibling
/// temp file first, then renamed into place).
///
/// # Errors
/// [`Error::Ai`] when the key is blank; [`Error::Io`] when the file cannot
/// be written.
pub fn set_api_key(key: &str) -> Result<()> {
    let key = key.trim();
    if key.is_empty() {
        return Err(Error::Ai("API Key 不能为空".into()));
    }
    let path = key_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| Error::io(&tmp, e))?;
        f.write_all(key.as_bytes())
            .and_then(|()| f.write_all(b"\n"))
            .and_then(|()| f.sync_all())
            .map_err(|e| Error::io(&tmp, e))?;
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(Error::io(&path, e));
    }
    Ok(())
}

/// Deletes the key file. Deleting one that is not there is not an error.
///
/// # Errors
/// [`Error::Io`] when the file exists but cannot be removed.
pub fn clear_api_key() -> Result<()> {
    let path = key_path()?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(&path, e)),
    }
}

/// True when a key can be found, without returning it. What the settings
/// panel's "已配置 / 未配置" line asks.
///
/// # Errors
/// The same as [`get_api_key`].
pub fn has_api_key() -> Result<bool> {
    Ok(get_api_key()?.is_some())
}

// ---- the prompt ----------------------------------------------------------

/// Which language the model should answer in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    /// 简体中文.
    #[default]
    ZhCn,
    /// English.
    En,
}

impl Lang {
    /// The BCP 47-ish tag, as the app's settings and the CLI spell it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Lang::ZhCn => "zh-CN",
            Lang::En => "en",
        }
    }

    /// Anything starting with `zh` is Chinese; everything else — including a
    /// tag this build does not know — is English, the same fallback the
    /// app's `resolveLocale` uses. Never fails: a prompt without a language
    /// is worse than a prompt in the wrong one.
    #[must_use]
    pub fn from_tag(tag: &str) -> Lang {
        if tag.trim().to_ascii_lowercase().starts_with("zh") {
            Lang::ZhCn
        } else {
            Lang::En
        }
    }
}

/// Everything [`build_prompt`] needs about one task, gathered by
/// [`crate::Service::explain_task`].
#[derive(Debug, Clone)]
pub struct ExplainInput {
    /// The task itself.
    pub task: Task,
    /// The newest [`RUNS_IN_PROMPT`] runs, newest first.
    pub runs: Vec<Run>,
    /// The tail of the newest run's captured output, already concatenated
    /// (stdout then stderr, each labelled). Truncated again to
    /// [`MAX_LOG_BYTES`] by [`build_prompt`], so an over-long value here is
    /// safe rather than a leak.
    pub log_tail: String,
    /// Language the answer must be in.
    pub lang: Lang,
}

/// A system + user prompt pair, and the hash that identifies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    /// System prompt: who the model is and what it must produce.
    pub system: String,
    /// User prompt: the task, its history and its log tail.
    pub user: String,
}

impl Prompt {
    /// `sha256(system + "\n" + user)`, lowercase hex.
    ///
    /// Stored with the insight so a caller can tell whether the stored answer
    /// was produced for the task as it is now.
    #[must_use]
    pub fn hash(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.system.as_bytes());
        h.update(b"\n");
        h.update(self.user.as_bytes());
        format!("{:x}", h.finalize())
    }
}

/// The environment as the prompt is allowed to see it: **names only**.
///
/// A task's environment is where API tokens, database URLs and passwords
/// live. The model needs to know that a task sets `AWS_PROFILE` — that is
/// often the answer to "why does it fail from launchd but not from my
/// shell" — and never needs to know what it is set to.
#[must_use]
pub fn redact_env(task: &Task) -> Vec<String> {
    task.env.keys().cloned().collect()
}

/// Keeps the **last** `max` bytes of `s`, cut back to a `char` boundary, and
/// prefixes a marker line when anything was dropped.
///
/// From the end, because the last lines of a failing log are the ones that
/// say why. On a boundary, because the result goes into a JSON string.
#[must_use]
pub fn truncate_tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = s.len() - max;
    while cut < s.len() && !s.is_char_boundary(cut) {
        cut += 1;
    }
    format!(
        "[... 前面 {} 字节已省略 / {} earlier bytes omitted ...]\n{}",
        cut,
        cut,
        &s[cut..]
    )
}

fn describe_run(r: &Run) -> String {
    let started = r.started_at.to_rfc3339_opts(SecondsFormat::Secs, true);
    let outcome = if r.finished_at.is_none() {
        "运行中 / running".to_string()
    } else {
        let code = match r.exit_code {
            Some(c) => format!("exit {c}"),
            None => "killed by signal".to_string(),
        };
        let why = r.stop_reason.map_or("-", StopReason::as_str);
        format!("{code}, stop_reason={why}")
    };
    let duration = match (r.finished_at, r.started_at) {
        (Some(f), s) => format!("{}s", (f - s).num_seconds().max(0)),
        _ => "-".to_string(),
    };
    format!(
        "- {started} | {outcome} | duration={duration} | trigger={}",
        r.trigger_kind.as_str()
    )
}

/// Builds the prompt describing `input`'s task to a model.
///
/// The system half fixes the role, the four questions to answer and the
/// output language; the user half is the data. Both halves are plain text —
/// no tool definitions, no JSON schema — because the answer is rendered as
/// Markdown and nothing parses it.
///
/// Redaction and truncation both happen here rather than at the call site, so
/// there is exactly one place where "what leaves the machine" is decided:
/// [`redact_env`] drops every environment *value*, and [`truncate_tail`]
/// bounds the log at [`MAX_LOG_BYTES`].
#[must_use]
pub fn build_prompt(input: &ExplainInput) -> Prompt {
    let task = &input.task;
    let system = match input.lang {
        Lang::ZhCn => concat!(
            "你是 Launchkeeper 的任务分析助手。Launchkeeper 是 macOS 上管理 launchd ",
            "定时任务的工具：每个任务由 launchd 按计划拉起 launchkeeper-runner，",
            "由 runner 执行用户脚本并记录退出码和日志。\n\n",
            "根据用户给出的任务定义、最近的运行历史和最新一次运行的日志尾部，回答四件事：\n",
            "1. 这个任务在做什么（从脚本路径、参数、解释器、工作目录和触发规则推断）；\n",
            "2. 它最近是否健康；\n",
            "3. 如果失败了，最可能的原因是什么；\n",
            "4. 具体建议，优先给可以直接执行的动作。\n\n",
            "用简体中文回答，用 Markdown 组织，控制在 400 字以内。",
            "只根据给出的信息推断，不要编造日志里没有的内容；",
            "信息不足时直接说明还需要看什么。\n",
            "注意：出于隐私，环境变量只给出了名字，没有值——不要猜测或索要它们的值。"
        ),
        Lang::En => concat!(
            "You are Launchkeeper's task analyst. Launchkeeper manages launchd ",
            "scheduled jobs on macOS: launchd starts launchkeeper-runner on a ",
            "schedule, and the runner executes the user's script and records its ",
            "exit code and logs.\n\n",
            "Given the task definition, its recent run history and the tail of the ",
            "newest run's log, answer four things:\n",
            "1. what this task does (infer it from the script path, arguments, ",
            "interpreter, working directory and trigger);\n",
            "2. whether it has been healthy lately;\n",
            "3. if it failed, the most likely cause;\n",
            "4. concrete suggestions, favouring actions the user can take directly.\n\n",
            "Answer in English, in Markdown, under 300 words. Reason only from what ",
            "you were given — do not invent log content — and say plainly what else ",
            "you would need to see when the information is not enough.\n",
            "Note: for privacy, environment variables are given by name only, with no ",
            "values. Do not guess or ask for their values."
        ),
    };

    let env_names = redact_env(task);
    let env_line = if env_names.is_empty() {
        "(none)".to_string()
    } else {
        format!("{} (names only, values redacted)", env_names.join(", "))
    };
    let runs: Vec<String> = input
        .runs
        .iter()
        .take(RUNS_IN_PROMPT)
        .map(describe_run)
        .collect();
    let runs_block = if runs.is_empty() {
        "(no runs recorded yet)".to_string()
    } else {
        runs.join("\n")
    };
    let log = truncate_tail(&input.log_tail, MAX_LOG_BYTES);
    let log_block = if log.trim().is_empty() {
        "(empty)".to_string()
    } else {
        log
    };

    let user = format!(
        "## Task\n\
         name: {name}\n\
         display_name: {display}\n\
         description: {desc}\n\
         label: {label}\n\
         program: {program}\n\
         args: {args:?}\n\
         working_dir: {cwd}\n\
         env: {env_line}\n\
         trigger: {trigger} ({trigger_text})\n\
         keep_alive: {keep_alive}\n\
         is_service: {is_service}\n\
         timeout_secs: {timeout}\n\
         tags: {tags:?}\n\
         adopted: {adopted}\n\n\
         ## Recent runs (newest first, at most {limit})\n{runs_block}\n\n\
         ## Newest run log tail (at most {max_log} bytes)\n```\n{log_block}\n```\n",
        name = task.name.as_str(),
        display = task.display_name,
        desc = task.description.as_deref().unwrap_or("(none)"),
        label = task.label(),
        program = task.script_path.display(),
        args = task.args,
        cwd = task.working_dir.as_ref().map_or_else(
            || "(default: $HOME)".to_string(),
            |p| p.display().to_string()
        ),
        trigger = serde_json::to_string(&task.trigger).unwrap_or_else(|_| "?".into()),
        trigger_text = task.trigger.describe(),
        keep_alive = task.keep_alive,
        is_service = task.is_service(),
        timeout = task
            .timeout_secs
            .map_or_else(|| "(none)".to_string(), |t| t.to_string()),
        tags = task.tags,
        adopted = task.adopted,
        limit = RUNS_IN_PROMPT,
        max_log = MAX_LOG_BYTES,
    );

    Prompt {
        system: system.to_string(),
        user,
    }
}

// ---- calling the model ---------------------------------------------------

/// One stored explanation. Mirrors the `ai_insights` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Insight {
    /// Task this explains.
    pub task_name: TaskName,
    /// When it was generated.
    pub created_at: DateTime<Utc>,
    /// Model id that produced it.
    pub model: String,
    /// [`Prompt::hash`] of the prompt that produced it.
    pub prompt_hash: String,
    /// The answer, Markdown.
    pub content: String,
}

fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| Error::Ai(format!("构造 HTTP 客户端失败: {e}")))
}

/// Digs a human-readable message out of a provider's error body.
///
/// Anthropic answers `{"error": {"type": ..., "message": ...}}`, OpenAI
/// `{"error": {"message": ...}}`, and a gateway in front of either may answer
/// with plain text or HTML. The body is returned verbatim (trimmed) when it
/// is not the JSON shape we know, because an unparsed message the user can
/// read beats a parsed one that says "unknown error".
fn error_message(body: &str) -> String {
    let trimmed = body.trim();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed)
        && let Some(msg) = v
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(serde_json::Value::as_str)
    {
        return msg.to_string();
    }
    if trimmed.is_empty() {
        "（响应体为空）".to_string()
    } else {
        trimmed.chars().take(500).collect()
    }
}

/// Extracts the assistant text from a successful response body.
fn parse_content(provider: AiProvider, body: &str) -> Result<String> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| Error::Ai(format!("无法解析模型响应（不是 JSON）: {e}")))?;
    let text = match provider {
        // Anthropic: content is a list of blocks; concatenate every `text`
        // one and ignore the rest (a `thinking` block, say), so a model that
        // returns more than plain text still yields its answer.
        AiProvider::Anthropic => {
            v.get("content")
                .and_then(serde_json::Value::as_array)
                .map(|blocks| {
                    blocks
                        .iter()
                        .filter(|b| {
                            b.get("type").and_then(serde_json::Value::as_str) == Some("text")
                        })
                        .filter_map(|b| b.get("text").and_then(serde_json::Value::as_str))
                        .collect::<Vec<_>>()
                        .join("")
                })
        }
        AiProvider::OpenAiCompatible => v
            .get("choices")
            .and_then(serde_json::Value::as_array)
            .and_then(|c| c.first())
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
    };
    match text {
        Some(t) if !t.trim().is_empty() => Ok(t),
        _ => Err(Error::Ai(format!(
            "模型响应里没有可用的正文: {}",
            body.chars().take(300).collect::<String>()
        ))),
    }
}

/// Sends one prompt and returns the raw answer text.
///
/// Shared by [`explain`] and [`test_connection`] so that both exercise the
/// exact same request shape, headers and error mapping — a 「测试连接」 that
/// succeeded while the real call would have failed is worse than no test
/// button at all.
///
/// # Errors
/// [`Error::AiHttp`] for a non-2xx response (with the status and the
/// provider's own message), [`Error::Ai`] for a transport failure or an
/// unparseable body.
pub fn complete(cfg: &AiConfig, api_key: &str, prompt: &Prompt, max_tokens: u32) -> Result<String> {
    if api_key.trim().is_empty() {
        return Err(Error::AiNoKey);
    }
    if cfg.model.trim().is_empty() {
        return Err(Error::Ai("没有配置模型名（ai config --model）".into()));
    }
    let url = cfg.endpoint();
    let http = client()?;
    let req = match cfg.provider {
        AiProvider::Anthropic => http
            .post(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&serde_json::json!({
                "model": cfg.model,
                "max_tokens": max_tokens,
                "system": prompt.system,
                "messages": [{"role": "user", "content": prompt.user}],
            })),
        AiProvider::OpenAiCompatible => {
            http.post(&url)
                .bearer_auth(api_key)
                .json(&serde_json::json!({
                    "model": cfg.model,
                    "max_tokens": max_tokens,
                    "messages": [
                        {"role": "system", "content": prompt.system},
                        {"role": "user", "content": prompt.user},
                    ],
                }))
        }
    };

    let resp = req
        .send()
        .map_err(|e| Error::Ai(format!("请求 {url} 失败: {e}")))?;
    let status = resp.status();
    let body = resp
        .text()
        .map_err(|e| Error::Ai(format!("读取响应体失败: {e}")))?;
    if !status.is_success() {
        return Err(Error::AiHttp {
            status: status.as_u16(),
            message: error_message(&body),
        });
    }
    parse_content(cfg.provider, &body)
}

/// Explains one task: builds the prompt, calls the model, and answers with
/// the [`Insight`] to store.
///
/// This function does **not** touch the database — [`crate::Service::explain_task`]
/// does the storing — so it stays a pure "prompt in, answer out" unit that a
/// test can point at a local mock server.
///
/// # Errors
/// [`Error::AiNoKey`], [`Error::AiHttp`], [`Error::Ai`].
pub fn explain(cfg: &AiConfig, api_key: &str, input: &ExplainInput) -> Result<Insight> {
    let prompt = build_prompt(input);
    let content = complete(cfg, api_key, &prompt, MAX_TOKENS)?;
    Ok(Insight {
        task_name: input.task.name.clone(),
        // Truncated to the millisecond the database stores (`store::ts`
        // writes RFC 3339 with `SecondsFormat::Millis`), so the value
        // returned to the caller is byte-for-byte the value a later
        // `get_insight` answers with — otherwise "the insight I just got"
        // and "the insight that is stored" differ in their microseconds and
        // every equality check downstream is subtly wrong.
        created_at: Utc::now().trunc_subsecs(3),
        model: cfg.model.clone(),
        prompt_hash: prompt.hash(),
        content,
    })
}

/// A one-line ping: asks the configured model to answer with a single word,
/// so that 「测试连接」 costs a handful of tokens rather than a full
/// explanation.
///
/// # Errors
/// The same as [`complete`].
pub fn test_connection(cfg: &AiConfig, api_key: &str) -> Result<String> {
    let prompt = Prompt {
        system: "You are a connectivity check. Reply with exactly one word: ok".to_string(),
        user: "ping".to_string(),
    };
    Ok(complete(cfg, api_key, &prompt, 16)?.trim().to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use chrono::TimeZone;

    use super::*;
    use crate::store::{RunId, TriggerKind};
    use crate::task::{CalendarEntry, Trigger};

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    fn sample_task() -> Task {
        let mut t = Task::new(
            TaskName::new("nightly-sync").unwrap(),
            "/usr/bin/python3",
            Trigger::Calendar {
                entries: vec![CalendarEntry::daily(21, 0)],
            },
        );
        t.args = vec!["/Users/me/sync.py".into(), "--once".into()];
        t.working_dir = Some(PathBuf::from("/Users/me/proj"));
        t.env = BTreeMap::from([
            (
                "AWS_SECRET_ACCESS_KEY".to_string(),
                "hunter2-super".to_string(),
            ),
            ("S3_URL".to_string(), "https://s3.internal:9000".to_string()),
        ]);
        t.tags = vec!["同步".into()];
        t
    }

    fn sample_run(id: i64, exit: Option<i32>) -> Run {
        Run {
            id: RunId(id),
            task_name: TaskName::new("nightly-sync").unwrap(),
            started_at: at(id * 60),
            finished_at: Some(at(id * 60 + 12)),
            exit_code: exit,
            pid: Some(4242),
            stdout_path: PathBuf::from("/tmp/a.out"),
            stderr_path: PathBuf::from("/tmp/a.err"),
            trigger_kind: TriggerKind::Scheduled,
            stop_reason: Some(StopReason::Exited),
        }
    }

    fn input(log: &str, lang: Lang) -> ExplainInput {
        ExplainInput {
            task: sample_task(),
            runs: (1..=12).map(|i| sample_run(i, Some(0))).collect(),
            log_tail: log.to_string(),
            lang,
        }
    }

    #[test]
    fn env_values_never_reach_the_prompt() {
        let p = build_prompt(&input("all good\n", Lang::ZhCn));
        let whole = format!("{}\n{}", p.system, p.user);
        // The names are there — they are often the answer to "why does it
        // work in my shell and not from launchd".
        assert!(whole.contains("AWS_SECRET_ACCESS_KEY"));
        assert!(whole.contains("S3_URL"));
        // The values are not, anywhere.
        assert!(
            !whole.contains("hunter2-super"),
            "环境变量的值不能出现在 prompt 里"
        );
        assert!(!whole.contains("s3.internal"));
        assert!(whole.contains("values redacted"));
    }

    #[test]
    fn redact_env_answers_names_only() {
        assert_eq!(
            redact_env(&sample_task()),
            vec!["AWS_SECRET_ACCESS_KEY".to_string(), "S3_URL".to_string()]
        );
    }

    #[test]
    fn the_log_tail_is_truncated_from_the_front_and_bounded() {
        // 40 KiB of log, with a unique marker at each end.
        let mut log = String::from("VERY-FIRST-LINE\n");
        log.push_str(&"x".repeat(40 * 1024));
        log.push_str("\nVERY-LAST-LINE");
        let p = build_prompt(&input(&log, Lang::En));

        assert!(
            p.user.contains("VERY-LAST-LINE"),
            "日志尾部必须保留：失败的原因在最后几行"
        );
        assert!(
            !p.user.contains("VERY-FIRST-LINE"),
            "超出上限的开头必须被截掉"
        );
        assert!(p.user.contains("earlier bytes omitted"));
        // The whole user prompt stays within the log budget plus the fixed
        // task/runs preamble, which is far under a kilobyte here.
        assert!(
            p.user.len() < MAX_LOG_BYTES + 4096,
            "user prompt 长度 {} 超出预期",
            p.user.len()
        );
    }

    #[test]
    fn truncate_tail_cuts_on_a_char_boundary() {
        // Every char is 3 bytes, so a naive byte cut would split one.
        let s = "中".repeat(100);
        let out = truncate_tail(&s, 50);
        // Round-trips as valid UTF-8 (it is a `String`, so this is really a
        // check that the function did not panic slicing mid-char) and kept
        // the end.
        assert!(out.ends_with('中'));
        assert!(out.contains("omitted"));
        // Under the limit, nothing is touched at all.
        assert_eq!(truncate_tail("short", 50), "short");
        assert_eq!(truncate_tail(&"a".repeat(50), 50), "a".repeat(50));
    }

    #[test]
    fn only_ten_runs_are_described() {
        let p = build_prompt(&input("", Lang::En));
        let lines = p
            .user
            .lines()
            .filter(|l| l.contains("stop_reason="))
            .count();
        assert_eq!(lines, RUNS_IN_PROMPT);
    }

    #[test]
    fn an_empty_log_and_no_runs_still_produce_a_prompt() {
        let mut i = input("", Lang::ZhCn);
        i.runs.clear();
        let p = build_prompt(&i);
        assert!(p.user.contains("(no runs recorded yet)"));
        assert!(p.user.contains("(empty)"));
    }

    #[test]
    fn language_selects_the_system_prompt() {
        let zh = build_prompt(&input("", Lang::ZhCn));
        let en = build_prompt(&input("", Lang::En));
        assert!(zh.system.contains("简体中文"));
        assert!(en.system.contains("Answer in English"));
        // Same data, different instructions — so the hash differs and a
        // language switch really does re-explain.
        assert_ne!(zh.hash(), en.hash());
    }

    #[test]
    fn the_hash_is_stable_and_content_dependent() {
        let a = build_prompt(&input("same", Lang::ZhCn));
        let b = build_prompt(&input("same", Lang::ZhCn));
        let c = build_prompt(&input("different", Lang::ZhCn));
        assert_eq!(a.hash(), b.hash());
        assert_ne!(a.hash(), c.hash());
        assert_eq!(a.hash().len(), 64);
        assert!(a.hash().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn lang_tags_fall_back_to_english() {
        assert_eq!(Lang::from_tag("zh-CN"), Lang::ZhCn);
        assert_eq!(Lang::from_tag("zh-Hans-CN"), Lang::ZhCn);
        assert_eq!(Lang::from_tag("en"), Lang::En);
        assert_eq!(Lang::from_tag("fr-FR"), Lang::En);
        assert_eq!(Lang::from_tag(""), Lang::En);
        assert_eq!(Lang::ZhCn.as_str(), "zh-CN");
    }

    #[test]
    fn endpoints_and_base_urls() {
        let a = AiConfig::default();
        assert_eq!(a.provider, AiProvider::Anthropic);
        assert_eq!(a.model, "claude-sonnet-5");
        assert_eq!(a.endpoint(), "https://api.anthropic.com/v1/messages");

        let o = AiConfig {
            provider: AiProvider::OpenAiCompatible,
            base_url: None,
            model: "gpt-4o-mini".into(),
        };
        assert_eq!(o.endpoint(), "https://api.openai.com/v1/chat/completions");

        // A trailing slash — and a blank string, which is what an emptied
        // text field sends — are both handled.
        let local = AiConfig {
            provider: AiProvider::OpenAiCompatible,
            base_url: Some("http://127.0.0.1:11434/v1/".into()),
            model: "llama3".into(),
        };
        assert_eq!(
            local.endpoint(),
            "http://127.0.0.1:11434/v1/chat/completions"
        );
        let blank = AiConfig {
            base_url: Some("   ".into()),
            ..AiConfig::default()
        };
        assert_eq!(blank.endpoint(), "https://api.anthropic.com/v1/messages");
    }

    #[test]
    fn provider_strings_round_trip() {
        for (p, s) in [
            (AiProvider::Anthropic, "anthropic"),
            (AiProvider::OpenAiCompatible, "openai_compatible"),
        ] {
            assert_eq!(p.as_str(), s);
            assert_eq!(AiProvider::parse(s).unwrap(), p);
            assert_eq!(serde_json::to_string(&p).unwrap(), format!("\"{s}\""));
        }
        // The spellings a person types.
        assert_eq!(
            AiProvider::parse("OpenAI-Compatible").unwrap(),
            AiProvider::OpenAiCompatible
        );
        assert_eq!(
            AiProvider::parse("openai").unwrap(),
            AiProvider::OpenAiCompatible
        );
        assert!(matches!(
            AiProvider::parse("llama").unwrap_err(),
            Error::Ai(_)
        ));
    }

    #[test]
    fn config_json_tolerates_missing_fields() {
        // A file written by a version that did not have `base_url` yet.
        let cfg: AiConfig = serde_json::from_str(r#"{"model":"gpt-4o"}"#).unwrap();
        assert_eq!(cfg.provider, AiProvider::Anthropic);
        assert_eq!(cfg.base_url, None);
        assert_eq!(cfg.model, "gpt-4o");
        // And an empty object is exactly the default.
        assert_eq!(
            serde_json::from_str::<AiConfig>("{}").unwrap(),
            AiConfig::default()
        );
    }

    #[test]
    fn responses_are_parsed_per_provider() {
        let anthropic = r#"{"id":"msg_1","content":[{"type":"thinking","thinking":"hm"},{"type":"text","text":"这个任务每天 21:00 同步"}],"model":"claude-sonnet-5"}"#;
        assert_eq!(
            parse_content(AiProvider::Anthropic, anthropic).unwrap(),
            "这个任务每天 21:00 同步"
        );
        let openai =
            r#"{"choices":[{"message":{"role":"assistant","content":"It syncs nightly."}}]}"#;
        assert_eq!(
            parse_content(AiProvider::OpenAiCompatible, openai).unwrap(),
            "It syncs nightly."
        );
        // An empty answer is an error, not an empty insight.
        assert!(parse_content(AiProvider::Anthropic, r#"{"content":[]}"#).is_err());
        assert!(parse_content(AiProvider::OpenAiCompatible, r#"{"choices":[]}"#).is_err());
        assert!(parse_content(AiProvider::Anthropic, "not json").is_err());
    }

    #[test]
    fn error_bodies_become_readable_messages() {
        assert_eq!(
            error_message(
                r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#
            ),
            "invalid x-api-key"
        );
        assert_eq!(
            error_message(r#"{"error":{"message":"model not found","code":"404"}}"#),
            "model not found"
        );
        // A gateway's HTML, or anything else, comes back verbatim rather
        // than as "unknown error".
        assert_eq!(error_message("  <html>502</html> "), "<html>502</html>");
        assert_eq!(error_message(""), "（响应体为空）");
    }

    #[test]
    fn a_blank_key_is_refused_before_any_request_is_made() {
        let cfg = AiConfig::default();
        let i = input("", Lang::ZhCn);
        assert!(matches!(
            explain(&cfg, "   ", &i).unwrap_err(),
            Error::AiNoKey
        ));
    }
}
