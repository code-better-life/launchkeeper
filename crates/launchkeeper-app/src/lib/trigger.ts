// TriggerEditor 内部表单状态 <-> 后端 Trigger 类型 的双向纯函数转换。
//
// 后端 CalendarEntry 每条只能带一个 weekday（或 null），但编辑器里希望一条
// "规则"可以勾选多个星期（周一、周三、周五 每天 9:00 这种）。所以：
// - 表单 -> 后端：一条表单 entry 若选了 N 个星期，展开成 N 条 CalendarEntry
//   （相同 hour/minute/day，不同 weekday）；一个星期都没选则生成 1 条
//   weekday: null 的 entry。
// - 后端 -> 表单：按 (hour, minute, day) 分组，把同组里出现的 weekday 收集回
//   多选数组；weekday 为 null 的单独成组（因为它代表"不限星期"）。
//
// launchd 里 weekday 0 和 7 都表示周日；本模块只产出 0-6，绝不产出 7。

import type { CalendarEntry, Trigger } from "./bindings";
import type { TranslationKey } from "./i18n/translate";

export type IntervalUnit = "seconds" | "minutes" | "hours";

export interface CalendarFormEntry {
  hour: number;
  minute: number;
  /** 0-6，周日在前：日一二三四五六。空数组表示不限星期。 */
  weekdays: number[];
  /** 1-31，或 null 表示不限日期。 */
  day: number | null;
}

export interface TriggerForm {
  /** "manual" 是 M2.5 的服务型任务：没有任何自动触发，只由启动/停止驱动。 */
  kind: "at_login" | "interval" | "calendar" | "manual";
  intervalValue: number;
  intervalUnit: IntervalUnit;
  calendarEntries: CalendarFormEntry[];
}

export function defaultTriggerForm(): TriggerForm {
  return {
    kind: "at_login",
    intervalValue: 1,
    intervalUnit: "hours",
    calendarEntries: [{ hour: 9, minute: 0, weekdays: [], day: null }],
  };
}

const UNIT_SECONDS: Record<IntervalUnit, number> = {
  seconds: 1,
  minutes: 60,
  hours: 3600,
};

export function intervalToSeconds(value: number, unit: IntervalUnit): number {
  return Math.max(1, Math.round(value * UNIT_SECONDS[unit]));
}

/**
 * 表单能不能变成一个合法的 Trigger；返回一个**词典键**（由表单翻译成当前语言
 * 展示），或 null。
 *
 * 返回键而不是成品句子（M3 §5）：这个函数是纯的、被单测按例子钉住的，让它去
 * 认识"现在是哪种语言"就得给它塞一个 locale 参数，而调用它的地方（TaskForm）
 * 手边正好就有 `t()`。
 *
 * 数字输入框清空时 Svelte 的 `bind:value` 给出的是 null（不是 0），直接送进
 * intervalToSeconds 会被 Math.max(1, NaN) 悄悄变成 1 秒——一个用户从没输入过
 * 的、每秒钟跑一次的计划任务。宁可拦下来报错。
 */
export function triggerFormError(form: TriggerForm): TranslationKey | null {
  if (form.kind === "interval") {
    const v = form.intervalValue as number | null | undefined;
    if (v == null || !Number.isFinite(v) || v <= 0) {
      return "form.error.interval_number";
    }
  }
  if (form.kind === "calendar") {
    for (const e of form.calendarEntries) {
      if (!isInRange(e.hour, 0, 23)) return "form.error.hour_range";
      if (!isInRange(e.minute, 0, 59)) return "form.error.minute_range";
      if (e.day != null && !isInRange(e.day, 1, 31)) {
        return "form.error.day_range";
      }
    }
  }
  return null;
}

function isInRange(v: number | null | undefined, lo: number, hi: number): boolean {
  return v != null && Number.isInteger(v) && v >= lo && v <= hi;
}

/** 选一个能整除且尽量大的单位，方便展示；不能整除时退回秒。 */
export function secondsToInterval(
  seconds: number,
): { value: number; unit: IntervalUnit } {
  if (seconds % 3600 === 0) return { value: seconds / 3600, unit: "hours" };
  if (seconds % 60 === 0) return { value: seconds / 60, unit: "minutes" };
  return { value: seconds, unit: "seconds" };
}

function groupKey(hour: number, minute: number, day: number | null): string {
  return `${hour}:${minute}:${day ?? "*"}`;
}

/** 表单状态 -> 后端 Trigger。 */
export function formToTrigger(form: TriggerForm): Trigger {
  if (form.kind === "at_login") {
    return { kind: "at_login" };
  }
  if (form.kind === "manual") {
    return { kind: "manual" };
  }
  if (form.kind === "interval") {
    return {
      kind: "interval",
      seconds: intervalToSeconds(form.intervalValue, form.intervalUnit),
    };
  }

  const entries: CalendarEntry[] = [];
  for (const e of form.calendarEntries) {
    if (e.weekdays.length === 0) {
      entries.push({
        hour: e.hour,
        minute: e.minute,
        weekday: null,
        day: e.day,
      });
    } else {
      for (const w of e.weekdays) {
        // 只产出 0-6，7 永远不发出。
        const weekday = w === 7 ? 0 : w;
        entries.push({ hour: e.hour, minute: e.minute, weekday, day: e.day });
      }
    }
  }
  return { kind: "calendar", entries };
}

/** 后端 Trigger -> 表单状态。 */
export function triggerToForm(trigger: Trigger): TriggerForm {
  const base = defaultTriggerForm();
  if (trigger.kind === "at_login") {
    return { ...base, kind: "at_login" };
  }
  if (trigger.kind === "manual") {
    return { ...base, kind: "manual" };
  }
  if (trigger.kind === "interval") {
    const { value, unit } = secondsToInterval(trigger.seconds);
    return { ...base, kind: "interval", intervalValue: value, intervalUnit: unit };
  }

  const groups = new Map<string, CalendarFormEntry>();
  const order: string[] = [];
  for (const raw of trigger.entries) {
    const weekday = raw.weekday === 7 ? 0 : raw.weekday;
    const key = groupKey(raw.hour, raw.minute, raw.day);
    let g = groups.get(key);
    if (!g) {
      g = { hour: raw.hour, minute: raw.minute, weekdays: [], day: raw.day };
      groups.set(key, g);
      order.push(key);
    }
    if (weekday !== null && weekday !== undefined && !g.weekdays.includes(weekday)) {
      g.weekdays.push(weekday);
    }
  }
  const calendarEntries = order.map((k) => groups.get(k)!);
  return {
    ...base,
    kind: "calendar",
    calendarEntries: calendarEntries.length > 0 ? calendarEntries : base.calendarEntries,
  };
}
