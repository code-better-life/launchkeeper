// 触发规则的本地化描述。core 的 `Trigger::describe` 只有中文（CLI 用它），
// 界面上一律用这里按结构化 `Trigger` 生成的文案，中英文各一套。
// 聚合规则与 core 保持一致：每天 / 每周 / 每月 / 周几且几号 四类，同一时间合并。
import type { CalendarEntry, Trigger } from "./bindings";
import type { Locale } from "./i18n/translate";
import { translate } from "./i18n/translate";

function hhmm(e: CalendarEntry): string {
  return `${String(e.hour).padStart(2, "0")}:${String(e.minute).padStart(2, "0")}`;
}

function weekdayName(locale: Locale, w: number): string {
  const n = w % 7;
  return locale === "en"
    ? ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][n]
    : translate(locale, `weekday.short.${n}` as never);
}

function joinList(locale: Locale, items: string[]): string {
  return items.join(locale === "en" ? ", " : "、");
}

/** `每 2 小时` / `every 2 hours`；能整除的最大单位。 */
export function describeInterval(seconds: number, locale: Locale): string {
  const en = locale === "en";
  const unit = (n: number, zh: string, one: string, many: string) =>
    en ? `every ${n === 1 ? one : `${n} ${many}`}` : `每 ${n} ${zh}`;
  if (seconds >= 86400 && seconds % 86400 === 0) return unit(seconds / 86400, "天", "day", "days");
  if (seconds >= 3600 && seconds % 3600 === 0) return unit(seconds / 3600, "小时", "hour", "hours");
  if (seconds >= 60 && seconds % 60 === 0) return unit(seconds / 60, "分钟", "minute", "minutes");
  return unit(seconds, "秒", "second", "seconds");
}

function describeCalendar(entries: CalendarEntry[], locale: Locale): string {
  const en = locale === "en";
  if (entries.length === 0) return en ? "calendar (no rules)" : "日历（无规则）";
  const parts: string[] = [];

  const daily = Array.from(
    new Set(entries.filter((e) => e.weekday == null && e.day == null).map(hhmm)),
  ).sort();
  if (daily.length > 0) parts.push(en ? `daily at ${joinList(locale, daily)}` : `每天 ${joinList(locale, daily)}`);

  const weekly = new Map<string, number[]>();
  for (const e of entries.filter((e) => e.weekday != null && e.day == null)) {
    const t = hhmm(e);
    const ws = weekly.get(t) ?? [];
    if (!ws.some((x) => x % 7 === e.weekday! % 7)) ws.push(e.weekday!);
    weekly.set(t, ws);
  }
  for (const [t, ws] of weekly) {
    ws.sort((a, b) => (a % 7) - (b % 7));
    const names = joinList(locale, ws.map((w) => weekdayName(locale, w)));
    parts.push(en ? `${names} at ${t}` : `每周${names} ${t}`);
  }

  const monthly = new Map<string, number[]>();
  for (const e of entries.filter((e) => e.day != null && e.weekday == null)) {
    const t = hhmm(e);
    const ds = monthly.get(t) ?? [];
    if (!ds.includes(e.day!)) ds.push(e.day!);
    monthly.set(t, ds);
  }
  for (const [t, ds] of monthly) {
    ds.sort((a, b) => a - b);
    const names = joinList(locale, ds.map(String));
    parts.push(en ? `monthly on day ${names} at ${t}` : `每月 ${names} 日 ${t}`);
  }

  for (const e of entries.filter((e) => e.weekday != null && e.day != null)) {
    const w = weekdayName(locale, e.weekday!);
    parts.push(
      en
        ? `day ${e.day} of the month when it is a ${w}, at ${hhmm(e)}`
        : `每月 ${e.day} 日且周${w} ${hhmm(e)}`,
    );
  }
  return parts.join(locale === "en" ? "; " : "；");
}

/**
 * 列表行、详情摘要和外部 agent 行共用的触发描述。
 * `trigger` 为 null（外部 plist 用了不支持的触发键）时退回后端给的
 * `fallback` 文案。
 */
export function describeTrigger(
  trigger: Trigger | null | undefined,
  locale: Locale,
  fallback = "",
): string {
  if (!trigger) return fallback;
  switch (trigger.kind) {
    case "at_login":
      return locale === "en" ? "at login" : "登录时";
    case "manual":
      return locale === "en" ? "start/stop by hand" : "手动启停";
    case "interval":
      return describeInterval(trigger.seconds, locale);
    case "calendar":
      return describeCalendar(trigger.entries, locale);
  }
}
