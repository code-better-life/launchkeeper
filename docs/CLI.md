# `launchkeeper` reference

`launchkeeper` is a first-class, script- and AI-agent-friendly interface
to `launchkeeper-core`. Every command that mutates state, plus `list`,
`show`, `runs`, `status`, `logs`, `interpreters`, `explain` and `ai config`,
has a stable `--json` shape so a script or an AI agent never has to scrape
human-formatted text.

The binary is called `launchkeeper` (the crate itself is still named
`launchkeeper-cli`, at `crates/launchkeeper-cli/`, since renaming the crate
would also rename its `CARGO_BIN_EXE_*` env var and every path under
`target/`; only the `[[bin]]` name changed). If you'd rather type something
shorter than `launchkeeper`, see [Short alias](#short-alias) below — `lk` is
already taken on crates.io/npm, so the recommended alias is `lkp`.

Design contract: `docs/M1-design.md` §10 (human design notes) and this file
(the wire contract).

## Global options

These apply to every subcommand (clap `global = true`):

- `--runner <PATH>` — path to the `launchkeeper-runner` binary. Overrides
  `LAUNCHKEEPER_RUNNER` and the sibling-binary lookup.
- `--data-dir <PATH>` — overrides where Launchkeeper keeps its database and
  logs. Overrides `LAUNCHKEEPER_DATA_DIR`. See
  [Data directory discovery](#data-directory-discovery) for the full
  precedence and `config data-dir` for a way to set this without an env var.
- `--json` / `-j` — print machine-readable JSON instead of human text. No-op
  for `plist`, which always prints raw XML regardless of this flag.

## Subcommand aliases

A handful of the more frequently typed subcommands have a short alias (clap
`alias`, not a shell alias — these work no matter how you invoke the
binary):

| Command | Alias |
|---|---|
| `list` | `ls` |
| `remove` | `rm` |
| `enable` | `on` |
| `disable` | `off` |
| `status` | `st` |
| `logs` | `log` |
| `set-trigger` | `trigger` |

`runs` deliberately has **no** alias (`rs`/`hist` would be one more thing to
remember for a command that is already short and used less often than
`logs`).

**Watch the short flags:** `-e` and `-d` are `--every` and `--daily` (see
[the trigger flag grammar](#the-trigger-flag-grammar)), *not* `--env` and
`--data-dir`. Neither `--env` nor `--data-dir` has a short form, on purpose:
`-e 30m` and `-e FOO=bar` would be impossible to tell apart at a glance, and
`--data-dir` is the one flag where guessing wrong means writing to the wrong
database. Type those two out in full.

## Exit codes

- `0` — success.
- `1` — a runtime error (task not found, validation failure, launchctl
  error, I/O error, ...). In `--json` mode, mutating commands print
  `{"ok": false, "error": "<message>"}` to stdout instead of a human error
  line on stderr, then exit `1`.
- `2` — a `clap` usage error (bad flags, missing required argument, unknown
  subcommand). Printed by `clap` itself to stderr; `--json` has no effect on
  this path since the command was never dispatched.

## Environment variables

- `LAUNCHKEEPER_DATA_DIR` — overrides `~/Library/Application
  Support/Launchkeeper/`, where the SQLite database and task log directories
  live. Read by `launchkeeper_core::paths::data_dir`. `--data-dir` on the CLI
  sets this variable internally before any core call, so the two are
  equivalent; `--data-dir` wins if both are given. See
  [Data directory discovery](#data-directory-discovery) for the full
  precedence, including the lower-priority config-file source below this
  variable.
- `LAUNCHKEEPER_RUNNER` — path to the `launchkeeper-runner` binary to install
  and bake into generated plists. Only consulted when `--runner` is not
  given. If neither is set, the CLI looks for `launchkeeper-runner` next to
  its own executable, and fails with a clear message if that is missing too.
  Read directly in `crates/launchkeeper-cli/src/main.rs` (`resolve_runner_source`).

- `LAUNCHKEEPER_AI_API_KEY` — the API key for [`explain`](#explain-name---refresh---lang-tag).
  **Outranks the key file**: when it is set and non-blank, `<data_dir>/ai-key`
  is not consulted at all. That is what makes `explain` usable from CI or a
  headless session (and what keeps this repository's own tests away from
  the real data dir).
  A blank value counts as "not set", so `export LAUNCHKEEPER_AI_API_KEY=` does
  not shadow a stored key with nothing.

Two more core-level variables exist for testing but are not part of the
day-to-day CLI contract: `LAUNCHKEEPER_LAUNCH_AGENTS_DIR` (overrides
`~/Library/LaunchAgents/` — also the directory `agents`/`adopt` scan) and
`LAUNCHKEEPER_RUNNER_LOG_DIR` (overrides `~/Library/Logs/Launchkeeper/`,
where launchd itself writes runner stdout/stderr).

## Data directory discovery

`launchkeeper_core::paths::data_dir()` (used for the SQLite database, task
logs, the installed runner copy and the login-shell `PATH` cache) resolves in
this order:

1. `--data-dir <PATH>` on the command line, or the `LAUNCHKEEPER_DATA_DIR`
   environment variable (the CLI sets the env var from `--data-dir` before
   any core call, so the two are equivalent — `--data-dir` simply wins if
   both are given).
2. `~/.config/launchkeeper/data-dir` (respects `XDG_CONFIG_HOME`, i.e.
   `$XDG_CONFIG_HOME/launchkeeper/data-dir` when that variable is set and
   non-empty) — a plain text file with the data directory path on its first
   line, trimmed. Missing or blank is treated as absent, not an error.
3. The default, `~/Library/Application Support/Launchkeeper/`.

`launchkeeper config data-dir [<path>]` reads and writes step 2:

- No argument: prints the currently resolved data directory and which of
  the three sources produced it (`env` / `config_file` / `default`).
  `--json`: `{"data_dir": "...", "source": "...", "config_file": "..."}`
  (a plain read, no `ok` envelope — same convention as `show`).
- With a path: writes it to `~/.config/launchkeeper/data-dir` (creating the
  directory as needed), resolved the same way `--script`/`--cwd` are (cwd
  join + lexical `.`/`..` normalization, no symlink resolution, and the path
  need not already exist). `--json`: the usual
  `{"ok": true, "data_dir": "...", "config_file": "..."}` /
  `{"ok": false, "error": "..."}` mutation envelope. The success shape also
  carries `"warning": "..."` — and only then — when `LAUNCHKEEPER_DATA_DIR`
  is set in the environment and therefore outranks the file that was just
  written.

This exists for machines where exporting `LAUNCHKEEPER_DATA_DIR` from the
shell rc is inconvenient (see `CLAUDE.local.md` for this repo's own external-disk
setup) — write the file once with `config data-dir <path>` instead of adding
an `export` line. `LAUNCHKEEPER_DATA_DIR`, if still set in the environment,
always wins over the file.

## The trigger flag grammar

Shared by `add` and `set-trigger` (a required, mutually exclusive group —
exactly one must be given):

| Flag | Short | Grammar | launchd shape | Example |
|---|---|---|---|---|
| `--at-login` | | (no value) | `RunAtLoad` | `--at-login` |
| `--every <DURATION>` | `-e` | `<n>h`, `<n>m`, `<n>s`, or a concatenation like `1h30m`, `1h30m10s` | `StartInterval` (seconds) | `-e 30m` |
| `--daily <HH:MM>` | `-d` | 24-hour clock; repeatable for several times a day | `StartCalendarInterval` entries with hour+minute only | `-d 09:00 -d 21:30` |
| `--weekly <weekday[,weekday...]@HH:MM>` | `-w` | weekday is `mon`..`sun` (English 3-letter, case-insensitive) or launchd's numeric `0`-`7` (0 and 7 both mean Sunday); repeatable | one `StartCalendarInterval` entry per weekday | `-w mon,fri@09:30` |
| `--monthly <day@HH:MM>` | `-M` (capital — `-m` is `--manual`) | day is `1`-`31`; repeatable | one `StartCalendarInterval` entry with `day` set | `-M 15@08:00` |
| `--manual` | `-m` | (no value) | none of the three trigger keys — the job only runs when you `start` it | `-m --keep-alive` |

`--timeout <DURATION>` (on `add` and `set`) uses the same duration grammar as
`--every`. A run that exceeds it is killed and recorded with exit code 124.

`--keep-alive` maps to launchd `KeepAlive` and is only valid together with
`--at-login` or `--manual` — combining it with a timed trigger is rejected by
`Task::validate` because launchd would restart the job the instant the
script exits, defeating the schedule.

### Service tasks (`--manual`)

A `--manual` task is a **service**: no schedule, started and stopped by hand
with `start` / `stop` / `restart`. `is_service` is `true` for it in every
JSON shape.

`--manual --keep-alive` writes `KeepAlive = {SuccessfulExit = false}` instead
of a plain `KeepAlive = true`, which means:

- a crash or a signal (non-zero / non-exit) → launchd restarts the job, at
  most once every `ThrottleInterval` (60 s);
- a clean exit 0 → launchd leaves it down. That is what `stop` relies on: it
  sends SIGTERM, the runner forwards it to the script and then exits 0
  itself, so the service stays stopped instead of bouncing right back.

One consequence measured on real launchd (`docs/M2.5-design.md` §3.1) and
worth knowing before you script this: **`enable` on a `--manual --keep-alive`
task starts it immediately**, because any form of `KeepAlive` makes launchd
start the job at `bootstrap` time. Without `--keep-alive`, `enable` only
registers the job and nothing runs until `start`.

## Commands

For every subcommand's exact flag list, run `launchkeeper <cmd> --help`
— this section mirrors that output but the `--help` text is the source of
truth if they ever drift.

### `add <name> --script <path> <trigger...> [options]`

Registers a new task. Does **not** touch launchd — call `enable` afterwards.

Flags: `--script <PATH>` (required), `--arg <S>` (repeatable), `--interpreter
<ID|PATH|direct>`, `--raw`, `--cwd <PATH>`, `--env <K=V>` (repeatable), one
trigger flag (required, see grammar above), `--display-name <S>`,
`--description <S>`, `--tag <S>` (repeatable), `--keep-alive`, `--timeout
<DURATION>`.

`--script` is resolved to an absolute path without following symlinks (cwd
join + lexical `.`/`..` normalization) and must exist at add time.

#### Interpreters (M3)

`--script` names the **script**, not necessarily the program launchd execs.
`--arg` is the script's own arguments. The CLI resolves how to actually run
it — see `launchkeeper_core::interpreters` (`docs/M3-design.md` §1) for the
full scan/recommendation rules; in short, by extension then by shebang, with
a project's own `.venv`/`uv.lock` beating a bare `python3`.

- No `--interpreter`/`--raw`: the CLI runs
  `interpreters::scan(Some(script), cwd_or_script_dir, effective_path, false)`
  and picks the `recommended` candidate. If nothing is recommended and the
  script is not itself executable, `add` fails with the candidate list —
  same list `interpreters` below prints — and a pointer at `--interpreter`.
  A script that **is** directly executable (with or without a recognized
  shebang) falls back to running itself, which is what keeps a plain
  `--script /bin/echo`-style invocation working with no extra flags.
- `--interpreter <ID>` — an exact candidate `id` from `launchkeeper
  interpreters` (e.g. `/usr/bin/python3`, or `/Users/x/.local/bin/uv run`
  for `uv` with its `run` prefix baked in). `--interpreter <PATH>` — any
  other existing path, used as a bare program with no prefix args.
  `--interpreter direct` — skip interpreters entirely and have launchd exec
  the script itself; fails if the script is not executable.
- `--raw` — the pre-M3 behavior: `--script`/`--arg` are stored exactly as
  given, no resolution attempted. This is how tasks created before M3 (and
  any task where the "script" is really an opaque command line, like `uv run
  some/thing.py --flag`) keep working: `--script ~/.local/bin/uv --arg run
  --arg scripts/sync_cloud.py --raw`. Conflicts with `--interpreter`.
  `show`/`list`/`--json`'s `interpreter_label` still labels a `--raw` task
  correctly (e.g. `uv run · uv 安装`) — `interpreters::detect_from_task`
  decomposes `script_path`/`args` after the fact regardless of how they got
  there.

Composition (non-`--raw`) goes through `interpreters::apply`: the stored
`script_path`/`args` become the resolved interpreter's program plus its
prefix args, the script, then the script's own args — e.g. `--script
sync.py --arg --once` with `uv run` recommended stores `script_path:
/Users/x/.local/bin/uv`, `args: ["run", "/abs/sync.py", "--once"]`.

Human mode prints the pick on success, e.g.:

```
运行方式: python3 · 3.9.6 · 系统 /usr/bin/python3（按 .py 扩展名推荐）
```

JSON (`--json`): `{"ok": true, "task": <TaskJson>}` on success,
`{"ok": false, "error": "<message>"}` on failure (e.g. duplicate name,
invalid trigger, `keep_alive` without `--at-login`, or no interpreter could
be resolved).

```console
$ launchkeeper --data-dir /tmp/lk --runner /usr/bin/true \
    add nightly-sync --script /usr/local/bin/sync.sh --daily 21:00 --json
{
  "ok": true,
  "task": {
    "name": "nightly-sync",
    "display_name": "nightly-sync",
    "description": null,
    "script_path": "/usr/local/bin/sync.sh",
    "args": [],
    "script": "/usr/local/bin/sync.sh",
    "script_args": [],
    "interpreter_label": "直接执行",
    "working_dir": null,
    "env": {},
    "trigger": {"kind": "calendar", "entries": [{"minute": 0, "hour": 21, "weekday": null, "day": null}]},
    "trigger_text": "每天 21:00",
    "keep_alive": false,
    "is_service": false,
    "timeout_secs": null,
    "tags": [],
    "favorite": false,
    "notify_on_fail": true,
    "created_at": "2026-01-01T00:00:00.000Z",
    "updated_at": "2026-01-01T00:00:00.000Z",
    "enabled": false,
    "loaded": false,
    "pid": null,
    "uptime_secs": null,
    "last_run": null
  }
}
```

`/usr/local/bin/sync.sh` has no recognized shebang here, so nothing was
`recommended` and the script's own execute bit decided it: `script_path`
(what launchd execs) and `script` (the decomposed "real" script) are the
same path, and `interpreter_label` reads `直接执行`. A `.py` script picked up
by `python3` would instead show `script_path: ".../python3"`, `script:
"/abs/path/to/it.py"`, `interpreter_label: "python3 · 系统"`.

### `set <name> [options]`

Edits fields of an existing task in place, without re-adding it. If the task
is currently enabled, `Service::update_task` rewrites the plist and reloads
it into launchd (bootout + bootstrap) as part of the same call; a launchd
failure leaves the database row untouched.

Flags (all optional; only the ones you pass change anything):

- `--display-name <S>`, `--description <S>` — overwrite.
- `--script <PATH>` — overwrite (same absolute-path resolution as `add`).
- `--arg <S>` (repeatable) — **replaces** the script's own argument list if
  given at all (old arguments are cleared first, not appended to). Omit
  entirely to leave the current arguments untouched.
- `--interpreter <ID|PATH|direct>` / `--raw` — same grammar and mutual
  exclusion as `add`. Touching any of `--script`/`--arg`/`--interpreter`
  (and not `--raw`) re-resolves and re-composes `script_path`/`args` the same
  way `add` does (`interpreters::scan`/`apply`); whichever of "script",
  "script's own args" or "interpreter" you did *not* pass is kept from what
  the task currently runs (decoded via `detect_from_task`) rather than
  re-guessed — e.g. `set job --arg --once` alone keeps the task's existing
  interpreter and just changes the script's own arguments. `--raw` makes
  `--script`/`--arg` overwrite verbatim, exactly like before this feature
  existed. Prints the same "运行方式: ..." line `add` does when resolution
  actually ran.
- `--cwd <PATH>` — overwrite.
- `--env <K=V>` (repeatable) — **merges**: each key is set/overwritten in the
  existing environment map, other keys are left alone.
- `--unset-env <K>` (repeatable) — removes a key from the environment map.
- `--tag <S>` (repeatable) — **replaces** the full tag list if given at all,
  the same replace-not-merge behavior as `--arg` (chosen for consistency:
  `add` also treats `--tag` as a single flat list built from scratch).
- `--timeout <DURATION>` / `--no-timeout` — set or clear; these two conflict
  with each other.
- `--favorite` / `--no-favorite` — conflict with each other.
- `--notify` / `--no-notify` — conflict with each other.
- `--keep-alive` / `--no-keep-alive` — conflict with each other; the usual
  `keep_alive` + trigger validation still applies after the edit (so
  `--keep-alive` needs the task's trigger to be `--at-login` or `--manual`).

JSON (`--json`): same `{"ok": true/false, ...}` envelope as `add`.

```console
$ launchkeeper --data-dir /tmp/lk --runner /usr/bin/true \
    set nightly-sync --timeout 30m --favorite --json
{"ok": true, "task": {"name": "nightly-sync", "favorite": true, "timeout_secs": 1800, ...}}
```

### `set-trigger <name> <trigger...>`

Changes only the trigger, using the same grammar and mutual-exclusion group
as `add`. Also goes through `Service::update_task`, so an enabled task is
reloaded into launchd automatically.

JSON: same envelope as `add`/`set`.

### `list`

Lists every task.

JSON: an array of `TaskJson` (same shape as `add`'s `task` field, one per
task, in the order `Store::list_tasks` returns them).

```console
$ launchkeeper --data-dir /tmp/lk list --json
[{"name": "nightly-sync", "trigger_text": "每天 21:00", "enabled": true, "loaded": true, "pid": null, "last_run": {"id": 3, "exit_code": 0, ...}, ...}]
```

### `show <name>`

Shows one task in full, including a `运行方式:` line reconstructed from the
stored `script_path`/`args` via `interpreters::detect_from_task` (so it's
accurate for a `--raw` task too, e.g. `运行方式: uv run · uv 安装`). Human mode
additionally prints its last 5 runs as a table; JSON mode only exposes the
single most recent run under `last_run` (use `runs --json` for history).

JSON: a single `TaskJson` object (not wrapped — no `ok`/`task` envelope,
since this is a read, not a mutation). Carries `interpreter_label`/`script`/
`script_args` alongside the raw `script_path`/`args` — see `add` above.

### `enable <name>` / `disable <name> [--yes]`

`enable` writes the plist and `launchctl bootstrap`s it (rebuilding the
login-shell PATH cache along the way). `disable` `bootout`s the job and
deletes the plist, keeping the database row and logs.

JSON: `{"ok": true, "task": <TaskJson>}` with the post-action `enabled`/
`loaded` state, or `{"ok": false, "error": ...}`.

**External labels.** When no task by that name exists but a LaunchAgent with
that *label* does (see [`agents`](#agents)), `enable`/`disable` — and `run`
without `--direct` — act on it through `launchctl` alone: `bootstrap` the
file, `bootout` the label, `kickstart` the label. The plist itself is never
read for writing, so this works on agents Launchkeeper could never adopt. A
task always wins the name lookup, and an adopted task *is* a task (its name
is its label), so it goes down the normal path.

Those three print an `agent` instead of a `task`:
`{"ok": true, "agent": <AgentJson>}` / `{"ok": false, "error": ...}`, where
`AgentJson` is the object [`agents --json`](#agents) documents, re-read after
the action. An external `run` records no run history — there is no runner in
front of the script to record it; `adopt` the agent to get that.

**Disabling a LaunchAgent that belongs to other software.** When the external
label `disable`/`off` resolves to is one Launchkeeper would *refuse* to adopt
(`agents` prints a reason for it: an Apple/Homebrew label prefix, a program
inside an `.app` bundle, `WatchPaths`, ...), that plist was installed by some
other product — an updater, a sync client, a background helper. `off` prints
a warning naming the label, the refusal reason and the plist path, says that
the file is not modified and that `on <label>` puts it back, then asks for
confirmation on stdin. `--yes` / `-y` skips the prompt.

In `--json` mode nothing is ever prompted: without `--yes` the command fails
with `{"ok": false, "error": "... --json 模式下禁用它必须加 --yes"}` and
launchctl is not called at all. Adoptable external agents, and agents
Launchkeeper itself manages, are unaffected — they disable as before.

### `remove <name> [--yes]`

Deletes the task entirely: launchd job (bootout), plist, database row
(cascades to run history), and the log directory.

Without `--yes`, human mode prompts for confirmation on stdin; **in `--json`
mode the prompt is skipped and `--yes` is required** — omitting it is
treated as an error (`{"ok": false, "error": "--json 模式下删除任务必须加 --yes"}`)
so that a non-interactive caller can never get stuck waiting on stdin.

JSON on success: `{"ok": true, "task": <TaskJson>}`, a snapshot of the task
as it was immediately before deletion (so the caller still knows what was
removed).

### `run <name> [--direct]`

Without `--direct`, asks launchd to `kickstart` the job now — the task must
be `enable`d first, or this fails with a message pointing at `--direct` or
`enable`. With `--direct`, execs the runner directly
(`launchkeeper-runner run <name> --manual`), bypassing launchd entirely;
this works even for a disabled task.

Exit-code note: in **human, `--direct`** mode only, the CLI's own exit code
mirrors the script's exit code (for shell scripting convenience), not a flat
`0`/`1`. Every other mode — human non-direct, and **all** `--json` runs —
uses the standard `0` ok / `1` error convention: in `--json --direct` mode a
non-zero script exit becomes `{"ok": false, "error": "脚本以退出码 <n> 结束"}`
with CLI exit code `1`.

JSON on success: `{"ok": true, "task": <TaskJson>}` (the task after the run,
so `last_run` reflects what just happened for `--direct`; for a launchd
`kickstart` the run record may not exist yet since it is asynchronous).

For a `--manual` service, `run` without `--direct` is exactly `start`.

### `start <name>` / `stop <name>` / `restart <name>`

The service verbs. They work on any task, but only really make sense for a
`--manual` one.

- `start` — loads the job into launchd first if it is not loaded yet (so it
  needs a runner binary, same as `enable`), then `launchctl kickstart`s it.
  Starting an already-running job re-kickstarts it; it does not fail.
- `stop` — `launchctl kill TERM`, then polls launchd for up to 8 seconds and
  escalates to `kill KILL` if the process is still there. The job stays
  **loaded** (no `bootout`), so `start` can bring it back without rewriting
  the plist. A task that is not loaded, or loaded but not running, is a
  no-op that still exits `0`.
- `restart` — `stop` then `start`. Deliberately not `kickstart -k`, which
  would kill the runner without letting it shut the script down and record
  the run.

Stopping this way records the run with `stop_reason: "stopped"` and, for a
script that does not handle SIGTERM itself, `exit_code: null`.

JSON: the usual `{"ok": true, "task": <TaskJson>}` / `{"ok": false, "error":
...}` envelope. The `task` is re-read after the action, so `pid`,
`uptime_secs`, `enabled` and `loaded` reflect the new state.

```console
$ launchkeeper start bridge --json | jq '.task | {pid, enabled, uptime_secs}'
$ launchkeeper stop bridge --json  | jq '.ok'
$ launchkeeper runs bridge --limit 1 --json | jq '.[0] | {exit_code, stop_reason}'
{"exit_code": null, "stop_reason": "stopped"}
```

### `logs <name> [--last N] [--err] [--tail N]`

Prints one run's captured output. `--last 1` (default) is the newest run;
`--last 2` the second-newest, etc. `--err` selects stderr instead of stdout.
`--tail N` keeps only the last `N` lines, in both human and `--json` mode.

Human mode without `--tail` writes the raw log bytes verbatim (no UTF-8
assumption). Any other combination (`--tail` given, or `--json`) decodes the
file as UTF-8 (lossily) to be able to count/join lines.

JSON: `{"run_id": <i64>, "stream": "stdout"|"stderr", "path": "<abs path>", "content": "<string>", "truncated": <bool>}`.
`truncated` is `false` unless `--tail` was given **and** it actually cut
content (log shorter than `N` lines: `false`).

```console
$ launchkeeper --data-dir /tmp/lk logs nightly-sync --err --tail 20 --json
{"run_id": 7, "stream": "stderr", "path": "/tmp/lk/logs/nightly-sync/...", "content": "...", "truncated": true}
```

### `runs <name> [--limit N]`

Lists recent runs, newest first, default limit 20.

JSON: an array of `RunJson`:
`{"id": <i64>, "started_at": <RFC3339>, "finished_at": <RFC3339|null>, "exit_code": <i32|null>, "duration_ms": <i64|null>, "trigger_kind": "scheduled"|"manual", "stop_reason": "exited"|"stopped"|"timeout"|null, "running": <bool>, "stdout_path": "<path>", "stderr_path": "<path>"}`.
`duration_ms` and `exit_code` are `null` while `running` is `true`.

`stop_reason` says *why* the run ended, which `exit_code` alone cannot:

| value | meaning |
|---|---|
| `"exited"` | the script finished on its own, whatever its exit code |
| `"stopped"` | `stop`/`restart` (or any SIGTERM/SIGINT to the runner) terminated it |
| `"timeout"` | it hit `--timeout` and was killed; `exit_code` is `124` |
| `null` | still running, or a row written before schema v2 |

The human table shows the same thing in the `STOP` column
(`正常结束` / `手动停止` / `超时` / `-`).

### `plist <name>`

Prints the plist that would be generated for the task, as raw XML, to
stdout. `--json` has **no effect** here — it always prints XML, never a JSON
wrapper — because the whole point of this command is to hand the exact
bytes launchd would load to a human or another tool for inspection.

### `status`

One row per task: `KIND` (`服务` for a `--manual` task, `定时` otherwise),
`enabled` (database + plist state), `loaded` (launchd actually knows the
label right now), `pid`, `UPTIME`, launchd `state`, and the last run's exit
code.

JSON: an array of
`{"name": <str>, "enabled": <bool>, "loaded": <bool>, "is_service": <bool>, "pid": <u32|null>, "uptime_secs": <i64|null>, "state": <str|null>, "last_run": <RunJson|null>}`.
`state` and `pid` are `null` when `loaded` is `false`.

`uptime_secs` is how long the in-flight run has been going — launchd's `pid`
minus the newest run's `started_at`. It is `null` unless launchd reports a
pid *and* the newest run has no `finished_at`, so for a scheduled task it is
almost always `null` and for a running service it is the service's uptime.
`TaskJson` (from `add`/`show`/`list`/...) carries the same field.

### `agents`

Lists the LaunchAgents in `~/Library/LaunchAgents` that Launchkeeper did not
create, each with a verdict on whether it could be [adopted](#adopt-label---dry-run---yes).
Read-only in every sense: no plist is opened for writing and launchd is only
asked `print`.

Columns: `LABEL`, `TRIGGER`, `LOADED`, `PID`, `ADOPTABLE`. `LABEL` is what
`adopt` — and the external form of `enable`/`disable`/`run` — takes as its
argument; it is not necessarily the file name.

Plists named `com.launchkeeper.*` are left out (they are ordinary tasks — use
`list`). An adopted plist *is* listed, since it keeps its original label, and
shows as `否：已经由 Launchkeeper 管理`.

```console
$ launchkeeper agents
LABEL                    TRIGGER           LOADED  PID    ADOPTABLE
com.example.daily        每天 21:00        ✓       -      可接管
com.example.hourly       每 1 小时         –       -      可接管
com.example.watcher      不支持的触发方式  ✓       41231  否：plist 使用了 WatchPaths，Launchkeeper 没有对应的触发模型
```

JSON: an array of agent objects (a read, so no `ok` envelope):

```jsonc
{
  "label": "com.example.daily",
  "path": "/Users/me/Library/LaunchAgents/my-daily-job.plist",
  "program": "/Users/me/bin/job.sh",
  "args": [],
  "command": "/Users/me/bin/job.sh",
  "working_dir": null,
  "env": {},
  "trigger": {"kind": "calendar", "entries": [...]},  // null when untranslatable
  "trigger_text": "每天 21:00",
  "keep_alive": false,
  "loaded": true,
  "pid": null,
  "last_exit": 0,
  "adoptable": true,
  "reason": null,                    // the refusal, when adoptable is false
  "managed_by_launchkeeper": false
}
```

An agent is **not** adoptable when any of these holds — `reason` says which:

- its label starts with `com.apple.` or `homebrew.mxcl.`;
- its plist uses `WatchPaths`, `QueueDirectories`, `Sockets`, `MachServices`,
  `LaunchEvents` or `inetdCompatibility` (those keys *are* the trigger, and
  there is no equivalent in Launchkeeper's model, so `trigger` is `null` too);
- it has neither `Program` nor `ProgramArguments`, or the program path is
  relative;
- the program lives under `/Applications`, `/System`, `/Library`,
  `/opt/homebrew/Cellar`, `/usr/local/Cellar`, or inside any `*.app/` bundle;
- the plist is not in `~/Library/LaunchAgents`;
- its schedule does not translate — a `StartCalendarInterval` with a `Month`,
  or without both `Hour` and `Minute` (launchd reads a missing field as
  "every", which `--daily`/`--weekly`/`--monthly` cannot express), or a
  `KeepAlive` dictionary with any key other than `SuccessfulExit`;
- Launchkeeper already manages it.

### `adopt <label> [--dry-run] [--yes]`

Takes a hand-written LaunchAgent under management **in place**. The label does
not change and the plist file stays where it is; what changes is that
`ProgramArguments` now points at `launchkeeper-runner`, so the job starts
recording runs, exit codes and per-run logs like any other task.

The adopted task's **name is the label** (`com.example.daily`, dots and all) —
that is what `show`, `runs`, `logs`, `enable`, `unadopt` and every other
command take from then on. Task names accept a full label since M3:
`^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$`.

Order of operations: copy the original to `<plist>.bak` → insert the task row
→ rewrite the plist (adding `LaunchkeeperManaged`) → `bootout` + `bootstrap`.
Anything that fails after the copy rolls the earlier steps back, so there is
no half-adopted state. If `<plist>.bak` already exists the command refuses
and says so, rather than overwriting somebody's only backup.

`--dry-run` prints the key-by-key diff and stops; nothing is written.
Without `--yes`, human mode prints the same diff and then prompts on stdin;
**in `--json` mode `--yes` is required** (same rule as `remove`), so a
non-interactive caller can never hang.

```console
$ launchkeeper adopt com.example.daily --dry-run
接管 com.example.daily
  plist:  /Users/me/Library/LaunchAgents/my-daily-job.plist
  备份到: /Users/me/Library/LaunchAgents/my-daily-job.plist.bak
  任务名: com.example.daily（= label，不变）
  触发:   每天 21:00
  移除:
    - ProgramArguments = [/Users/me/bin/job.sh]
    - StandardOutPath = /Users/me/daily.log
  新增:
    + EnvironmentVariables = {LAUNCHKEEPER_DATA_DIR=...}
    + LaunchkeeperManaged = true
    + ProgramArguments = [.../launchkeeper-runner, run, com.example.daily]
    + ...
  保持:
      Label = com.example.daily
      StartCalendarInterval = {Minute=0, Hour=21}

（--dry-run，什么都没写）
$ launchkeeper adopt com.example.daily --yes
已接管 com.example.daily（label 不变）
撤销: launchkeeper unadopt com.example.daily
```

JSON for `--dry-run`: `{"ok": true, "dry_run": true, "plan": {...}}`, where
`plan` carries `label`, `path`, `backup_path`, the three diff groups
(`removed` / `added` / `kept`, each an array of `{"key": ..., "value": ...}`
with values flattened to one line) and `task`, the full `TaskJson` that would
be stored. A key whose value merely *changes* appears in `removed` with the
old value and in `added` with the new one.

JSON for a real adopt: the usual `{"ok": true, "task": <TaskJson>}` envelope.

The original `StandardOutPath`/`StandardErrorPath` are dropped — the runner
captures output per run instead — and mentioned once in the new task's
`description` so the old files are not a mystery.

### `unadopt <name> [--yes]`

Undoes `adopt`: `bootout`, rename `<plist>.bak` back over the managed plist
(so the bytes that return are exactly the bytes that were there — comments,
formatting and all), `bootstrap` the restored job, then delete the task row.
Run **log files are kept**; the run *history* rows go with the row.

Refuses a task that was not adopted (use `remove` for those) and a task whose
`.bak` has gone missing.

JSON: `{"ok": true, "task": <TaskJson>}`, a snapshot taken just before the
task disappeared — same convention as `remove`. `--yes` is required in
`--json` mode.

### `interpreters [--script <path>] [--cwd <dir>]`

Lists interpreters `interpreters::scan` finds on `PATH` (and, with
`--script`, the project's own `.venv`/`uv.lock`/`package.json`, and the
`recommended` candidate for that script) — the exact discovery
`add`/`set` use internally, exposed directly for inspection, scripting, or
an AI agent deciding what `--interpreter` value to pass. Never opens the
database — no `--data-dir` content is required to exist first, same as
`completions`/`config`.

`--script <PATH>` must exist (same resolution as `add`'s `--script`); omit
it to just list what's on `PATH` with nothing marked `recommended`.
`--cwd <PATH>` is where project-marker scanning starts; defaults to
`--script`'s own directory, same as `add`.

Unlike `add`/`set`'s internal scan (which skips `--version` for speed),
this command always resolves versions — it exists to be looked at.

Human mode is a table: `ID` (`Interpreter::id`, what you'd pass to
`--interpreter`), `程序` (display name, e.g. `python3`/`uv run`), `版本`,
`来源` (`系统`/`Homebrew`/`项目 .venv`/`uv 安装`/`PATH`/`自定义`), and
`推荐/理由` (`-`, `推荐`, or `推荐：<reason>`).

JSON (`--json`): an array, each candidate's raw `Interpreter` fields
(`kind`, `program`, `prefix_args`, `version`, `origin`, `recommended`,
`reason`) plus the two derived strings `id` and `label`:

```console
$ launchkeeper interpreters --script ~/proj/scripts/sync.py --json
[
  {
    "id": "/Users/x/proj/.venv/bin/python",
    "kind": "python",
    "program": "/Users/x/proj/.venv/bin/python",
    "prefix_args": [],
    "version": "3.13.2",
    "origin": "project_venv",
    "recommended": true,
    "reason": "按 .py 扩展名推荐；项目里有 .venv",
    "label": "python · 项目 .venv"
  },
  {
    "id": "/usr/bin/python3",
    "kind": "python",
    "program": "/usr/bin/python3",
    "prefix_args": [],
    "version": "3.9.6",
    "origin": "system",
    "recommended": false,
    "reason": null,
    "label": "python3 · 系统"
  }
]
```

### `explain <name> [--refresh] [--lang <tag>]`

Asks the configured model what a task does, whether it has been healthy, why
it last failed, and what to do about it. The answer is Markdown, stored one
per task, and shown again on the next `explain` **without** calling the model.

- `--refresh` / `-r` — ignore the stored answer and generate a new one,
  overwriting it. Without this flag an existing answer is returned as it is
  and **no request is made**: an explanation costs money and several seconds.
- `--lang <tag>` — language for the answer, default `zh-CN`. Anything
  starting with `zh` is Chinese; everything else is English.

Configure the provider and the key with [`ai`](#ai-config----provider---base-url---model)
first — without a key this fails before sending anything.

What is sent: the task's definition, its newest 10 runs (timestamps, exit
codes, `stop_reason`, durations — no log bodies), and the tail of the newest
run's stdout/stderr, capped at 16 KiB. **Environment variable *values* are
never sent** — only the names, which are often the answer to "why does it
work in my shell and fail from launchd". See `docs/M4-design.md` §1.4 for the
full redaction policy.

JSON on success: a bare object (a read, like `show`), where `cached` says
whether this call reused the stored answer (`true`) or actually called the
model (`false`):

```console
$ launchkeeper explain nightly-sync --json
{
  "task": "nightly-sync",
  "created_at": "2026-09-10T12:00:00.000Z",
  "model": "claude-sonnet-5",
  "prompt_hash": "4291cd50e0359788...",
  "content": "这个任务每天 21:00 从对象存储拉新文件……",
  "cached": false
}
```

JSON on failure: `{"ok": false, "error": "<message>"}` and exit code 1 — an
unknown task, no API key, or the provider's own error (`AI 接口返回 HTTP
401: invalid x-api-key`).

`prompt_hash` is the sha256 of the exact prompt behind `content`, so two
explanations of the same task can be told apart without diffing their prose.

Deleting the task deletes its explanation with it (database cascade).

### `ai config [--provider] [--base-url] [--model]`

Prints, or sets, the **non-secret** AI configuration. It lives in
`<data_dir>/ai.json`, so it follows `--data-dir` / `LAUNCHKEEPER_DATA_DIR`
like everything else and the desktop app reads the same file.

- `--provider <anthropic|openai_compatible>` — which wire format the endpoint
  speaks. `openai` and `openai-compatible` are accepted spellings of the
  second.
- `--base-url <URL>` — endpoint root, e.g. `http://127.0.0.1:11434/v1` for a
  local Ollama. Pass an **empty string** to go back to the provider default
  (`https://api.anthropic.com` / `https://api.openai.com/v1`).
- `--model <ID>` — passed to the provider verbatim. Default
  `claude-sonnet-5`.

The endpoint is `<base>/v1/messages` for `anthropic` and
`<base>/chat/completions` for `openai_compatible`.

**The API key is never in this output**, or in `ai.json`. What you get
instead is `has_api_key` and `api_key_source` (`"env"` when
`LAUNCHKEEPER_AI_API_KEY` supplied it, `"file"` when `<data_dir>/ai-key`
did, `null` when there is none).

JSON with no flags — a plain read, no `ok` envelope:

```console
$ launchkeeper ai config --json
{
  "provider": "anthropic",
  "base_url": null,
  "effective_base_url": "https://api.anthropic.com",
  "endpoint": "https://api.anthropic.com/v1/messages",
  "model": "claude-sonnet-5",
  "config_file": "/Users/me/Library/Application Support/Launchkeeper/ai.json",
  "has_api_key": true,
  "api_key_source": "file"
}
```

JSON with any flag — the usual mutation envelope, carrying the config as it
now stands: `{"ok": true, "config": {...}}` / `{"ok": false, "error": "..."}`.

### `ai key set` / `ai key clear`

`set` reads the key from **stdin** and stores it in `<data_dir>/ai-key` (mode 0600)
(service `launchkeeper`, account `ai-api-key`). It takes **no argument on
purpose**: a key on a command line ends up in shell history, in `ps` output,
and in any process listing on the machine.

```console
$ echo "$MY_KEY" | launchkeeper ai key set
$ launchkeeper ai key set        # prompts on a terminal (input is echoed — the prompt says so)
```

`clear` deletes the key file. It cannot unset `LAUNCHKEEPER_AI_API_KEY`
in your shell, and says so when that variable is still set.

JSON for both: the same `{"ok": true, "config": {...}}` envelope `ai config`
uses, so a caller can confirm `has_api_key` flipped.

### `ai test`

Sends a one-line ping to the configured endpoint — the exact request shape
`explain` uses, so a passing test really does mean `explain` will work — and
prints the model's reply. Costs a handful of tokens.

JSON: `{"ok": true, "endpoint": "...", "model": "...", "reply": "ok"}`, or
`{"ok": false, "error": "..."}` with exit code 1.

None of the four `ai` subcommands opens the database, so `ai key set` works
on a machine that has no tasks yet.

### `docs`

Prints this file to stdout. `docs/CLI.md` is embedded in the binary with
`include_str!` at compile time, so `launchkeeper docs` works from an
installed binary on a machine with no checkout of the repository — which is
the point: an AI assistant (or anyone) can read the full contract without
being told where the source lives. `--json` has no effect; the output is
Markdown either way.

Like `completions`, `config` and `ai-prompt`, this never opens the SQLite
database and never reads `~/Library/LaunchAgents`, so it works before a
single task exists.

```bash
launchkeeper docs | less
launchkeeper docs > /tmp/launchkeeper-cli.md
```

### `ai-prompt [--lang zh-CN|en]`

Prints a short, copy-pasteable prompt that tells an AI assistant that
Launchkeeper is installed on this machine, points it at `launchkeeper docs`
for the details, and lays down the house rules (`--json` for reads, don't
parse human text, leave other people's LaunchAgents alone, confirm before
anything destructive). Paste it into an assistant's system prompt, a
`CLAUDE.md`, or a chat.

`--lang` takes `zh-CN` or `en` (anything starting with `zh` is Chinese,
everything else English). Without it, `LC_ALL` then `LANG` decide, and an
unset or empty environment gives Chinese — the language the rest of this
binary's human output is written in. `--json` has no effect.

```bash
launchkeeper ai-prompt              # follows LC_ALL/LANG, zh-CN by default
launchkeeper ai-prompt --lang en | pbcopy
```

The same text is reproduced in [README.md](../README.md) and
[README.zh-CN.md](../README.zh-CN.md) under "For AI assistants".

### `completions <bash|zsh|fish>`

Prints a static shell completion script to stdout — see
[Shell completion](#shell-completion) below for install lines and for the
alternative dynamic activation. `--json` has no effect.

### `config data-dir [<path>]`

Prints or persistently sets the data directory. See
[Data directory discovery](#data-directory-discovery) above for the full
precedence and the JSON shapes. Unlike every other subcommand, this one (and
`completions`) never opens the SQLite database — it only touches
`~/.config/launchkeeper/data-dir`.

## Shell completion

Two independent mechanisms, both backed by `clap_complete` (added to the
workspace with the `unstable-dynamic` feature):

### Static (`completions <shell>`)

`launchkeeper completions <bash|zsh|fish>` prints a self-contained script
that completes `--flags` and subcommand (and alias) names. It has no idea
which tasks exist — task-name arguments complete as plain text. Install:

```console
# zsh — needs a directory on fpath; ~/.zfunc is a common choice
$ mkdir -p ~/.zfunc
$ launchkeeper completions zsh > ~/.zfunc/_launchkeeper
# then, once, in ~/.zshrc (before `compinit` if it isn't already there):
$ fpath=(~/.zfunc $fpath)

# bash (Homebrew's bash-completion@2, or any dir bash-completion scans)
$ launchkeeper completions bash > $(brew --prefix)/etc/bash_completion.d/launchkeeper

# fish
$ launchkeeper completions fish > ~/.config/fish/completions/launchkeeper.fish
```

Re-run the relevant line after upgrading Launchkeeper if the flag/subcommand
set changed; nothing here auto-updates.

### Dynamic (`COMPLETE=<shell>`, also completes task names)

`clap_complete`'s environment-activated completion additionally completes
`<name>` arguments (`show`, `enable`, `start`, `logs`, `set-trigger`, ...)
with live task names read from the store at the ordinary, resolved data
dir — no `completions` subcommand involved; the binary intercepts a
`COMPLETE` environment variable at the very start of `main` before `clap`
even parses arguments. Install (one line, self-correcting on every new
shell since it re-invokes `launchkeeper` rather than freezing a script):

```console
# zsh, in ~/.zshrc
$ echo 'source <(COMPLETE=zsh launchkeeper)' >> ~/.zshrc

# bash, in ~/.bashrc
$ echo 'source <(COMPLETE=bash launchkeeper)' >> ~/.bashrc

# fish, as a completions file
$ echo 'COMPLETE=fish launchkeeper | source' >> ~/.config/fish/completions/launchkeeper.fish
```

Two things this deliberately will not do, because a TAB keystroke has to be
cheap and side-effect-free:

- **It never creates or upgrades anything.** The completer opens the database
  read-only (`Store::open_read_only`): no data directory is created, the
  journal mode is left alone, and no migration runs. Before Launchkeeper has
  ever been used — or against a database written by a newer version — TAB
  simply offers no task names instead of quietly creating or converting a
  database from inside your shell.
- **It never waits.** The busy timeout is 300 ms, versus the five seconds an
  ordinary command allows. If another process is holding the write lock at
  that instant, you get no candidates rather than a frozen terminal.

It also only completes against the plain, ambient data dir resolution — a
`--data-dir` typed earlier on the very same command line is not taken into
account, since the shell only hands the completer the value being typed, not
the rest of the line. Use the static scripts above if that matters for your
setup, or just type the full task name.

Both mechanisms can be installed at once without conflict — pick one, or
use the dynamic one for its task-name completion and ignore `completions`
entirely.

## Short alias

`launchkeeper` is not shortened by default — no symlink or extra binary is
installed. If typing it in full gets old, add your own shell alias:

```console
$ echo 'alias lkp=launchkeeper' >> ~/.zshrc
```

`lkp` is the suggested name because the obvious short one, `lk`, is already
taken on both crates.io and npm — publishing a binary or package under it
later would conflict with someone else's project.

## Rules

- Only plists that are Launchkeeper's own are ever written or deleted: those
  named `com.launchkeeper.<name>`, plus the ones it *adopted* in place, which
  carry a `LaunchkeeperManaged` key (see [`adopt`](#adopt-label---dry-run---yes)).
  Every other LaunchAgent is inspected read-only. `enable`/`disable`/`run` can
  still start and stop one through `launchctl` — that changes launchd state,
  never the file.
- Adoption always backs the original plist up to `<plist>.bak` first, and
  refuses outright if such a file already exists (it would be the only copy
  of somebody's original). `unadopt` renames the backup back over the managed
  plist, so what returns is byte-for-byte what was there.
- "Enable" means `launchctl bootstrap gui/$UID <plist>`; "disable"/"remove"
  mean `launchctl bootout`. The CLI never uses the legacy `load`/`unload`
  commands.
- `launchd` is the only scheduler here — nothing in this CLI or the runner
  keeps a background timer running. If a task looks "stuck" not firing on
  schedule, the plist/launchd state (`status`, `plist`) is where to look,
  not this process.
- Rebuilding or relocating the `launchkeeper-runner` binary — especially onto
  an external or otherwise TCC-protected volume — can trigger a fresh macOS
  permission prompt (Full Disk Access / Desktop / Documents / removable
  volumes) the next time an enabled task fires, because TCC grants are tied
  to the exact signed binary launchd exec's. If tasks that read protected
  directories suddenly start failing after a rebuild, check TCC settings
  before assuming the task itself is broken. Machine-specific toolchain
  paths and constraints belong in each developer's own local, un-checked-in
  notes, not in this document.

## Recipes

### (a) Create a daily task and verify it ran

```console
$ launchkeeper add daily-report --script /usr/local/bin/report.sh --daily 08:00 --json
$ launchkeeper enable daily-report --json
$ launchkeeper run daily-report --direct --json      # exercise it immediately instead of waiting for 08:00
$ launchkeeper runs daily-report --limit 1 --json    # confirm a run row exists with exit_code 0
$ launchkeeper show daily-report --json | jq '.last_run'
```

### (b) Change a schedule

```console
$ launchkeeper set-trigger daily-report --weekly mon,wed,fri@08:00 --json
$ launchkeeper show daily-report --json | jq '.trigger, .trigger_text'
```
`set-trigger` reloads launchd automatically if `daily-report` is enabled —
no separate `disable`/`enable` cycle needed.

### (c) Find why last night's run failed

```console
$ launchkeeper runs daily-report --limit 5 --json | jq '.[] | {id, exit_code, started_at}'
# spot the failing run's id, then:
$ launchkeeper logs daily-report --last 1 --err --tail 50 --json | jq -r '.content'
```
`exit_code: 124` means the run hit `--timeout` and was killed.

Or hand the same evidence to a model and read its verdict:

```console
$ launchkeeper explain daily-report --refresh --json | jq -r '.content'
```

### (d) Run a long-lived service

```console
$ launchkeeper add bridge --script ~/bin/bridge.sh --manual --keep-alive --json
$ launchkeeper start bridge --json | jq '.task.pid'
$ launchkeeper status --json | jq '.[] | select(.name=="bridge") | {pid, uptime_secs}'
$ launchkeeper logs bridge --tail 40            # what it has printed so far
$ launchkeeper stop bridge --json | jq '.ok'
$ launchkeeper runs bridge --limit 1 --json | jq '.[0].stop_reason'   # "stopped"
```

Because of `--keep-alive`, a crash brings the service back on its own (at
most once a minute, `ThrottleInterval`), while `stop` keeps it down until the
next `start`. Drop `--keep-alive` if you would rather a crash stay a crash.

### (e) Take over a hand-written plist

```console
$ launchkeeper agents                                     # what is out there, and what could be adopted
$ launchkeeper adopt com.example.daily --dry-run           # exactly which plist keys would change
$ launchkeeper adopt com.example.daily --yes --json | jq '.task.name'
"com.example.daily"
$ launchkeeper run com.example.daily                       # now it records runs
$ launchkeeper runs com.example.daily --limit 1 --json | jq '.[0] | {exit_code, stop_reason}'
$ launchkeeper logs com.example.daily
$ launchkeeper unadopt com.example.daily --yes             # byte-for-byte back to how it was
```

The label never changes, so anything else on the machine that refers to the
job by label keeps working, and `launchctl print gui/$UID/<label>` still
finds it. The task's name is that label from then on.

Adoption is refused for anything that belongs to somebody else (Apple,
Homebrew, an app bundle) or whose trigger has no equivalent in Launchkeeper's
model — `agents --json | jq '.[] | select(.adoptable == false) | {label, reason}'`
lists why. To start or stop such an agent without adopting it, `enable`,
`disable` and `run` take its label directly and only talk to `launchctl`.

### (f) Add a Python script and let the CLI pick the interpreter

```console
$ launchkeeper interpreters --script ~/proj/scripts/sync.py     # see what would be picked, and why
$ launchkeeper add py-sync --script ~/proj/scripts/sync.py --arg --once --daily 21:00
运行方式: python3 · 3.9.6 · 系统 /usr/bin/python3（按 .py 扩展名推荐）
已添加任务 py-sync
$ launchkeeper show py-sync --json | jq '{script_path, args, script, script_args, interpreter_label}'
```

`args`/`script_path` is what launchd actually execs (`python3 /abs/sync.py
--once`); `script`/`script_args` is the decomposed view (`sync.py`,
`["--once"]`) — both are always present, so a caller that only cares about
"what script, what args" never has to know which interpreter got picked.
Pass `--interpreter <id>` (an `id` from the `interpreters` output above) to
override the pick, or `--interpreter direct` to skip interpreters and run
the script itself.

### (g) Configure a model and have it explain a failing task

```console
$ launchkeeper ai config --provider anthropic --model claude-sonnet-5 --json
$ echo "$ANTHROPIC_API_KEY" | launchkeeper ai key set     # into <data_dir>/ai-key (0600), never argv
$ launchkeeper ai test --json | jq '.ok'
true
$ launchkeeper explain daily-report --json | jq -r '.content'
$ launchkeeper explain daily-report --json | jq '.cached'   # true: served from the database, nothing sent
true
$ launchkeeper explain daily-report --refresh --lang en --json | jq -r '.content'
```

A local OpenAI-compatible endpoint instead of a hosted one:

```console
$ launchkeeper ai config --provider openai --base-url http://127.0.0.1:11434/v1 --model llama3.2 --json
```

`ai config --json | jq '{has_api_key, api_key_source}'` is how a script
checks whether a key is available without ever seeing it.
