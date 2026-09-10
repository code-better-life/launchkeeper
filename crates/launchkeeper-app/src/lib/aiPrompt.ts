// 「给 AI 的提示词」（docs/M4-design.md §4）。
//
// 设置面板里那块只读文本：用户复制它，粘给 Claude Code / Codex 之类的终端助手，
// 助手就知道这台机器上有 `launchkeeper`、该先读 `launchkeeper docs`、查询要加
// `--json`、破坏性操作要先问。
//
// 刻意不放进词典（`i18n/zh-CN.ts`）：那里是一句一条的界面文案，这是一整段带
// 换行和反引号的多行文本，塞进去会把词典读成一堵墙；而且它是**要被复制走的
// 内容**，不是界面上的标签，纯函数 + 单测更合适。

import type { Locale } from "./i18n";

const ZH = `本机安装了 Launchkeeper，一个基于 macOS launchd 的定时任务 / 常驻服务管理器，命令行工具是 \`launchkeeper\`。
操作前先运行 \`launchkeeper docs\` 阅读完整 CLI 参考（子命令、参数、JSON 输出格式）。
约定：
- 查询类命令一律加 \`--json\`，按结构化输出解析，不要解析人类可读文本。
- 创建任务用 \`launchkeeper add\`，脚本路径给 \`--script\`，运行方式一般让它自动推断；触发规则用 \`-e\`（每天几点）/\`-d\`（每隔多久）/\`-m\`（手动启停的服务）。
- 修改前先 \`launchkeeper show <name> --json\` 看当前定义；改完用 \`launchkeeper runs <name>\` 和 \`launchkeeper log <name>\` 验证。
- 只操作 Launchkeeper 管理的任务；\`launchkeeper agents\` 列出的其他 LaunchAgents 除非用户要求接管，否则只读。
- 破坏性操作（rm、unadopt、off）先向用户确认。`;

const EN = `This machine has Launchkeeper installed: a manager for scheduled tasks and long-running services built on macOS launchd. Its command-line tool is \`launchkeeper\`.
Before doing anything, run \`launchkeeper docs\` to read the full CLI reference (subcommands, flags, JSON output shapes).
Conventions:
- Always pass \`--json\` to read-only commands and parse the structured output; never parse the human-readable text.
- Create a task with \`launchkeeper add\`, give the script path to \`--script\`, and normally let it infer how to run the script; set the trigger with \`-e\` (daily at a time) / \`-d\` (every interval) / \`-m\` (a service you start and stop by hand).
- Before changing anything, run \`launchkeeper show <name> --json\` to see the current definition; afterwards verify with \`launchkeeper runs <name>\` and \`launchkeeper log <name>\`.
- Only touch tasks Launchkeeper manages; the other LaunchAgents listed by \`launchkeeper agents\` are read-only unless the user asks you to adopt one.
- Confirm with the user before any destructive operation (rm, unadopt, off).`;

/** 这段提示词，按界面当前语言。 */
export function aiPrompt(locale: Locale): string {
  return locale === "zh-CN" ? ZH : EN;
}
