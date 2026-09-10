//! Path derivation and the PATH cache, exercised against a temporary data dir.
//!
//! Everything lives in one `#[test]` because it mutates process-wide
//! environment variables; cargo gives each integration test file its own
//! process, so nothing else is disturbed. The real `~/Library` is never
//! touched.

use launchkeeper_core::task::TaskName;
use launchkeeper_core::{Task, Trigger, env, logs, paths};

#[test]
fn data_dir_override_drives_every_derived_path() {
    let tmp = tempfile::tempdir().unwrap();
    let agents = tempfile::tempdir().unwrap();
    // SAFETY: single-threaded test process, set once before anything reads it.
    unsafe {
        std::env::set_var(paths::DATA_DIR_ENV, tmp.path());
        std::env::set_var(paths::LAUNCH_AGENTS_DIR_ENV, agents.path());
    }

    let name = TaskName::new("sync").unwrap();
    assert_eq!(paths::data_dir().unwrap(), tmp.path());
    assert_eq!(
        paths::db_path().unwrap(),
        tmp.path().join("launchkeeper.db")
    );
    assert_eq!(paths::logs_dir().unwrap(), tmp.path().join("logs"));
    assert_eq!(
        paths::task_log_dir(&name).unwrap(),
        tmp.path().join("logs").join("sync")
    );
    assert_eq!(
        paths::path_cache_file().unwrap(),
        tmp.path().join("login-shell-path")
    );
    assert_eq!(paths::launch_agents_dir().unwrap(), agents.path());
    let task = Task::new(name.clone(), "/usr/bin/true", Trigger::AtLogin);
    assert_eq!(
        paths::plist_path(&task).unwrap(),
        agents.path().join("com.launchkeeper.sync.plist")
    );
    assert_eq!(
        paths::own_plist_path(&name).unwrap(),
        agents.path().join("com.launchkeeper.sync.plist")
    );
    // 接管来的任务用存下来的路径，label 也不带前缀
    let mut adopted = Task::new(
        TaskName::new("com.example.report").unwrap(),
        "/usr/bin/true",
        Trigger::AtLogin,
    );
    adopted.adopted = true;
    adopted.plist_path = Some(agents.path().join("report.plist"));
    assert_eq!(adopted.label(), "com.example.report");
    assert_eq!(
        paths::plist_path(&adopted).unwrap(),
        agents.path().join("report.plist")
    );

    // PATH 缓存
    assert_eq!(env::read_path_cache().unwrap(), None);
    env::write_path_cache("/a/bin:/b/bin").unwrap();
    assert_eq!(
        env::read_path_cache().unwrap(),
        Some("/a/bin:/b/bin".to_string())
    );
    // 缓存文件是单行纯文本
    let raw = std::fs::read_to_string(paths::path_cache_file().unwrap()).unwrap();
    assert_eq!(raw, "/a/bin:/b/bin\n");
    // 有缓存时 effective_path 直接用缓存，不去 spawn shell
    assert_eq!(env::effective_path(), "/a/bin:/b/bin");
    // 空缓存视为没有缓存
    std::fs::write(paths::path_cache_file().unwrap(), "  \n").unwrap();
    assert_eq!(env::read_path_cache().unwrap(), None);

    // 真的抓一次 login shell PATH；抓不到就必须报错而不是给假值
    match env::capture_login_shell_path() {
        Ok(p) => {
            assert!(!p.is_empty());
            assert!(!p.contains('\n'), "PATH 不应含换行: {p:?}");
        }
        Err(e) => {
            let msg = e.to_string();
            assert!(msg.contains("抓取 login shell PATH 失败"), "{msg}");
        }
    }
    assert!(env::FALLBACK_PATH.starts_with("/opt/homebrew/bin:"));

    // create_run_logs 会建目录、建文件，并在同秒冲突时加后缀
    let now = chrono::Utc::now();
    let first = logs::create_run_logs(&name, now).unwrap();
    let (out, err) = (first.stdout_path.clone(), first.stderr_path.clone());
    assert!(out.starts_with(tmp.path().join("logs").join("sync")));
    assert!(out.parent().unwrap().is_dir());
    assert!(out.exists() && err.exists());
    let second = logs::create_run_logs(&name, now).unwrap();
    let (out2, err2) = (second.stdout_path.clone(), second.stderr_path.clone());
    assert_ne!(out, out2);
    assert_ne!(err, err2);
    assert!(
        out2.file_name().unwrap().to_string_lossy().contains("-1."),
        "{out2:?}"
    );

    // data_dir() 的第三级来源：~/.config/launchkeeper/data-dir 一行文本文件，
    // 优先级低于 LAUNCHKEEPER_DATA_DIR 环境变量、高于默认路径。用临时
    // XDG_CONFIG_HOME 隔离，不碰真实 ~/.config。
    let xdg = tempfile::tempdir().unwrap();
    let config_dir_dir = xdg.path().join("launchkeeper");
    let config_file = config_dir_dir.join("data-dir");
    unsafe {
        std::env::set_var(paths::XDG_CONFIG_HOME_ENV, xdg.path());
    }
    assert_eq!(paths::config_dir().unwrap(), config_dir_dir);
    assert_eq!(paths::config_data_dir_file().unwrap(), config_file);

    // LAUNCHKEEPER_DATA_DIR 仍然设置着（本测试函数开头设的 tmp），即使配置文件
    // 存在也应该优先用环境变量。
    std::fs::create_dir_all(&config_dir_dir).unwrap();
    std::fs::write(&config_file, "/from/config/file\n").unwrap();
    let (dir, source) = paths::data_dir_source().unwrap();
    assert_eq!(dir, tmp.path());
    assert_eq!(source, paths::DataDirSource::Env);
    assert_eq!(source.as_str(), "env");

    // 清掉环境变量后，配置文件生效。
    unsafe {
        std::env::remove_var(paths::DATA_DIR_ENV);
    }
    let (dir, source) = paths::data_dir_source().unwrap();
    assert_eq!(dir, std::path::PathBuf::from("/from/config/file"));
    assert_eq!(source, paths::DataDirSource::ConfigFile);
    assert_eq!(source.as_str(), "config_file");
    assert_eq!(paths::data_dir().unwrap(), dir);

    // 空白配置文件视为不存在，落到默认路径（依赖真实 home_dir，只算路径不碰
    // 文件系统，所以可以放心和 dirs::home_dir() 比较）。
    std::fs::write(&config_file, "   \n").unwrap();
    let (dir, source) = paths::data_dir_source().unwrap();
    assert_eq!(source, paths::DataDirSource::Default);
    assert_eq!(source.as_str(), "default");
    assert_eq!(
        dir,
        dirs::home_dir()
            .unwrap()
            .join("Library")
            .join("Application Support")
            .join("Launchkeeper")
    );

    // 配置文件整个不存在也一样落到默认路径。
    std::fs::remove_file(&config_file).unwrap();
    assert_eq!(
        paths::data_dir_source().unwrap().1,
        paths::DataDirSource::Default
    );

    // 复原：后面（如果将来在此文件追加测试代码）依赖 LAUNCHKEEPER_DATA_DIR 指向 tmp。
    unsafe {
        std::env::set_var(paths::DATA_DIR_ENV, tmp.path());
        std::env::remove_var(paths::XDG_CONFIG_HOME_ENV);
    }
}
