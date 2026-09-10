//! Error type shared by every module of `launchkeeper-core`.

use std::path::PathBuf;

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong inside `launchkeeper-core`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// `dirs` could not determine the current user's home directory.
    #[error("无法确定用户主目录")]
    NoHomeDir,

    /// A task name failed validation (see [`crate::task::TaskName::new`]).
    #[error("非法任务名: {0}")]
    InvalidName(String),

    /// A task field other than name/trigger failed validation.
    #[error("非法任务: {0}")]
    InvalidTask(String),

    /// A trigger failed [`crate::task::Trigger::validate`].
    #[error("非法触发规则: {0}")]
    InvalidTrigger(String),

    /// `insert_task` was called for a name that already exists.
    #[error("任务已存在: {0}")]
    TaskExists(String),

    /// The named task is not in the database.
    #[error("任务不存在: {0}")]
    TaskNotFound(String),

    /// An operation needed the runner binary path but the service was built
    /// without one.
    #[error("未提供 launchkeeper-runner 路径，该操作需要它")]
    NoRunner,

    /// The task exists but is not loaded into launchd.
    #[error("任务未启用: {0}")]
    NotEnabled(String),

    /// A write was refused because the plist at that path is not
    /// Launchkeeper's: its name is not `com.launchkeeper.*` and it carries no
    /// `LaunchkeeperManaged` marker. See
    /// [`crate::plist::is_ours`].
    #[error(
        "拒绝覆盖不属于 Launchkeeper 的 plist: {0}（既不是 com.launchkeeper.* \
         也没有 LaunchkeeperManaged 标记；要接管它请用 adopt）"
    )]
    PlistNotOurs(PathBuf),

    /// An adoption or un-adoption could not proceed. The message says why.
    #[error("接管失败: {0}")]
    Adopt(String),

    /// No LaunchAgent with that label was found in the agents directory.
    #[error("找不到 LaunchAgent: {0}")]
    AgentNotFound(String),

    /// A run id was not found in the `runs` table.
    #[error("运行记录不存在: {0}")]
    RunNotFound(i64),

    /// A `launchctl` invocation exited non-zero.
    #[error("launchctl {command} 失败 (exit {code:?}): {stderr}")]
    Launchctl {
        /// The sub-command and arguments, joined by spaces.
        command: String,
        /// Process exit code, `None` when killed by a signal.
        code: Option<i32>,
        /// Raw stderr (plus stdout when stderr was empty), untouched.
        stderr: String,
    },

    /// Capturing the login shell `PATH` failed.
    #[error("抓取 login shell PATH 失败: {0}")]
    EnvCapture(String),

    /// A row in the database could not be decoded into a domain type.
    #[error("数据损坏: {0}")]
    Corrupt(String),

    /// No AI API key is configured: neither `$LAUNCHKEEPER_AI_API_KEY` nor
    /// the keychain entry has one. Its own variant rather than a
    /// [`Error::Ai`] string because both the app and the CLI want to answer
    /// it with a "go configure it here" hint rather than a bare failure.
    #[error(
        "还没有配置 AI API Key：用 `launchkeeper ai key set` 存进钥匙串，\
         或设置环境变量 LAUNCHKEEPER_AI_API_KEY"
    )]
    AiNoKey,

    /// The AI provider answered with a non-2xx status. The message is the
    /// provider's own, dug out of its error body
    /// (`{"error": {"message": ...}}` for both supported providers).
    #[error("AI 接口返回 HTTP {status}: {message}")]
    AiHttp {
        /// HTTP status code.
        status: u16,
        /// The provider's message, or the raw body when it was not JSON.
        message: String,
    },

    /// Anything else in [`crate::ai`]: bad configuration, a transport
    /// failure, an unparseable response, a keychain error.
    #[error("AI 功能出错: {0}")]
    Ai(String),

    /// An I/O error, annotated with the path involved when known.
    #[error("IO 错误{}: {source}", .path.as_ref().map(|p| format!(" ({})", p.display())).unwrap_or_default())]
    Io {
        /// Path the operation was performed on, when known.
        path: Option<PathBuf>,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },

    /// A SQLite error.
    #[error("数据库错误: {0}")]
    Db(#[from] rusqlite::Error),

    /// A JSON (de)serialization error.
    #[error("JSON 错误: {0}")]
    Json(#[from] serde_json::Error),

    /// A plist (de)serialization error.
    #[error("plist 错误: {0}")]
    Plist(#[from] plist::Error),
}

impl From<std::io::Error> for Error {
    fn from(source: std::io::Error) -> Self {
        Error::Io { path: None, source }
    }
}

impl Error {
    /// Wraps an [`std::io::Error`] together with the path it happened on.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: Some(path.into()),
            source,
        }
    }
}
