import { describe, expect, it } from "vitest";
import { describeInterval, describeTrigger } from "./describeTrigger";

const cal = (entries: { minute: number; hour: number; weekday: number | null; day: number | null }[]) =>
  ({ kind: "calendar", entries }) as const;

describe("describeTrigger", () => {
  it("simple kinds in both languages", () => {
    expect(describeTrigger({ kind: "at_login" }, "zh-CN")).toBe("登录时");
    expect(describeTrigger({ kind: "at_login" }, "en")).toBe("at login");
    expect(describeTrigger({ kind: "manual" }, "en")).toBe("start/stop by hand");
  });

  it("intervals pick the largest whole unit and pluralise in English", () => {
    expect(describeInterval(7200, "zh-CN")).toBe("每 2 小时");
    expect(describeInterval(7200, "en")).toBe("every 2 hours");
    expect(describeInterval(3600, "en")).toBe("every hour");
    expect(describeInterval(90, "en")).toBe("every 90 seconds");
    expect(describeInterval(172800, "zh-CN")).toBe("每 2 天");
  });

  it("daily times are sorted and deduplicated", () => {
    const t = cal([
      { minute: 0, hour: 21, weekday: null, day: null },
      { minute: 30, hour: 8, weekday: null, day: null },
      { minute: 0, hour: 21, weekday: null, day: null },
    ]);
    expect(describeTrigger(t, "zh-CN")).toBe("每天 08:30、21:00");
    expect(describeTrigger(t, "en")).toBe("daily at 08:30, 21:00");
  });

  it("weekly entries merge by time and treat 7 as Sunday", () => {
    const t = cal([
      { minute: 0, hour: 3, weekday: 7, day: null },
      { minute: 0, hour: 3, weekday: 1, day: null },
      { minute: 0, hour: 3, weekday: 0, day: null },
    ]);
    expect(describeTrigger(t, "zh-CN")).toBe("每周日、一 03:00");
    expect(describeTrigger(t, "en")).toBe("Sun, Mon at 03:00");
  });

  it("monthly and combined rules", () => {
    expect(describeTrigger(cal([{ minute: 0, hour: 8, weekday: null, day: 1 }]), "en")).toBe(
      "monthly on day 1 at 08:00",
    );
    expect(describeTrigger(cal([{ minute: 0, hour: 8, weekday: 5, day: 13 }]), "zh-CN")).toBe(
      "每月 13 日且周五 08:00",
    );
  });

  it("mixed rule kinds are joined with a separator", () => {
    const t = cal([
      { minute: 0, hour: 9, weekday: null, day: null },
      { minute: 0, hour: 3, weekday: 0, day: null },
    ]);
    expect(describeTrigger(t, "en")).toBe("daily at 09:00; Sun at 03:00");
  });

  it("falls back to the backend text when the trigger is unknown", () => {
    expect(describeTrigger(null, "en", "unsupported trigger")).toBe("unsupported trigger");
  });
});
