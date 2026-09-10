//! The Launchkeeper desktop app.
//!
//! Design contract: `docs/M2-design.md`. The Rust side owns nothing the core
//! library does not already own — it is a thin, typed bridge between the
//! frontend and [`launchkeeper_core::Service`], with `tauri-specta` keeping
//! `src/lib/bindings.ts` in step with the signatures in [`commands`].
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod about;
pub mod ai;
pub mod commands;
pub mod error;
pub mod i18n;
pub mod notify;
pub mod settings;
pub mod state;
pub mod tray;

use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, RunEvent, WindowEvent};
use tauri_specta::{Event, collect_commands, collect_events};

use crate::commands::TaskView;
use crate::state::AppState;

/// How often the background scan runs while the window is on screen
/// (`docs/M2-design.md` §3.4).
const SCAN_INTERVAL_VISIBLE: Duration = Duration::from_secs(2);

/// How often it runs while the window is hidden or minimized. The tray still
/// needs to follow launchd, but nobody is watching a list, so the scan backs
/// off instead of stopping (H2).
const SCAN_INTERVAL_HIDDEN: Duration = Duration::from_secs(10);

/// Where the generated TypeScript bindings live, relative to `src-tauri/`.
const BINDINGS_PATH: &str = "../src/lib/bindings.ts";

/// Emitted after every background scan (`docs/M2-design.md` §3.4). The
/// frontend re-renders from this instead of polling `list_tasks` itself.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct TasksChanged(
    /// Every task, in the same order and shape `list_tasks` returns.
    pub Vec<TaskView>,
);

/// Builds the `tauri-specta` builder: the one place commands and events are
/// registered, so that the runtime handler and the generated bindings can
/// never disagree.
pub fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::list_tasks,
            commands::get_task,
            commands::create_task,
            commands::update_task,
            commands::delete_task,
            commands::set_enabled,
            commands::run_now,
            commands::start_service,
            commands::stop_service,
            commands::restart_service,
            commands::list_runs,
            commands::read_log,
            commands::scan_interpreters,
            commands::check_script,
            commands::task_log_dir,
            commands::reveal_in_finder,
            commands::list_external_agents,
            commands::adoption_plan,
            commands::adopt_agent,
            commands::unadopt_task,
            commands::external_set_enabled,
            commands::external_run,
            settings::get_settings,
            settings::set_settings,
            settings::data_dir,
            ai::get_ai_config,
            ai::set_ai_config,
            ai::set_ai_api_key,
            ai::has_ai_api_key,
            ai::clear_ai_api_key,
            ai::test_ai_connection,
            ai::explain_task,
            ai::get_task_insight,
            about::app_info,
            about::open_url,
        ])
        .events(collect_events![TasksChanged])
}

/// The TypeScript exporter configuration.
///
/// The 64-bit values that cross the bridge — SQLite row ids and log file
/// sizes — are all far below 2^53, so `specta-typescript`'s default mapping
/// of `i64`/`u64` onto `number` is what we want, and the frontend stays free
/// of `BigInt` ceremony.
fn typescript() -> specta_typescript::Typescript {
    specta_typescript::Typescript::new().header("// @ts-nocheck\n// 由 tauri-specta 生成，请勿手改。\n// 重新生成: cargo test -p launchkeeper-app export_bindings")
}

/// Runs the app. Called by `main.rs`.
///
/// # Panics
/// When Tauri itself cannot start, which is not a recoverable condition.
pub fn run() {
    let builder = specta_builder();

    // In a debug build the bindings are refreshed on every start, so that
    // `pnpm tauri dev` picks up a changed command signature without a
    // separate step. Release builds never write into the source tree.
    #[cfg(debug_assertions)]
    if let Err(e) = builder.export(typescript(), BINDINGS_PATH) {
        eprintln!("导出 bindings.ts 失败: {e}");
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            let handle = app.handle().clone();
            let state = AppState::new().map_err(|e| e.message)?;
            // The tray is seeded from the database alone — no launchctl —
            // so that a cold start does not pay for one `launchctl print`
            // per task before the window appears (M7). The first scan-loop
            // iteration, which runs immediately, corrects it.
            let first = commands::seed_views(&state).unwrap_or_else(|e| {
                eprintln!("首次扫描失败: {e}");
                Vec::new()
            });
            app.manage(state);
            tray::create(
                &handle,
                &first,
                i18n::Lang::from_settings(&settings::AppSettings::load().unwrap_or_default()),
            )?;
            // Closing the window hides it: the tray item is the app's real
            // home, and `退出` is the only way out (§3.5).
            // Once hidden the app also leaves the Dock (Accessory policy):
            // only the menu bar icon remains, the way the user wants it.
            // `tray::show_main_window` flips the policy back before showing.
            if let Some(window) = handle.get_webview_window("main") {
                let w = window.clone();
                let app_for_close = handle.clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = w.hide();
                        let _ =
                            app_for_close.set_activation_policy(tauri::ActivationPolicy::Accessory);
                    }
                });
            }
            std::thread::spawn(move || scan_loop(&handle, first));
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");

    app.run(|handle, event| {
        // Clicking the dock icon of an app whose windows are all closed.
        if let RunEvent::Reopen { .. } = event {
            tray::show_main_window(handle);
        }
    });
}

/// The background scan of §3.4.
///
/// The scan itself always runs: the tray is a consumer even when the window
/// is closed, and a menu bar that stops following launchd the moment the
/// window is hidden is the bug H2 describes. What visibility changes is the
/// pace (2 s on screen, 10 s hidden) and whether `TasksChanged` is emitted at
/// all — a hidden or minimized window has no list to update, and the frontend
/// reloads from `list_tasks` when it comes back.
///
/// The event is also emitted only when the payload actually changed, so an
/// idle, visible app wakes the frontend zero times per minute instead of
/// thirty. A *running service* is the deliberate exception: its
/// `uptime_secs` ticks, so its payload differs on every scan and the list's
/// "运行 3 分 20 秒" stays live instead of freezing at whatever it said when
/// something else last changed.
fn scan_loop(app: &AppHandle, first: Vec<TaskView>) {
    let mut last_json = serde_json::to_string(&first).unwrap_or_default();
    let mut last_menu = tray::menu_signature(&first, current_lang());
    // Set when the payload changed while nobody could see it, so that the
    // list is up to date the moment the window comes back.
    let mut pending_emit = false;
    // What `notify::on_scan` compares each iteration's fresh `views`
    // against. Starts at the same seed the tray and `last_json` start from,
    // so the very first real scan is "nothing changed yet" rather than
    // "everything just appeared" — see `notify`'s module docs on why a
    // task's first appearance in a comparison must never notify.
    //
    // Because this seed *is* the first `prev`, the run ids it carries have
    // to be handed to the dedup state too: found in `prev`, a task skips the
    // first-appearance branch that would otherwise have recorded them, and a
    // run that failed long before this launch would be announced as news on
    // every start (H2).
    notify::seed_global(&first);
    let mut prev_views = first.clone();
    let _ = TasksChanged(first).emit(app);

    loop {
        let on_screen = window_on_screen(app);
        let views = match commands::all_views(&app.state::<AppState>()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("后台扫描失败: {e}");
                std::thread::sleep(SCAN_INTERVAL_HIDDEN);
                continue;
            }
        };
        notify::on_scan(app, &prev_views, &views);
        prev_views = views.clone();
        let json = match serde_json::to_string(&views) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("序列化扫描结果失败: {e}");
                std::thread::sleep(SCAN_INTERVAL_HIDDEN);
                continue;
            }
        };
        let changed = json != last_json;
        if changed {
            last_json = json;

            // The language is part of the signature (`tray::menu_signature`),
            // so a language change picked up here rebuilds the menu even
            // though `set_settings` normally gets there first.
            let lang = current_lang();
            let menu_sig = tray::menu_signature(&views, lang);
            let rebuild = menu_sig != last_menu;
            last_menu = menu_sig;
            // Menus and tray icons are main-thread-only on macOS.
            let handle = app.clone();
            let for_tray = views.clone();
            let _ = app.run_on_main_thread(move || {
                if let Err(e) = tray::refresh(&handle, &for_tray, lang, rebuild) {
                    eprintln!("刷新菜单栏失败: {e}");
                }
            });
        }
        if changed || pending_emit {
            if on_screen {
                pending_emit = false;
                let _ = TasksChanged(views).emit(app);
            } else {
                pending_emit = true;
            }
        }

        std::thread::sleep(if on_screen {
            SCAN_INTERVAL_VISIBLE
        } else {
            SCAN_INTERVAL_HIDDEN
        });
    }
}

/// The language the tray menu should be in right now: whatever
/// `settings.json` says, or the system language (M3 §5). Read from the file
/// each time for the same reason `notify::on_scan` re-reads it — at most once
/// per scan, a few dozen bytes, and no cache to invalidate when the settings
/// sheet writes a new value.
fn current_lang() -> i18n::Lang {
    i18n::Lang::from_settings(&settings::AppSettings::load().unwrap_or_default())
}

/// True when the main window is both visible and not minimized. A minimized
/// window is as invisible to the user as a hidden one, and `is_visible` alone
/// still reports `true` for it.
fn window_on_screen(app: &AppHandle) -> bool {
    let Some(w) = app.get_webview_window("main") else {
        return false;
    };
    w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regenerates `src/lib/bindings.ts`.
    ///
    /// This is a test rather than a build script so that CI can run it and
    /// then check `git diff --exit-code` on the generated file: the commands
    /// and the TypeScript the frontend compiles against are verified to
    /// match, instead of merely being generated together.
    #[test]
    fn export_bindings() {
        specta_builder()
            .export(typescript(), BINDINGS_PATH)
            .expect("导出 bindings.ts");
    }
}
