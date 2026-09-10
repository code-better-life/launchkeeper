// `lib/nextRun.ts`：下次触发时间的预测（M4 §2）。
//
// 所有时刻都用**本地时间**构造（`new Date(y, m, d, h, min)`），断言也用本地
// 字段读回来：这个模块算的就是墙上时钟。整个测试进程的时区由
// `vite.config.ts` 的 `test.env.TZ` 钉在 `America/New_York`，因为夏令时那几条
// 用例需要一个真的有夏令时的地方——作者的机器（Asia/Shanghai）和多半的 CI
// 都没有，而"这台机器恰好有没有夏令时"不该决定一条断言跑不跑。
//
// 那两次跳变：2026-03-08 02:00 跳到 03:00（那天只有 23 小时），
// 2026-11-01 02:00 退回 01:00（那天有 25 小时）。

import { describe, expect, it } from "vitest";

import { nextRunAt, nextRunKey } from "./nextRun";
import type { Trigger } from "./bindings";

/** 本地时间的一个时刻。month 从 1 开始，省得每条用例都心算一次。 */
function at(
  y: number,
  month: number,
  d: number,
  h = 0,
  min = 0,
  s = 0,
): Date {
  return new Date(y, month - 1, d, h, min, s, 0);
}

function task(trigger: Trigger, startedAt: Date | null = null, enabled = true) {
  return {
    trigger,
    enabled,
    last_run: startedAt ? { started_at: startedAt.toISOString() } : null,
  };
}

const daily = (hour: number, minute: number): Trigger => ({
  kind: "calendar",
  entries: [{ hour, minute, weekday: null, day: null }],
});

describe("nextRunAt: 算不出来的那些", () => {
  it("returns null for a service, for at_login and for a disabled task", () => {
    const now = at(2026, 9, 10, 12, 0);
    expect(nextRunAt(task({ kind: "manual" }), now)).toBeNull();
    expect(nextRunAt(task({ kind: "at_login" }), now)).toBeNull();
    // 未启用 = plist 不在 launchd 里，日历规则再明确它也不会跑。
    expect(nextRunAt(task(daily(21, 0), null, false), now)).toBeNull();
  });

  it("gives an interval task no next run until it has run once", () => {
    const now = at(2026, 9, 10, 12, 0);
    const t = task({ kind: "interval", seconds: 3600 });
    expect(nextRunAt(t, now)).toBeNull();
    // 调用方知道 job 是什么时候被 launchd 加载的时，那就是计时起点。
    const loaded = at(2026, 9, 10, 11, 40);
    expect(nextRunAt(t, now, loaded)).toEqual(at(2026, 9, 10, 12, 40));
  });

  it("refuses a nonsensical interval instead of looping", () => {
    const now = at(2026, 9, 10, 12, 0);
    const anchor = at(2026, 9, 10, 11, 0);
    expect(nextRunAt(task({ kind: "interval", seconds: 0 }, anchor), now)).toBeNull();
    expect(nextRunAt(task({ kind: "interval", seconds: -60 }, anchor), now)).toBeNull();
  });
});

describe("nextRunAt: StartInterval", () => {
  it("counts from the start of the last run", () => {
    const now = at(2026, 9, 10, 12, 0);
    const t = task({ kind: "interval", seconds: 6 * 3600 }, at(2026, 9, 10, 9, 30));
    expect(nextRunAt(t, now)).toEqual(at(2026, 9, 10, 15, 30));
  });

  it("skips forward past intervals that were missed entirely", () => {
    // 上次运行是 30 小时前，间隔 6 小时：报的应该是下一个还没到的时刻，
    // 而不是早就过去的那几个（一个过去的时间排在"最快要跑"的最前面是错的）。
    const now = at(2026, 9, 10, 12, 0);
    const t = task({ kind: "interval", seconds: 6 * 3600 }, at(2026, 9, 9, 6, 0));
    expect(nextRunAt(t, now)).toEqual(at(2026, 9, 10, 18, 0));
    // 正好落在 now 上的那个不算"下次"，再往后一格。
    const t2 = task({ kind: "interval", seconds: 3600 }, at(2026, 9, 10, 11, 0));
    expect(nextRunAt(t2, now)).toEqual(at(2026, 9, 10, 13, 0));
  });

  it("measures real seconds, not wall clock, across a DST change", () => {
    // launchd 的 StartInterval 数的是真实秒数：春天少的那一小时不补，秋天多的
    // 那一小时也不多跑一次——所以墙上时钟会跟着挪一格。
    const before = at(2026, 3, 7, 12, 0);
    const t = task({ kind: "interval", seconds: 24 * 3600 }, before);
    const next = nextRunAt(t, before)!;
    expect(next.getTime() - before.getTime()).toBe(24 * 3600 * 1000);
    expect(next.getHours()).toBe(13); // 3-08 少一小时，12:00 变成 13:00
    // 秋天反过来。
    const fall = at(2026, 10, 31, 12, 0);
    const back = nextRunAt(task({ kind: "interval", seconds: 24 * 3600 }, fall), fall)!;
    expect(back.getTime() - fall.getTime()).toBe(24 * 3600 * 1000);
    expect(back.getHours()).toBe(11); // 11-01 多一小时
  });
});

describe("nextRunAt: StartCalendarInterval", () => {
  it("takes today when the time is still ahead, tomorrow when it has passed", () => {
    expect(nextRunAt(task(daily(21, 0)), at(2026, 9, 10, 12, 0))).toEqual(
      at(2026, 9, 10, 21, 0),
    );
    expect(nextRunAt(task(daily(21, 0)), at(2026, 9, 10, 21, 30))).toEqual(
      at(2026, 9, 11, 21, 0),
    );
    // 正好是那一刻时算下一天：那次触发已经在发生了。
    expect(nextRunAt(task(daily(21, 0)), at(2026, 9, 10, 21, 0))).toEqual(
      at(2026, 9, 11, 21, 0),
    );
  });

  it("takes the earliest of several entries", () => {
    const trigger: Trigger = {
      kind: "calendar",
      entries: [
        { hour: 21, minute: 0, weekday: null, day: null },
        { hour: 6, minute: 30, weekday: null, day: null },
        { hour: 13, minute: 15, weekday: null, day: null },
      ],
    };
    // 中午 12 点看：今天 13:15 最近。
    expect(nextRunAt(task(trigger), at(2026, 9, 10, 12, 0))).toEqual(
      at(2026, 9, 10, 13, 15),
    );
    // 晚上 22 点看：全部落到明天，最早的是 06:30。
    expect(nextRunAt(task(trigger), at(2026, 9, 10, 22, 0))).toEqual(
      at(2026, 9, 11, 6, 30),
    );
  });

  it("honours a weekday, with 0 and 7 both meaning Sunday", () => {
    // 2026-09-10 是周四。
    expect(at(2026, 9, 10).getDay()).toBe(4);
    const monday: Trigger = {
      kind: "calendar",
      entries: [{ hour: 9, minute: 0, weekday: 1, day: null }],
    };
    expect(nextRunAt(task(monday), at(2026, 9, 10, 12, 0))).toEqual(
      at(2026, 9, 14, 9, 0),
    );
    const sunday0: Trigger = {
      kind: "calendar",
      entries: [{ hour: 9, minute: 0, weekday: 0, day: null }],
    };
    const sunday7: Trigger = {
      kind: "calendar",
      entries: [{ hour: 9, minute: 0, weekday: 7, day: null }],
    };
    const now = at(2026, 9, 10, 12, 0);
    expect(nextRunAt(task(sunday0), now)).toEqual(at(2026, 9, 13, 9, 0));
    expect(nextRunAt(task(sunday7), now)).toEqual(nextRunAt(task(sunday0), now));
  });

  it("honours a day of the month, including one that skips short months", () => {
    const first: Trigger = {
      kind: "calendar",
      entries: [{ hour: 3, minute: 0, weekday: null, day: 1 }],
    };
    expect(nextRunAt(task(first), at(2026, 9, 10, 12, 0))).toEqual(
      at(2026, 10, 1, 3, 0),
    );
    // 31 号：9 月没有，下一次是 10 月 31 日。
    const day31: Trigger = {
      kind: "calendar",
      entries: [{ hour: 3, minute: 0, weekday: null, day: 31 }],
    };
    expect(nextRunAt(task(day31), at(2026, 9, 10, 12, 0))).toEqual(
      at(2026, 10, 31, 3, 0),
    );
  });

  it("requires both when a weekday and a day of the month are given", () => {
    // core 的 describe_calendar 说的是「每月 X 日且周 Y」——两者都要匹配。
    // 2026-11-01 是周日。
    expect(at(2026, 11, 1).getDay()).toBe(0);
    const trigger: Trigger = {
      kind: "calendar",
      entries: [{ hour: 8, minute: 0, weekday: 0, day: 1 }],
    };
    expect(nextRunAt(task(trigger), at(2026, 9, 10, 12, 0))).toEqual(
      at(2026, 11, 1, 8, 0),
    );
  });

  it("gives up rather than scanning years for a combination that almost never happens", () => {
    // 「每月 31 日且周一」在一年之内不一定有；扫不到就当算不出来，排到最后。
    const trigger: Trigger = {
      kind: "calendar",
      entries: [{ hour: 8, minute: 0, weekday: 1, day: 31 }],
    };
    const result = nextRunAt(task(trigger), at(2026, 9, 10, 12, 0));
    if (result !== null) {
      // 真扫到了也必须在 366 天以内，且确实是个周一的 31 号。
      expect(result.getDate()).toBe(31);
      expect(result.getDay()).toBe(1);
      expect(result.getTime() - at(2026, 9, 10, 12, 0).getTime()).toBeLessThan(
        367 * 24 * 3600 * 1000,
      );
    }
  });

  it("keeps the wall-clock time across a DST change instead of drifting an hour", () => {
    // 21:00 的任务在跳变那天仍然是 21:00——因为下一天是用日历字段构造的，
    // 不是"上次 + 86400000"。春天那天真实只隔了 23 小时，秋天隔了 25 小时。
    const springEve = at(2026, 3, 7, 22, 0);
    const spring = nextRunAt(task(daily(21, 0)), springEve)!;
    expect([spring.getMonth() + 1, spring.getDate(), spring.getHours()]).toEqual([3, 8, 21]);
    expect(spring.getTime() - springEve.getTime()).toBe(22 * 3600 * 1000);

    const fallEve = at(2026, 10, 31, 22, 0);
    const fall = nextRunAt(task(daily(21, 0)), fallEve)!;
    expect([fall.getMonth() + 1, fall.getDate(), fall.getHours()]).toEqual([11, 1, 21]);
    expect(fall.getTime() - fallEve.getTime()).toBe(24 * 3600 * 1000);
  });

  it("normalises a time inside the hour DST skips, the way launchd catches up", () => {
    // 2026-03-08 的 02:30 在纽约不存在。JS 把它规整到 03:30，正好也是
    // launchd 醒来之后补跑的语义——总之不该因此返回 null 或者昨天的时刻。
    const trigger: Trigger = {
      kind: "calendar",
      entries: [{ hour: 2, minute: 30, weekday: null, day: null }],
    };
    const next = nextRunAt(task(trigger), at(2026, 3, 7, 12, 0))!;
    expect(next.getDate()).toBe(8);
    expect(next.getHours()).toBe(3);
    expect(next.getMinutes()).toBe(30);
  });
});

describe("nextRunKey", () => {
  it("is the timestamp, or +Infinity when there is no next run", () => {
    const now = at(2026, 9, 10, 12, 0);
    expect(nextRunKey(task(daily(21, 0)), now)).toBe(at(2026, 9, 10, 21, 0).getTime());
    expect(nextRunKey(task({ kind: "manual" }), now)).toBe(Number.POSITIVE_INFINITY);
  });
});

/** 时区确实是钉住的——上面那几条夏令时用例全指望它。 */
describe("the test timezone", () => {
  it("is the one vite.config.ts pins", () => {
    // 3-08 那天纽约是 UTC-4（夏令时），前一天还是 UTC-5。
    expect(at(2026, 3, 7, 12, 0).getTimezoneOffset()).toBe(300);
    expect(at(2026, 3, 8, 12, 0).getTimezoneOffset()).toBe(240);
  });
});
