//! Real `launchctl` round trip, `#[ignore]`d by default.
//!
//! Run with `cargo test -p launchkeeper-core --test launchctl -- --ignored`.
//!
//! These tests do write into the real `~/Library/LaunchAgents`, but only files
//! named `com.launchkeeper.test.<uuid>.plist`, and a Drop guard boots the job
//! out and deletes the plist even if the test panics.

use std::path::{Path, PathBuf};

use launchkeeper_core::plist::{PlistOptions, write_plist};
use launchkeeper_core::task::{Task, TaskName, Trigger};
use launchkeeper_core::{Error, launchctl, paths};

/// Creates a `com.launchkeeper.test.<uuid>` plist in the real LaunchAgents
/// directory and guarantees cleanup on drop.
struct TempJob {
    name: TaskName,
    plist: PathBuf,
    _log_dir: tempfile::TempDir,
}

impl TempJob {
    fn new() -> TempJob {
        let uuid = uuid::Uuid::new_v4().to_string();
        let name = TaskName::new(&format!("test.{uuid}")).unwrap();
        assert!(name.label().starts_with("com.launchkeeper.test."));

        let log_dir = tempfile::tempdir().unwrap();
        // ProgramArguments 变成 ["/bin/echo", "run", "<name>"]：跑得快、退出码 0
        let mut task = Task::new(
            name.clone(),
            "/bin/echo",
            Trigger::Interval { seconds: 86400 },
        );
        task.display_name = "launchctl integration test".into();

        let plist = paths::plist_path(&task).unwrap();
        assert!(!plist.exists(), "uuid 撞车了？{} 已存在", plist.display());
        write_plist(
            &task,
            &PlistOptions {
                runner_path: Path::new("/bin/echo"),
                runner_log: &log_dir.path().join("runner.log"),
                data_dir: log_dir.path(),
            },
            &plist,
        )
        .unwrap();

        TempJob {
            name,
            plist,
            _log_dir: log_dir,
        }
    }

    fn label(&self) -> String {
        self.name.label()
    }
}

impl Drop for TempJob {
    fn drop(&mut self) {
        let _ = launchctl::bootout(&self.label());
        let _ = std::fs::remove_file(&self.plist);
    }
}

#[test]
#[ignore = "touches the real launchd domain"]
fn bootstrap_kickstart_bootout_round_trip() {
    let job = TempJob::new();
    let label = job.label();

    assert!(
        launchctl::status(&label).unwrap().is_none(),
        "测试开始前不该已经加载"
    );

    launchctl::bootstrap(&job.plist).unwrap();
    let st = launchctl::status(&label)
        .unwrap()
        .expect("bootstrap 之后应当能 print 到");
    assert!(!st.state.is_empty(), "state 不该为空: {st:?}");

    launchctl::kickstart(&label).unwrap();
    // kickstart 之后 job 可能已经退出，status 仍应是 Some
    assert!(launchctl::status(&label).unwrap().is_some());

    launchctl::bootout(&label).unwrap();
    assert!(
        launchctl::status(&label).unwrap().is_none(),
        "bootout 之后不该还在"
    );
}

#[test]
#[ignore = "touches the real launchd domain"]
fn bootstrap_twice_reports_the_eio_hint() {
    let job = TempJob::new();
    launchctl::bootstrap(&job.plist).unwrap();

    let err = launchctl::bootstrap(&job.plist).unwrap_err();
    match err {
        Error::Launchctl {
            ref command,
            code,
            ref stderr,
        } => {
            assert!(command.starts_with("bootstrap "), "{command}");
            assert!(
                stderr.contains("job 已加载，需先 bootout"),
                "EIO 时应带提示，实际 code={code:?} stderr={stderr:?}"
            );
        }
        other => panic!("期望 Error::Launchctl，得到 {other:?}"),
    }

    // reload 是修改后重新加载的唯一正确姿势，必须成功
    launchctl::reload(&job.plist, &job.label()).unwrap();
    assert!(launchctl::status(&job.label()).unwrap().is_some());
}

#[test]
#[ignore = "touches the real launchd domain"]
fn bootout_of_an_unloaded_label_is_ok() {
    let job = TempJob::new();
    let label = job.label();
    assert!(launchctl::status(&label).unwrap().is_none());
    launchctl::bootout(&label).unwrap();
    launchctl::bootout(&label).unwrap();
}

#[test]
#[ignore = "touches the real launchd domain"]
fn kickstart_of_an_unloaded_label_fails_loudly() {
    let job = TempJob::new();
    let err = launchctl::kickstart(&job.label()).unwrap_err();
    assert!(matches!(err, Error::Launchctl { .. }), "{err:?}");
}
