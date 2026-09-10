//! Installing the `launchkeeper-runner` binary into the data directory.
//!
//! Plists must point at the installed copy rather than at wherever the runner
//! happens to live on disk: a runner on an external volume hangs in dyld when
//! launchd execs it, because a background process cannot pass the TCC check
//! for that volume. Both the CLI and the app funnel through [`install`] so
//! that "is the installed copy current" is decided in exactly one place.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::paths;

/// Name of the temporary file used for the atomic install.
const TMP_NAME: &str = ".launchkeeper-runner.tmp";

/// Copies `source` into `<data_dir>/bin/` when the installed copy is missing
/// or out of date, and returns the installed path
/// ([`paths::installed_runner_path`]).
///
/// "Out of date" means a different size, or a source that is strictly newer.
/// The copy goes to a temporary file next to the destination and is then
/// renamed, so a concurrent launchd exec never sees a half-written binary.
///
/// Passing the installed path itself is a no-op.
///
/// # Errors
/// [`Error::NoHomeDir`] and I/O errors (a missing `source` among them).
pub fn install(source: &Path) -> Result<PathBuf> {
    install_to(source, &paths::installed_runner_path()?)
}

/// [`install`] with an explicit destination. Split out so that tests can
/// exercise the freshness rules without mutating the process environment
/// (`LAUNCHKEEPER_DATA_DIR` is global, and this crate forbids `unsafe`).
fn install_to(source: &Path, dest: &Path) -> Result<PathBuf> {
    let dest = dest.to_path_buf();
    if source == dest {
        return Ok(dest);
    }
    let src_meta = fs::metadata(source).map_err(|e| Error::io(source, e))?;
    let up_to_date = fs::metadata(&dest).is_ok_and(|d| {
        d.len() == src_meta.len()
            && match (d.modified(), src_meta.modified()) {
                (Ok(a), Ok(b)) => a >= b,
                _ => false,
            }
    });
    if up_to_date {
        return Ok(dest);
    }
    let dir = dest
        .parent()
        .expect("installed runner path always has a parent");
    fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    let tmp = dir.join(TMP_NAME);
    fs::copy(source, &tmp).map_err(|e| Error::io(&tmp, e))?;
    fs::rename(&tmp, &dest).map_err(|e| Error::io(&dest, e))?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_file(path: &Path, content: &[u8]) {
        let mut f = fs::File::create(path).expect("create");
        f.write_all(content).expect("write");
    }

    #[test]
    fn installs_when_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("launchkeeper-runner");
        write_file(&src, b"binary-v1");
        let dest = dir.path().join("bin").join("launchkeeper-runner");

        assert_eq!(install_to(&src, &dest).expect("install"), dest);
        assert_eq!(fs::read(&dest).expect("read"), b"binary-v1");
    }

    #[test]
    fn reinstalls_when_size_differs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("launchkeeper-runner");
        let dest = dir.path().join("bin").join("launchkeeper-runner");
        write_file(&src, b"v1");
        install_to(&src, &dest).expect("first install");

        write_file(&src, b"a-much-longer-v2");
        install_to(&src, &dest).expect("second install");
        assert_eq!(fs::read(&dest).expect("read"), b"a-much-longer-v2");
    }

    #[test]
    fn leaves_an_up_to_date_copy_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("launchkeeper-runner");
        let dest = dir.path().join("bin").join("launchkeeper-runner");
        write_file(&src, b"same-size");
        install_to(&src, &dest).expect("install");

        // Same length, but the destination is newer, so it must survive.
        write_file(&dest, b"SAME-SIZE");
        install_to(&src, &dest).expect("no-op install");
        assert_eq!(fs::read(&dest).expect("read"), b"SAME-SIZE");
    }

    #[test]
    fn installing_the_destination_is_a_noop() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("launchkeeper-runner");
        // Nothing on disk at all, yet this must not fail.
        assert_eq!(install_to(&dest, &dest).expect("noop"), dest);
    }

    #[test]
    fn missing_source_is_an_io_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = install_to(&dir.path().join("nope"), &dir.path().join("dest"))
            .expect_err("should fail");
        assert!(matches!(err, Error::Io { .. }), "got {err:?}");
    }
}
