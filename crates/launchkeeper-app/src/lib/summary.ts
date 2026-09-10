// 任务详情「配置」页那份**只读概览**的纯函数（docs/M3-design.md §3.4 追补）。
//
// 点开一个任务先看到的是这份概览，不是编辑器：点一下左边的行就掉进一个装满
// 输入框的表单里，用户不知道自己碰了什么、会不会存下去。要改再点「编辑」。
//
// 这里只做字符串：把 TaskInput 上那些机器味的字段（拼好的 script_path + args、
// 秒数、EnvVar 数组）翻成一行一行看得懂的话。和 lib/format.ts、lib/external.ts
// 一样收一个显式的 `locale`（M3 §5），因此可以一次测两种语言。
//
// 触发规则那一行用的是后端算好的 `TaskView::trigger_text`（`Trigger::describe`），
// 和列表行、CLI 是同一个字符串——本期它仍然只有中文。

import type { EnvVar, TaskInput } from "./bindings";
import { translate, type Locale, type TranslationKey } from "./i18n/translate";
import { detectFromTask, displayName } from "./interpreter";
import { secondsToInterval } from "./trigger";

/** 概览里的一行：左边是词典键（组件自己 `t()` 它），右边是拼好的值。 */
export interface SummaryRow {
  /** 这一行的标签，用的是表单里同名那一格的键——两边说的是同一件事。 */
  label: TranslationKey;
  value: string;
  /** 路径、命令、环境变量用等宽字体。 */
  mono?: boolean;
  /** 值里有换行（环境变量一行一个），组件据此保留换行。 */
  multiline?: boolean;
}

/** 概览需要、但 TaskInput 里没有的那几样。 */
export interface SummaryExtras {
  /** 后端 `Trigger::describe()`，和列表行是同一个字符串。 */
  triggerText: string;
  /** `task_log_dir` 的结果；还没拉回来时是 null。 */
  logDir?: string | null;
  /** 接管来的任务的原 plist 路径（M3 §3.1），普通任务是 null。 */
  adoptedFrom?: string | null;
}

/** 「运行方式」那一行：`uv run`、`python3 -u`、`直接执行`。 */
export function runModeText(task: TaskInput, locale: Locale): string {
  const split = detectFromTask(task.script_path, task.args);
  // 拆不开的命令（script_path 是个解释器、后面却没有脚本）没有"运行方式"
  // 可说——那条命令本身就是全部，脚本那一行已经把它写出来了。
  if (!split) return translate(locale, "common.placeholder");
  const name = displayName(split.interpreter, locale);
  return split.interp_args.length > 0
    ? `${name} ${split.interp_args.join(" ")}`
    : name;
}

/** 「脚本」那一行：拆得开就是脚本本身，拆不开就是存下来的那条命令。 */
export function scriptText(task: TaskInput): string {
  const split = detectFromTask(task.script_path, task.args);
  return split ? split.script : task.script_path;
}

/** 「参数」那一行：脚本自己的参数，不含解释器的前缀参数。 */
export function argsText(task: TaskInput, locale: Locale): string {
  const split = detectFromTask(task.script_path, task.args);
  const args = split ? split.args : task.args;
  return args.length > 0 ? args.join(" ") : translate(locale, "summary.none");
}

/** 「环境变量」那一格：一行一个 `KEY=value`。 */
export function envText(env: EnvVar[], locale: Locale): string {
  if (env.length === 0) return translate(locale, "summary.none");
  return env.map((e) => `${e.key}=${e.value}`).join("\n");
}

/**
 * 「超时限制」那一行。
 *
 * 走 `secondsToInterval`（表单里那个下拉框用的同一个函数），600 秒说的是
 * 「10 分钟」而不是「600 秒」——表单里用户填的就是 10 分钟。
 */
export function timeoutText(
  secs: number | null | undefined,
  locale: Locale,
): string {
  if (secs == null) return translate(locale, "form.timeout_none");
  const { value, unit } = secondsToInterval(secs);
  return `${value} ${translate(locale, `unit.${unit}` as TranslationKey)}`;
}

/**
 * 只读概览的全部行，按界面上从上到下的顺序。
 *
 * 描述、日志目录、接管来源这三行**没有值时整行不出现**：一屏"—"只会把真正
 * 有内容的那几行淹掉。其余的行即使是空的也留着（参数「无」、超时「无限制」），
 * 那几个"没有"本身就是配置的一部分。
 */
export function summaryRows(
  task: TaskInput,
  extras: SummaryExtras,
  locale: Locale,
): SummaryRow[] {
  const rows: SummaryRow[] = [
    { label: "form.display_name", value: task.display_name || task.name },
  ];
  if (task.description) {
    rows.push({ label: "form.description", value: task.description });
  }
  rows.push(
    { label: "form.script", value: scriptText(task), mono: true },
    { label: "form.run_with", value: runModeText(task, locale) },
    { label: "form.args", value: argsText(task, locale), mono: true },
    {
      label: "form.working_dir",
      value:
        task.working_dir ?? translate(locale, "summary.working_dir_default"),
      mono: task.working_dir != null,
    },
    {
      label: "form.env",
      value: envText(task.env, locale),
      mono: task.env.length > 0,
      multiline: task.env.length > 0,
    },
    {
      label: "form.trigger",
      value:
        extras.triggerText +
        (task.keep_alive ? translate(locale, "summary.keep_alive_suffix") : ""),
    },
    { label: "form.timeout", value: timeoutText(task.timeout_secs, locale) },
    {
      label: "form.tags",
      value:
        task.tags.length > 0
          ? task.tags.join(translate(locale, "external.diff.join"))
          : translate(locale, "summary.none"),
    },
    {
      label: "form.notify_on_fail",
      value: translate(locale, task.notify_on_fail ? "summary.on" : "summary.off"),
    },
  );
  if (extras.logDir) {
    rows.push({ label: "summary.log_dir", value: extras.logDir, mono: true });
  }
  if (extras.adoptedFrom) {
    rows.push({
      label: "detail.adopted_from",
      value: extras.adoptedFrom,
      mono: true,
    });
  }
  return rows;
}
