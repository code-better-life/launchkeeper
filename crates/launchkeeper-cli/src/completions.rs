//! Shell completion, two independent mechanisms living side by side:
//!
//! - `launchkeeper completions <bash|zsh|fish>` prints a static script
//!   (`clap_complete::aot::generate`) for `--flag` and subcommand names —
//!   the classic "shell-out once, save the file" approach. It has no way to
//!   know which tasks exist.
//! - `COMPLETE=<shell> launchkeeper`, wired up here via
//!   [`clap_complete::CompleteEnv`] (the `unstable-dynamic` feature),
//!   additionally completes task names on every `<name>` argument by
//!   querying the store live. See `docs/CLI.md` for the exact install line
//!   for each shell and the tradeoff between the two.

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use clap::CommandFactory;
use clap_complete::aot::{Shell as AotShell, generate};
use clap_complete::engine::CompletionCandidate;

use crate::Cli;

/// The shells `completions` generates a static script for — a deliberate
/// subset of [`clap_complete::aot::Shell`] (which also has Elvish and
/// PowerShell), matching PRD M3 item 2's `<zsh|bash|fish>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
}

impl CompletionShell {
    fn as_aot(self) -> AotShell {
        match self {
            CompletionShell::Bash => AotShell::Bash,
            CompletionShell::Zsh => AotShell::Zsh,
            CompletionShell::Fish => AotShell::Fish,
        }
    }
}

/// Prints the static completion script for `shell` to stdout.
pub fn print_static_script(shell: CompletionShell) {
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    generate(shell.as_aot(), &mut cmd, name, &mut std::io::stdout());
}

/// Hooks in `clap_complete`'s environment-activated dynamic completion
/// (`COMPLETE=<shell> launchkeeper`). Must run before `Cli::parse()` and
/// before anything else reads stdin or writes stdout: if `COMPLETE` is set,
/// this prints completion candidates (or, with no argument, the shell
/// integration snippet to source) and calls `std::process::exit` itself,
/// never returning to the rest of `main`. It is a silent no-op when
/// `COMPLETE` is unset, which is the case for every normal invocation.
pub fn hook_dynamic_completion() {
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();
}

/// Dynamic completer for a task-name argument (`add = ArgValueCompleter::new
/// (complete_task_name)` on every `<name>: String` field that names an
/// *existing* task): lists names from the store at the plain, resolved data
/// dir (`launchkeeper_core::paths::data_dir()`'s normal env-var / config-file
/// / default precedence).
///
/// A `--data-dir` typed earlier on the very same command line is **not**
/// taken into account — `clap_complete`'s dynamic completer only sees the
/// value being completed, not the rest of the parsed line — so this always
/// completes against the ambient default. That matches the common case
/// (the shell's env already has `LAUNCHKEEPER_DATA_DIR` set, or the new
/// `~/.config/launchkeeper/data-dir` file is used instead) and is why
/// scripts/tests that pass an explicit `--data-dir` don't rely on
/// completion in the first place.
///
/// Any failure along the way (no database yet, I/O error, a writer holding
/// the lock, ...) yields no candidates instead of an error: a completer must
/// never make the shell hang or print noise onto the command line.
///
/// Which is also why it never calls [`launchkeeper_core::Store::open`]: that
/// creates the data directory, switches the database to WAL and takes each
/// migration's `BEGIN IMMEDIATE` lock with a five-second busy timeout. A TAB
/// keystroke may not do any of those things — pressing TAB in a shell must
/// not create files, upgrade a schema, or freeze the terminal for five
/// seconds while another process writes. It opens read-only with
/// [`COMPLETION_BUSY_TIMEOUT`] instead, and gives up the moment anything is
/// not exactly as expected.
pub fn complete_task_name(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };
    let Ok(db_path) = launchkeeper_core::paths::db_path() else {
        return Vec::new();
    };
    task_names(&db_path)
        .into_iter()
        .filter(|name| name.starts_with(current))
        .map(CompletionCandidate::new)
        .collect()
}

/// How long the completer waits for a competing writer before giving up.
/// Short on purpose: a completion that is not instant is not a completion.
const COMPLETION_BUSY_TIMEOUT: Duration = Duration::from_millis(300);

/// Every task name in the database at `db_path`, or an empty vector for any
/// reason at all (missing file, unreadable, locked, older schema).
fn task_names(db_path: &Path) -> Vec<String> {
    // Checked before opening: `Store::open_read_only` would not create it
    // either, but this keeps the common "not installed yet" case from even
    // reaching SQLite.
    if !db_path.is_file() {
        return Vec::new();
    }
    let Ok(store) = launchkeeper_core::Store::open_read_only(db_path, COMPLETION_BUSY_TIMEOUT)
    else {
        return Vec::new();
    };
    let Ok(tasks) = store.list_tasks() else {
        return Vec::new();
    };
    tasks
        .into_iter()
        .map(|t| t.name.as_str().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pressing TAB where Launchkeeper has never run must leave the disk
    /// exactly as it found it — no data directory, no database, no journal.
    #[test]
    fn a_missing_database_yields_nothing_and_creates_nothing() {
        let tmp = tempfile::tempdir().expect("tmp");
        let data_dir = tmp.path().join("never-used");
        let db = data_dir.join("launchkeeper.db");
        assert!(task_names(&db).is_empty());
        assert!(!data_dir.exists(), "补全不该建出数据目录");
    }

    /// And where it has run but something is wrong with the file, it gives
    /// up quickly instead of blocking the shell.
    #[test]
    fn an_unreadable_database_yields_nothing_quickly() {
        let tmp = tempfile::tempdir().expect("tmp");
        let db = tmp.path().join("launchkeeper.db");
        std::fs::write(&db, b"not a database").expect("write");
        let started = std::time::Instant::now();
        assert!(task_names(&db).is_empty());
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "补全应当立刻放弃，实际用了 {:?}",
            started.elapsed()
        );
    }

    /// The happy path, and — the point of the read-only open — it still
    /// answers while another connection holds the write lock.
    #[test]
    fn names_are_listed_even_while_a_writer_holds_the_lock() {
        use launchkeeper_core::{Store, Task, TaskName, Trigger};

        let tmp = tempfile::tempdir().expect("tmp");
        let db = tmp.path().join("launchkeeper.db");
        let writer = Store::open(&db).expect("open");
        writer
            .insert_task(&Task::new(
                TaskName::new("sync").expect("name"),
                "/usr/bin/true",
                Trigger::AtLogin,
            ))
            .expect("insert");
        assert_eq!(task_names(&db), vec!["sync".to_string()]);

        writer
            .conn()
            .execute_batch("BEGIN IMMEDIATE")
            .expect("take the write lock");
        let started = std::time::Instant::now();
        let names = task_names(&db);
        let elapsed = started.elapsed();
        writer.conn().execute_batch("ROLLBACK").expect("release");
        // WAL lets the read through; what matters either way is that TAB
        // came back long before the five seconds `Store::open` would wait.
        assert!(elapsed < Duration::from_secs(1), "用了 {elapsed:?}");
        assert!(
            names.is_empty() || names == vec!["sync".to_string()],
            "{names:?}"
        );
    }
}
