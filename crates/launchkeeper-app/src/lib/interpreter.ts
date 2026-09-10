// 运行方式（解释器）在前端这一侧的规则，见 docs/M3-design.md §1。
//
// 候选列表、版本号、推荐理由全部由后端 `scan_interpreters` 给出，这里只做两件
// 后端做不了的事：表单保存时把「脚本 + 参数 + 运行方式」拼成一条 script_path
// + args，以及打开一个已有任务时把它拆回来。两者都是纯字符串运算，走一趟 IPC
// 只为了拼一个数组不划算。
//
// ⚠️ 与 crates/launchkeeper-core/src/interpreters.rs 的 `apply` /
// `detect_from_task` / `KNOWN` 是同一套规则的两份实现，改一边必须改另一边；
// 两边的单元测试用的是同一组例子（uv run、直接执行、带参数的脚本）。

import type { InterpreterKind, InterpreterView, Origin } from "./bindings";
import { translate, type Locale } from "./i18n/translate";

/** 组成一条命令所需要的最小信息，`InterpreterView` 的结构子集。 */
export interface RunWith {
  kind: InterpreterKind;
  program: string;
  prefix_args: string[];
}

/** core 的 `KNOWN` 表，顺序一致。 */
const KNOWN: { names: string[]; kind: InterpreterKind; prefix: string[] }[] = [
  { names: ["python3", "python"], kind: "python", prefix: [] },
  { names: ["uv"], kind: "uv", prefix: ["run"] },
  { names: ["node"], kind: "node", prefix: [] },
  { names: ["bun"], kind: "bun", prefix: [] },
  { names: ["deno"], kind: "deno", prefix: ["run"] },
  { names: ["ruby"], kind: "ruby", prefix: [] },
  { names: ["zsh", "bash", "sh"], kind: "shell", prefix: [] },
];

export function basename(path: string): string {
  const parts = path.split("/").filter((p) => p.length > 0);
  return parts[parts.length - 1] ?? "";
}

/**
 * core `is_known_program`：文件名是不是已知解释器 `known`——要么就是它，要么是它
 * 加一个版本号（`python3.13`、`ruby2.7`、`node18`）。版本号只能由数字和点组成，
 * 所以 `node.js`、`python.py` 是「名字像解释器的脚本」，不是解释器。
 */
function isKnownProgram(name: string, known: string): boolean {
  if (!name.startsWith(known)) return false;
  const rest = name.slice(known.length);
  return (
    rest.length === 0 ||
    (/[0-9]/.test(rest) && /^[0-9.]+$/.test(rest))
  );
}

/** 名字能对上的那条 `KNOWN`，对不上就是 `undefined`。 */
function knownFor(name: string) {
  return KNOWN.find((k) => k.names.some((n) => isKnownProgram(name, n)));
}

/** core `Interpreter::id`：程序路径加上前缀参数。 */
export function interpreterId(i: RunWith): string {
  return i.prefix_args.length > 0
    ? `${i.program} ${i.prefix_args.join(" ")}`
    : i.program;
}

/** core `Interpreter::display_name`。「直接执行」这一条要跟着界面语言走。 */
export function displayName(i: RunWith, locale: Locale): string {
  if (i.kind === "direct") return translate(locale, "interp.direct");
  const base = basename(i.program) || i.program;
  return i.prefix_args.length > 0
    ? `${base} ${i.prefix_args.join(" ")}`
    : base;
}

/**
 * core `apply`：脚本 + 参数 + 运行方式 -> 任务真正存下来的 script_path / args。
 *
 * `direct`（以及没选运行方式时）就是脚本本身；其余是「解释器 + 前缀参数 +
 * 解释器自己的参数（`python3 -u` 里的 `-u`）+ 脚本 + 脚本自己的参数」。
 */
export function composeTaskCommand(
  script: string,
  args: string[],
  interp: RunWith | null,
  interpArgs: string[] = [],
): { script_path: string; args: string[] } {
  if (!interp || interp.kind === "direct") {
    return { script_path: script, args: [...args] };
  }
  return {
    script_path: interp.program,
    args: [...interp.prefix_args, ...interpArgs, script, ...args],
  };
}

/** `detectFromTask` 的结果，core `Detected` 的镜像。 */
export interface Detected {
  interpreter: RunWith;
  /** 属于解释器而不是脚本的那些参数，`python3 -u x.py` 里的 `-u`。 */
  interp_args: string[];
  script: string;
  args: string[];
}

/**
 * core `detect_from_task`：把存下来的 script_path / args 拆回「运行方式 +
 * 解释器参数 + 脚本 + 脚本参数」。
 *
 * script_path 不是已知解释器时就是 `direct`；是已知解释器但后面没有脚本时返回
 * `null`（这条命令没法用「脚本 + 运行方式」表达，表单退回原样展示）。
 */
export function detectFromTask(
  scriptPath: string,
  args: string[],
): Detected | null {
  const name = basename(scriptPath);
  if (!name) return null;
  const known = knownFor(name);
  if (!known) {
    return {
      interpreter: { kind: "direct", program: scriptPath, prefix_args: [] },
      interp_args: [],
      script: scriptPath,
      args: [...args],
    };
  }
  let rest = args;
  const prefix: string[] = [];
  for (const want of known.prefix) {
    if (rest[0] === want) {
      prefix.push(want);
      rest = rest.slice(1);
    } else {
      break;
    }
  }
  // 接着是解释器自己的 flag：开头那些以 `-` 打头的都归解释器，`--` 结束。
  const interpArgs: string[] = [];
  while (rest.length > 0) {
    const first = rest[0];
    if (first === "--") {
      interpArgs.push(first);
      rest = rest.slice(1);
      break;
    }
    if (first.startsWith("-") && first.length > 1) {
      interpArgs.push(first);
      rest = rest.slice(1);
    } else {
      break;
    }
  }
  if (rest.length === 0) return null;
  return {
    interpreter: { kind: known.kind, program: scriptPath, prefix_args: prefix },
    interp_args: interpArgs,
    script: rest[0],
    args: rest.slice(1),
  };
}

/**
 * 把一个只知道路径的运行方式（用户手填的，或从已有任务里拆出来的）包装成一行
 * 候选，好让下拉框在扫描结果回来之前也能显示当前值。
 */
export function viewOf(
  i: RunWith,
  locale: Locale,
  detail?: string,
): InterpreterView {
  return {
    id: interpreterId(i),
    kind: i.kind,
    program: i.program,
    prefix_args: [...i.prefix_args],
    version: null,
    origin: "custom",
    label: displayName(i, locale),
    detail:
      detail ??
      (i.kind === "direct" ? translate(locale, "interp.script_itself") : i.program),
    recommended: false,
    reason: null,
    group: "path",
  };
}

/**
 * 用户手填路径的候选。
 *
 * 手填的路径先按文件名和 `KNOWN` 对一遍：填 `/opt/x/bin/uv` 得到的是
 * `uv run`（带前缀参数）而不是一个光秃秃的 `custom`——后者拼出来的命令是
 * `uv /path/script.py`，不是用户想要的那条，而且这个任务打开表单时
 * `detectFromTask` 又会把它认成 `uv`，前后对不上。
 */
export function customInterpreter(
  program: string,
  locale: Locale,
): InterpreterView {
  const known = knownFor(basename(program));
  if (known) {
    return viewOf(
      { kind: known.kind, program, prefix_args: [...known.prefix] },
      locale,
      program,
    );
  }
  return viewOf({ kind: "custom", program, prefix_args: [] }, locale, program);
}

/** 按界面稿的三个分组切开候选列表，顺序不变。 */
export function groupInterpreters(list: InterpreterView[]): {
  recommended: InterpreterView[];
  project: InterpreterView[];
  path: InterpreterView[];
} {
  return {
    recommended: list.filter((i) => i.group === "recommended"),
    project: list.filter((i) => i.group === "project"),
    path: list.filter((i) => i.group === "path"),
  };
}

/** 解释器来源的本地化名称（core 的 `Origin::describe` 只有中文）。 */
export function originText(origin: Origin, locale: Locale): string {
  return translate(locale, `interp.origin.${origin}` as never);
}

function shortenHome(p: string): string {
  const m = /^\/Users\/[^/]+(\/.*)?$/.exec(p);
  return m ? `~${m[1] ?? ""}` : p;
}

/** 下拉框一行的名字：`uv run · uv 安装` / `uv run · installed by uv`。 */
export function interpreterLabel(v: InterpreterView, locale: Locale): string {
  return v.kind === "direct"
    ? translate(locale, "interp.direct")
    : `${displayName(v, locale)} · ${originText(v.origin, locale)}`;
}

/** 下拉框一行的说明：`0.10.2 · uv 安装 ~/.local/bin/uv`。 */
export function interpreterDetail(v: InterpreterView, locale: Locale): string {
  if (v.kind === "direct") return translate(locale, "interp.script_itself");
  if (v.origin === "custom" && v.version == null) return v.detail;
  const place = `${originText(v.origin, locale)} ${shortenHome(v.program)}`;
  return v.version ? `${v.version} · ${place}` : place;
}
