//! The single error type every `#[tauri::command]` returns.
//!
//! The frontend only ever sees a string: `docs/M2-design.md` §3.2 pins the
//! wire shape to `{ message }`, and §4 says a failed command is shown as a
//! toast carrying the backend message verbatim. Keeping one struct (rather
//! than mapping [`launchkeeper_core::Error`] variant by variant) means the
//! Chinese messages core already writes reach the user unchanged.

use serde::Serialize;

/// A command failure, as the frontend receives it.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct AppError {
    /// Human-readable, already-localised message.
    pub message: String,
}

impl AppError {
    /// Builds an error from anything printable.
    pub fn new(message: impl std::fmt::Display) -> AppError {
        AppError {
            message: message.to_string(),
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}

impl From<launchkeeper_core::Error> for AppError {
    fn from(e: launchkeeper_core::Error) -> AppError {
        AppError::new(e)
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> AppError {
        AppError::new(e)
    }
}

impl AppError {
    /// [`launchkeeper_core::Error`] as an AI command should report it
    /// (M4 §1).
    ///
    /// Only one variant is rewritten, and for a concrete reason: core's
    /// [`launchkeeper_core::Error::AiNoKey`] message tells the reader to run
    /// `launchkeeper ai key set`, which is the right advice in a terminal
    /// and the wrong advice in a window that has a 设置 → AI panel with a
    /// key field in it. Every other variant already carries a message
    /// written for a human — an HTTP status with the provider's own text, a
    /// key file failure — and is passed through untouched, the same way
    /// [`From<launchkeeper_core::Error>`] does for every other command.
    #[must_use]
    pub fn ai(e: launchkeeper_core::Error) -> AppError {
        match e {
            launchkeeper_core::Error::AiNoKey => AppError::new(
                "还没有配置 AI API Key：在「设置 → AI」里填一个 / No AI API key configured: add one under Settings → AI",
            ),
            other => AppError::new(other),
        }
    }
}

/// Result alias used by every command.
pub type AppResult<T> = std::result::Result<T, AppError>;
