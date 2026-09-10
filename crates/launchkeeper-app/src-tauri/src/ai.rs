//! The AI commands (PRD M4 item 1, `docs/M4-design.md` §1).
//!
//! A thin bridge, like the rest of this crate: every decision — what the
//! prompt contains, what is redacted out of it, which wire format to speak,
//! where the key lives — belongs to [`launchkeeper_core::ai`], and is shared
//! byte-for-byte with the CLI. What lives here is the Tauri surface: the
//! `spawn_blocking` wrapper, the RFC 3339 timestamps the frontend wants, and
//! the one error message that has to read differently in a window than in a
//! terminal ([`crate::error::AppError::ai`]).
//!
//! A separate module rather than more of `commands.rs` because these
//! commands share nothing with the task CRUD there: no [`crate::state::AppState`]
//! for the two config commands, no launchd, no plists.

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};

use launchkeeper_core::ai::{self, AiConfig};

use crate::error::{AppError, AppResult};
use crate::settings::AppSettings;
use crate::state::AppState;

/// One stored explanation, as the detail pane renders it.
///
/// `created_at` is an RFC 3339 string rather than a number so the frontend
/// can hand it straight to `Date`, the same convention `RunView` follows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct InsightView {
    /// Task this explains.
    pub task_name: String,
    /// RFC 3339, UTC.
    pub created_at: String,
    /// Model that produced it, so the pane can say what wrote this.
    pub model: String,
    /// sha256 of the prompt behind `content`.
    pub prompt_hash: String,
    /// The answer, Markdown.
    pub content: String,
}

impl From<&ai::Insight> for InsightView {
    fn from(i: &ai::Insight) -> InsightView {
        InsightView {
            task_name: i.task_name.as_str().to_string(),
            created_at: i
                .created_at
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            model: i.model.clone(),
            prompt_hash: i.prompt_hash.clone(),
            content: i.content.clone(),
        }
    }
}

/// Runs `f` on the blocking pool. A private copy of `commands::blocking` for
/// the same reason `settings.rs` keeps one: that function is not `pub`, and
/// `commands.rs` is owned by concurrent work.
async fn blocking<T, F>(f: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(f).await {
        Ok(r) => r,
        Err(e) => Err(AppError::new(format!("后台任务失败: {e}"))),
    }
}

/// The language the model should answer in: whatever the 设置 sheet says,
/// resolved through the same [`crate::i18n::Lang`] the tray and the
/// notifications use, so an explanation never comes back in a different
/// language than the window around it.
fn answer_lang() -> ai::Lang {
    let settings = AppSettings::load().unwrap_or_default();
    match crate::i18n::Lang::from_settings(&settings) {
        crate::i18n::Lang::Zh => ai::Lang::ZhCn,
        crate::i18n::Lang::En => ai::Lang::En,
    }
}

/// The non-secret AI configuration (`<data_dir>/ai.json`). Never carries the
/// API key — [`has_ai_api_key`] is how the panel knows whether one is set.
#[tauri::command]
#[specta::specta]
pub async fn get_ai_config() -> AppResult<AiConfig> {
    blocking(|| ai::load_config().map_err(AppError::ai)).await
}

/// Replaces the AI configuration and hands it back, so the panel updates
/// from the same round trip instead of assuming the write landed —
/// `set_settings` does the same.
#[tauri::command]
#[specta::specta]
pub async fn set_ai_config(config: AiConfig) -> AppResult<AiConfig> {
    blocking(move || {
        ai::save_config(&config).map_err(AppError::ai)?;
        Ok(config)
    })
    .await
}

/// Stores the API key in the macOS key file.
///
/// The key crosses the IPC bridge exactly once, on its way in, and is never
/// returned by any command — not by [`get_ai_config`], not by
/// [`has_ai_api_key`]. The panel's field is write-only for the same reason
/// the CLI refuses to take a key as an argument.
#[tauri::command]
#[specta::specta]
pub async fn set_ai_api_key(key: String) -> AppResult<()> {
    blocking(move || ai::set_api_key(&key).map_err(AppError::ai)).await
}

/// Whether a key can be found at all — the key file entry, or the
/// `LAUNCHKEEPER_AI_API_KEY` override. What the panel's 「已配置 / 未配置」
/// line asks.
#[tauri::command]
#[specta::specta]
pub async fn has_ai_api_key() -> AppResult<bool> {
    blocking(|| ai::has_api_key().map_err(AppError::ai)).await
}

/// Deletes the key file entry. Cannot clear the environment override — that
/// belongs to whoever started the app.
#[tauri::command]
#[specta::specta]
pub async fn clear_ai_api_key() -> AppResult<()> {
    blocking(|| ai::clear_api_key().map_err(AppError::ai)).await
}

/// 「测试连接」: one short ping to the configured endpoint, answering with the
/// model's reply. Exercises the exact request shape [`explain_task`] uses, so
/// a green test really does mean 解读 will work.
#[tauri::command]
#[specta::specta]
pub async fn test_ai_connection() -> AppResult<String> {
    blocking(|| {
        let cfg = ai::load_config().map_err(AppError::ai)?;
        let key = ai::get_api_key()
            .map_err(AppError::ai)?
            .ok_or_else(|| AppError::ai(launchkeeper_core::Error::AiNoKey))?;
        ai::test_connection(&cfg, &key).map_err(AppError::ai)
    })
    .await
}

/// Explains a task, storing the answer.
///
/// `refresh = false` returns what is already stored without calling the
/// model, which is what opening the 「AI 解读」 tab does; the 「重新解读」
/// button passes `true`.
///
/// This is the one command in the app that can take tens of seconds. It runs
/// on the blocking pool like every other command (`docs/M2-design.md` §3.2),
/// so the window and the background scan keep going while it waits — and the
/// service mutex is held only for the database reads and the final write,
/// not for the HTTP request, because [`launchkeeper_core::Service::explain_task`]
/// does the gathering and the storing around a call that owns no lock of its
/// own.
#[tauri::command]
#[specta::specta]
pub async fn explain_task(app: AppHandle, name: String, refresh: bool) -> AppResult<InsightView> {
    blocking(move || {
        let state = app.state::<AppState>();
        let n = launchkeeper_core::TaskName::new(&name)?;
        let cfg = ai::load_config().map_err(AppError::ai)?;
        let lang = answer_lang();
        let insight = state
            .service()
            .explain_task(&n, &cfg, lang, refresh)
            .map_err(AppError::ai)?;
        Ok(InsightView::from(&insight))
    })
    .await
}

/// The stored explanation of a task, or `null` when it has never been
/// explained. A plain read: no network, no key needed, so the tab can render
/// last week's answer on a machine that is offline.
#[tauri::command]
#[specta::specta]
pub async fn get_task_insight(app: AppHandle, name: String) -> AppResult<Option<InsightView>> {
    blocking(move || {
        let state = app.state::<AppState>();
        let n = launchkeeper_core::TaskName::new(&name)?;
        let stored = state.service().insight(&n).map_err(AppError::ai)?;
        Ok(stored.as_ref().map(InsightView::from))
    })
    .await
}
