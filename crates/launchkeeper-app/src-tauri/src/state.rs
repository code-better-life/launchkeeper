//! Application state: one [`Service`] behind a mutex (`docs/M2-design.md`
//! §3.1).
//!
//! [`launchkeeper_core::Store`] wraps a `rusqlite::Connection`, which is
//! `Send` but not `Sync`, so the whole `Service` — not just the store — goes
//! behind a `Mutex`. Commands are short, so lock contention is irrelevant.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use launchkeeper_core::{Service, Store, paths};

use crate::error::{AppError, AppResult};

/// Environment variable pointing at a runner binary to install, checked
/// before the bundle/`target/` lookup. Same name the CLI honours.
pub const RUNNER_ENV: &str = "LAUNCHKEEPER_RUNNER";

/// File name of the runner binary, in the bundle and in `target/`.
const RUNNER_NAME: &str = "launchkeeper-runner";

/// Shared state handed to every command via `tauri::State`.
pub struct AppState {
    /// The service. Poisoning is recovered from rather than propagated: a
    /// panicking command must not brick the whole app.
    service: Mutex<Service>,
    /// Where the runner was installed, when it could be installed at all.
    /// `None` means the app is running without one and every plist-writing
    /// command will fail with [`launchkeeper_core::Error::NoRunner`].
    runner_path: Option<PathBuf>,
}

impl AppState {
    /// Opens the database at [`paths::db_path`], installs the runner if a
    /// source can be found, and builds the service.
    ///
    /// A missing runner is deliberately *not* fatal: listing tasks, reading
    /// logs and disabling jobs all work without one, and the UI can then tell
    /// the user what is wrong instead of refusing to start.
    ///
    /// # Errors
    /// Path and database errors only.
    pub fn new() -> AppResult<AppState> {
        let store = Store::open(&paths::db_path()?)?;
        let runner_path = install_runner();
        let service = match runner_path.clone() {
            Some(p) => Service::new(store, p),
            None => Service::without_runner(store),
        };
        Ok(AppState {
            service: Mutex::new(service),
            runner_path,
        })
    }

    /// Locks the service. Recovers from a poisoned mutex.
    pub fn service(&self) -> MutexGuard<'_, Service> {
        self.service.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The installed runner binary.
    ///
    /// # Errors
    /// [`AppError`] when no runner could be installed at startup.
    pub fn runner_path(&self) -> AppResult<&std::path::Path> {
        self.runner_path.as_deref().ok_or_else(|| {
            AppError::new(
                "找不到 launchkeeper-runner：请重新安装 Launchkeeper，或设置 LAUNCHKEEPER_RUNNER",
            )
        })
    }
}

/// What to do with the runner binary that was found.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RunnerPlan {
    /// Point plists straight at this path and copy nothing. Used when the app
    /// runs from a `.app`: the bundled runner is signed with the same
    /// identity as the app, lives on the boot volume next to it, and is
    /// replaced wholesale by the next app update — everything the
    /// `<data_dir>/bin/` copy exists to guarantee, without the copy. Copying
    /// it out would also strip the bundle's own code-signing context from
    /// the path recorded in every plist.
    UseInPlace(PathBuf),
    /// Copy this into `<data_dir>/bin/` first (the dev and loose-binary
    /// case). See [`launchkeeper_core::runner_install`].
    Install(PathBuf),
}

/// Makes the runner usable and returns the path plists should name.
/// Returns `None` when no source binary could be found or the copy failed.
fn install_runner() -> Option<PathBuf> {
    let source = match runner_plan()? {
        RunnerPlan::UseInPlace(p) => return Some(p),
        RunnerPlan::Install(p) => p,
    };
    // A runner found under the workspace `target/` is a developer build of
    // unknown profile (often debug). It must never replace a runner that is
    // already installed: overwriting changes the binary's code hash, which
    // invalidates the TCC grant launchd relies on, and the next scheduled
    // run then blocks on an authorization dialog nobody is there to click.
    if is_workspace_build(&source)
        && let Ok(installed) = launchkeeper_core::paths::installed_runner_path()
        && installed.exists()
    {
        eprintln!(
            "开发模式：沿用已安装的 runner {}，不用 {} 覆盖",
            installed.display(),
            source.display()
        );
        return Some(installed);
    }
    match launchkeeper_core::runner_install::install(&source) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("安装 runner 失败（{}）: {e}", source.display());
            None
        }
    }
}

fn workspace_root() -> Option<PathBuf> {
    // `src-tauri/` -> `launchkeeper-app/` -> `crates/` -> workspace root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map(Path::to_path_buf)
}

fn is_workspace_build(source: &Path) -> bool {
    workspace_root().is_some_and(|w| source.starts_with(w.join("target")))
}

/// Where the runner binary comes from, in order:
///
/// 1. `$LAUNCHKEEPER_RUNNER`, for development and tests;
/// 2. next to the app executable — inside a bundle that is
///    `Launchkeeper.app/Contents/MacOS/`, where `bundle.externalBin` puts it;
/// 3. `<workspace>/target/{debug,release}/launchkeeper-runner`, so that
///    `pnpm tauri dev` works without a bundling step.
///
/// Only case 2, and only when the executable really is inside a `.app`,
/// yields [`RunnerPlan::UseInPlace`]; see that variant.
fn runner_plan() -> Option<RunnerPlan> {
    let exe = std::env::current_exe().ok();
    let workspace = workspace_root();
    locate_runner(
        std::env::var_os(RUNNER_ENV).map(PathBuf::from),
        exe.as_deref().and_then(Path::parent),
        workspace.as_deref(),
    )
}

/// True when `dir` is the `Contents/MacOS/` of a `.app` bundle — the only
/// place a runner is already signed, on the boot volume, and updated in
/// lockstep with the app, and therefore the only one used in place.
fn is_app_bundle_macos_dir(dir: &Path) -> bool {
    dir.file_name() == Some(OsStr::new("MacOS"))
        && dir.parent().and_then(Path::file_name) == Some(OsStr::new("Contents"))
        && dir
            .parent()
            .and_then(Path::parent)
            .and_then(Path::extension)
            == Some(OsStr::new("app"))
}

/// The lookup of [`runner_plan`], with its three inputs passed in so that
/// it can be tested against temporary directories rather than against
/// whatever this machine happens to have built.
///
/// Between a debug and a release build, the newer one wins: whichever the
/// developer built last is the one they mean, and picking `debug`
/// unconditionally would silently pin a stale runner into every plist.
fn locate_runner(
    env_override: Option<PathBuf>,
    exe_dir: Option<&Path>,
    workspace: Option<&Path>,
) -> Option<RunnerPlan> {
    if let Some(p) = env_override
        && !p.as_os_str().is_empty()
    {
        return Some(RunnerPlan::Install(p));
    }
    if let Some(dir) = exe_dir {
        let sibling = dir.join(RUNNER_NAME);
        if sibling.exists() {
            return Some(if is_app_bundle_macos_dir(dir) {
                RunnerPlan::UseInPlace(sibling)
            } else {
                RunnerPlan::Install(sibling)
            });
        }
    }
    let workspace = workspace?;
    ["debug", "release"]
        .into_iter()
        .map(|profile| workspace.join("target").join(profile).join(RUNNER_NAME))
        .filter_map(|p| {
            let modified = std::fs::metadata(&p).ok()?.modified().ok()?;
            Some((modified, p))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, p)| RunnerPlan::Install(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn touch(path: &Path, at: SystemTime) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, b"runner").expect("write");
        // `set_times` needs a handle to the file, not the path.
        let f = std::fs::File::options()
            .write(true)
            .open(path)
            .expect("open");
        f.set_times(std::fs::FileTimes::new().set_modified(at))
            .expect("set mtime");
    }

    fn target(root: &Path, profile: &str) -> PathBuf {
        root.join("target").join(profile).join(RUNNER_NAME)
    }

    #[test]
    fn env_override_wins_and_is_not_probed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ghost = dir.path().join("nowhere/launchkeeper-runner");
        assert_eq!(
            locate_runner(Some(ghost.clone()), None, Some(dir.path())),
            Some(RunnerPlan::Install(ghost))
        );
    }

    #[test]
    fn an_empty_override_is_ignored() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sibling = dir.path().join(RUNNER_NAME);
        touch(&sibling, SystemTime::now());
        assert_eq!(
            locate_runner(Some(PathBuf::new()), Some(dir.path()), None),
            Some(RunnerPlan::Install(sibling))
        );
    }

    #[test]
    fn the_bundle_sibling_beats_the_target_dir_and_is_used_in_place() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bundle = dir.path().join("Launchkeeper.app/Contents/MacOS");
        let sibling = bundle.join(RUNNER_NAME);
        touch(&sibling, SystemTime::now());
        touch(&target(dir.path(), "debug"), SystemTime::now());
        assert_eq!(
            locate_runner(None, Some(&bundle), Some(dir.path())),
            Some(RunnerPlan::UseInPlace(sibling))
        );
    }

    /// A runner sitting next to a *loose* executable is still copied into
    /// `<data_dir>/bin/`: only a `.app` gets the in-place treatment.
    #[test]
    fn a_sibling_outside_a_bundle_is_installed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bin = dir.path().join("bin");
        let sibling = bin.join(RUNNER_NAME);
        touch(&sibling, SystemTime::now());
        assert_eq!(
            locate_runner(None, Some(&bin), None),
            Some(RunnerPlan::Install(sibling))
        );
    }

    #[test]
    fn only_a_dot_app_counts_as_a_bundle() {
        let dir = dirs_for("Launchkeeper.app/Contents/MacOS");
        assert!(is_app_bundle_macos_dir(&dir));
        assert!(!is_app_bundle_macos_dir(&dirs_for("Contents/MacOS")));
        assert!(!is_app_bundle_macos_dir(&dirs_for(
            "Launchkeeper.app/MacOS"
        )));
        assert!(!is_app_bundle_macos_dir(&dirs_for(
            "Launchkeeper.app/Contents/Resources"
        )));
    }

    fn dirs_for(suffix: &str) -> PathBuf {
        Path::new("/tmp/x").join(suffix)
    }

    #[test]
    fn the_newest_build_wins() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = SystemTime::now() - Duration::from_secs(3600);
        touch(&target(dir.path(), "debug"), old);
        touch(&target(dir.path(), "release"), SystemTime::now());
        assert_eq!(
            locate_runner(None, None, Some(dir.path())),
            Some(RunnerPlan::Install(target(dir.path(), "release")))
        );

        touch(&target(dir.path(), "debug"), SystemTime::now());
        touch(&target(dir.path(), "release"), old);
        assert_eq!(
            locate_runner(None, None, Some(dir.path())),
            Some(RunnerPlan::Install(target(dir.path(), "debug")))
        );
    }

    #[test]
    fn nothing_anywhere_is_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            locate_runner(None, Some(dir.path()), Some(dir.path())),
            None
        );
    }
}
