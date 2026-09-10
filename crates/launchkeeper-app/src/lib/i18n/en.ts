// English dictionary (PRD M3 item 5, `docs/M3-design.md` §5).
//
// Typed as `Record<TranslationKey, string>` against `zh-CN.ts`, so a missing
// or misspelled key is a compile error rather than a blank label someone
// notices in a screenshot months later.
//
// Length matters here: several of these land in fixed-width buttons, table
// headers and a 320 px list column, so the short ones stay short ("Run now",
// not "Run this task immediately"). Where English needs a plural and Chinese
// does not, the `*_one` / `*_other` pair carries it.

import type { TranslationKey } from "./zh-CN";

export const en: Record<TranslationKey, string> = {
  // ---- Common --------------------------------------------------------
  "common.loading": "Loading…",
  "common.close": "Close",
  "common.cancel": "Cancel",
  "common.ok": "OK",
  "common.save": "Save",
  "common.saving": "Saving…",
  "common.refresh": "Refresh",
  "common.remove": "Remove",
  "common.placeholder": "—",

  // ---- Top bar -------------------------------------------------------
  "topbar.search_placeholder": "Search tasks…",
  "topbar.refresh_title": "Reload the task list and the other LaunchAgents",
  "topbar.new_task": "New Task",
  "topbar.settings": "Settings",

  // ---- Task list -----------------------------------------------------
  "list.empty.title": "No tasks yet",
  "list.empty.hint": "Click “New Task” in the top right to start",
  "list.empty.no_match": "No matching tasks",
  "list.group.managed": "Managed by Launchkeeper",
  "list.group.external": "Other LaunchAgents",
  // 侧栏只有 320 px，这一行还要放下分组标题和「刷新」：写全"read-only,
  // adoptable"会换行，而每一行自己带着「Adopt」按钮，可接管这件事说得已经很清楚。
  "list.group.external_note": "· read-only",
  "list.group.rescan_title": "Rescan ~/Library/LaunchAgents",
  "list.group.scanning": "Scanning…",
  "list.group.more_collapsed": "{n} more that cannot be adopted",
  "list.group.more_expand": "Expand",
  "list.group.more_collapse": "Collapse",
  "list.group.more_help":
    "These plists use triggers or keys Launchkeeper cannot model yet (WatchPaths, Sockets and the like), or their program lives inside an app bundle, /Library or a system directory. They stay read-only: you can enable, disable and run them, but not adopt them.",
  "list.group.count": "{n} total",
  "list.group.count_filtered": "{n} of {total}",

  // ---- List sorting (M4 §2) ------------------------------------------
  "list.sort.label": "Sort by",
  "list.sort.title": "Choose how this group is sorted",
  // 这四条挤在 320 px 侧栏的分组标题里，所以只写排序键本身，不重复
  // "Sort by"——那句话已经在 aria-label 和 tooltip 上了。
  "list.sort.name": "Name",
  "list.sort.last_run": "Last run",
  "list.sort.status": "Status",
  "list.sort.next_run": "Next run",

  // ---- Status dot tooltips -------------------------------------------
  "status.ok": "Last run succeeded",
  "status.failed": "Last run failed",
  "status.running": "Running",
  "status.never": "Never run",
  "status.disabled": "Not enabled",
  "status.stopped": "Stopped",

  // ---- Task row ------------------------------------------------------
  "row.uptime": "Up {duration}",
  "row.pid": "pid {pid}",
  "row.never_run": "Never run",
  "row.start_service": "Start service",
  "row.stop_service": "Stop service",
  "row.disabled_badge": "Disabled",
  "row.toggle_on": "Enabled — click to disable",
  "row.toggle_off": "Disabled — click to enable",

  // ---- Context menu --------------------------------------------------
  "menu.run_now": "Run now",
  "menu.start": "Start",
  "menu.stop": "Stop",
  "menu.restart": "Restart",
  "menu.edit": "Edit",
  "menu.enable": "Enable",
  "menu.disable": "Disable",
  "menu.reveal_script": "Show script in Finder",
  "menu.reveal_plist": "Show plist in Finder",
  "menu.adopt": "Adopt",
  "menu.delete": "Delete",

  // ---- Confirmation dialogs ------------------------------------------
  "confirm.unsaved.title": "Unsaved changes",
  "confirm.unsaved.create": "This new task has not been saved. Discard it?",
  "confirm.unsaved.edit":
    "This task has changes that have not been saved. Discard them?",
  "confirm.unsaved.discard": "Discard",
  "confirm.delete.title": "Delete task",
  "confirm.delete.message":
    "Delete “{name}”? This also removes its launchd job, its database rows and its logs, and cannot be undone.",
  "confirm.unadopt.title": "Undo adoption",
  "confirm.unadopt.message":
    "This restores {path} byte for byte to what it was before the adoption and deletes the task from Launchkeeper (its run history goes with it; the log files stay). Anything you changed about it in Launchkeeper since — trigger, environment variables, timeout — is discarded with it, and the LaunchAgent keeps running under its original definition.",
  "confirm.unadopt.confirm": "Undo adoption",
  "confirm.unadopt.fallback_path": "the original plist",

  // ---- Task detail ---------------------------------------------------
  "detail.create_title": "New task",
  "detail.empty": "Select a task on the left to see its details",
  "detail.adopted_from": "Adopted from",
  "detail.unknown_path": "(unknown path)",
  "detail.undo_adopt": "Undo adoption",
  "detail.tab.config": "Configuration",
  "detail.tab.history": "Run history",
  "detail.edit": "Edit",
  "detail.edit_title": "Edit this task's configuration",

  // ---- Read-only summary on the configuration tab --------------------
  "summary.none": "None",
  "summary.working_dir_default": "$HOME (not set)",
  "summary.log_dir": "Log directory",
  "summary.start_at_login": "Start at login",
  "summary.start_at_login_note": "= registered with launchd",
  "summary.on": "On",
  "summary.off": "Off",
  "summary.keep_alive_suffix": " · KeepAlive",

  // ---- Run history ---------------------------------------------------
  "history.empty": "No runs yet",
  "history.col.id": "#",
  "history.col.started": "Started",
  "history.col.duration": "Duration",
  "history.col.result": "Result",
  "history.col.stop": "Ended by",
  "history.col.source": "Source",
  "history.source.manual": "Manual",
  "history.source.scheduled": "Scheduled",
  "history.stdout": "Standard output",
  "history.stderr": "Standard error",

  // ---- Log view ------------------------------------------------------
  "log.truncated":
    "Showing the last {tail} bytes ({total} bytes in the file, truncated)",
  "log.empty": "(empty)",

  // ---- Task form -----------------------------------------------------
  "form.name": "Task name (unique id)",
  "form.name_placeholder": "e.g. report-sync",
  "form.display_name": "Display name",
  "form.display_name_placeholder": "A name for people",
  "form.description": "Description",
  "form.script": "Script",
  "form.script_placeholder": "/absolute/path/to/script.py",
  "form.run_with": "Run with",
  "form.run_with_manual": "Chosen by hand",
  "form.interp_args": "Interpreter arguments: {args}",
  "form.args": "Arguments",
  "form.args_add": "+ Add argument",
  "form.working_dir": "Working directory",
  "form.working_dir_placeholder": "Empty means $HOME",
  "form.env": "Environment variables",
  "form.env_key_placeholder": "KEY",
  "form.env_value_placeholder": "value",
  "form.env_add": "+ Add variable",
  "form.trigger": "Trigger",
  "form.keep_alive":
    "Keep alive (KeepAlive — only for “At login” and “Manual start/stop”)",
  "form.keep_alive_hint":
    "Restarted automatically after a crash, but not after you stop it; note that once this is checked, enabling the task starts it.",
  "form.timeout": "Timeout",
  "form.timeout_check": "Limit how long one run may take",
  "form.timeout_none": "No limit",
  "form.tags": "Tags",
  "form.tag_placeholder": "Type and press Enter",
  "form.favorite": "Favourite (show in the menu bar)",
  "form.notify_on_fail": "Notify on failure",
  "form.error.name_required": "The task name cannot be empty",
  "form.error.script_required": "The script path cannot be empty",
  "form.error.script_absolute":
    "The script must be an absolute path (starting with /)",
  "form.error.timeout_number": "The timeout must be a number greater than 0",
  "form.error.script_missing": "No such script: {path}",
  "form.error.not_executable":
    "Pick how to run it: with nothing selected launchd executes this file itself, and it is not executable",
  "form.error.interval_number": "The interval must be a number greater than 0",
  "form.error.hour_range": "The hour must be a whole number from 0 to 23",
  "form.error.minute_range": "The minute must be a whole number from 0 to 59",
  "form.error.day_range":
    "The day must be a whole number from 1 to 31, or left empty",

  // ---- Time units ----------------------------------------------------
  "unit.seconds": "seconds",
  "unit.minutes": "minutes",
  "unit.hours": "hours",

  // ---- Trigger editor ------------------------------------------------
  "trigger.at_login": "At login",
  "trigger.interval": "Repeat on an interval",
  "trigger.calendar": "At calendar times",
  "trigger.manual": "Manual start/stop (service)",
  "trigger.manual_hint":
    "No automatic trigger at all: the task runs when you press “Start” and ends when you press “Stop” — for bridges, listeners and other processes meant to stay up.",
  "trigger.every": "Run once every",
  // Chinese puts “运行一次” after the unit; English says it up front, so this
  // trailing half of the sentence is deliberately empty here.
  "trigger.run_once": "",
  "trigger.hour_suffix": "h",
  "trigger.minute_suffix": "min",
  "trigger.weekday_hint": "none selected = every day",
  "trigger.monthly_prefix": "On day",
  "trigger.monthly_suffix": "of the month (empty = any day)",
  "trigger.day_placeholder": "any",
  "trigger.add_entry": "+ Add time",
  "weekday.short.0": "S",
  "weekday.short.1": "M",
  "weekday.short.2": "T",
  "weekday.short.3": "W",
  "weekday.short.4": "T",
  "weekday.short.5": "F",
  "weekday.short.6": "S",

  // ---- Interpreter picker --------------------------------------------
  "interp.scanning": "Scanning interpreters…",
  "interp.choose": "Choose how to run it",
  "interp.group.recommended": "Recommended",
  "interp.group.project": "In this project",
  "interp.group.project_empty":
    "In this project · no .venv or uv.lock here",
  "interp.group.path": "Other interpreters on PATH",
  "interp.custom": "Custom path…",
  "interp.custom_placeholder": "/absolute/path/to/interpreter",
  "interp.custom_confirm": "OK",
  "interp.badge_default": "Default",
  "interp.direct": "Run directly",
  "interp.script_itself": "the script itself",

  // ---- Settings ------------------------------------------------------
  "settings.title": "Settings",
  "settings.language": "Language",
  "settings.language.system": "Follow system",
  // The two language names stay in their own language in both dictionaries:
  // someone who opened the app in the wrong one has to find their way back.
  "settings.language.zh": "中文",
  "settings.language.en": "English",
  "settings.data_dir": "Data directory",
  "settings.notify": "Failure notifications",
  "settings.notify.task": "Notify when a scheduled task fails",
  "settings.notify.service":
    "Notify when a service crashes and is restarted",
  "settings.notify.hint":
    "Still only applies to tasks with “Notify on failure” checked; this is the master switch, the one in a task’s own form is that task’s.",
  "settings.menubar": "Menu bar",
  "settings.menubar.login": "Start Launchkeeper at login",
  "settings.history_limit": "Runs kept in history",
  "settings.coming_soon": "Coming soon",
  "settings.load_failed": "Could not load the settings",

  // M4 AI / theme -------------------------------------------------------
  // ---- Appearance (M4 §4) --------------------------------------------
  "settings.theme": "Appearance",
  "settings.theme.system": "System",
  "settings.theme.light": "Light",
  "settings.theme.dark": "Dark",
  // ---- AI (M4 §1 / §4) -----------------------------------------------
  "settings.ai": "AI",
  "settings.ai.hint":
    "「AI insight」 uses the model configured here. A task's environment variables are sent by name only — the values never leave this machine.",
  "settings.ai.provider": "Provider",
  "settings.ai.provider.anthropic": "Anthropic",
  "settings.ai.provider.openai": "OpenAI-compatible",
  "settings.ai.base_url": "Base URL",
  "settings.ai.base_url.hint": "Leave empty for the provider's default endpoint",
  "settings.ai.model": "Model",
  "settings.ai.key": "API key",
  "settings.ai.key.placeholder": "Paste a key; it is never shown again",
  "settings.ai.key.saved": "Saved",
  "settings.ai.key.missing": "Not set",
  "settings.ai.key.save": "Save",
  "settings.ai.key.clear": "Clear",
  "settings.ai.key.hint":
    "Kept in the ai-key file in the data directory (readable only by you). It never appears in command output, logs or plists.",
  "settings.ai.test": "Test connection",
  "settings.ai.testing": "Testing…",
  "settings.ai.test_ok": "Connected: {reply}",
  // ---- Prompt for your AI assistant (M4 §4) --------------------------
  "settings.prompt": "Prompt for your AI assistant",
  "settings.prompt.hint":
    "Copy this to the AI assistant in your terminal so it knows launchkeeper is installed here and how to drive it.",
  "settings.prompt.copy": "Copy",
  "settings.prompt.copied": "Copied to the clipboard",
  "settings.prompt.copy_failed": "Could not copy: {message}",
  // ---- AI insight tab (M4 §1 / §4) -----------------------------------
  "detail.tab.insight": "AI insight",
  "insight.explain": "Explain",
  "insight.refresh": "Explain again",
  "insight.running": "Explaining…",
  "insight.meta": "{time} · {model}",
  "insight.empty.title": "This task has not been explained yet",
  "insight.empty.body":
    "Press Explain to send the task's definition, its last 10 runs and the tail of its most recent log to the model, and have it say what this task does, how healthy it has been, and what a failure was likely about.",
  "insight.empty.kept":
    "The answer is stored and shown straight away next time you open this tab, until you press Explain again.",
  "insight.empty.privacy":
    "Environment variables are sent by name only — never their values — and at most the last 16 KiB of the log.",
  "insight.open_settings": "Open settings",

  // ---- Adoption dialog -----------------------------------------------
  "adopt.title": "Adopt {label}",
  "adopt.lead_before":
    "The name and the file location stay the same. The changes below will be made; the original file is backed up as",
  "adopt.lead_after": ", and this can be undone at any time.",
  "adopt.loading": "Reading the plist…",
  "adopt.reload_check": "Reload right after adopting (bootout + bootstrap)",
  "adopt.no_reload_hint":
    "Without reloading: the file has changed, but launchd keeps running the old definition until the next login.",
  "adopt.busy": "Adopting…",
  "adopt.confirm_reload": "Adopt and reload",
  "adopt.confirm": "Adopt",

  // ---- Other LaunchAgents --------------------------------------------
  "external.subtitle.managed": "Managed by Launchkeeper",
  "external.subtitle.not_adoptable": "{reason} · shown only",
  "external.subtitle.adoptable": "Hand-written plist · no run history",
  "external.reason.fallback": "Cannot be adopted",
  "external.why_not_adoptable": "Why this cannot be adopted",
  "external.status.unloaded": "Not loaded into launchd",
  "external.status.running": "Running · pid {pid}",
  "external.status.loaded": "Loaded, waiting for its trigger",
  "external.headline.managed": "Managed by Launchkeeper",
  "external.headline.handwritten": "Hand-written plist",
  "external.headline.loaded": "Loaded",
  "external.headline.unloaded": "Not loaded",
  "external.log.merged": "{path} (merged)",
  "external.log.split": "{out} · stderr {err}",
  "external.log.err_only": "stderr {err}",
  "external.log.none": "Not set (launchd throws the output away)",
  "external.adopt.managed_tip":
    "This plist is already managed by Launchkeeper",
  "external.adopt.tip":
    "Bring it under Launchkeeper (same name and location; the original is backed up as .bak)",
  "external.diff.kept": "{keys} unchanged",
  "external.diff.join": ", ",
  "external.run_disabled_tip": "Not loaded into launchd — enable it first",
  "external.enable_tip":
    "Affects launchd only; the plist file is not touched",
  "external.detail.run_tip": "launchctl kickstart — no run history is kept",
  "external.detail.run_disabled_tip":
    "Not loaded into launchd — enable it first",
  "external.detail.lead":
    "This LaunchAgent was not created by Launchkeeper. You can enable, disable or run it as it is, but there is no run history and there are no logs. Adopting it gives you both, with the same name and in the same place.",
  "external.detail.program": "Program",
  "external.detail.trigger": "Trigger",
  "external.detail.working_dir": "Working directory",
  "external.detail.log": "Logs",
  "external.detail.plist": "plist",
  "external.detail.launchd": "launchd status",
  "external.detail.working_dir_unset": "Not set ($HOME)",
  "external.detail.last_exit": "last exit code {code}",

  // ---- Times, durations and exit codes (lib/format.ts) ----------------
  "fmt.rel.now": "just now",
  "fmt.rel.second_one": "{n} second ago",
  "fmt.rel.second_other": "{n} seconds ago",
  "fmt.rel.minute_one": "{n} minute ago",
  "fmt.rel.minute_other": "{n} minutes ago",
  "fmt.rel.hour_one": "{n} hour ago",
  "fmt.rel.hour_other": "{n} hours ago",
  "fmt.rel.day_one": "{n} day ago",
  "fmt.rel.day_other": "{n} days ago",
  "fmt.rel.month_one": "{n} month ago",
  "fmt.rel.month_other": "{n} months ago",
  "fmt.rel.year_one": "{n} year ago",
  "fmt.rel.year_other": "{n} years ago",
  "fmt.dur.ms": "{n} ms",
  "fmt.dur.sec": "{n} s",
  "fmt.dur.min_sec": "{m} min {s} s",
  "fmt.dur.hour_min": "{h} h {m} min",
  "fmt.up.sec": "{s} s",
  "fmt.up.min_sec": "{m} min {s} s",
  "fmt.up.hour_min": "{h} h {m} min",
  "fmt.up.day_hour": "{d} d {h} h",
  "fmt.stop.stopped": "Stopped by hand",
  "fmt.stop.timeout": "Timed out",
  "fmt.stop.timeout_killed": "Timed out and was terminated",
  "fmt.exit.running": "Running…",
  "fmt.exit.signal": "Killed by an external signal (not Launchkeeper: logout, kill, system)",
  "fmt.exit.success": "Succeeded",
  "fmt.exit.failed": "Failed (exit code {code})",
  // M4 CLI docs / external disable
  "confirm.external_disable.title": "Disable another product's LaunchAgent?",
  "confirm.external_disable.message":
    "{label} is a LaunchAgent installed by other software (an updater, a sync client, a background helper); Launchkeeper does not manage it. Why it cannot be adopted: {reason}.\nDisabling it may stop that software updating or syncing, or stop it working at all.\nThe plist file is not modified — the job is only unloaded from launchd, and you can enable it again at any time.",
  "confirm.external_disable.confirm": "Disable anyway",
  // M3 about
  "settings.about": "About",
  "settings.about.version": "Version {version}",
  "settings.about.tagline": "Scheduled tasks and resident services on macOS, run by launchd.",
  "settings.about.repo": "GitHub repository",
  "settings.about.issues": "Report an issue",
  "settings.about.coffee": "Buy me a coffee ☕",
  "settings.about.open_failed": "Could not open link: {error}",
  // interpreter origins
  "interp.origin.system": "system",
  "interp.origin.homebrew": "Homebrew",
  "interp.origin.project_venv": "project .venv",
  "interp.origin.uv": "installed by uv",
  "interp.origin.user_local": "user install",
  "interp.origin.path": "PATH",
  "interp.origin.custom": "custom",
  "external.trigger_unsupported": "unsupported trigger",
};
