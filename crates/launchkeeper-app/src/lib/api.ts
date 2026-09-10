// bindings.ts 的薄封装。
//
// tauri-specta 生成的 `commands.*` 返回一个判别联合而不是抛异常。这里统一转成
// 「成功返回值 / 抛 Error」，让调用方可以直接 try/catch，并把后端 AppError 的
// message 原样交给 toast（docs/M2-design.md §4）。
//
// VITE_MOCK=1 时（纯前端开发/截图用），commands 和 events 换成 lib/mock.ts
// 里的内存实现，不触碰真实 Tauri IPC；生产路径完全不受影响。
import { commands as realCommands, events as realEvents } from "./bindings";
import type { AppError } from "./bindings";

const isMock = import.meta.env.VITE_MOCK === "1";

// 动态 import：生产构建里 VITE_MOCK 没有定义，Vite 把这个条件常量折叠掉，
// mock.ts 整个模块不会进产物。
const mock = isMock ? await import("./mock") : null;

const commands = (mock?.mockCommands ?? realCommands) as typeof realCommands;
export const events = (mock?.mockEvents ?? realEvents) as typeof realEvents;

/** tauri-specta 的 `typedError` 返回值形状；生成文件没导出它，这里复述一遍。 */
export type Result<T, E> =
  | { status: "ok"; data: T }
  | { status: "error"; error: E };

/** 取出成功值，失败则抛出带后端原文的 Error。 */
export function unwrap<T>(r: Result<T, AppError>): T {
  if (r.status === "ok") return r.data;
  throw new Error(r.error.message);
}

export const api = {
  listTasks: async () => unwrap(await commands.listTasks()),
  getTask: async (name: string) => unwrap(await commands.getTask(name)),
  createTask: async (input: Parameters<typeof commands.createTask>[0]) =>
    unwrap(await commands.createTask(input)),
  updateTask: async (
    name: string,
    input: Parameters<typeof commands.updateTask>[1],
  ) => unwrap(await commands.updateTask(name, input)),
  deleteTask: async (name: string) => unwrap(await commands.deleteTask(name)),
  setEnabled: async (name: string, enabled: boolean) =>
    unwrap(await commands.setEnabled(name, enabled)),
  runNow: async (name: string) => unwrap(await commands.runNow(name)),
  startService: async (name: string) => unwrap(await commands.startService(name)),
  // 后端最多会阻塞 STOP_GRACE（8 秒）等进程真的退出，所以调用方别把界面锁死。
  stopService: async (name: string) => unwrap(await commands.stopService(name)),
  restartService: async (name: string) =>
    unwrap(await commands.restartService(name)),
  listRuns: async (name: string, limit: number) =>
    unwrap(await commands.listRuns(name, limit)),
  readLog: async (
    runId: Parameters<typeof commands.readLog>[0],
    stream: Parameters<typeof commands.readLog>[1],
    tailBytes: number,
  ) => unwrap(await commands.readLog(runId, stream, tailBytes)),
  // 会在后台线程上跑一遍 PATH 扫描并对每个程序执行一次 --version（结果在后端
  // 进程内缓存），调用方按脚本路径防抖，别每敲一个字符调一次。
  scanInterpreters: async (script: string | null, projectDir: string | null) =>
    unwrap(await commands.scanInterpreters(script, projectDir)),
  // 表单保存前用它确认脚本真的在那儿、launchd 起得来它；一次 stat，不缓存。
  checkScript: async (path: string) => unwrap(await commands.checkScript(path)),
  taskLogDir: async (name: string) => unwrap(await commands.taskLogDir(name)),
  revealInFinder: async (path: string) =>
    unwrap(await commands.revealInFinder(path)),
  // ---- M3 §3.4：其他 LaunchAgents ----
  // 这一组要读一遍 ~/Library/LaunchAgents 并对每个 label 问一次 launchctl，
  // 所以不在每 2 秒的后台扫描里，由前端在需要时（初次加载、接管/撤销之后、
  // 点刷新）自己拉。
  listExternalAgents: async () => unwrap(await commands.listExternalAgents()),
  adoptionPlan: async (label: string) =>
    unwrap(await commands.adoptionPlan(label)),
  adoptAgent: async (label: string, reload: boolean) =>
    unwrap(await commands.adoptAgent(label, reload)),
  unadoptTask: async (name: string) => unwrap(await commands.unadoptTask(name)),
  externalSetEnabled: async (label: string, enabled: boolean) =>
    unwrap(await commands.externalSetEnabled(label, enabled)),
  externalRun: async (label: string) => unwrap(await commands.externalRun(label)),
  // ---- M3 §4：设置 / 失败通知 ----
  getSettings: async () => unwrap(await commands.getSettings()),
  setSettings: async (settings: Parameters<typeof commands.setSettings>[0]) =>
    unwrap(await commands.setSettings(settings)),
  dataDir: async () => unwrap(await commands.dataDir()),
  // ---- M4 §1 / §4：AI ----
  // 非机密配置（provider / base_url / model）走 ai.json，Key 只往里写、不往
  // 外读——`hasAiApiKey` 只答有没有，任何命令都不会把 Key 交回前端。
  getAiConfig: async () => unwrap(await commands.getAiConfig()),
  setAiConfig: async (config: Parameters<typeof commands.setAiConfig>[0]) =>
    unwrap(await commands.setAiConfig(config)),
  setAiApiKey: async (key: string) => unwrap(await commands.setAiApiKey(key)),
  hasAiApiKey: async () => unwrap(await commands.hasAiApiKey()),
  clearAiApiKey: async () => unwrap(await commands.clearAiApiKey()),
  // 真的会发一次请求出去，可能等好几秒；调用方自己置 loading。
  testAiConnection: async () => unwrap(await commands.testAiConnection()),
  appInfo: async () => commands.appInfo(),
  openUrl: async (url: string) => unwrap(await commands.openUrl(url)),
  // 唯一一个能跑几十秒的命令：`refresh = false` 时后端直接返回库里那份，
  // 不联网也不花钱（M4 §1.7），所以打开标签页那次可以放心调。
  explainTask: async (name: string, refresh: boolean) =>
    unwrap(await commands.explainTask(name, refresh)),
  getTaskInsight: async (name: string) =>
    unwrap(await commands.getTaskInsight(name)),
};
