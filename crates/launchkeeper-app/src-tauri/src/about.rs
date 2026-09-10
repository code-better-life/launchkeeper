//! The 「关于」 block of the settings sheet: the app's version and a way to
//! open the project's links in the default browser.
//!
//! `open_url` shells out to `open` rather than pulling in the opener plugin
//! for two links; it only accepts http(s) so a stray string can never launch
//! anything else.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::{AppError, AppResult};

/// What the 关于 block shows.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AppInfo {
    /// Cargo package version of the app crate, e.g. `0.1.0`.
    pub version: String,
}

/// Version and build facts for the 关于 block.
#[tauri::command]
#[specta::specta]
pub fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// Opens an `http://` / `https://` URL in the default browser.
#[tauri::command]
#[specta::specta]
pub fn open_url(url: String) -> AppResult<()> {
    if !is_web_url(&url) {
        return Err(AppError::new(format!("只能打开 http(s) 链接: {url}")));
    }
    let status = std::process::Command::new("/usr/bin/open")
        .arg(&url)
        .status()
        .map_err(|e| AppError::new(format!("无法调用 open: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(AppError::new(format!("open 退出码 {status}")))
    }
}

/// True for an absolute http(s) URL with a host and no control characters.
fn is_web_url(url: &str) -> bool {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    match rest {
        Some(r) => {
            !r.is_empty()
                && !r.starts_with('/')
                && !url.chars().any(|c| c.is_control() || c.is_whitespace())
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_urls_pass() {
        assert!(is_web_url("https://github.com/x/y"));
        assert!(is_web_url("http://example.com"));
        assert!(!is_web_url("file:///etc/passwd"));
        assert!(!is_web_url("https://"));
        assert!(!is_web_url("https:///path"));
        assert!(!is_web_url("https://a.b c"));
        assert!(!is_web_url("open -a Calculator"));
    }

    #[test]
    fn version_is_the_crate_version() {
        assert_eq!(app_info().version, env!("CARGO_PKG_VERSION"));
    }
}
