// 下次触发时间的预测（PRD M4 项 2，docs/M4-design.md §2）。
//
// 纯函数，没有副作用，也不读任何全局状态：`now` 一律由调用方传进来，这样
// "下午 5 点的任务在下午 4 点看是今天、在下午 6 点看是明天"这种事在测试里是
// 一个参数，而不是一次 mock。
//
// 这是**预测**，不是 launchd 的答案：launchd 不对外报"下次什么时候跑"，所以
// 这里按同一套 plist 语义自己算一遍。它只用来排序和展示，算错的代价是列表顺序
// 不对，不会影响任何真的会执行的东西。

import type { CalendarEntry, Trigger } from "./bindings";

/** 算下次触发所需的全部信息，`TaskView` 的一个子集。 */
export interface NextRunTask {
  /** 触发规则。 */
  trigger: Trigger;
  /** 已注册到 launchd。没注册的任务不会自己跑。 */
  enabled: boolean;
  /** 最近一次运行，`null` 表示从没跑过。 */
  last_run: { started_at: string } | null;
}

/**
 * 日历规则往后找多少天。
 *
 * 366 天覆盖"每月 31 日"（一年里有 7 个月有 31 号）和任何单独的星期/日期规则。
 * weekday 和 day 同时指定时（launchd 要求两者都匹配，见 core 的
 * `describe_calendar`）可能要等更久——比如"每月 31 日且周日"平均 5 到 6 年
 * 一次——这时返回 null，排到列表末尾，比给一个要滚 2000 次循环才算出来的日期
 * 有用得多。
 */
const SCAN_DAYS = 366;

/**
 * 下次触发时间，算不出来时返回 null。
 *
 * 算不出来的四种情况，界面上一视同仁地排在最后：
 *
 * - **未启用**：plist 不在 launchd 里，它不会自己跑；
 * - **手动启停**（服务）：只由启动/停止驱动，没有"下次"；
 * - **登录时**：下次触发是下次登录，那不是一个时间点；
 * - **按间隔但从没跑过**：`StartInterval` 从 job 被加载的那一刻开始计时，而
 *   launchd 不告诉我们那是什么时候。调用方手里有加载时间时可以用 `loadedAt`
 *   传进来（`launchctl print` 里能读到），否则这一条也没有答案。
 */
export function nextRunAt(
  task: NextRunTask,
  now: Date,
  loadedAt: Date | null = null,
): Date | null {
  if (!task.enabled) return null;
  const trigger = task.trigger;
  switch (trigger.kind) {
    case "manual":
    case "at_login":
      return null;
    case "interval":
      return nextInterval(trigger.seconds, task, now, loadedAt);
    case "calendar":
      return nextCalendar(trigger.entries, now);
  }
}

/** 排序用的键：算不出下次触发的排在最后。 */
export function nextRunKey(
  task: NextRunTask,
  now: Date,
  loadedAt: Date | null = null,
): number {
  const at = nextRunAt(task, now, loadedAt);
  return at === null ? Number.POSITIVE_INFINITY : at.getTime();
}

/**
 * `StartInterval`：从上次开跑（没有就从 job 加载）起，每 N 秒一次。
 *
 * 算的是**真实经过的秒数**，不是墙上时钟——launchd 的 `StartInterval` 也是
 * 这样，夏令时那天的间隔任务不会多跑或少跑一次。所以这里直接加毫秒，不碰
 * 日历字段。
 *
 * 上次运行已经是很久以前时（睡眠、任务被禁用过一阵子）一次跳到下一个还没到的
 * 时间点，而不是报一个早就过去的时刻：过去的时间排在"最快要跑的"最前面是错的。
 */
function nextInterval(
  seconds: number,
  task: NextRunTask,
  now: Date,
  loadedAt: Date | null,
): Date | null {
  if (!Number.isFinite(seconds) || seconds <= 0) return null;
  const started = task.last_run?.started_at;
  const anchor = started ? Date.parse(started) : (loadedAt?.getTime() ?? NaN);
  if (!Number.isFinite(anchor)) return null;

  const step = seconds * 1000;
  const nowMs = now.getTime();
  let next = anchor + step;
  if (next <= nowMs) {
    const elapsed = Math.floor((nowMs - anchor) / step);
    next = anchor + (elapsed + 1) * step;
  }
  return new Date(next);
}

/** 一组 `StartCalendarInterval` 规则里最早的那个下次触发。 */
function nextCalendar(entries: CalendarEntry[], now: Date): Date | null {
  let best: Date | null = null;
  for (const entry of entries) {
    const at = nextCalendarEntry(entry, now);
    if (at && (best === null || at.getTime() < best.getTime())) best = at;
  }
  return best;
}

/**
 * 一条日历规则的下次触发。
 *
 * 逐天往后试，每天都用**本地日历字段**构造那个时刻
 * （`new Date(y, m, d + i, hour, minute)`），不是"今天这个时刻 + i × 86400000"
 * ——后者在夏令时切换那天会把 21:00 的任务算成 20:00 或 22:00。用日历字段构造
 * 则不管那天有 23 还是 25 小时，21:00 就是 21:00，和 launchd 的行为一致。
 * 春季跳过的那一小时里（比如 02:30）JS 会规整到 03:30，也正是 launchd 补跑的
 * 语义。
 *
 * weekday 和 day 同时指定时按"两者都匹配"处理，和 core 的 `describe_calendar`
 * 说的是同一件事（"每月 X 日且周 Y"）。
 */
function nextCalendarEntry(entry: CalendarEntry, now: Date): Date | null {
  const nowMs = now.getTime();
  // launchd 里 weekday 0 和 7 都是周日。
  const weekday = entry.weekday === null ? null : entry.weekday % 7;
  for (let i = 0; i <= SCAN_DAYS; i++) {
    const at = new Date(
      now.getFullYear(),
      now.getMonth(),
      now.getDate() + i,
      entry.hour,
      entry.minute,
      0,
      0,
    );
    if (at.getTime() <= nowMs) continue;
    if (entry.day !== null && at.getDate() !== entry.day) continue;
    if (weekday !== null && at.getDay() !== weekday) continue;
    return at;
  }
  return null;
}
