//! The two things `launchkeeper` prints for the benefit of an AI assistant
//! rather than a human: the CLI reference itself, and the short prompt that
//! points an assistant at it (M4 §3, "external agents").
//!
//! Both are compile-time constants. `docs/CLI.md` is pulled in with
//! [`include_str!`], which cargo records in the crate's dep-info, so editing
//! the Markdown rebuilds this crate — no `build.rs` and no
//! `cargo:rerun-if-changed` needed. The point of embedding it at all is that
//! `launchkeeper docs` has to work from an installed binary, with no
//! checkout of the repository anywhere on the machine.

use launchkeeper_core::ai::Lang;

/// `docs/CLI.md`, verbatim.
pub const CLI_REFERENCE: &str = include_str!("../../../docs/CLI.md");

/// The prompt in Chinese.
///
/// The trigger flags named here are the real ones: `-d` is `--daily` (a time
/// of day) and `-e` is `--every` (an interval) — see `trigger_args.rs`.
const PROMPT_ZH: &str = "\
本机安装了 Launchkeeper，一个基于 macOS launchd 的定时任务 / 常驻服务管理器，命令行工具是 `launchkeeper`。
操作前先运行 `launchkeeper docs` 阅读完整 CLI 参考（子命令、参数、JSON 输出格式）。
约定：
- 查询类命令一律加 `--json`，按结构化输出解析，不要解析人类可读文本。
- 创建任务用 `launchkeeper add`，脚本路径给 `--script`，运行方式一般让它自动推断；触发规则用 `-d`（每天几点）/`-e`（每隔多久）/`-m`（手动启停的服务）。
- 修改前先 `launchkeeper show <name> --json` 看当前定义；改完用 `launchkeeper runs <name>` 和 `launchkeeper log <name>` 验证。
- 只操作 Launchkeeper 管理的任务；`launchkeeper agents` 列出的其他 LaunchAgents 除非用户要求接管，否则只读。
- 破坏性操作（rm、unadopt、off）先向用户确认。
";

/// The same prompt in English.
const PROMPT_EN: &str = "\
Launchkeeper is installed on this machine: a manager for scheduled tasks and long-running services built on macOS launchd. Its command-line tool is `launchkeeper`.
Before you do anything, run `launchkeeper docs` and read the full CLI reference (subcommands, flags, JSON output shapes).
Conventions:
- Always pass `--json` to read-only commands and parse the structured output; never parse the human-readable text.
- Create a task with `launchkeeper add`: give the script path to `--script` and normally let it infer how to run it; choose the trigger with `-d` (a time of day) / `-e` (an interval) / `-m` (a service you start and stop by hand).
- Before changing anything, run `launchkeeper show <name> --json` to see the current definition; afterwards verify with `launchkeeper runs <name>` and `launchkeeper log <name>`.
- Only touch tasks Launchkeeper manages; the other LaunchAgents that `launchkeeper agents` lists are read-only unless the user asks you to adopt one.
- Ask the user before any destructive operation (rm, unadopt, off).
";

/// The prompt in `lang`.
#[must_use]
pub fn prompt(lang: Lang) -> &'static str {
    match lang {
        Lang::ZhCn => PROMPT_ZH,
        Lang::En => PROMPT_EN,
    }
}

/// The language to print human text in when the user did not ask for one.
///
/// `LC_ALL` wins over `LANG`, as it does everywhere else in a POSIX shell.
/// An unset (or empty) environment answers Chinese, because that is what
/// every other line this binary prints is written in: an assistant reading a
/// Chinese `show` and an English prompt in the same terminal is the odd
/// combination, not the useful one.
#[must_use]
pub fn default_lang() -> Lang {
    for var in ["LC_ALL", "LANG"] {
        match std::env::var(var) {
            Ok(v) if !v.trim().is_empty() => return Lang::from_tag(&v),
            _ => {}
        }
    }
    Lang::ZhCn
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reference_is_embedded_whole() {
        assert!(CLI_REFERENCE.starts_with("# `launchkeeper` reference"));
        assert!(CLI_REFERENCE.contains("### `agents`"));
    }

    #[test]
    fn both_prompts_name_the_docs_command() {
        for lang in [Lang::ZhCn, Lang::En] {
            assert!(prompt(lang).contains("launchkeeper docs"));
            assert!(prompt(lang).contains("--json"));
        }
    }

    /// The flags in the prompt have to be the flags `add` actually takes.
    #[test]
    fn the_prompt_names_the_real_trigger_flags() {
        assert!(PROMPT_ZH.contains("`-d`（每天几点）"));
        assert!(PROMPT_ZH.contains("`-e`（每隔多久）"));
        assert!(PROMPT_EN.contains("`-d` (a time of day)"));
        assert!(PROMPT_EN.contains("`-e` (an interval)"));
    }
}
