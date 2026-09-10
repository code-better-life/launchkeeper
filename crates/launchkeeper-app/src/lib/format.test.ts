import { describe, expect, it } from "vitest";
import {
  formatDuration,
  formatExit,
  formatUptime,
  relativeTime,
  stopReasonText,
} from "./format";

describe("relativeTime", () => {
  const now = new Date("2026-09-10T12:00:00Z");

  it("returns 刚刚 for just now", () => {
    expect(relativeTime("2026-09-10T11:59:55Z", now, "zh-CN")).toBe("刚刚");
    expect(relativeTime("2026-09-10T11:59:55Z", now, "en")).toBe("just now");
  });

  it("treats a timestamp from the future as just now", () => {
    expect(relativeTime("2026-09-10T12:00:30Z", now, "en")).toBe("just now");
  });

  it("returns seconds for under a minute", () => {
    expect(relativeTime("2026-09-10T11:59:30Z", now, "zh-CN")).toBe("30 秒前");
    expect(relativeTime("2026-09-10T11:59:30Z", now, "en")).toBe("30 seconds ago");
  });

  it("returns minutes boundary", () => {
    expect(relativeTime("2026-09-10T11:58:00Z", now, "zh-CN")).toBe("2 分钟前");
    expect(relativeTime("2026-09-10T11:01:00Z", now, "zh-CN")).toBe("59 分钟前");
    expect(relativeTime("2026-09-10T11:58:00Z", now, "en")).toBe("2 minutes ago");
  });

  it("returns hours boundary", () => {
    expect(relativeTime("2026-09-10T09:00:00Z", now, "zh-CN")).toBe("3 小时前");
    expect(relativeTime("2026-09-09T13:00:00Z", now, "zh-CN")).toBe("23 小时前");
    expect(relativeTime("2026-09-10T09:00:00Z", now, "en")).toBe("3 hours ago");
  });

  it("returns days boundary", () => {
    expect(relativeTime("2026-09-09T12:00:00Z", now, "zh-CN")).toBe("1 天前");
    expect(relativeTime("2026-08-20T12:00:00Z", now, "zh-CN")).toBe("21 天前");
    expect(relativeTime("2026-08-20T12:00:00Z", now, "en")).toBe("21 days ago");
  });

  it("returns months boundary", () => {
    expect(relativeTime("2026-08-01T12:00:00Z", now, "zh-CN")).toBe("1 个月前");
    expect(relativeTime("2026-08-01T12:00:00Z", now, "en")).toBe("1 month ago");
  });

  it("returns years boundary", () => {
    expect(relativeTime("2024-09-10T12:00:00Z", now, "zh-CN")).toBe("2 年前");
    expect(relativeTime("2024-09-10T12:00:00Z", now, "en")).toBe("2 years ago");
  });

  /**
   * 英文的单复数是这个模块唯一一处"两种语言的规则不一样"的地方：中文两个键写
   * 的是同一句话，英文的 `_one` / `_other` 必须真的分开。
   */
  it("uses the singular form in English for exactly one unit", () => {
    expect(relativeTime("2026-09-10T11:59:00Z", now, "en")).toBe("1 minute ago");
    expect(relativeTime("2026-09-10T11:00:00Z", now, "en")).toBe("1 hour ago");
    expect(relativeTime("2026-09-09T12:00:00Z", now, "en")).toBe("1 day ago");
    expect(relativeTime("2025-09-10T12:00:00Z", now, "en")).toBe("1 year ago");
    // 中文那边同一个数字不多一个字。
    expect(relativeTime("2026-09-10T11:59:00Z", now, "zh-CN")).toBe("1 分钟前");
  });
});

describe("formatDuration", () => {
  it("handles null/undefined", () => {
    expect(formatDuration(null, "zh-CN")).toBe("—");
    expect(formatDuration(undefined, "en")).toBe("—");
  });

  it("handles sub-second durations", () => {
    expect(formatDuration(500, "zh-CN")).toBe("500 毫秒");
    expect(formatDuration(500, "en")).toBe("500 ms");
  });

  it("handles seconds", () => {
    expect(formatDuration(1200, "zh-CN")).toBe("1.2 秒");
    expect(formatDuration(59999, "zh-CN")).toBe("60.0 秒");
    expect(formatDuration(1200, "en")).toBe("1.2 s");
  });

  it("handles minutes and seconds", () => {
    expect(formatDuration(65000, "zh-CN")).toBe("1 分 5 秒");
    expect(formatDuration(65000, "en")).toBe("1 min 5 s");
  });

  it("handles hours and minutes", () => {
    expect(formatDuration(3600_000 + 2 * 60_000, "zh-CN")).toBe("1 时 02 分");
    expect(formatDuration(3600_000 + 2 * 60_000, "en")).toBe("1 h 02 min");
  });
});

describe("formatExit", () => {
  it("shows running state regardless of exit code", () => {
    expect(formatExit(null, true, null, "zh-CN")).toBe("运行中…");
    expect(formatExit(null, true, null, "en")).toBe("Running…");
  });

  it("shows success for exit code 0", () => {
    expect(formatExit(0, false, "2026-09-10T12:00:00Z", "zh-CN")).toBe("成功");
    expect(formatExit(0, false, "2026-09-10T12:00:00Z", "en")).toBe("Succeeded");
  });

  it("shows failure with code for nonzero exit", () => {
    expect(formatExit(1, false, "2026-09-10T12:00:00Z", "zh-CN")).toBe(
      "失败（退出码 1）",
    );
    expect(formatExit(1, false, "2026-09-10T12:00:00Z", "en")).toBe(
      "Failed (exit code 1)",
    );
  });

  it("shows external signal only when Launchkeeper did not stop it", () => {
    expect(formatExit(null, false, "2026-09-10T12:00:00Z", "zh-CN")).toContain(
      "被外部信号终止",
    );
    expect(formatExit(null, false, "2026-09-10T12:00:00Z", "en")).toContain(
      "external signal",
    );
  });

  it("names a user stop and a timeout kill by their stop_reason", () => {
    expect(formatExit(null, false, "2026-09-10T12:00:00Z", "zh-CN", "stopped")).toBe("手动停止");
    expect(formatExit(143, false, "2026-09-10T12:00:00Z", "en", "stopped")).toBe("Stopped by hand");
    expect(formatExit(124, false, "2026-09-10T12:00:00Z", "zh-CN", "timeout")).toBe("超时，已被终止");
  });

  it("shows placeholder when never run", () => {
    expect(formatExit(null, false, null, "zh-CN")).toBe("—");
    expect(formatExit(null, false, null, "en")).toBe("—");
  });
});

describe("formatUptime", () => {
  it("handles null/undefined", () => {
    expect(formatUptime(null, "zh-CN")).toBe("—");
    expect(formatUptime(undefined, "zh-CN")).toBe("—");
    expect(formatUptime(Number.NaN, "en")).toBe("—");
  });

  it("shows plain seconds under a minute", () => {
    expect(formatUptime(0, "zh-CN")).toBe("0 秒");
    expect(formatUptime(45, "zh-CN")).toBe("45 秒");
    expect(formatUptime(59, "zh-CN")).toBe("59 秒");
    expect(formatUptime(45, "en")).toBe("45 s");
  });

  it("shows minutes and zero-padded seconds under an hour", () => {
    expect(formatUptime(60, "zh-CN")).toBe("1 分 00 秒");
    expect(formatUptime(785, "zh-CN")).toBe("13 分 05 秒");
    expect(formatUptime(3599, "zh-CN")).toBe("59 分 59 秒");
    expect(formatUptime(785, "en")).toBe("13 min 05 s");
  });

  it("shows hours and minutes under a day", () => {
    expect(formatUptime(3600, "zh-CN")).toBe("1 小时 00 分");
    expect(formatUptime(2 * 3600 + 13 * 60 + 40, "zh-CN")).toBe("2 小时 13 分");
    expect(formatUptime(2 * 3600 + 13 * 60 + 40, "en")).toBe("2 h 13 min");
  });

  it("shows days and hours beyond that", () => {
    expect(formatUptime(86400, "zh-CN")).toBe("1 天 00 小时");
    expect(formatUptime(3 * 86400 + 4 * 3600, "zh-CN")).toBe("3 天 04 小时");
    expect(formatUptime(3 * 86400 + 4 * 3600, "en")).toBe("3 d 04 h");
  });

  it("never shows a negative uptime", () => {
    expect(formatUptime(-5, "zh-CN")).toBe("0 秒");
  });
});

describe("stopReasonText", () => {
  it("names a manual stop and a timeout", () => {
    expect(stopReasonText("stopped", "zh-CN")).toBe("手动停止");
    expect(stopReasonText("timeout", "zh-CN")).toBe("超时");
    expect(stopReasonText("stopped", "en")).toBe("Stopped by hand");
    expect(stopReasonText("timeout", "en")).toBe("Timed out");
  });

  it("says nothing for a normal exit or a missing reason", () => {
    // 结果列已经写了成功/失败，这一列再说一遍"正常结束"只是噪音。
    expect(stopReasonText("exited", "zh-CN")).toBe("");
    expect(stopReasonText(null, "zh-CN")).toBe("");
    expect(stopReasonText(undefined, "en")).toBe("");
  });
});
