//! The handful of strings Rust itself puts in front of the user
//! (PRD M3 item 5, `docs/M3-design.md` §5).
//!
//! Almost every string the user reads is rendered by the frontend, which has
//! its own dictionaries in `src/lib/i18n/`. Two places cannot go through it:
//!
//! - the **tray menu**, which macOS draws from strings handed to it by
//!   `tray.rs` — the webview is not involved and may not even be running;
//! - **notification** titles and bodies, produced by `notify.rs` on the scan
//!   thread.
//!
//! So this module is a deliberately tiny mirror of the frontend's dictionary:
//! a `Key` enum and one `match` per language. It is not a general i18n
//! framework and should not grow into one — anything with more than a
//! sentence in it belongs in the frontend's dictionary, where the translator
//! is already looking.
//!
//! The language comes from the same `AppSettings::language` the settings
//! sheet writes (`"system"` / `"zh-CN"` / `"en"`, absent = follow the
//! system), so the tray and the window never disagree about which language
//! the app is in.

use std::sync::OnceLock;

use crate::settings::AppSettings;

/// A language this app has strings for. Anything else follows
/// [`Lang::En`] — the same fallback the frontend's `resolveLocale` uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    /// 简体中文.
    Zh,
    /// English.
    En,
}

/// One translatable string. Keys with a value in them (a task name, an exit
/// code) are the *fixed* half only; the caller formats the rest, because the
/// two languages put the value in the same place and `format!` needs a
/// literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Tray: show the main window.
    TrayOpen,
    /// Tray: quit the app.
    TrayQuit,
    /// Tray: the verb in front of a stopped service's name.
    TrayStart,
    /// Tray: the verb in front of a running service's name.
    TrayStop,
    /// Tray summary: the word next to the number of running tasks.
    TraySummaryRunning,
    /// Tray summary: the word next to the number of failed tasks.
    TraySummaryFailed,
    /// Tray summary: the word next to the number of disabled tasks.
    TraySummaryDisabled,
    /// Notification title: what follows the task's display name.
    NotifyFailedSuffix,
    /// Notification body for a `KeepAlive` service that crashed.
    NotifyServiceCrash,
    /// Notification body: what precedes the exit code.
    NotifyExitCode,
    /// Notification body for a run killed by a signal.
    NotifySignal,
}

/// The string for `key` in `lang`.
#[must_use]
pub fn t(lang: Lang, key: Key) -> &'static str {
    match (lang, key) {
        (Lang::Zh, Key::TrayOpen) => "打开 Launchkeeper",
        (Lang::En, Key::TrayOpen) => "Open Launchkeeper",
        (Lang::Zh, Key::TrayQuit) => "退出",
        (Lang::En, Key::TrayQuit) => "Quit",
        (Lang::Zh, Key::TrayStart) => "启动",
        (Lang::En, Key::TrayStart) => "Start",
        (Lang::Zh, Key::TrayStop) => "停止",
        (Lang::En, Key::TrayStop) => "Stop",
        (Lang::Zh, Key::TraySummaryRunning) => "运行中",
        (Lang::En, Key::TraySummaryRunning) => "running",
        (Lang::Zh, Key::TraySummaryFailed) => "失败",
        (Lang::En, Key::TraySummaryFailed) => "failed",
        (Lang::Zh, Key::TraySummaryDisabled) => "已停用",
        (Lang::En, Key::TraySummaryDisabled) => "disabled",
        (Lang::Zh, Key::NotifyFailedSuffix) => "失败",
        (Lang::En, Key::NotifyFailedSuffix) => "failed",
        (Lang::Zh, Key::NotifyServiceCrash) => "服务崩溃，已自动重启",
        (Lang::En, Key::NotifyServiceCrash) => "Service crashed and was restarted",
        (Lang::Zh, Key::NotifyExitCode) => "退出码",
        (Lang::En, Key::NotifyExitCode) => "Exit code",
        (Lang::Zh, Key::NotifySignal) => "被信号终止",
        (Lang::En, Key::NotifySignal) => "Killed by a signal",
    }
}

/// `<count> <word>` in the order the language wants it: 中文 puts the noun
/// first (`运行中 2`), English the number (`2 running`).
///
/// The tray's summary line (PRD M4 item 3) is the only place that needs this,
/// and it needs it three times in a row, so the rule lives here rather than in
/// `tray.rs`: a caller composing the line should not have to know which half
/// of the phrase each language leads with.
#[must_use]
pub fn count_phrase(lang: Lang, key: Key, n: usize) -> String {
    let word = t(lang, key);
    match lang {
        Lang::Zh => format!("{word} {n}"),
        Lang::En => format!("{n} {word}"),
    }
}

impl Lang {
    /// A BCP 47-ish tag (`zh-Hans-CN`, `en_US.UTF-8`, `zh`) to a language, or
    /// `None` when it is neither of the two.
    #[must_use]
    pub fn from_tag(tag: &str) -> Option<Lang> {
        let tag = tag.trim().to_ascii_lowercase();
        if tag.starts_with("zh") {
            Some(Lang::Zh)
        } else if tag.starts_with("en") {
            Some(Lang::En)
        } else {
            None
        }
    }

    /// What `settings.language` says: an explicit choice, or the system
    /// language when the field is `"system"`, absent, or something this
    /// version does not understand (a hand-edited `settings.json` must not
    /// be able to break the menu bar).
    #[must_use]
    pub fn from_settings(settings: &AppSettings) -> Lang {
        match settings.language.as_deref() {
            Some("zh-CN") => Lang::Zh,
            Some("en") => Lang::En,
            _ => Lang::system(),
        }
    }

    /// The macOS UI language, cached for the life of the process.
    ///
    /// `defaults read -g AppleLanguages` is asked first and `$LANG` second,
    /// in that order on purpose: the app is normally started by Finder or
    /// launchd, where `LANG` is unset or a leftover from a build shell, while
    /// `AppleLanguages` is exactly the list the user dragged into order in
    /// System Settings. The subprocess runs at most once — the answer cannot
    /// change without the app being restarted anyway, since macOS restarts
    /// apps on a language change.
    #[must_use]
    pub fn system() -> Lang {
        static CACHED: OnceLock<Lang> = OnceLock::new();
        *CACHED.get_or_init(|| {
            apple_languages()
                .as_deref()
                .and_then(first_known_tag)
                .or_else(|| {
                    std::env::var("LANG")
                        .ok()
                        .as_deref()
                        .and_then(Lang::from_tag)
                })
                .unwrap_or(Lang::En)
        })
    }
}

/// The raw `defaults read -g AppleLanguages` output, or `None` if the command
/// is unavailable or fails (it does not exist off macOS, and this crate's
/// tests run wherever CI puts them).
fn apple_languages() -> Option<String> {
    let out = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLanguages"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The first language tag in `defaults`' plist-ish output that this app has
/// strings for.
///
/// The output looks like `(\n    "zh-Hans-CN",\n    en\n)` — quoted or bare,
/// one per line. Taking the first *known* tag rather than the first tag
/// outright matters: a user whose list is `fr, zh-Hans, en` reads French
/// best but Chinese second, and answering "English" there would be worse
/// than answering "Chinese".
fn first_known_tag(raw: &str) -> Option<Lang> {
    raw.lines()
        .map(|l| {
            l.trim()
                .trim_matches(|c| c == '(' || c == ')' || c == ',' || c == '"')
        })
        .filter(|l| !l.is_empty())
        .find_map(Lang::from_tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_map_to_the_two_languages_we_have() {
        assert_eq!(Lang::from_tag("zh-Hans-CN"), Some(Lang::Zh));
        assert_eq!(Lang::from_tag("zh"), Some(Lang::Zh));
        assert_eq!(Lang::from_tag("en_US.UTF-8"), Some(Lang::En));
        assert_eq!(Lang::from_tag("EN"), Some(Lang::En));
        // Not "close enough to English": we have no strings for it, and the
        // caller decides what to do about that.
        assert_eq!(Lang::from_tag("fr-FR"), None);
        assert_eq!(Lang::from_tag(""), None);
    }

    #[test]
    fn defaults_output_is_parsed_and_unknown_languages_are_skipped() {
        let raw = "(\n    \"zh-Hans-CN\",\n    en\n)\n";
        assert_eq!(first_known_tag(raw), Some(Lang::Zh));
        // A language we have no dictionary for does not get to decide; the
        // next one the user listed does.
        let raw = "(\n    \"fr-FR\",\n    \"zh-Hans\",\n    en\n)\n";
        assert_eq!(first_known_tag(raw), Some(Lang::Zh));
        let raw = "(\n    en\n)\n";
        assert_eq!(first_known_tag(raw), Some(Lang::En));
        // Nothing usable in there at all.
        assert_eq!(first_known_tag("(\n    \"fr-FR\"\n)\n"), None);
        assert_eq!(first_known_tag(""), None);
    }

    #[test]
    fn an_explicit_setting_wins_over_the_system() {
        let zh = AppSettings {
            language: Some("zh-CN".into()),
            ..AppSettings::default()
        };
        let en = AppSettings {
            language: Some("en".into()),
            ..AppSettings::default()
        };
        assert_eq!(Lang::from_settings(&zh), Lang::Zh);
        assert_eq!(Lang::from_settings(&en), Lang::En);
        // "system", a missing field and a value from some other version all
        // mean the same thing, and none of them may panic.
        for raw in [
            None,
            Some("system".to_string()),
            Some("klingon".to_string()),
        ] {
            let s = AppSettings {
                language: raw,
                ..AppSettings::default()
            };
            assert_eq!(Lang::from_settings(&s), Lang::system());
        }
    }

    #[test]
    fn every_key_has_a_string_in_both_languages() {
        let keys = [
            Key::TrayOpen,
            Key::TrayQuit,
            Key::TrayStart,
            Key::TrayStop,
            Key::TraySummaryRunning,
            Key::TraySummaryFailed,
            Key::TraySummaryDisabled,
            Key::NotifyFailedSuffix,
            Key::NotifyServiceCrash,
            Key::NotifyExitCode,
            Key::NotifySignal,
        ];
        for key in keys {
            let zh = t(Lang::Zh, key);
            let en = t(Lang::En, key);
            assert!(!zh.is_empty(), "{key:?} 中文为空");
            assert!(!en.is_empty(), "{key:?} 英文为空");
            // Every one of these differs between the two languages; a copy
            // left untranslated would show up here.
            assert_ne!(zh, en, "{key:?} 两种语言一模一样，多半是漏翻了");
        }
    }

    #[test]
    fn a_count_phrase_puts_the_number_where_the_language_wants_it() {
        assert_eq!(
            count_phrase(Lang::Zh, Key::TraySummaryRunning, 2),
            "运行中 2"
        );
        assert_eq!(
            count_phrase(Lang::En, Key::TraySummaryRunning, 2),
            "2 running"
        );
        // 0 is a number like any other here: the tray line always shows all
        // three counts, so "失败 0" / "0 failed" has to read correctly too.
        assert_eq!(count_phrase(Lang::Zh, Key::TraySummaryFailed, 0), "失败 0");
        assert_eq!(
            count_phrase(Lang::En, Key::TraySummaryFailed, 0),
            "0 failed"
        );
    }
}
