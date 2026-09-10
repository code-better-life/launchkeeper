// 「Launchkeeper 管理」这一组的排序（PRD M4 项 2，docs/M4-design.md §2）。
//
// 四种排序都是纯函数，收显式的 `now` 和 `locale`（和 `lib/format.ts`、
// `lib/external.ts` 一样，M3 §5）：排序键里有时间和人类语言的字母序，这两样
// 都不该由这一层自己去某个全局状态里摸。
//
// 「其他 LaunchAgents」那一组不走这里：它有自己固定的"可接管优先"顺序
// （`lib/external.ts` 的 `groupExternal`），那一组的问题是"我手写的那几个
// plist 在哪儿"，不是"哪个先跑"。

import type { TaskStatus, TaskView } from "./bindings";
import type { Locale, TranslationKey } from "./i18n/translate";
import { nextRunKey } from "./nextRun";

/** 下拉框里的四项，顺序就是渲染顺序。 */
export const SORT_MODES = ["name", "last_run", "status", "next_run"] as const;

export type SortMode = (typeof SORT_MODES)[number];

/** 没有存过选择时的排序。 */
export const DEFAULT_SORT: SortMode = "name";

/** 下拉框每一项的词典键。 */
export const SORT_LABEL: Record<SortMode, TranslationKey> = {
  name: "list.sort.name",
  last_run: "list.sort.last_run",
  status: "list.sort.status",
  next_run: "list.sort.next_run",
};

/**
 * 任意字符串 -> 排序方式。
 *
 * `settings.json` 是可以手改的，`localStorage` 是可以被别的东西写脏的，两边
 * 读出来的东西都当外部输入看：不认识的值一律回到默认，而不是让列表停在一个
 * 谁也没选过的状态。
 */
export function sortMode(raw: string | null | undefined): SortMode {
  return (SORT_MODES as readonly string[]).includes(raw ?? "")
    ? (raw as SortMode)
    : DEFAULT_SORT;
}

/**
 * 状态排序的档位：失败在最前，其次运行中，其余的并列。
 *
 * "其余"不再细分（成功 / 从未运行 / 已停止 / 已停用 各是一档）是故意的：这个
 * 排序回答的是"有什么需要我管的"，而在没有问题的任务之间，用户找的是名字，
 * 不是状态。所以同档之内按名字排。
 */
export function statusRank(status: TaskStatus): number {
  if (status === "failed") return 0;
  if (status === "running") return 1;
  return 2;
}

/** 显示名的字母序；同名再按 name（唯一）兜底，保证是个全序。 */
function byName(a: TaskView, b: TaskView, locale: Locale): number {
  return (
    a.display_name.localeCompare(b.display_name, locale) ||
    a.name.localeCompare(b.name, "en")
  );
}

/** 最近运行时间，从没跑过的是 -Infinity（"最久以前"）。 */
function lastRunMs(t: TaskView): number {
  const started = t.last_run?.started_at;
  if (!started) return Number.NEGATIVE_INFINITY;
  const ms = Date.parse(started);
  return Number.isFinite(ms) ? ms : Number.NEGATIVE_INFINITY;
}

/**
 * 比大小，不做减法。
 *
 * 两个键都是 ±Infinity 时（两个都没跑过、两个都算不出下次触发）`a - b` 是
 * NaN，而一个返回 NaN 的比较函数会让 `sort` 得到一个没定义的顺序——正是这两种
 * 排序里最常见的一批任务。
 */
function compareKeys(a: number, b: number): number {
  if (a === b) return 0;
  return a < b ? -1 : 1;
}

/**
 * 排好序的一份新数组，入参不动。
 *
 * 每种排序最后都落到名字上：`Array.prototype.sort` 在现代引擎里是稳定的，但
 * "稳定"保住的是**上一次**的顺序，而这个列表每 2 秒被后端整个换掉一次，上一次
 * 的顺序不是什么可以依赖的东西。显式的名字兜底才能让两个都失败、都没跑过的
 * 任务每次都在同一个位置上。
 */
export function sortTasks(
  tasks: TaskView[],
  mode: SortMode,
  now: Date,
  locale: Locale,
): TaskView[] {
  const copy = tasks.slice();
  switch (mode) {
    case "last_run":
      // 最近跑过的在最前；从没跑过的在最后。
      copy.sort(
        (a, b) =>
          compareKeys(lastRunMs(b), lastRunMs(a)) || byName(a, b, locale),
      );
      break;
    case "status":
      copy.sort(
        (a, b) =>
          statusRank(a.status) - statusRank(b.status) || byName(a, b, locale),
      );
      break;
    case "next_run":
      // 最快要跑的在最前；算不出下次触发的（服务、登录时、未启用）在最后。
      copy.sort(
        (a, b) =>
          compareKeys(nextRunKey(a, now), nextRunKey(b, now)) ||
          byName(a, b, locale),
      );
      break;
    case "name":
      copy.sort((a, b) => byName(a, b, locale));
      break;
  }
  return copy;
}

// ---- 持久化：settings.json 为主，localStorage 兜底 ----------------------

/** 排序方式在 localStorage 里的键。 */
export const LIST_SORT_KEY = "launchkeeper.listSort";

/**
 * 上一次的选择，`localStorage` 里那一份。
 *
 * 真正的存放处是 `AppSettings.list_sort`（跟着数据目录走，CLI 和别的窗口看到
 * 的是同一个值）；这一份是它写不进去时的备胎，以及界面刚起来、`get_settings`
 * 还没回来之前的第一帧。读写都 try/catch：webview 里 `localStorage` 可能整个
 * 不可用，记不住一个排序是小事，为它抛异常把列表画不出来是大事（和
 * `lib/external.ts` 的折叠状态同一个理由）。
 */
export function readListSort(): SortMode | null {
  try {
    const raw = localStorage.getItem(LIST_SORT_KEY);
    return raw === null ? null : sortMode(raw);
  } catch {
    return null;
  }
}

export function writeListSort(mode: SortMode) {
  try {
    localStorage.setItem(LIST_SORT_KEY, mode);
  } catch {
    // 记不住就算了。
  }
}
