// 纯前端 mock：VITE_MOCK=1 时替代 bindings.ts 的 commands / events，
// 让 UI 不依赖真实 Tauri 后端也能跑起来做界面开发/截图。
// 不导出给生产路径使用；api.ts 按 import.meta.env.VITE_MOCK 二选一。
import type {
  AdoptionPlanView,
  AiConfig,
  AppError,
  AppSettings,
  InsightView,
  ExternalAgentView,
  InterpreterView,
  LogChunk,
  LogStream,
  RunHandle,
  RunView,
  TaskDetail,
  TaskInput,
  TaskView,
  TasksChanged,
} from "./bindings";
import { detectFromTask, displayName } from "./interpreter";

type Result<T, E> = { status: "ok"; data: T } | { status: "error"; error: E };
function ok<T>(data: T): Result<T, AppError> {
  return { status: "ok", data };
}
function err<T>(message: string): Result<T, AppError> {
  return { status: "error", error: { message } };
}

const now = () => new Date().toISOString();
const minutesAgo = (m: number) => new Date(Date.now() - m * 60_000).toISOString();
const hoursAgo = (h: number) => new Date(Date.now() - h * 3_600_000).toISOString();

interface MockTask {
  input: TaskInput;
  enabled: boolean;
  loaded_pid: number | null;
  created_at: string;
  updated_at: string;
  runs: RunView[];
  /** M3 §3.4：接管来的任务，详情页多一行「接管自 …·撤销接管」。 */
  adopted?: boolean;
  plist_path?: string;
}

const tasks = new Map<string, MockTask>();

tasks.set("report-sync", {
  enabled: true,
  loaded_pid: null,
  created_at: hoursAgo(240),
  updated_at: hoursAgo(48),
  input: {
    name: "report-sync",
    display_name: "夜间报告同步",
    description: "每天 21:00 从对象存储拉新文件到云盘",
    // 用 uv run 跑项目里的脚本：拆开来是「脚本 sync.py + 运行方式 uv run」
    script_path: "/Users/demo/.local/bin/uv",
    args: ["run", "/Users/demo/projects/report/sync.py"],
    working_dir: null,
    env: [],
    trigger: { kind: "calendar", entries: [{ hour: 21, minute: 0, weekday: null, day: null }] },
    keep_alive: false,
    timeout_secs: 600,
    tags: ["同步", "定时"],
    favorite: true,
    notify_on_fail: true,
  },
  runs: [
    {
      id: 3,
      started_at: hoursAgo(3),
      finished_at: hoursAgo(3),
      exit_code: 0,
      duration_ms: 4200,
      trigger_kind: "scheduled",
      stop_reason: "exited",
      running: false,
    },
    {
      id: 2,
      started_at: hoursAgo(27),
      finished_at: hoursAgo(27),
      exit_code: 0,
      duration_ms: 3900,
      trigger_kind: "scheduled",
      stop_reason: "exited",
      running: false,
    },
  ],
});

tasks.set("log-cleanup", {
  enabled: true,
  loaded_pid: null,
  created_at: hoursAgo(500),
  updated_at: hoursAgo(500),
  input: {
    name: "log-cleanup",
    display_name: "日志清理",
    description: "清理超过 30 天的日志文件",
    script_path: "/Users/demo/scripts/log-cleanup.sh",
    args: ["--days", "30"],
    working_dir: "/Users/demo",
    env: [{ key: "LOG_DIR", value: "/var/log/demo" }],
    trigger: { kind: "interval", seconds: 3600 * 6 },
    keep_alive: false,
    timeout_secs: null,
    tags: ["清理"],
    favorite: false,
    notify_on_fail: false,
  },
  runs: [
    {
      id: 5,
      started_at: hoursAgo(1),
      finished_at: hoursAgo(1),
      // 超时被杀，runner 记的退出码是 124
      exit_code: 124,
      duration_ms: 800,
      trigger_kind: "scheduled",
      stop_reason: "timeout",
      running: false,
    },
  ],
});

tasks.set("watch-dog", {
  enabled: true,
  loaded_pid: 8123,
  created_at: hoursAgo(30),
  updated_at: hoursAgo(30),
  input: {
    name: "watch-dog",
    display_name: "常驻看门狗",
    description: "登录时启动的常驻监控脚本",
    script_path: "/Users/demo/scripts/watchdog.sh",
    args: [],
    working_dir: null,
    env: [],
    trigger: { kind: "at_login" },
    keep_alive: true,
    timeout_secs: null,
    tags: [],
    favorite: false,
    notify_on_fail: false,
  },
  runs: [
    {
      id: 6,
      started_at: minutesAgo(2),
      finished_at: null,
      exit_code: null,
      duration_ms: null,
      trigger_kind: "manual",
      stop_reason: null,
      running: true,
    },
  ],
});

// M2.5：一个正在跑的常驻服务，一个停着的常驻服务。
tasks.set("wiki-bridge", {
  enabled: true,
  loaded_pid: 44192,
  created_at: hoursAgo(72),
  updated_at: hoursAgo(72),
  input: {
    name: "wiki-bridge",
    display_name: "审批 bridge",
    description: "常驻的 HTTP bridge，手动启停；崩溃后自动重启",
    script_path: "/Users/demo/scripts/wiki-bridge.sh",
    args: ["--port", "8765"],
    working_dir: null,
    env: [],
    trigger: { kind: "manual" },
    keep_alive: true,
    timeout_secs: null,
    tags: ["服务"],
    favorite: true,
    notify_on_fail: true,
  },
  runs: [
    {
      id: 21,
      started_at: minutesAgo(133),
      finished_at: null,
      exit_code: null,
      duration_ms: null,
      trigger_kind: "manual",
      stop_reason: null,
      running: true,
    },
    {
      id: 20,
      started_at: hoursAgo(9),
      finished_at: hoursAgo(3),
      exit_code: null,
      duration_ms: 6 * 3_600_000,
      trigger_kind: "manual",
      stop_reason: "stopped",
      running: false,
    },
  ],
});

tasks.set("mail-poller", {
  enabled: true,
  loaded_pid: null,
  created_at: hoursAgo(100),
  updated_at: hoursAgo(100),
  input: {
    name: "mail-poller",
    display_name: "邮件轮询服务",
    description: "停着的常驻服务，按需启动",
    script_path: "/usr/bin/python3",
    args: ["/Users/demo/scripts/mail-poller.py"],
    working_dir: null,
    env: [{ key: "IMAP_HOST", value: "imap.example.com" }],
    trigger: { kind: "manual" },
    keep_alive: false,
    timeout_secs: null,
    tags: ["服务"],
    favorite: false,
    notify_on_fail: false,
  },
  runs: [
    {
      id: 18,
      started_at: hoursAgo(26),
      finished_at: hoursAgo(25),
      exit_code: 0,
      duration_ms: 3_600_000,
      trigger_kind: "manual",
      stop_reason: "stopped",
      running: false,
    },
  ],
});

tasks.set("never-run", {
  enabled: false,
  loaded_pid: null,
  created_at: hoursAgo(5),
  updated_at: hoursAgo(5),
  input: {
    name: "never-run",
    display_name: "尚未运行的新任务",
    description: null,
    script_path: "/Users/demo/scripts/todo.sh",
    args: [],
    working_dir: null,
    env: [],
    trigger: { kind: "at_login" },
    keep_alive: false,
    timeout_secs: null,
    tags: ["草稿"],
    favorite: false,
    notify_on_fail: false,
  },
  runs: [],
});

// M3 §3.4：一个已经接管过的任务。名字就是 label（TaskName 放宽后允许），
// plist 还在用户自己那个位置，详情页据此显示「接管自 …·撤销接管」。
tasks.set("com.example.mail-archive", {
  enabled: true,
  adopted: true,
  plist_path: "/Users/demo/Library/LaunchAgents/com.example.mail-archive.plist",
  loaded_pid: null,
  created_at: hoursAgo(20),
  updated_at: hoursAgo(20),
  input: {
    name: "com.example.mail-archive",
    display_name: "com.example.mail-archive",
    description: "接管自手写 plist；原日志在 ~/Library/Logs/mail-archive.log",
    script_path: "/bin/zsh",
    args: ["/Users/demo/scripts/mail-archive.sh"],
    working_dir: "/Users/demo",
    env: [],
    trigger: { kind: "interval", seconds: 7200 },
    keep_alive: false,
    timeout_secs: null,
    tags: [],
    favorite: false,
    notify_on_fail: false,
  },
  runs: [
    {
      id: 40,
      started_at: hoursAgo(2),
      finished_at: hoursAgo(2),
      exit_code: 0,
      duration_ms: 1500,
      trigger_kind: "scheduled",
      stop_reason: "exited",
      running: false,
    },
  ],
});

// M3 §3.4：机器上别人写的 LaunchAgents。三种情形各一个——可接管的手写 plist、
// 已经被 Launchkeeper 接管的（core 仍然把它列出来，managed = true）、以及
// 第三方 App 装的（灰掉、只展示）。
const externalAgents: ExternalAgentView[] = [
  {
    label: "com.example.backup-notes",
    path: "~/Library/LaunchAgents/com.example.backup-notes.plist",
    full_path: "/Users/demo/Library/LaunchAgents/com.example.backup-notes.plist",
    command: "/bin/zsh /Users/demo/scripts/backup-notes.sh",
    trigger_text: "每周日 03:00",
    trigger: { kind: "calendar", entries: [{ minute: 0, hour: 3, weekday: 0, day: null }] },
    working_dir: "~/Notes",
    stdout_path: "~/Library/Logs/backup-notes.log",
    stderr_path: "~/Library/Logs/backup-notes.log",
    keep_alive: false,
    loaded: true,
    pid: null,
    last_exit: 0,
    adoptable: true,
    adopt_reason: null,
    managed: false,
  },
  {
    label: "com.example.mail-archive",
    path: "~/Library/LaunchAgents/com.example.mail-archive.plist",
    full_path: "/Users/demo/Library/LaunchAgents/com.example.mail-archive.plist",
    command: "/Users/demo/opt/launchkeeper/bin/launchkeeper-runner run com.example.mail-archive",
    trigger_text: "每 2 小时",
    trigger: { kind: "interval", seconds: 7200 },
    working_dir: "~",
    stdout_path: null,
    stderr_path: null,
    keep_alive: false,
    loaded: true,
    pid: null,
    last_exit: 0,
    adoptable: false,
    adopt_reason: "已经由 Launchkeeper 管理",
    managed: true,
  },
  {
    label: "com.example.photo-sync",
    path: "~/Library/LaunchAgents/com.example.photo-sync.plist",
    full_path: "/Users/demo/Library/LaunchAgents/com.example.photo-sync.plist",
    command: "/usr/bin/python3 /Users/demo/scripts/photo-sync.py --incremental",
    trigger_text: "每天 02:30",
    trigger: { kind: "calendar", entries: [{ minute: 30, hour: 2, weekday: null, day: null }] },
    working_dir: "~/Pictures",
    stdout_path: "~/Library/Logs/photo-sync.log",
    stderr_path: null,
    keep_alive: false,
    loaded: false,
    pid: null,
    last_exit: null,
    adoptable: true,
    adopt_reason: null,
    managed: false,
  },
  {
    label: "homebrew.mxcl.postgresql@16",
    path: "~/Library/LaunchAgents/homebrew.mxcl.postgresql@16.plist",
    full_path: "/Users/demo/Library/LaunchAgents/homebrew.mxcl.postgresql@16.plist",
    command: "/opt/homebrew/opt/postgresql@16/bin/postgres -D /opt/homebrew/var/postgresql@16",
    trigger_text: "登录时启动",
    trigger: { kind: "at_login" },
    working_dir: null,
    stdout_path: "/opt/homebrew/var/log/postgresql@16.log",
    stderr_path: "/opt/homebrew/var/log/postgresql@16.log",
    keep_alive: true,
    loaded: true,
    pid: 812,
    last_exit: null,
    adoptable: false,
    adopt_reason: "label 前缀 homebrew.mxcl.，由 brew services 管理",
    managed: false,
  },
  {
    label: "com.adobe.ccxprocess",
    path: "~/Library/LaunchAgents/com.adobe.ccxprocess.plist",
    full_path: "/Users/demo/Library/LaunchAgents/com.adobe.ccxprocess.plist",
    command: "/Applications/Utilities/Adobe Creative Cloud/CCXProcess/CCXProcess.app/Contents/MacOS/CCXProcess",
    trigger_text: "登录时启动",
    trigger: { kind: "at_login" },
    working_dir: null,
    stdout_path: null,
    stderr_path: null,
    keep_alive: false,
    loaded: true,
    pid: 640,
    last_exit: null,
    adoptable: false,
    adopt_reason: "程序在 .app 包里，属于第三方应用安装的组件",
    managed: false,
  },
  {
    label: "com.google.keystone.agent",
    path: "~/Library/LaunchAgents/com.google.keystone.agent.plist",
    full_path: "/Users/demo/Library/LaunchAgents/com.google.keystone.agent.plist",
    command: "/Library/Google/GoogleSoftwareUpdate/GoogleSoftwareUpdate.bundle/Contents/Helpers/ksadmin --ping",
    trigger_text: "每 1 小时",
    trigger: { kind: "interval", seconds: 3600 },
    working_dir: null,
    stdout_path: null,
    stderr_path: null,
    keep_alive: false,
    loaded: true,
    pid: null,
    last_exit: 0,
    adoptable: false,
    adopt_reason: "程序在 /Library 下，属于第三方应用安装的组件",
    managed: false,
  },
];

/** 后端 agents::adoption_plan 的假结果，形状照界面稿。 */
function mockPlan(agent: ExternalAgentView): AdoptionPlanView {
  return {
    label: agent.label,
    name: agent.label,
    path: agent.path,
    backup_path: `${agent.path}.bak`,
    removed: [
      { key: "EnvironmentVariables", value: "TZ=Asia/Shanghai" },
      { key: "ProgramArguments", value: agent.command },
      { key: "StandardOutPath", value: agent.stdout_path ?? "" },
    ],
    added: [
      {
        key: "EnvironmentVariables",
        value: "LAUNCHKEEPER_DATA_DIR=/Users/demo/opt/launchkeeper, TZ=Asia/Shanghai",
      },
      {
        key: "ProgramArguments",
        value: `/Users/demo/opt/launchkeeper/bin/launchkeeper-runner run ${agent.label}`,
      },
      { key: "LaunchkeeperManaged", value: "true" },
      { key: "ThrottleInterval", value: "1" },
    ],
    kept: [
      { key: "Label", value: agent.label },
      { key: "StartCalendarInterval", value: "Hour=3, Minute=0, Weekday=0" },
      { key: "WorkingDirectory", value: agent.working_dir ?? "" },
    ],
  };
}

// 会持续增长的“运行中”日志，用来练习 readLog 轮询。
const runningLog = { stdout: "启动中…\n", stderr: "" };
setInterval(() => {
  const anyRunning = Array.from(tasks.values()).some((t) => t.runs[0]?.running);
  if (anyRunning) {
    runningLog.stdout += `[${new Date().toLocaleTimeString("zh-CN")}] 心跳正常\n`;
  }
}, 1000);

function isService(t: MockTask): boolean {
  return t.input.trigger.kind === "manual";
}

/** 后端 commands.rs::status_of 的镜像，服务型和定时型两套判定。 */
function computeStatus(t: MockTask): TaskView["status"] {
  if (!t.enabled) return "disabled";
  const last = t.runs[0];
  if (isService(t)) {
    if (t.loaded_pid !== null) return "running";
    const badly =
      last != null &&
      last.finished_at !== null &&
      last.exit_code !== 0 &&
      last.stop_reason !== "stopped";
    if (t.input.keep_alive && badly) return "failed";
    return "stopped";
  }
  if (!last) return "never";
  if (last.running) return "running";
  if (last.exit_code === 0) return "ok";
  return "failed";
}

/** 后端 commands.rs::uptime_secs 的镜像。 */
function computeUptime(t: MockTask): number | null {
  if (t.loaded_pid === null) return null;
  const last = t.runs[0];
  if (!last || last.finished_at !== null) return null;
  return Math.max(0, Math.round((Date.now() - new Date(last.started_at).getTime()) / 1000));
}

/** 后端 commands.rs 里 `interpreter_label` 的镜像（core `Interpreter::label`）。 */
function interpreterLabel(input: TaskInput): string | null {
  const got = detectFromTask(input.script_path, input.args);
  if (!got) return null;
  // 这是后端 `interpreter_label` 的镜像，后端那一份本期仍然只有中文
  // （docs/M3-design.md §5），所以这里也固定中文。
  if (got.interpreter.kind === "direct") return "直接执行";
  return `${displayName(got.interpreter, "zh-CN")} · ${mockOrigin(got.interpreter.program)}`;
}

/** core `classify_origin` 的简化镜像，够 mock 用。 */
function mockOrigin(program: string): string {
  if (program.includes("/.venv/bin/")) return "项目 .venv";
  if (/^\/(usr\/)?s?bin\//.test(program)) return "系统";
  if (program.startsWith("/opt/homebrew/") || program.startsWith("/usr/local/"))
    return "Homebrew";
  if (program.includes("/.local/bin/"))
    return program.endsWith("/uv") ? "uv 安装" : "用户安装";
  return "PATH";
}


// 英文界面下把 mock 数据里的中文显示名 / 描述 / 标签换成英文（README 截图用）。
// 只在读出时翻译，写入仍按用户填的存。
const EN_TEXT: Record<string, string> = {
  "夜间报告同步": "Nightly report sync",
  "每天 21:00 从对象存储拉新文件到云盘": "Pulls new files from object storage to the cloud drive at 21:00 daily",
  "日志清理": "Log cleanup",
  "清理超过 30 天的日志文件": "Deletes log files older than 30 days",
  "常驻看门狗": "Resident watchdog",
  "登录时启动的常驻监控脚本": "Monitoring script that starts at login and stays resident",
  "审批 bridge": "Approval bridge",
  "常驻的 HTTP bridge，手动启停；崩溃后自动重启": "Resident HTTP bridge, started by hand; restarts after a crash",
  "邮件轮询服务": "Mail polling service",
  "停着的常驻服务，按需启动": "Idle resident service, started on demand",
  "尚未运行的新任务": "New task not run yet",
  "接管自手写 plist；原日志在 ~/Library/Logs/mail-archive.log": "Adopted from a hand-written plist; original log at ~/Library/Logs/mail-archive.log",
  "同步": "sync",
  "定时": "scheduled",
  "清理": "cleanup",
  "服务": "service",
  "草稿": "draft",
};
function uiIsEnglish(): boolean {
  try {
    return (document.documentElement.lang || "").toLowerCase().startsWith("en");
  } catch {
    return false;
  }
}
function tx(text: string): string;
function tx(text: string | null): string | null;
function tx(text: string | null): string | null {
  if (text == null) return text;
  return uiIsEnglish() ? (EN_TEXT[text] ?? text) : text;
}
function txInput(input: TaskInput): TaskInput {
  return uiIsEnglish()
    ? { ...input, display_name: tx(input.display_name), description: tx(input.description), tags: input.tags.map((x) => tx(x)) }
    : input;
}

function toView(t: MockTask): TaskView {
  return {
    name: t.input.name,
    display_name: tx(t.input.display_name),
    description: tx(t.input.description),
    trigger: t.input.trigger,
    trigger_text: describeTrigger(t.input.trigger),
    interpreter_label: interpreterLabel(t.input),
    is_service: isService(t),
    keep_alive: t.input.keep_alive,
    notify_on_fail: t.input.notify_on_fail,
    enabled: t.enabled,
    loaded_pid: t.loaded_pid,
    uptime_secs: computeUptime(t),
    last_run: t.runs[0] ?? null,
    tags: t.input.tags.map((x) => tx(x)),
    favorite: t.input.favorite,
    status: computeStatus(t),
  };
}

function describeTrigger(trigger: TaskInput["trigger"]): string {
  if (trigger.kind === "at_login") return "登录时启动";
  if (trigger.kind === "manual") return "手动启停";
  if (trigger.kind === "interval") {
    const s = trigger.seconds;
    if (s % 3600 === 0) return `每 ${s / 3600} 小时`;
    if (s % 60 === 0) return `每 ${s / 60} 分钟`;
    return `每 ${s} 秒`;
  }
  const first = trigger.entries[0];
  const hh = String(first.hour).padStart(2, "0");
  const mm = String(first.minute).padStart(2, "0");
  return `每天 ${hh}:${mm}${trigger.entries.length > 1 ? ` 等 ${trigger.entries.length} 条规则` : ""}`;
}

type Listener = (payload: TasksChanged) => void;
const listeners = new Set<Listener>();
function broadcast() {
  const payload = Array.from(tasks.values()).map(toView);
  for (const l of listeners) l(payload);
}

// 后端每 2 秒扫一次并在有变化时发事件；有服务在跑时 uptime_secs 每次都不一样，
// 所以真机上列表里的"运行 x 分 y 秒"是会动的。这里照做，免得 mock 里看着是死的。
setInterval(() => {
  if (Array.from(tasks.values()).some((t) => t.loaded_pid !== null)) broadcast();
}, 2000);

// scan_interpreters 的假结果：一台装了系统 python、Homebrew python/node、
// uv 的机器。推荐规则按扩展名走，和 core::interpreters::recommend 一致的那几条
// （.py -> python3 / uv run，.js -> node，可执行 + shebang -> 直接执行）。
function row(
  kind: InterpreterView["kind"],
  program: string,
  version: string | null,
  origin: InterpreterView["origin"],
  prefix: string[] = [],
): InterpreterView {
  const originText: Record<InterpreterView["origin"], string> = {
    system: "系统",
    homebrew: "Homebrew",
    project_venv: "项目 .venv",
    uv: "uv 安装",
    user_local: "用户安装",
    path: "PATH",
    custom: "自定义",
  };
  const base = program.split("/").pop() ?? program;
  const label = kind === "direct" ? "直接执行" : [base, ...prefix].join(" ");
  const place = `${originText[origin]} ${program}`;
  return {
    id: prefix.length > 0 ? `${program} ${prefix.join(" ")}` : program,
    kind,
    program,
    prefix_args: prefix,
    version,
    origin,
    label,
    detail:
      kind === "direct"
        ? "脚本自带 shebang，交给 launchd 直接运行"
        : version
          ? `${version} · ${place}`
          : place,
    recommended: false,
    reason: null,
    group: "path",
  };
}

function mockScan(script: string | null): InterpreterView[] {
  const list: InterpreterView[] = [];
  const ext = (script ?? "").split(".").pop()?.toLowerCase() ?? "";
  const venvProject = (script ?? "").includes("report");
  if (venvProject) {
    list.push(
      row("python", "/Users/demo/projects/report/.venv/bin/python", "3.13.2", "project_venv"),
    );
  }
  list.push(
    row("python", "/usr/bin/python3", "3.9.6", "system"),
    row("python", "/opt/homebrew/bin/python3", "3.13.2", "homebrew"),
    row("uv", "/Users/demo/.local/bin/uv", "0.8.4", "uv", ["run"]),
    row("node", "/opt/homebrew/opt/node@22/bin/node", "22.22.0", "homebrew"),
    row("ruby", "/usr/bin/ruby", "2.6.10", "system"),
    row("shell", "/bin/zsh", "5.9", "system"),
  );
  const shellish = ext === "sh" || ext === "bash" || ext === "zsh";
  if (script && shellish) list.push(row("direct", script, null, "custom"));

  let pick: InterpreterView | undefined;
  let reason: string | null = null;
  if (script && shellish) {
    pick = list.find((i) => i.kind === "direct");
    reason = "脚本可执行且带 shebang，交给 launchd 直接运行";
  } else if (ext === "py") {
    pick = list.find((i) => i.origin === "project_venv") ?? list[venvProject ? 1 : 0];
    reason = venvProject
      ? "按 .py 扩展名推荐；项目里有 .venv"
      : "按 .py 扩展名推荐；项目里没有 .venv 或 uv.lock";
  } else if (ext === "js" || ext === "mjs") {
    pick = list.find((i) => i.kind === "node");
    reason = `按 .${ext} 扩展名推荐`;
  } else if (ext === "rb") {
    pick = list.find((i) => i.kind === "ruby");
    reason = "按 .rb 扩展名推荐";
  }
  if (pick) {
    pick.recommended = true;
    pick.reason = reason;
    pick.group = "recommended";
  }
  for (const i of list) {
    if (!i.recommended) i.group = i.origin === "project_venv" ? "project" : "path";
  }
  return [...list.filter((i) => i.recommended), ...list.filter((i) => !i.recommended)];
}

let nextId = 100;

// ---- M3 §4：设置 / 失败通知 ----
// 后端 `settings::AppSettings::default()` 的镜像；VITE_MOCK 下没有真实
// settings.json，就用一个模块级变量当它，刷新页面就丢，够界面开发用。
let mockSettings: AppSettings = {
  notify_on_task_failure: true,
  notify_on_service_crash: true,
  language: null,
  list_sort: null,
  theme: null,
};

// ---- M4 §1 / §4：AI ----
// `ai.json` 和钥匙串的内存替身。默认「已配置」，这样 VITE_MOCK 下打开「AI 解读」
// 标签页看到的是正常路径；点「清除」之后就能看没有 Key 时的报错长什么样。
let mockAiConfig: Required<AiConfig> = {
  provider: "anthropic",
  base_url: null,
  model: "claude-sonnet-5",
};
let mockApiKey: string | null = "sk-ant-mock-key";

const MOCK_INSIGHT_MD = `## 这个任务在做什么

每天 21:00 用 \`uv run\` 跑 \`sync.py\`，从对象存储拉当天新增的文件 写进
OneDrive 同步目录。触发方式是 \`StartCalendarInterval\`，机器睡过 21:00 的话
launchd 会在唤醒后补跑一次。

## 最近健不健康

最近 10 次运行里 **9 次成功**，1 次在 3 天前以退出码 1 结束。失败那次的日志
尾部是：

\`\`\`
s3: connection reset by peer
\`\`\`

看起来是一次网络抖动，之后每一次都成功了，不像是配置问题。

## 建议

- 给它加一个重试（脚本内 2~3 次退避重试），一次抖动就不必等到第二天。
- 任务设了 \`AWS_PROFILE\`，而 launchd 拉起来的进程只有
  \`/usr/bin:/bin:/usr/sbin:/sbin\`——确认凭据文件的路径是绝对路径。
- 超时目前是 30 分钟，按最近的运行时长（都在 1 分钟内）可以收紧到 5 分钟，
  卡住时能更早发现。`;

/** 后端 `AppError::ai` 对 `Error::AiNoKey` 的改写，一字不差地抄过来。 */
const NO_KEY_MESSAGE =
  "还没有配置 AI API Key：在「设置 → AI」里填一个 / No AI API key configured: add one under Settings → AI";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** 一个任务一条，和后端 `ai_insights` 表的主键一致（M4 §1.3）。 */
const insights = new Map<string, InsightView>([
  [
    "report-sync",
    {
      task_name: "report-sync",
      created_at: hoursAgo(20),
      model: "claude-sonnet-5",
      prompt_hash: "3f2a9c1e".repeat(8),
      content: MOCK_INSIGHT_MD,
    },
  ],
]);

export const mockCommands = {
  listTasks: async () => ok(Array.from(tasks.values()).map(toView)),
  getTask: async (name: string) => {
    const t = tasks.get(name);
    if (!t) return err<TaskDetail>(`任务不存在：${name}`);
    return ok<TaskDetail>({
      task: txInput(t.input),
      created_at: t.created_at,
      updated_at: t.updated_at,
      view: toView(t),
      adopted: t.adopted ?? false,
      plist_path: t.plist_path ?? null,
    });
  },
  createTask: async (input: TaskInput) => {
    if (tasks.has(input.name)) return err<TaskView>(`任务名已存在：${input.name}`);
    const t: MockTask = {
      input,
      enabled: false,
      loaded_pid: null,
      created_at: now(),
      updated_at: now(),
      runs: [],
    };
    tasks.set(input.name, t);
    broadcast();
    return ok(toView(t));
  },
  updateTask: async (name: string, input: TaskInput) => {
    const t = tasks.get(name);
    if (!t) return err<TaskView>(`任务不存在：${name}`);
    t.input = input;
    t.updated_at = now();
    broadcast();
    return ok(toView(t));
  },
  deleteTask: async (name: string) => {
    if (!tasks.has(name)) return err<null>(`任务不存在：${name}`);
    tasks.delete(name);
    broadcast();
    return ok(null);
  },
  setEnabled: async (name: string, enabled: boolean) => {
    const t = tasks.get(name);
    if (!t) return err<TaskView>(`任务不存在：${name}`);
    t.enabled = enabled;
    broadcast();
    return ok(toView(t));
  },
  runNow: async (name: string) => {
    const t = tasks.get(name);
    if (!t) return err<null>(`任务不存在：${name}`);
    if (isService(t)) {
      // 服务型任务的"立即运行"就是"启动"，和后端 run_now 一致。
      await mockCommands.startService(name);
      return ok(null);
    }
    const id = nextId++;
    t.runs.unshift({
      id,
      started_at: now(),
      finished_at: null,
      exit_code: null,
      duration_ms: null,
      trigger_kind: "manual",
      stop_reason: null,
      running: true,
    });
    broadcast();
    setTimeout(() => {
      const run = t.runs.find((r) => r.id === id);
      if (run) {
        run.finished_at = now();
        run.exit_code = 0;
        run.duration_ms = 2500;
        run.running = false;
        broadcast();
      }
    }, 2500);
    return ok(null);
  },
  startService: async (name: string) => {
    const t = tasks.get(name);
    if (!t) return err<TaskView>(`任务不存在：${name}`);
    if (t.loaded_pid === null) {
      // start 会顺手 enable（后端 Service::start 也是这样）
      t.enabled = true;
      t.loaded_pid = 40000 + Math.floor(Math.random() * 20000);
      t.runs.unshift({
        id: nextId++,
        started_at: now(),
        finished_at: null,
        exit_code: null,
        duration_ms: null,
        trigger_kind: "manual",
        stop_reason: null,
        running: true,
      });
      broadcast();
    }
    return ok(toView(t));
  },
  stopService: async (name: string) => {
    const t = tasks.get(name);
    if (!t) return err<TaskView>(`任务不存在：${name}`);
    const run = t.runs[0];
    if (t.loaded_pid !== null && run?.running) {
      t.loaded_pid = null;
      run.finished_at = now();
      // 子进程被 SIGTERM 打死 -> 没有退出码，只有 stop_reason
      run.exit_code = null;
      run.duration_ms = Date.now() - new Date(run.started_at).getTime();
      run.stop_reason = "stopped";
      run.running = false;
      broadcast();
    }
    return ok(toView(t));
  },
  restartService: async (name: string) => {
    await mockCommands.stopService(name);
    return mockCommands.startService(name);
  },
  listRuns: async (name: string, limit: number) => {
    const t = tasks.get(name);
    if (!t) return err<RunView[]>(`任务不存在：${name}`);
    return ok(t.runs.slice(0, limit));
  },
  readLog: async (runId: RunHandle, stream: LogStream, tailBytes: number) => {
    const isRunning = Array.from(tasks.values()).some((t) =>
      t.runs.some((r) => r.id === runId && r.running),
    );
    let content: string;
    if (isRunning) {
      content = stream === "stdout" ? runningLog.stdout : runningLog.stderr;
    } else {
      content =
        stream === "stdout"
          ? `[mock] run ${runId} 标准输出示例\n第二行输出\n完成。\n`
          : `[mock] run ${runId} 无标准错误输出\n`;
    }
    const total = content.length;
    const truncated = tailBytes > 0 && total > tailBytes;
    const chunk: LogChunk = {
      content: truncated ? content.slice(-tailBytes) : content,
      total_bytes: total,
      truncated,
    };
    return ok(chunk);
  },
  scanInterpreters: async (script: string | null, _projectDir: string | null) => {
    // 真机上要跑一遍 PATH 扫描 + 每个程序一次 --version，这里也慢一点，
    // 好让防抖和"扫描中…"的状态在 mock 下也能看到。
    await new Promise((r) => setTimeout(r, 120));
    return ok(mockScan(script));
  },
  checkScript: async (path: string) => {
    // mock 下没有真实文件系统：只要看着像个绝对路径就当它存在且可执行，好让
    // 表单的校验路径可以被点到，又不至于把每个演示任务都判成"脚本不存在"。
    const okish = path.startsWith("/");
    return ok({ exists: okish, executable: okish });
  },
  taskLogDir: async (name: string) => {
    if (!tasks.has(name)) return err<string>(`任务不存在：${name}`);
    return ok(`/Users/demo/Library/Application Support/Launchkeeper/logs/${name}`);
  },
  revealInFinder: async (path: string) => {
    console.info("[mock] 在 Finder 中显示", path);
    return ok(null);
  },

  // ---- M3 §3.4：其他 LaunchAgents ----
  listExternalAgents: async () => {
    // 真机上要读一遍 ~/Library/LaunchAgents 并对每个 label 问一次 launchctl，
    // 这里也慢一点，好让「扫描中…」在 mock 下也能被看到。
    await new Promise((r) => setTimeout(r, 80));
    return ok(externalAgents.map((a) => ({ ...a })));
  },
  adoptionPlan: async (label: string) => {
    const a = externalAgents.find((x) => x.label === label);
    if (!a) return err<AdoptionPlanView>(`找不到 LaunchAgent：${label}`);
    if (!a.adoptable)
      return err<AdoptionPlanView>(`${label}: ${a.adopt_reason ?? "不可接管"}`);
    await new Promise((r) => setTimeout(r, 80));
    return ok(mockPlan(a));
  },
  adoptAgent: async (label: string, reload: boolean) => {
    const a = externalAgents.find((x) => x.label === label);
    if (!a) return err<TaskView>(`找不到 LaunchAgent：${label}`);
    if (!a.adoptable) return err<TaskView>(`${label}: ${a.adopt_reason ?? "不可接管"}`);
    // 接管后 label 和文件位置都不变，那一行仍然在「其他 LaunchAgents」里列着，
    // 只是从此归 Launchkeeper 管（core 的 managed_by_launchkeeper）。
    a.managed = true;
    a.adoptable = false;
    a.adopt_reason = "已经由 Launchkeeper 管理";
    a.command = `/Users/demo/opt/launchkeeper/bin/launchkeeper-runner run ${label}`;
    a.loaded = reload ? true : a.loaded;
    const t: MockTask = {
      input: {
        name: label,
        display_name: label,
        description: a.stdout_path ? `接管自手写 plist；原日志在 ${a.stdout_path}` : "接管自手写 plist",
        script_path: "/bin/zsh",
        args: ["/Users/demo/scripts/backup-notes.sh"],
        working_dir: a.working_dir,
        env: [],
        trigger: { kind: "calendar", entries: [{ hour: 3, minute: 0, weekday: 0, day: null }] },
        keep_alive: a.keep_alive,
        timeout_secs: null,
        tags: [],
        favorite: false,
        notify_on_fail: false,
      },
      enabled: a.loaded,
      adopted: true,
      plist_path: a.full_path,
      loaded_pid: null,
      created_at: now(),
      updated_at: now(),
      runs: [],
    };
    tasks.set(label, t);
    broadcast();
    return ok(toView(t));
  },
  unadoptTask: async (name: string) => {
    const t = tasks.get(name);
    if (!t) return err<null>(`任务不存在：${name}`);
    if (!t.adopted) return err<null>(`${name} 不是接管来的任务`);
    tasks.delete(name);
    const a = externalAgents.find((x) => x.label === name);
    if (a) {
      a.managed = false;
      a.adoptable = true;
      a.adopt_reason = null;
      a.command = "/bin/zsh /Users/demo/scripts/backup-notes.sh";
    }
    broadcast();
    return ok(null);
  },
  externalSetEnabled: async (label: string, enabled: boolean) => {
    const a = externalAgents.find((x) => x.label === label);
    if (!a) return err<ExternalAgentView>(`找不到 LaunchAgent：${label}`);
    a.loaded = enabled;
    if (!enabled) a.pid = null;
    return ok({ ...a });
  },
  externalRun: async (label: string) => {
    const a = externalAgents.find((x) => x.label === label);
    if (!a) return err<null>(`找不到 LaunchAgent：${label}`);
    if (!a.loaded) return err<null>(`${label} 没有加载到 launchd，无法 kickstart`);
    console.info("[mock] kickstart", label);
    return ok(null);
  },

  // ---- M3 §4：设置 / 失败通知 ----
  getSettings: async () => ok({ ...mockSettings }),
  setSettings: async (settings: AppSettings) => {
    mockSettings = { ...settings };
    return ok({ ...mockSettings });
  },
  dataDir: async () => ok("/Users/demo/Library/Application Support/Launchkeeper"),

  // ---- M4 §1 / §4：AI ----
  getAiConfig: async () => ok<AiConfig>({ ...mockAiConfig }),
  setAiConfig: async (config: AiConfig) => {
    mockAiConfig = {
      provider: config.provider ?? "anthropic",
      base_url: config.base_url?.trim() ? config.base_url.trim() : null,
      model: config.model?.trim() ? config.model.trim() : "claude-sonnet-5",
    };
    return ok<AiConfig>({ ...mockAiConfig });
  },
  setAiApiKey: async (key: string) => {
    if (!key.trim()) return err<null>("API Key 不能为空");
    mockApiKey = key.trim();
    return ok(null);
  },
  hasAiApiKey: async () => ok(mockApiKey != null),
  appInfo: async () => ({ version: "0.1.0-mock" }),
  openUrl: async (url: string) => {
    console.log("[mock] open", url);
    return ok(null);
  },
  clearAiApiKey: async () => {
    mockApiKey = null;
    return ok(null);
  },
  testAiConnection: async () => {
    if (mockApiKey == null) return err<string>(NO_KEY_MESSAGE);
    await sleep(600);
    return ok("pong");
  },
  explainTask: async (name: string, refresh: boolean) => {
    if (!tasks.has(name)) return err<InsightView>(`任务不存在：${name}`);
    const cached = insights.get(name);
    // 后端的缓存语义（M4 §1.7）：不刷新且库里有，就一个请求都不发。
    if (cached && !refresh) return ok(cached);
    if (mockApiKey == null) return err<InsightView>(NO_KEY_MESSAGE);
    // 真的要等几秒；界面上的转圈得在 mock 下也看得见。
    await sleep(1800);
    const fresh: InsightView = {
      task_name: name,
      created_at: now(),
      model: mockAiConfig.model,
      prompt_hash: "9b7d0e42".repeat(8),
      content: cached ? `${MOCK_INSIGHT_MD}\n\n_（这是重新解读的结果。）_` : MOCK_INSIGHT_MD,
    };
    insights.set(name, fresh);
    return ok(fresh);
  },
  getTaskInsight: async (name: string) => ok(insights.get(name) ?? null),
};

export const mockEvents = {
  tasksChanged: {
    listen: async (cb: (event: { payload: TasksChanged }) => void) => {
      const l: Listener = (payload) => cb({ payload } as any);
      listeners.add(l);
      return () => listeners.delete(l);
    },
  },
};
