//! Persisted app settings (`docs/M3-design.md` §4, PRD §4.8 / M3 item 4:
//! "失败通知").
//!
//! `<data_dir>/settings.json`, via [`launchkeeper_core::paths::data_dir`] —
//! the same directory the database and logs live in, so it follows the same
//! `LAUNCHKEEPER_DATA_DIR` override the rest of the app does. There is
//! nothing here yet that needs a database row or a migration: one small JSON
//! file that either exists with sane defaults or does not exist at all.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::AppHandle;

use launchkeeper_core::paths;

use crate::error::{AppError, AppResult};

/// Everything the 设置 sheet can change today.
///
/// `#[serde(default)]` on the struct (via `Default`) means a `settings.json`
/// written by an older version of the app — missing a field this version
/// added — still parses: the missing field takes its default rather than
/// failing the whole read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct AppSettings {
    /// Post a macOS notification when a scheduled task's run finishes with a
    /// nonzero exit code, is killed by a timeout, or otherwise ends badly —
    /// see [`crate::notify`]. Still gated per task by
    /// [`launchkeeper_core::Task::notify_on_fail`].
    pub notify_on_task_failure: bool,
    /// Same, for a `KeepAlive` service crashing and being restarted by
    /// launchd.
    pub notify_on_service_crash: bool,
    /// UI language override: `"zh-CN"`, `"en"`, or `"system"` / `None` to
    /// follow the system language (PRD M3 item 5, `docs/M3-design.md` §5).
    ///
    /// Read by the frontend on start (`src/lib/i18n`) and by
    /// [`crate::i18n::Lang::from_settings`] for the tray menu and the
    /// notifications. Anything this version does not recognise is treated as
    /// "follow the system" rather than as an error — a hand-edited
    /// `settings.json` must not be able to leave the app without a language.
    pub language: Option<String>,
    /// How the 「Launchkeeper 管理」 group is sorted: `"name"`,
    /// `"last_run"`, `"status"` or `"next_run"` (PRD M4 item 2,
    /// `docs/M4-design.md` §2). `None` means the default, `"name"`.
    ///
    /// Rust never reads the value — the list is sorted in the frontend, which
    /// is also where the four modes are defined (`src/lib/sort.ts`). This
    /// field exists only so the choice outlives the window, which is why an
    /// unrecognised string is not an error here either: `sortMode()` on the
    /// frontend maps anything it does not know back to `"name"`.
    pub list_sort: Option<String>,
    /// Appearance: `"system"`, `"light"` or `"dark"` (`docs/M4-design.md`
    /// §4). `None` means the default, `"system"` — follow macOS.
    ///
    /// Like `list_sort`, Rust never reads this: the frontend stamps
    /// `data-theme` on `<html>` and asks the Tauri window for a matching
    /// title bar (`src/lib/theme.ts`). It lives here so the choice outlives
    /// the window, and an unrecognised string falls back to "follow the
    /// system" on the frontend rather than failing the read here — the same
    /// rule `language` follows.
    pub theme: Option<String>,
}

impl Default for AppSettings {
    fn default() -> AppSettings {
        AppSettings {
            notify_on_task_failure: true,
            notify_on_service_crash: true,
            language: None,
            list_sort: None,
            theme: None,
        }
    }
}

fn settings_path() -> AppResult<PathBuf> {
    Ok(paths::data_dir()?.join("settings.json"))
}

/// A temp file name no other writer can be using: pid (another process, e.g.
/// a second copy of the app or the test binary) plus a counter (another
/// thread in this one).
///
/// A single shared `settings.json.tmp` looked harmless while there was one
/// writer, but two concurrent saves would then take turns truncating *the
/// same* file and one of the two `rename`s would move a file the other had
/// already renamed away — losing that write, with an ENOENT the caller sees
/// as "保存失败" for a write that had in fact half happened.
fn tmp_path(path: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    path.with_extension(format!("json.tmp.{}.{n}", std::process::id()))
}

/// Serializes the read-modify-write in [`set_settings`].
///
/// Every command runs on the blocking pool, so two settings sheets — or the
/// language switch and a notification checkbox clicked in quick succession —
/// really can interleave. Without this, both would read the same `before`,
/// and the second `save` would write a whole struct built from a snapshot
/// that no longer describes the file.
fn settings_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

impl AppSettings {
    /// Loads `<data_dir>/settings.json`.
    ///
    /// A missing file is the common case — every install until the user
    /// opens 设置 for the first time and something actually writes it — and
    /// is treated as [`AppSettings::default`], not an error. A file that
    /// exists but fails to parse (hand-edited into invalid JSON, say) falls
    /// back to the default too, with a message on stderr: a corrupt
    /// preferences file must never be the reason the app — and the failure
    /// notifications this module exists for — stops working.
    ///
    /// # Errors
    /// Only for a read failure that is not "file does not exist" (permission
    /// errors and the like), and for [`launchkeeper_core::Error::NoHomeDir`].
    pub fn load() -> AppResult<AppSettings> {
        let path = settings_path()?;
        match fs::read_to_string(&path) {
            Ok(raw) => Ok(serde_json::from_str(&raw).unwrap_or_else(|e| {
                eprintln!(
                    "settings.json 解析失败（{}），使用默认设置: {e}",
                    path.display()
                );
                AppSettings::default()
            })),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AppSettings::default()),
            Err(e) => Err(AppError::new(format!(
                "读取设置失败（{}）: {e}",
                path.display()
            ))),
        }
    }

    /// Writes `<data_dir>/settings.json` atomically.
    ///
    /// A sibling `settings.json.tmp.<pid>.<n>` is written, flushed and
    /// `fsync`ed, then `rename`d over the real path — same directory, so the
    /// rename is a single filesystem operation. A crash or a `pnpm tauri dev`
    /// restart mid-write can therefore only ever leave the *old*
    /// `settings.json` (or a stray, ignored `.tmp.*` file) behind, never a
    /// half-written one that the next [`AppSettings::load`] would have to
    /// fall back from. The name is unique per writer so that two concurrent
    /// saves cannot truncate and rename each other's temp file.
    ///
    /// # Errors
    /// Filesystem errors, and [`launchkeeper_core::Error::NoHomeDir`].
    pub fn save(&self) -> AppResult<()> {
        let path = settings_path()?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = tmp_path(&path);
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| AppError::new(format!("序列化设置失败: {e}")))?;
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(json.as_bytes())?;
            f.sync_all()?;
        }
        if let Err(e) = fs::rename(&tmp, &path) {
            let _ = fs::remove_file(&tmp);
            return Err(e.into());
        }
        Ok(())
    }
}

/// Runs `f` on the blocking pool. A private copy of `commands::blocking` —
/// that one is not `pub`, and this module has exactly two commands, not
/// enough to justify exporting it across a module boundary owned by
/// concurrent work on `commands.rs`.
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

/// The current settings, `AppSettings::default()` on a fresh install.
#[tauri::command]
#[specta::specta]
pub async fn get_settings() -> AppResult<AppSettings> {
    blocking(AppSettings::load).await
}

/// Replaces the settings wholesale and returns them back, so the frontend
/// can update its store from the same round trip rather than assuming the
/// write succeeded.
///
/// When the language changed, the tray menu is rebuilt from here (M3 §5).
/// The frontend re-renders itself the moment the user clicks — it owns its
/// own dictionaries — but the menu bar's strings come from Rust, and the
/// scan loop only rebuilds the menu when something about a *task* changed,
/// which might not happen for hours.
#[tauri::command]
#[specta::specta]
pub async fn set_settings(app: AppHandle, settings: AppSettings) -> AppResult<AppSettings> {
    let handle = app.clone();
    blocking(move || {
        // Read and write under one lock: `before` is only used to decide
        // whether the language changed, but reading it from a file another
        // save is halfway through writing would rebuild the tray menu (or
        // fail to) on the strength of a value that was never current.
        let _guard = settings_lock().lock().unwrap_or_else(|e| e.into_inner());
        let before = AppSettings::load().unwrap_or_default();
        settings.save()?;
        if crate::i18n::Lang::from_settings(&before) != crate::i18n::Lang::from_settings(&settings)
        {
            crate::tray::refresh_language(&handle);
        }
        Ok(settings)
    })
    .await
}

/// The data directory's absolute path, for the 设置 sheet's read-only
/// display (`docs/M3-design.md` §4). Not editable from the app —
/// `LAUNCHKEEPER_DATA_DIR` / `~/.config/launchkeeper/data-dir` are for
/// machines where the default (`~/Library/Application Support/Launchkeeper`)
/// is not where the user wants it, and neither is a setting `settings.json`
/// itself could ever live inside — the file finds its own directory before
/// there is a directory to read it from.
#[tauri::command]
#[specta::specta]
pub async fn data_dir() -> AppResult<String> {
    blocking(|| Ok(paths::data_dir()?.to_string_lossy().into_owned())).await
}

// Tests live in `tests/settings.rs`, not here: they need `std::env::set_var`
// to point `paths::data_dir` at a temp dir, which — since Rust made
// `set_var` an `unsafe fn` — cannot appear anywhere under this crate's
// `#![forbid(unsafe_code)]` (`lib.rs`). An integration test file is its own
// crate with no such attribute, the same reason `launchkeeper-core`'s own
// `set_var`-using tests live under its `tests/`, not inline in `src/`.
