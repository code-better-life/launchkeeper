//! Failure notifications (PRD §4.8, M3 item 4 in `docs/PRD.md` §8; design in
//! `docs/M3-design.md` §4).
//!
//! Split into a pure half and an impure half on purpose:
//!
//! - [`plan_notifications`] takes the previous and current scan's
//!   [`TaskView`]s (`docs/M2-design.md` §3.4 already computes both, once
//!   every 2–10 s), the current [`AppSettings`], and a [`SeenState`] it
//!   mutates, and returns the [`Notification`]s that should fire. No
//!   filesystem, no `tauri-plugin-notification`, no clock beyond what the
//!   timestamps already carry — every rule below is tested against
//!   hand-built `TaskView`s with no I/O in sight.
//! - [`on_scan`] is the thin, impure wrapper `lib.rs`'s scan loop calls once
//!   per iteration: it loads [`AppSettings`], holds the process-wide
//!   [`SeenState`] behind a mutex, calls [`plan_notifications`], and hands
//!   the result to the notification plugin.
//!
//! ## Detection rules
//!
//! A task's *newest* run is the only thing a [`TaskView`] carries — no
//! history — so the rules below are phrased in terms of what changed between
//! two consecutive scans of that one field, `last_run`, plus `loaded_pid` for
//! services:
//!
//! 1. **A run just finished badly.** `last_run` is no longer `running`, its
//!    `exit_code` is not `Some(0)`, and its `stop_reason` is not `Stopped` —
//!    covers a nonzero exit *and* a timeout (the runner reports timeouts as
//!    exit code 124 with `stop_reason: Timeout`, both of which fail this
//!    check). A user-initiated stop (`stop_reason: Stopped`) is deliberately
//!    exempt: the runner reports it as `exit_code: None` (killed by SIGTERM)
//!    precisely so this is not read as a crash (`docs/M2.5-design.md` §3.1).
//!    Dedup is by run id, remembered in [`SeenState`] — the same failed run
//!    staying `last_run` across several scans (nothing else happened) must
//!    notify exactly once, and a *second* failure later, even on the same
//!    task, is a new run id and notifies again.
//! 2. **A `KeepAlive` service's pid changed underneath a scan.** Rule 1
//!    alone misses a service that crashes and is relaunched by launchd
//!    *between* two scans: by the time the next scan runs, the failed run's
//!    row has already been superseded by the restart's new, still-`running`
//!    row, so `now.last_run` never shows the failure at all. A pid that
//!    changed while the task stayed enabled is what is left to notice it by.
//!    This needs no separate dedup state: `prev.loaded_pid` on the *next*
//!    scan already equals the pid that just triggered this rule, so the
//!    comparison is naturally quiet until it changes again. It is also
//!    skipped when *this* scan already saw the run that explains the new pid:
//!    a failure (rule 1 has just reported it) or a user-initiated stop, which
//!    is what 停止 / 重启 from the UI looks like one scan later.
//!
//! Both rules are skipped for a task's very first appearance in a comparison
//! (no matching entry in `prev`) — there is nothing to call a *change*
//! against, and firing on whatever state a task already happened to be in
//! when Launchkeeper started would notify about failures from before the app
//! was even running. [`SeenState`] is still seeded in that case, so a later
//! scan does not mistake the pre-existing state for a fresh one either. The
//! app's *startup* seed goes through the same door explicitly: `lib.rs` calls
//! [`seed_global`] with the views the scan loop starts from, because those
//! views are also the first `prev` — every task in them is therefore *found*
//! in `prev`, the first-appearance branch never runs, and a run that failed
//! days ago would notify again on every single app start.
//!
//! Both rules also respect two independent switches before a notification is
//! produced: the per-task [`TaskView::notify_on_fail`] (owned by the task's
//! own form, `docs/M2-design.md` §3.3) and the matching half of the global
//! [`AppSettings`] (owned by 设置). Either off silences that task. Which half
//! applies is decided by `is_service && keep_alive`, not by `is_service`
//! alone: `Task::is_service` is true for *every* `Manual` task, and a
//! hand-run one-shot script is a task — it gets the task switch and the exit
//! code in its body, not 「服务崩溃，已自动重启」.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

use tauri::AppHandle;
use tauri_plugin_notification::{NotificationExt, PermissionState};

use launchkeeper_core::StopReason;

use crate::commands::{RunView, TaskView};
use crate::i18n::{Key, Lang, t};
use crate::settings::AppSettings;

/// One notification [`plan_notifications`] decided should fire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// `Launchkeeper · <显示名> 失败`.
    pub title: String,
    /// `退出码 N · <时间>`, or `服务崩溃，已自动重启` for a service.
    pub body: String,
    /// The task this is about, for logging when delivery fails.
    pub task_name: String,
}

/// Dedup state [`plan_notifications`] owns across calls. Keyed by task name
/// rather than carried inside [`TaskView`] because it is Launchkeeper's own
/// bookkeeping, not anything the frontend or the database has an opinion
/// about.
#[derive(Debug, Default)]
pub struct SeenState {
    /// Task name -> id of the last run this module has already produced a
    /// notification (or a deliberate non-notification) for. See rule 1 in
    /// the module docs.
    last_run_id: HashMap<String, i64>,
}

/// The process-wide [`SeenState`], held for the life of the app. A single
/// background thread runs the scan loop (`lib.rs`), so contention is not a
/// concern — this exists to give [`on_scan`] somewhere to keep state between
/// calls without threading it through `AppState` (Mutex<Service>` already
/// serializes on the database; this is unrelated bookkeeping that outlives
/// no particular scan).
fn seen_state() -> &'static Mutex<SeenState> {
    static SEEN: OnceLock<Mutex<SeenState>> = OnceLock::new();
    SEEN.get_or_init(|| Mutex::new(SeenState::default()))
}

/// True when a finished run is one a user watching the task would call a
/// failure: not a plain success, and not the runner reporting "I was asked
/// to stop" (`docs/M2-design.md`'s `commands::ended_badly`, mirrored here for
/// the frontend's [`RunView`] rather than core's `Run`).
fn ended_badly(run: &RunView) -> bool {
    !run.running && run.exit_code != Some(0) && run.stop_reason != Some(StopReason::Stopped)
}

/// Whether this task is one the "服务崩溃" half of the settings — and the
/// service wording — is about.
///
/// [`TaskView::is_service`] alone is true for *every* `Manual` task
/// (`Task::is_service`), including a one-shot script the user only ever runs
/// by hand. Such a task failing is a task failure, not a service crash: it
/// belongs to `notify_on_task_failure` and should say which exit code it
/// died with. Only a `KeepAlive` service is the resident process the crash
/// wording describes, which is also exactly what rule 2 keys on.
fn is_resident_service(v: &TaskView) -> bool {
    v.is_service && v.keep_alive
}

/// Whether `v`'s own switch and the matching half of `settings` both allow a
/// notification.
fn allowed(v: &TaskView, settings: &AppSettings) -> bool {
    if !v.notify_on_fail {
        return false;
    }
    if is_resident_service(v) {
        settings.notify_on_service_crash
    } else {
        settings.notify_on_task_failure
    }
}

/// `2026-09-10 21:00` style local time, for the notification body. Falls
/// back to the raw RFC 3339 string on a parse failure — which should not
/// happen for anything `commands::rfc3339` produced, but a malformed
/// timestamp must degrade to an ugly notification, not a missing one.
fn local_time(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_else(|_| rfc3339.to_string())
}

/// `Launchkeeper · <display name> failed`, in `lang`. The display name is the
/// user's own and is never translated (M3 §5).
fn notification_title(v: &TaskView, lang: Lang) -> String {
    format!(
        "Launchkeeper · {} {}",
        v.display_name,
        t(lang, Key::NotifyFailedSuffix)
    )
}

fn notification_for(v: &TaskView, run: &RunView, lang: Lang) -> Notification {
    let body = if is_resident_service(v) {
        t(lang, Key::NotifyServiceCrash).to_string()
    } else {
        let when = run.finished_at.as_deref().unwrap_or(&run.started_at);
        match run.exit_code {
            Some(code) => format!(
                "{} {code} · {}",
                t(lang, Key::NotifyExitCode),
                local_time(when)
            ),
            // Killed by a signal other than the SIGTERM `stop_reason:
            // Stopped` already exempted above — e.g. SIGSEGV, SIGKILL from
            // something outside Launchkeeper.
            None => format!("{} · {}", t(lang, Key::NotifySignal), local_time(when)),
        }
    };
    Notification {
        title: notification_title(v, lang),
        body,
        task_name: v.name.clone(),
    }
}

/// Records the run ids `views` already carries, so that history which
/// predates this comparison is never mistaken for something that just
/// happened.
///
/// [`crate::commands::seed_views`] hands the scan loop a full set of views —
/// `last_run` included — before the first iteration, and that seed is also
/// the first `prev`. Without this call every task in it *is* found in `prev`,
/// so the "first appearance" branch below never runs, and a run that failed
/// days ago notifies again on every app start. Only terminal runs are
/// recorded, for the same reason the first-appearance branch only records
/// those: seeding a still-running run's id would make its eventual finish
/// look already accounted for.
pub fn seed(seen: &mut SeenState, views: &[TaskView]) {
    for v in views {
        if let Some(run) = &v.last_run
            && !run.running
        {
            seen.last_run_id.insert(v.name.clone(), run.id.0);
        }
    }
}

/// [`seed`], against the process-wide state [`on_scan`] uses. Called once by
/// `lib.rs` with the startup seed, before the scan loop starts.
pub fn seed_global(views: &[TaskView]) {
    seed(&mut lock_seen(), views);
}

/// The pure detection pass. See the module docs for the two rules.
pub fn plan_notifications(
    prev: &[TaskView],
    now: &[TaskView],
    settings: &AppSettings,
    seen: &mut SeenState,
) -> Vec<Notification> {
    let mut out = Vec::new();
    // One lookup for the whole pass: the language cannot change halfway
    // through a scan, and `Lang::system()` would otherwise be consulted once
    // per task.
    let lang = Lang::from_settings(settings);

    for v in now {
        let Some(prev_v) = prev.iter().find(|p| p.name == v.name) else {
            // First time this task appears in a comparison: nothing to call
            // a *change* against. A run already sitting in a terminal state
            // is seeded into the dedup map so a later scan does not treat
            // pre-existing history as a fresh failure; a run still
            // `running` is deliberately *not* seeded — there is nothing to
            // dedup yet, and seeding its id now would make rule 1 think the
            // eventual finish (bad or not) was "already accounted for" the
            // moment it lands.
            seed(seen, std::slice::from_ref(v));
            continue;
        };

        // Set when this scan already knows why the pid changed, so that rule
        // 2 does not report the same event a second time under the wrong
        // name.
        let mut pid_change_explained = false;

        // Rule 1: the newest run just finished badly, and this run id has
        // not already been accounted for.
        if let Some(run) = &v.last_run {
            if !run.running {
                let is_new = seen.last_run_id.get(v.name.as_str()) != Some(&run.id.0);
                if is_new && ended_badly(run) {
                    if allowed(v, settings) {
                        out.push(notification_for(v, run, lang));
                    }
                    pid_change_explained = true;
                } else if is_new && run.stop_reason == Some(StopReason::Stopped) {
                    // The user stopped (or restarted) this service in this
                    // very interval. The pid rule 2 is about to compare is
                    // the one *they* asked for — a stop-then-start is two
                    // deliberate acts, not a crash — so rule 2 sits this one
                    // out. It stays quiet from the next scan onwards by
                    // itself, since `prev.loaded_pid` will have caught up.
                    pid_change_explained = true;
                }
                seen.last_run_id.insert(v.name.clone(), run.id.0);
            }
        }

        // Rule 2: a KeepAlive service's pid changed since the last scan
        // without rule 1 just having explained why (the common case when it
        // does explain it: the crash *and* the restart both landed inside
        // one scan interval, so `last_run` briefly showed the failed run
        // before rule 1 saw it). See the module docs for why this rule needs
        // no dedup state of its own.
        if !pid_change_explained
            && is_resident_service(v)
            && v.enabled
            && let (Some(now_pid), Some(prev_pid)) = (v.loaded_pid, prev_v.loaded_pid)
            && now_pid != prev_pid
            && allowed(v, settings)
        {
            out.push(Notification {
                title: notification_title(v, lang),
                body: t(lang, Key::NotifyServiceCrash).to_string(),
                task_name: v.name.clone(),
            });
        }
    }

    // A task that is gone (deleted, or its adoption undone) can never be
    // compared against again, and its entry would otherwise sit in the map
    // for the life of the process. A task that comes *back* under the same
    // name is a fresh row with fresh run ids anyway.
    seen.last_run_id
        .retain(|name, _| now.iter().any(|v| &v.name == name));

    out
}

/// Whether the app is allowed to post a system notification, asked **once**
/// for the life of the process and remembered.
///
/// macOS asks once per app (identified by its signed bundle / cdhash, same
/// story as the TCC prompt `CLAUDE.md` documents for external-disk access);
/// an unsigned development build may never move `Prompt` to `Granted` at
/// all, in which case this — correctly — answers `false` and [`on_scan`]
/// logs instead of calling a plugin API that would itself fail. See
/// `docs/M3-design.md` §4 for what was actually observed in `pnpm tauri dev`.
///
/// The answer is cached because `request_permission` blocks until the user
/// deals with the system prompt: asking again on the next failing run would
/// mean a second modal for a decision that has already been made, and a
/// `Denied` app would pay for a plugin round trip on every failure forever.
/// The user changing the setting in System Settings mid-session is worth a
/// restart; a repeated prompt is not.
fn notifications_permitted(app: &AppHandle) -> bool {
    static PERMITTED: OnceLock<bool> = OnceLock::new();
    *PERMITTED.get_or_init(|| match app.notification().permission_state() {
        Ok(PermissionState::Granted) => true,
        Ok(PermissionState::Denied) => false,
        Ok(_) => app
            .notification()
            .request_permission()
            .map(|s| s == PermissionState::Granted)
            .unwrap_or(false),
        Err(e) => {
            eprintln!("查询通知权限失败: {e}");
            false
        }
    })
}

fn lock_seen() -> MutexGuard<'static, SeenState> {
    seen_state().lock().unwrap_or_else(|e| e.into_inner())
}

/// The scan loop's entry point (`lib.rs`), called once per iteration right
/// after a fresh `Vec<TaskView>` is computed.
///
/// Loads [`AppSettings`] fresh every call rather than caching them: this
/// runs at most once every 2 s, the file is a few dozen bytes, and a cache
/// would need its own invalidation the moment `set_settings` is called from
/// the 设置 sheet in the same process.
pub fn on_scan(app: &AppHandle, prev: &[TaskView], now: &[TaskView]) {
    let settings = match AppSettings::load() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("加载设置失败，本次扫描跳过失败通知: {e}");
            return;
        }
    };
    let notifications = plan_notifications(prev, now, &settings, &mut lock_seen());
    if notifications.is_empty() {
        return;
    }
    // Everything from here on happens on the main thread: the permission
    // request opens a system modal and blocks until it is answered, and
    // `show()` goes through the same AppKit machinery the tray does. Left
    // inline, one prompt nobody notices would freeze the scan loop — and
    // with it the tray and the task list — for as long as it sits there.
    let handle = app.clone();
    if let Err(e) = app.run_on_main_thread(move || {
        if !notifications_permitted(&handle) {
            return;
        }
        for n in notifications {
            if let Err(e) = handle
                .notification()
                .builder()
                .title(&n.title)
                .body(&n.body)
                .show()
            {
                eprintln!("发送失败通知失败（{}）: {e}", n.task_name);
            }
        }
    }) {
        eprintln!("投递失败通知到主线程失败: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use launchkeeper_core::{Trigger, TriggerKind};

    fn run(
        id: i64,
        running: bool,
        exit_code: Option<i32>,
        stop_reason: Option<StopReason>,
    ) -> RunView {
        RunView {
            id: crate::commands::RunHandle(id),
            started_at: "2026-09-10T13:00:00.000Z".into(),
            finished_at: (!running).then(|| "2026-09-10T13:00:05.000Z".into()),
            exit_code,
            duration_ms: (!running).then_some(5000),
            trigger_kind: TriggerKind::Scheduled,
            stop_reason,
            running,
        }
    }

    fn task(name: &str, is_service: bool, keep_alive: bool, notify_on_fail: bool) -> TaskView {
        TaskView {
            name: name.to_string(),
            display_name: format!("任务 {name}"),
            description: None,
            trigger: if is_service {
                Trigger::Manual
            } else {
                Trigger::AtLogin
            },
            trigger_text: String::new(),
            interpreter_label: None,
            is_service,
            keep_alive,
            enabled: true,
            loaded_pid: None,
            uptime_secs: None,
            last_run: None,
            tags: Vec::new(),
            favorite: false,
            notify_on_fail,
            status: crate::commands::TaskStatus::Never,
        }
    }

    /// Settings with the language pinned: these tests assert on the exact
    /// wording, and `None` would mean "follow the system", making them pass
    /// or fail depending on what language the machine running them is in.
    fn settings(task_on: bool, service_on: bool) -> AppSettings {
        AppSettings {
            notify_on_task_failure: task_on,
            notify_on_service_crash: service_on,
            language: Some("zh-CN".into()),
            ..AppSettings::default()
        }
    }

    #[test]
    fn first_scan_notifies_nothing_even_if_already_failed() {
        let mut seen = SeenState::default();
        let mut t = task("a", false, false, true);
        t.last_run = Some(run(1, false, Some(1), Some(StopReason::Exited)));
        let out = plan_notifications(
            &[],
            std::slice::from_ref(&t),
            &settings(true, true),
            &mut seen,
        );
        assert!(out.is_empty());
        // But it is remembered, so a later "no-op" scan (same run) also
        // stays quiet rather than treating the seed as a fresh success.
        let out2 = plan_notifications(
            std::slice::from_ref(&t),
            std::slice::from_ref(&t),
            &settings(true, true),
            &mut seen,
        );
        assert!(out2.is_empty());
    }

    /// H2: the app starts with a task whose newest run failed days ago.
    /// `scan_loop`'s `prev` is the startup seed — not an empty slice — so
    /// the first-appearance branch never fires for it, and only the explicit
    /// [`seed`] keeps that stale failure from being announced as news on
    /// every launch.
    #[test]
    fn a_stale_failed_run_present_at_startup_never_notifies() {
        let mut stale = task("a", false, false, true);
        stale.last_run = Some(run(7, false, Some(1), Some(StopReason::Exited)));
        let startup = vec![stale.clone()];

        let mut seen = SeenState::default();
        seed(&mut seen, &startup);

        // First real scan: prev is the seed the app started from.
        let out = plan_notifications(&startup, &startup, &settings(true, true), &mut seen);
        assert!(out.is_empty(), "启动时就在那儿的旧失败不该通知: {out:?}");

        // Restarting the app all over again does the same thing.
        let mut seen = SeenState::default();
        seed(&mut seen, &startup);
        assert!(
            plan_notifications(&startup, &startup, &settings(true, true), &mut seen).is_empty()
        );

        // But a genuinely new failure on the same task still notifies.
        let mut fresh = stale.clone();
        fresh.last_run = Some(run(8, false, Some(2), Some(StopReason::Exited)));
        let out = plan_notifications(
            &startup,
            std::slice::from_ref(&fresh),
            &settings(true, true),
            &mut seen,
        );
        assert_eq!(out.len(), 1, "新的一次失败仍然要通知");
    }

    /// Without the seed the same startup would notify — the assertion that
    /// keeps `seed` from being quietly dropped again.
    #[test]
    fn without_the_seed_that_same_startup_would_have_notified() {
        let mut stale = task("a", false, false, true);
        stale.last_run = Some(run(7, false, Some(1), Some(StopReason::Exited)));
        let startup = vec![stale];
        let mut seen = SeenState::default();
        let out = plan_notifications(&startup, &startup, &settings(true, true), &mut seen);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn success_then_failure_notifies_once() {
        let mut seen = SeenState::default();
        let mut before = task("a", false, false, true);
        before.last_run = Some(run(1, false, Some(0), Some(StopReason::Exited)));
        let mut after = before.clone();
        after.last_run = Some(run(2, false, Some(1), Some(StopReason::Exited)));

        let out = plan_notifications(
            std::slice::from_ref(&before),
            std::slice::from_ref(&after),
            &settings(true, true),
            &mut seen,
        );
        assert_eq!(out.len(), 1);
        assert!(out[0].body.starts_with("退出码 1"));
        assert_eq!(out[0].title, "Launchkeeper · 任务 a 失败");
    }

    /// The same failure, with the app set to English (M3 §5). The display
    /// name is the user's own and stays exactly as it is; only the fixed
    /// words around it change.
    #[test]
    fn the_notification_follows_the_language_setting() {
        let mut seen = SeenState::default();
        let mut before = task("a", false, false, true);
        before.last_run = Some(run(1, false, Some(0), Some(StopReason::Exited)));
        let mut after = before.clone();
        after.last_run = Some(run(2, false, Some(1), Some(StopReason::Exited)));
        let english = AppSettings {
            language: Some("en".into()),
            ..settings(true, true)
        };

        let out = plan_notifications(
            std::slice::from_ref(&before),
            std::slice::from_ref(&after),
            &english,
            &mut seen,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "Launchkeeper · 任务 a failed");
        assert!(out[0].body.starts_with("Exit code 1"), "{}", out[0].body);

        // A service crash says the same thing in the other language.
        let mut seen = SeenState::default();
        let mut svc = task("bridge", true, true, true);
        svc.enabled = true;
        svc.loaded_pid = Some(100);
        let mut after = svc.clone();
        after.loaded_pid = Some(101);
        let out = plan_notifications(
            std::slice::from_ref(&svc),
            std::slice::from_ref(&after),
            &english,
            &mut seen,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].body, "Service crashed and was restarted");
    }

    #[test]
    fn the_same_failed_run_does_not_notify_twice() {
        let mut seen = SeenState::default();
        let mut running = task("a", false, false, true);
        running.last_run = Some(run(1, true, None, None));
        let mut failed = running.clone();
        failed.last_run = Some(run(1, false, Some(1), Some(StopReason::Exited)));

        let first = plan_notifications(
            std::slice::from_ref(&running),
            std::slice::from_ref(&failed),
            &settings(true, true),
            &mut seen,
        );
        assert_eq!(first.len(), 1);

        // Nothing changed on the next scan: same failed run is still
        // `last_run`.
        let second = plan_notifications(
            std::slice::from_ref(&failed),
            std::slice::from_ref(&failed),
            &settings(true, true),
            &mut seen,
        );
        assert!(second.is_empty());
    }

    #[test]
    fn a_timeout_is_a_failure() {
        let mut seen = SeenState::default();
        let mut running = task("a", false, false, true);
        running.last_run = Some(run(1, true, None, None));
        let mut timed_out = running.clone();
        timed_out.last_run = Some(run(1, false, Some(124), Some(StopReason::Timeout)));

        let out = plan_notifications(
            std::slice::from_ref(&running),
            std::slice::from_ref(&timed_out),
            &settings(true, true),
            &mut seen,
        );
        assert_eq!(out.len(), 1);
        assert!(out[0].body.starts_with("退出码 124"));
    }

    #[test]
    fn a_user_initiated_stop_does_not_notify() {
        let mut seen = SeenState::default();
        let mut running = task("bridge", true, true, true);
        running.loaded_pid = Some(100);
        running.last_run = Some(run(1, true, None, None));
        let mut stopped = running.clone();
        stopped.loaded_pid = None;
        stopped.last_run = Some(run(1, false, None, Some(StopReason::Stopped)));

        let out = plan_notifications(
            std::slice::from_ref(&running),
            std::slice::from_ref(&stopped),
            &settings(true, true),
            &mut seen,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn a_service_crash_restart_notifies_via_the_run_row_when_the_scan_catches_it() {
        let mut seen = SeenState::default();
        let mut running = task("bridge", true, true, true);
        running.loaded_pid = Some(100);
        running.last_run = Some(run(1, true, None, None));
        let mut crashed = running.clone();
        crashed.loaded_pid = None;
        crashed.last_run = Some(run(1, false, Some(1), Some(StopReason::Exited)));

        let out = plan_notifications(
            std::slice::from_ref(&running),
            std::slice::from_ref(&crashed),
            &settings(true, true),
            &mut seen,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].body, "服务崩溃，已自动重启");
    }

    #[test]
    fn a_service_crash_restart_notifies_via_the_pid_change_when_the_scan_misses_the_run_row() {
        let mut seen = SeenState::default();
        let mut running = task("bridge", true, true, true);
        running.loaded_pid = Some(100);
        running.last_run = Some(run(1, true, None, None));
        // By the time the next scan runs, launchd has already relaunched it
        // under a new pid with a fresh, still-running run row (id 2) — the
        // failed run (id 1) is never observed as `last_run`.
        let mut restarted = running.clone();
        restarted.loaded_pid = Some(200);
        restarted.last_run = Some(run(2, true, None, None));

        let out = plan_notifications(
            std::slice::from_ref(&running),
            std::slice::from_ref(&restarted),
            &settings(true, true),
            &mut seen,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].body, "服务崩溃，已自动重启");

        // And it settles: the next scan's `prev` already carries pid 200, so
        // an unrelated later scan does not refire.
        let out2 = plan_notifications(
            std::slice::from_ref(&restarted),
            std::slice::from_ref(&restarted),
            &settings(true, true),
            &mut seen,
        );
        assert!(out2.is_empty());
    }

    #[test]
    fn per_task_opt_out_is_respected() {
        let mut seen = SeenState::default();
        let mut before = task("a", false, false, false);
        before.last_run = Some(run(1, false, Some(0), Some(StopReason::Exited)));
        let mut after = before.clone();
        after.last_run = Some(run(2, false, Some(1), Some(StopReason::Exited)));

        let out = plan_notifications(
            std::slice::from_ref(&before),
            std::slice::from_ref(&after),
            &settings(true, true),
            &mut seen,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn global_opt_out_is_respected_for_scheduled_tasks() {
        let mut seen = SeenState::default();
        let mut before = task("a", false, false, true);
        before.last_run = Some(run(1, false, Some(0), Some(StopReason::Exited)));
        let mut after = before.clone();
        after.last_run = Some(run(2, false, Some(1), Some(StopReason::Exited)));

        let out = plan_notifications(
            std::slice::from_ref(&before),
            std::slice::from_ref(&after),
            &settings(false, true),
            &mut seen,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn global_opt_out_is_respected_for_services() {
        let mut seen = SeenState::default();
        let mut running = task("bridge", true, true, true);
        running.loaded_pid = Some(100);
        running.last_run = Some(run(1, true, None, None));
        let mut restarted = running.clone();
        restarted.loaded_pid = Some(200);
        restarted.last_run = Some(run(2, true, None, None));

        let out = plan_notifications(
            std::slice::from_ref(&running),
            std::slice::from_ref(&restarted),
            &settings(true, false),
            &mut seen,
        );
        assert!(out.is_empty());
    }

    /// M5: `is_service` is true for every `Manual` task, including a
    /// one-shot script the user runs by hand. Failing, it is a *task*
    /// failure: the task switch decides, and the body names the exit code
    /// instead of claiming a service was restarted.
    #[test]
    fn a_manual_task_without_keep_alive_gets_the_task_wording_and_switch() {
        let mut running = task("once", true, false, true);
        running.last_run = Some(run(1, true, None, None));
        let mut failed = running.clone();
        failed.last_run = Some(run(1, false, Some(3), Some(StopReason::Exited)));

        // 服务开关关着也照样通知——它不是服务。
        let mut seen = SeenState::default();
        let out = plan_notifications(
            std::slice::from_ref(&running),
            std::slice::from_ref(&failed),
            &settings(true, false),
            &mut seen,
        );
        assert_eq!(out.len(), 1);
        assert!(out[0].body.starts_with("退出码 3"), "{}", out[0].body);

        // 反过来，关掉任务失败通知就该安静，哪怕服务开关开着。
        let mut seen = SeenState::default();
        let out = plan_notifications(
            std::slice::from_ref(&running),
            std::slice::from_ref(&failed),
            &settings(false, true),
            &mut seen,
        );
        assert!(out.is_empty(), "{out:?}");
    }

    /// M6: the user pressed 停止 / 重启. One scan later the pid has changed
    /// and the newest run says `Stopped` — rule 2 must not call that a
    /// crash, and must stay quiet afterwards too.
    #[test]
    fn a_user_restart_does_not_look_like_a_crash_to_rule_2() {
        let mut seen = SeenState::default();
        let mut running = task("bridge", true, true, true);
        running.loaded_pid = Some(100);
        running.last_run = Some(run(1, true, None, None));

        // The stop landed and launchd already has the new process, but the
        // new run row is not written yet: `last_run` is still the stopped one.
        let mut restarted = running.clone();
        restarted.loaded_pid = Some(200);
        restarted.last_run = Some(run(1, false, None, Some(StopReason::Stopped)));

        let out = plan_notifications(
            std::slice::from_ref(&running),
            std::slice::from_ref(&restarted),
            &settings(true, true),
            &mut seen,
        );
        assert!(out.is_empty(), "用户自己重启的服务不该报崩溃: {out:?}");

        // And the scan after that, once the new run row shows up.
        let mut settled = restarted.clone();
        settled.last_run = Some(run(2, true, None, None));
        let out = plan_notifications(
            std::slice::from_ref(&restarted),
            std::slice::from_ref(&settled),
            &settings(true, true),
            &mut seen,
        );
        assert!(out.is_empty(), "{out:?}");
    }

    /// L3: a task that is gone leaves nothing behind in the dedup map.
    #[test]
    fn seen_state_forgets_tasks_that_are_no_longer_there() {
        let mut seen = SeenState::default();
        let mut a = task("a", false, false, true);
        a.last_run = Some(run(1, false, Some(0), Some(StopReason::Exited)));
        let b = task("b", false, false, true);

        let both = vec![a.clone(), b.clone()];
        plan_notifications(&both, &both, &settings(true, true), &mut seen);
        assert_eq!(seen.last_run_id.len(), 1, "只有 a 有终态 run");

        // a is deleted; the next scan only carries b.
        let only_b = vec![b];
        plan_notifications(&both, &only_b, &settings(true, true), &mut seen);
        assert!(seen.last_run_id.is_empty(), "{:?}", seen.last_run_id);
    }

    #[test]
    fn a_service_without_keep_alive_never_matches_rule_2() {
        let mut seen = SeenState::default();
        let mut running = task("mail", true, false, true);
        running.loaded_pid = Some(100);
        running.last_run = Some(run(1, true, None, None));
        // Restarted manually by the user (start again after stop): a
        // different pid, but no KeepAlive, so this is not a crash.
        let mut again = running.clone();
        again.loaded_pid = Some(300);
        again.last_run = Some(run(3, true, None, None));

        let out = plan_notifications(
            std::slice::from_ref(&running),
            std::slice::from_ref(&again),
            &settings(true, true),
            &mut seen,
        );
        assert!(out.is_empty());
    }
}
