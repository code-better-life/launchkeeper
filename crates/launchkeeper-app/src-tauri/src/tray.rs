//! The menu bar item (`docs/M2-design.md` §3.5).
//!
//! Three template icons, one derived state, and a menu that is rebuilt only
//! when the favourite tasks actually change — the scan loop of §3.4 calls in
//! here every two seconds, and macOS closes an open menu when it is replaced.

use tauri::image::Image;
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};

use crate::commands::{TaskStatus, TaskView};
use crate::i18n::{Key, Lang, count_phrase, t};

/// The tray icon's id, so the scan loop can find it again.
pub const TRAY_ID: &str = "launchkeeper";

/// Menu id of the "open the window" item.
const ID_OPEN: &str = "lk:open";
/// Menu id of the "quit" item.
const ID_QUIT: &str = "lk:quit";
/// Menu id of the counts line at the top. It is a disabled item — a label,
/// not a command — but menu items still need an id.
const ID_SUMMARY: &str = "lk:summary";
/// Prefix of the per-task menu ids; the rest is the task name.
const TASK_PREFIX: &str = "lk:task:";

/// The three icon states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    /// Nothing running, nothing failed.
    Ok,
    /// At least one task's last run failed.
    Failed,
    /// At least one task is running (wins over a failure elsewhere).
    Running,
}

impl TrayState {
    /// The template PNG for this state, compiled in.
    ///
    /// Embedded rather than resolved from the resource directory so that the
    /// tray looks the same under `tauri dev` and inside a bundle, with no
    /// path lookup that can fail at startup.
    fn png(self) -> &'static [u8] {
        match self {
            TrayState::Ok => include_bytes!("../icons/tray-ok@2x.png"),
            TrayState::Failed => include_bytes!("../icons/tray-failed@2x.png"),
            TrayState::Running => include_bytes!("../icons/tray-running@2x.png"),
        }
    }

    fn image(self) -> tauri::Result<Image<'static>> {
        Image::from_bytes(self.png()).map(|i| i.to_owned())
    }
}

/// Running beats failed beats ok — the icon answers "is anything happening
/// right now?" first, and "is anything broken?" second.
///
/// With one exception, added in M2.5: a *service* that is up does not hide a
/// failure. A scheduled run is in flight for seconds, so showing it and
/// deferring the failure costs nothing; a service is up for weeks, and
/// letting it pin the icon to 运行中 would mean a daily task that started
/// failing never changes the menu bar again. So a running service only wins
/// when nothing is broken.
///
/// A `Disabled`, `Never` or `Stopped` task contributes nothing: a task the
/// user has switched off, or a service they stopped on purpose, is not a
/// problem to report.
pub fn derive_state(views: &[TaskView]) -> TrayState {
    let running = |v: &&TaskView| v.status == TaskStatus::Running;
    if views.iter().filter(running).any(|v| !v.is_service) {
        return TrayState::Running;
    }
    if views.iter().any(|v| v.status == TaskStatus::Failed) {
        return TrayState::Failed;
    }
    if views.iter().any(|v| running(&v)) {
        return TrayState::Running;
    }
    TrayState::Ok
}

/// The favourites shown in the menu, in list order.
fn favorites(views: &[TaskView]) -> Vec<&TaskView> {
    views.iter().filter(|v| v.favorite).collect()
}

/// True when clicking this favourite would stop it rather than start it.
///
/// Only a service gets a verb in the menu at all; for everything else the
/// item is still plain "run it now".
fn is_running_service(v: &TaskView) -> bool {
    v.is_service && v.status == TaskStatus::Running
}

/// What one favourite's menu item says.
///
/// The verb comes from [`crate::i18n`] (M3 §5); the name is the user's own
/// and is never translated. Both languages put the verb first, so one
/// `format!` covers them.
fn item_label(v: &TaskView, lang: Lang) -> String {
    if !v.is_service {
        return v.display_name.clone();
    }
    let verb = if is_running_service(v) {
        t(lang, Key::TrayStop)
    } else {
        t(lang, Key::TrayStart)
    };
    format!("{verb} {}", v.display_name)
}

/// The first line of the menu: 运行中 n · 失败 n · 已停用 n (PRD M4 item 3).
///
/// All three counts are always shown, zeros included — a fixed shape is read
/// at a glance, while a line whose segments come and go has to be read word
/// by word before it says anything. `None` only when there are no tasks at
/// all, where three zeros would be noise rather than a summary (and the menu
/// then skips the separator too).
///
/// The counts are over *every* task, not just the favourites the rest of the
/// menu lists: this line is the answer to "is anything broken right now?",
/// which is the question the menu bar exists to answer without opening the
/// window.
fn summary_line(views: &[TaskView], lang: Lang) -> Option<String> {
    if views.is_empty() {
        return None;
    }
    let count = |s: TaskStatus| views.iter().filter(|v| v.status == s).count();
    let parts = [
        (Key::TraySummaryRunning, count(TaskStatus::Running)),
        (Key::TraySummaryFailed, count(TaskStatus::Failed)),
        (Key::TraySummaryDisabled, count(TaskStatus::Disabled)),
    ];
    Some(
        parts
            .iter()
            .map(|(key, n)| count_phrase(lang, *key, *n))
            .collect::<Vec<_>>()
            .join(" · "),
    )
}

/// A cheap fingerprint of everything the menu renders. The menu is rebuilt
/// only when this changes.
///
/// A service's running state is part of it: its item flips between "start"
/// and "stop", so a menu that ignored the status would keep offering "start"
/// for a service that is already up (M2.5 §7). The language is part of it
/// too, by way of the labels themselves — switching languages therefore
/// rebuilds the menu on the next scan even if `set_settings` had not already
/// asked for a rebuild (M3 §5).
///
/// Since M4 the summary line is in here as well, which means a status change
/// on *any* task — favourite or not — rebuilds the menu whenever it moves one
/// of the three counts. That is the point of the line: a menu bar that says
/// 失败 0 while a task is failing is worse than one that is rebuilt a little
/// more often. Changes that do not move a count (Ok → Never, a run finishing
/// successfully) still cost nothing.
pub fn menu_signature(views: &[TaskView], lang: Lang) -> String {
    let favs = favorites(views)
        .iter()
        .map(|v| format!("{}\u{1}{}", v.name, item_label(v, lang)))
        .collect::<Vec<_>>()
        .join("\u{2}");
    format!(
        "{}\u{3}{favs}",
        summary_line(views, lang).unwrap_or_default()
    )
}

fn build_menu(app: &AppHandle, views: &[TaskView], lang: Lang) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::new(app)?;
    if let Some(line) = summary_line(views, lang) {
        // Disabled on purpose: it is a label, and a clickable one would only
        // raise the question of what clicking it does.
        menu.append(&MenuItem::with_id(
            app,
            ID_SUMMARY,
            line,
            false,
            None::<&str>,
        )?)?;
        menu.append(&PredefinedMenuItem::separator(app)?)?;
    }
    let favs = favorites(views);
    for v in &favs {
        let id = format!("{TASK_PREFIX}{}", v.name);
        menu.append(&MenuItem::with_id(
            app,
            id,
            item_label(v, lang),
            true,
            None::<&str>,
        )?)?;
    }
    if !favs.is_empty() {
        menu.append(&PredefinedMenuItem::separator(app)?)?;
    }
    menu.append(&MenuItem::with_id(
        app,
        ID_OPEN,
        t(lang, Key::TrayOpen),
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        ID_QUIT,
        t(lang, Key::TrayQuit),
        true,
        None::<&str>,
    )?)?;
    Ok(menu)
}

/// Creates the tray item. Called once, from `setup`.
///
/// # Errors
/// Tray or menu creation failures, and a corrupt embedded icon.
pub fn create(app: &AppHandle, views: &[TaskView], lang: Lang) -> tauri::Result<()> {
    let state = derive_state(views);
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(state.image()?)
        .icon_as_template(true)
        .menu(&build_menu(app, views, lang)?)
        .show_menu_on_left_click(true)
        .on_menu_event(on_menu_event)
        .build(app)?;
    Ok(())
}

/// Applies a fresh scan to the tray. `rebuild_menu` is false when the scan
/// loop knows the favourites are unchanged.
///
/// # Errors
/// Menu creation failures.
pub fn refresh(
    app: &AppHandle,
    views: &[TaskView],
    lang: Lang,
    rebuild_menu: bool,
) -> tauri::Result<()> {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Ok(());
    };
    let state = derive_state(views);
    tray.set_icon(Some(state.image()?))?;
    tray.set_icon_as_template(true)?;
    if rebuild_menu {
        tray.set_menu(Some(build_menu(app, views, lang)?))?;
    }
    Ok(())
}

/// Rebuilds the menu in the language `settings.json` currently says, from a
/// fresh scan. Called by `set_settings` when the language changed (M3 §5):
/// the scan loop would get there on its own only once something *else* about
/// a task changed, and a menu bar still in the old language after the user
/// picked a new one looks broken.
///
/// Runs the scan on the calling thread (a blocking pool thread) and hops to
/// the main thread only for the menu itself, which is where macOS requires
/// it. A failure is logged: a menu that stayed in the old language is not
/// worth failing the settings write the user actually asked for.
pub fn refresh_language(app: &AppHandle) {
    let views = match crate::commands::all_views(&app.state::<crate::state::AppState>()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("切换语言后重建菜单栏失败: {e}");
            return;
        }
    };
    let lang =
        crate::i18n::Lang::from_settings(&crate::settings::AppSettings::load().unwrap_or_default());
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Err(e) = refresh(&handle, &views, lang, true) {
            eprintln!("切换语言后重建菜单栏失败: {e}");
        }
    });
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id().as_ref().to_string();
    match id.as_str() {
        ID_OPEN => show_main_window(app),
        ID_QUIT => app.exit(0),
        _ => {
            if let Some(name) = id.strip_prefix(TASK_PREFIX) {
                activate_favorite(app, name);
            }
        }
    }
}

/// Does what the clicked item said: 停止 for a running service, otherwise
/// run/start it through exactly the same path as the `run_now` command.
///
/// The work happens on its own thread. Menu events arrive on the main
/// thread, and stopping a service waits up to
/// [`launchkeeper_core::service::STOP_GRACE`] for the process to go away —
/// long enough to freeze the whole app if it were done here.
///
/// A failure has nowhere to go from a menu click, so it is logged.
fn activate_favorite(app: &AppHandle, name: &str) {
    let app = app.clone();
    let name = name.to_string();
    std::thread::spawn(move || {
        let state = app.state::<crate::state::AppState>();
        let running = crate::commands::running_service(&state, &name).unwrap_or(false);
        let result = if running {
            crate::commands::lifecycle(&state, &name, crate::commands::Lifecycle::Stop).map(|_| ())
        } else {
            crate::commands::run_now_inner(&state, &name)
        };
        if let Err(e) = result {
            eprintln!("菜单栏操作任务失败（{name}）: {e}");
        }
    });
}

/// Shows and focuses the main window, creating nothing: the window always
/// exists, it is merely hidden when the user closes it.
pub fn show_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    // Bring the Dock icon back; closing the window switched to Accessory.
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

#[cfg(test)]
mod tests {
    use super::*;
    use launchkeeper_core::Trigger;

    fn view(name: &str, status: TaskStatus, favorite: bool) -> TaskView {
        TaskView {
            name: name.to_string(),
            display_name: format!("任务 {name}"),
            description: None,
            trigger: Trigger::AtLogin,
            trigger_text: "登录时".into(),
            interpreter_label: None,
            is_service: false,
            keep_alive: false,
            enabled: status != TaskStatus::Disabled,
            loaded_pid: None,
            uptime_secs: None,
            last_run: None,
            tags: Vec::new(),
            favorite,
            notify_on_fail: true,
            status,
        }
    }

    /// A favourite service in the given state.
    fn service(name: &str, status: TaskStatus) -> TaskView {
        TaskView {
            trigger: Trigger::Manual,
            trigger_text: "手动启停".into(),
            is_service: true,
            keep_alive: true,
            loaded_pid: (status == TaskStatus::Running).then_some(4242),
            ..view(name, status, true)
        }
    }

    #[test]
    fn empty_is_ok() {
        assert_eq!(derive_state(&[]), TrayState::Ok);
    }

    #[test]
    fn running_wins_over_failed() {
        let views = vec![
            view("a", TaskStatus::Failed, false),
            view("b", TaskStatus::Running, false),
        ];
        assert_eq!(derive_state(&views), TrayState::Running);
    }

    #[test]
    fn failed_wins_over_ok() {
        let views = vec![
            view("a", TaskStatus::Ok, false),
            view("b", TaskStatus::Failed, false),
        ];
        assert_eq!(derive_state(&views), TrayState::Failed);
    }

    #[test]
    fn disabled_never_and_stopped_are_not_failures() {
        let views = vec![
            view("a", TaskStatus::Disabled, false),
            view("b", TaskStatus::Never, false),
            view("c", TaskStatus::Ok, false),
            service("d", TaskStatus::Stopped),
        ];
        assert_eq!(derive_state(&views), TrayState::Ok);
    }

    #[test]
    fn a_running_service_shows_but_does_not_mask_a_failure() {
        // 没有别的问题时，服务在跑 -> 运行中
        assert_eq!(
            derive_state(&[service("bridge", TaskStatus::Running)]),
            TrayState::Running
        );
        // 有任务失败时，长期在跑的服务不该把失败盖掉
        let views = vec![
            service("bridge", TaskStatus::Running),
            view("cron", TaskStatus::Failed, false),
        ];
        assert_eq!(derive_state(&views), TrayState::Failed);
        // 但一次真正的定时运行（几秒钟的事）仍然优先
        let mut views = views;
        views.push(view("now", TaskStatus::Running, false));
        assert_eq!(derive_state(&views), TrayState::Running);
    }

    #[test]
    fn signature_tracks_favorites_and_the_summary_counts() {
        let mut views = vec![
            view("a", TaskStatus::Ok, true),
            view("b", TaskStatus::Ok, false),
        ];
        let before = menu_signature(&views, Lang::Zh);

        // A status change none of the three counts cares about (and that no
        // favourite's label reflects) must not rebuild the menu.
        views[0].status = TaskStatus::Never;
        views[1].status = TaskStatus::Ok;
        assert_eq!(menu_signature(&views, Lang::Zh), before);

        // One that does move a count must, even on a task the menu does not
        // list: the summary line counts every task (M4 §3).
        views[1].status = TaskStatus::Failed;
        assert_ne!(menu_signature(&views, Lang::Zh), before);
        views[1].status = TaskStatus::Ok;
        assert_eq!(menu_signature(&views, Lang::Zh), before);

        // Un-favouriting one, or renaming it, must.
        views[1].favorite = true;
        assert_ne!(menu_signature(&views, Lang::Zh), before);
        views[1].favorite = false;
        views[0].display_name = "改名了".into();
        assert_ne!(menu_signature(&views, Lang::Zh), before);
    }

    #[test]
    fn the_summary_line_counts_running_failed_and_disabled() {
        let views = vec![
            view("a", TaskStatus::Ok, false),
            view("b", TaskStatus::Failed, false),
            view("c", TaskStatus::Failed, false),
            view("d", TaskStatus::Disabled, false),
            view("e", TaskStatus::Never, false),
            service("f", TaskStatus::Running),
        ];
        // 服务型任务算进"运行中"：菜单栏问的是"现在有东西在跑吗"，一个跑着的
        // 常驻服务和一次定时运行一样是在跑。
        assert_eq!(
            summary_line(&views, Lang::Zh).as_deref(),
            Some("运行中 1 · 失败 2 · 已停用 1")
        );
        assert_eq!(
            summary_line(&views, Lang::En).as_deref(),
            Some("1 running · 2 failed · 1 disabled")
        );

        // 一切正常时三个零照样显示：行的形状固定，扫一眼就知道读到哪儿了。
        assert_eq!(
            summary_line(&[view("a", TaskStatus::Ok, false)], Lang::Zh).as_deref(),
            Some("运行中 0 · 失败 0 · 已停用 0")
        );
        // 一个任务都没有时干脆不要这一行（菜单里连分隔线也省了）。
        assert_eq!(summary_line(&[], Lang::Zh), None);
        assert_eq!(summary_line(&[], Lang::En), None);
    }

    #[test]
    fn a_service_item_names_the_verb_and_tracks_its_state() {
        let stopped = service("bridge", TaskStatus::Stopped);
        let running = service("bridge", TaskStatus::Running);
        assert_eq!(item_label(&stopped, Lang::Zh), "启动 任务 bridge");
        assert_eq!(item_label(&running, Lang::Zh), "停止 任务 bridge");
        // 同一条在英文下换的是动词，任务名一个字都不动
        assert_eq!(item_label(&stopped, Lang::En), "Start 任务 bridge");
        assert_eq!(item_label(&running, Lang::En), "Stop 任务 bridge");
        // 非服务型任务的菜单项还是纯显示名（点击 = 立即运行），两种语言一样
        assert_eq!(
            item_label(&view("cron", TaskStatus::Ok, true), Lang::Zh),
            "任务 cron"
        );
        assert_eq!(
            item_label(&view("cron", TaskStatus::Ok, true), Lang::En),
            "任务 cron"
        );

        // 服务起来/停下必须重建菜单，否则菜单会一直说"启动"
        assert_ne!(
            menu_signature(std::slice::from_ref(&stopped), Lang::Zh),
            menu_signature(std::slice::from_ref(&running), Lang::Zh),
        );
        // 但服务型任务的其它状态变化（崩溃 vs 停止）在菜单项上都还是"启动"
        // ——那一行本身不变（重建与否现在还要看 M4 的汇总行，见
        // `signature_tracks_favorites_and_the_summary_counts`）。
        assert_eq!(
            item_label(&service("bridge", TaskStatus::Failed), Lang::Zh),
            item_label(&stopped, Lang::Zh),
        );
        // 换语言必须重建：签名里带着标签本身（M3 §5）
        assert_ne!(
            menu_signature(std::slice::from_ref(&stopped), Lang::Zh),
            menu_signature(std::slice::from_ref(&stopped), Lang::En),
        );
    }

    #[test]
    fn every_state_has_a_decodable_icon() {
        for s in [TrayState::Ok, TrayState::Failed, TrayState::Running] {
            assert!(s.image().is_ok(), "{s:?} 图标无法解码");
        }
    }
}
