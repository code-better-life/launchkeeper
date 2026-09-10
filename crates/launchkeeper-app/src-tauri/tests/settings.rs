//! `settings::AppSettings`, exercised against a temporary data dir.
//!
//! Everything lives in one `#[test]` because it mutates the process-wide
//! `LAUNCHKEEPER_DATA_DIR` environment variable and cargo runs the tests in
//! one file on multiple threads by default; `launchkeeper-core`'s own
//! `tests/paths_env.rs` does the same for the same reason. Cargo gives each
//! integration test file its own process, so nothing outside this file is
//! disturbed, and the real `~/Library/Application Support/Launchkeeper/` is
//! never touched.

use launchkeeper_app_lib::settings::AppSettings;
use launchkeeper_core::paths;

#[test]
fn app_settings_persistence() {
    let dir = tempfile::tempdir().expect("tempdir");
    // SAFETY: single-threaded within this test — the whole suite is one
    // `#[test]` fn precisely so nothing else in this process reads or
    // writes `LAUNCHKEEPER_DATA_DIR` while it is set.
    unsafe {
        std::env::set_var(paths::DATA_DIR_ENV, dir.path());
    }

    // A fresh data dir has no settings.json: the default, not an error.
    assert_eq!(AppSettings::load().expect("load"), AppSettings::default());

    // Saving writes a settings.json a later load reads back unchanged, and
    // does not leave the atomic-write temp file behind.
    let s = AppSettings {
        notify_on_task_failure: false,
        notify_on_service_crash: false,
        language: Some("en".into()),
        list_sort: Some("next_run".into()),
        theme: Some("dark".into()),
    };
    s.save().expect("save");
    assert!(dir.path().join("settings.json").exists());
    assert!(!dir.path().join("settings.json.tmp").exists());
    assert_eq!(AppSettings::load().expect("load"), s);

    // A hand-corrupted file falls back to the default rather than erroring
    // — a bad settings.json must never be the reason failure notifications
    // (or the app) stop working.
    std::fs::write(dir.path().join("settings.json"), b"{ not json").expect("write");
    assert_eq!(AppSettings::load().expect("load"), AppSettings::default());

    // Saving again overwrites the corrupt file cleanly.
    AppSettings::default().save().expect("save over corrupt");
    assert_eq!(AppSettings::load().expect("load"), AppSettings::default());

    // A settings.json written before M4 has no `list_sort`; it must still
    // load, with the field at its default rather than failing the whole read
    // (`#[serde(default)]`, the reason every field here is optional).
    std::fs::write(
        dir.path().join("settings.json"),
        br#"{"notify_on_task_failure": false, "language": "zh-CN"}"#,
    )
    .expect("write");
    let old = AppSettings::load().expect("load pre-M4 file");
    assert_eq!(old.list_sort, None);
    assert!(!old.notify_on_task_failure);
    assert_eq!(old.language.as_deref(), Some("zh-CN"));

    // Same again for `theme` (M4 §4): a file written between M4 §2 and M4 §4
    // has `list_sort` but no `theme`, and must still load with the rest of
    // its fields intact rather than falling back to the whole default.
    std::fs::write(
        dir.path().join("settings.json"),
        br#"{"notify_on_service_crash": false, "list_sort": "status"}"#,
    )
    .expect("write");
    let pre_theme = AppSettings::load().expect("load pre-theme file");
    assert_eq!(pre_theme.theme, None);
    assert_eq!(pre_theme.list_sort.as_deref(), Some("status"));
    assert!(!pre_theme.notify_on_service_crash);
    assert!(pre_theme.notify_on_task_failure);

    concurrent_saves_all_land(dir.path());

    // SAFETY: see above.
    unsafe {
        std::env::remove_var(paths::DATA_DIR_ENV);
    }
}

/// M11: several saves at once must all succeed, leave no temp file behind,
/// and leave a `settings.json` that parses.
///
/// With a single shared `settings.json.tmp` they did not: two writers took
/// turns truncating the same file, and whichever `rename`d second found
/// nothing to rename — an error for a write the caller was told had failed,
/// on top of a settings file whose content came from neither of them
/// reliably. The temp name now carries the pid and a counter, so every writer
/// has its own.
///
/// The threads only read `LAUNCHKEEPER_DATA_DIR`; the one write to it happens
/// before they start and the next one after they are joined.
fn concurrent_saves_all_land(dir: &std::path::Path) {
    let langs = ["zh-CN", "en", "system"];
    let handles: Vec<_> = (0..12)
        .map(|i| {
            std::thread::spawn(move || {
                AppSettings {
                    notify_on_task_failure: i % 2 == 0,
                    notify_on_service_crash: i % 3 == 0,
                    language: Some(langs[i % langs.len()].to_string()),
                    list_sort: None,
                    theme: None,
                }
                .save()
            })
        })
        .collect();
    for h in handles {
        h.join().expect("线程不该 panic").expect("并发保存不该失败");
    }

    let strays: Vec<_> = std::fs::read_dir(dir)
        .expect("read_dir")
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .filter(|n| n.starts_with("settings.json.tmp"))
        .collect();
    assert!(strays.is_empty(), "不该留下临时文件: {strays:?}");
    // Whichever writer won, the file is one of theirs and parses.
    let loaded = AppSettings::load().expect("load");
    assert!(langs.contains(&loaded.language.as_deref().unwrap_or_default()));
}
