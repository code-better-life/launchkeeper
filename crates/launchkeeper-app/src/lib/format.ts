// 纯函数：时间/时长/退出码的展示格式化。没有任何副作用，方便单测。
//
// 每个函数最后收一个显式的 `locale`（M3 §5）：这一层不碰 runes，语言从调用它
// 的组件传进来（`locale.current`），测试因此可以一次跑两种语言，而不必先把某个
// 全局状态摆好。

import type { StopReason } from "./bindings";
import { plural, translate, type Locale } from "./i18n/translate";

/** 相对时间的短语，例如"3 小时前" / "3 hours ago" / "刚刚"。 */
export function relativeTime(iso: string, now: Date, locale: Locale): string {
  const then = new Date(iso).getTime();
  const nowMs = now.getTime();
  const diffSec = Math.round((nowMs - then) / 1000);

  if (diffSec < 10) return translate(locale, "fmt.rel.now");
  if (diffSec < 60) return plural(locale, "fmt.rel.second", diffSec);

  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return plural(locale, "fmt.rel.minute", diffMin);

  const diffHour = Math.floor(diffMin / 60);
  if (diffHour < 24) return plural(locale, "fmt.rel.hour", diffHour);

  const diffDay = Math.floor(diffHour / 24);
  if (diffDay < 30) return plural(locale, "fmt.rel.day", diffDay);

  const diffMonth = Math.floor(diffDay / 30);
  if (diffMonth < 12) return plural(locale, "fmt.rel.month", diffMonth);

  const diffYear = Math.floor(diffDay / 365);
  return plural(locale, "fmt.rel.year", diffYear);
}

/** 把毫秒时长格式化为人话，例如 "1.2 秒" / "1.2 s"、"3 分 5 秒" / "3 min 5 s"。 */
export function formatDuration(
  ms: number | null | undefined,
  locale: Locale,
): string {
  if (ms === null || ms === undefined) return translate(locale, "common.placeholder");
  if (ms < 1000) return translate(locale, "fmt.dur.ms", { n: ms });

  const totalSec = ms / 1000;
  if (totalSec < 60) {
    return translate(locale, "fmt.dur.sec", { n: totalSec.toFixed(1) });
  }

  const totalSecInt = Math.round(totalSec);
  const min = Math.floor(totalSecInt / 60);
  const sec = totalSecInt % 60;
  if (min < 60) {
    return translate(locale, "fmt.dur.min_sec", { m: min, s: sec });
  }

  const hour = Math.floor(min / 60);
  const remMin = min % 60;
  return translate(locale, "fmt.dur.hour_min", { h: hour, m: pad2(remMin) });
}

/**
 * 常驻服务已经跑了多久，例如 "45 秒"、"13 分 05 秒"、"2 小时 13 分"、
 * "3 天 04 小时"（英文："2 h 13 min"）。行里写成「运行 {formatUptime(secs)}」。
 *
 * 和 formatDuration 不同：那个是"一次运行花了多久"（毫秒、要精确到小数），
 * 这个是"到现在为止起来了多久"（秒、只要两级精度）。
 */
export function formatUptime(
  secs: number | null | undefined,
  locale: Locale,
): string {
  if (secs === null || secs === undefined || !Number.isFinite(secs)) {
    return translate(locale, "common.placeholder");
  }
  const s = Math.max(0, Math.floor(secs));
  if (s < 60) return translate(locale, "fmt.up.sec", { s });
  if (s < 3600) {
    return translate(locale, "fmt.up.min_sec", {
      m: Math.floor(s / 60),
      s: pad2(s % 60),
    });
  }
  if (s < 86400) {
    return translate(locale, "fmt.up.hour_min", {
      h: Math.floor(s / 3600),
      m: pad2(Math.floor((s % 3600) / 60)),
    });
  }
  return translate(locale, "fmt.up.day_hour", {
    d: Math.floor(s / 86400),
    h: pad2(Math.floor((s % 86400) / 3600)),
  });
}

function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

/**
 * 运行结束的原因，用在运行历史的 STOP 列。
 *
 * "正常结束"（exited）故意返回空串：那一列是用来解释"为什么没跑完"的，结果列
 * 已经写了成功/失败，再重复一遍只会把表格塞满。
 */
export function stopReasonText(
  reason: StopReason | null | undefined,
  locale: Locale,
): string {
  switch (reason) {
    case "stopped":
      return translate(locale, "fmt.stop.stopped");
    case "timeout":
      return translate(locale, "fmt.stop.timeout");
    default:
      return "";
  }
}

/** 退出码/运行状态转人话，用在运行历史行和最近结果展示上。 */
export function formatExit(
  exitCode: number | null | undefined,
  running: boolean,
  finishedAt: string | null | undefined,
  locale: Locale,
  stopReason: StopReason | null | undefined = null,
): string {
  if (running) return translate(locale, "fmt.exit.running");
  // Launchkeeper 自己终止的两种情况先说清楚，剩下的"没有退出码"才是外部信号
  // （注销、kill、系统），Launchkeeper 分不清是谁发的，只能说不是它。
  if (stopReason === "stopped") return translate(locale, "fmt.stop.stopped");
  if (stopReason === "timeout") return translate(locale, "fmt.stop.timeout_killed");
  if (exitCode === null || exitCode === undefined) {
    if (finishedAt) return translate(locale, "fmt.exit.signal");
    return translate(locale, "common.placeholder");
  }
  if (exitCode === 0) return translate(locale, "fmt.exit.success");
  return translate(locale, "fmt.exit.failed", { code: exitCode });
}
