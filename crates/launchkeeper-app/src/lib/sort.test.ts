// `lib/sort.ts`：管理组的四种排序（M4 §2）。
//
// 时区同 `nextRun.test.ts`，由 `vite.config.ts` 钉在 America/New_York。

import { describe, expect, it } from "vitest";

import {
  DEFAULT_SORT,
  LIST_SORT_KEY,
  SORT_LABEL,
  SORT_MODES,
  readListSort,
  sortMode,
  sortTasks,
  statusRank,
  writeListSort,
} from "./sort";
import { zhCN } from "./i18n/zh-CN";
import type { TaskStatus, TaskView, Trigger } from "./bindings";

const NOW = new Date(2026, 8, 10, 12, 0, 0, 0); // 2026-09-10 12:00 本地

interface Fixture {
  name: string;
  display?: string;
  status?: TaskStatus;
  trigger?: Trigger;
  lastRun?: Date | null;
  enabled?: boolean;
}

function view(f: Fixture): TaskView {
  const started = f.lastRun ?? null;
  return {
    name: f.name,
    display_name: f.display ?? f.name,
    description: null,
    trigger: f.trigger ?? { kind: "at_login" },
    trigger_text: "",
    interpreter_label: null,
    is_service: (f.trigger ?? { kind: "at_login" }).kind === "manual",
    keep_alive: false,
    enabled: f.enabled ?? true,
    loaded_pid: null,
    uptime_secs: null,
    last_run: started
      ? {
          id: 1,
          started_at: started.toISOString(),
          finished_at: started.toISOString(),
          exit_code: 0,
          duration_ms: 1000,
          trigger_kind: "scheduled",
          stop_reason: "exited",
          running: false,
        }
      : null,
    tags: [],
    favorite: false,
    notify_on_fail: false,
    status: f.status ?? "ok",
  };
}

const names = (list: TaskView[]) => list.map((t) => t.name);

describe("sortMode", () => {
  it("maps anything it does not know back to the default", () => {
    for (const mode of SORT_MODES) expect(sortMode(mode)).toBe(mode);
    expect(sortMode(null)).toBe(DEFAULT_SORT);
    expect(sortMode(undefined)).toBe(DEFAULT_SORT);
    expect(sortMode("")).toBe(DEFAULT_SORT);
    // 手改过的 settings.json / 别的版本写的值，都不该让列表停在一个没人选过的
    // 状态上。
    expect(sortMode("by-name")).toBe(DEFAULT_SORT);
    expect(sortMode("__proto__")).toBe(DEFAULT_SORT);
  });

  it("has a dictionary key for every mode", () => {
    for (const mode of SORT_MODES) {
      expect(SORT_LABEL[mode] in zhCN).toBe(true);
    }
  });
});

describe("sortTasks: 按名称", () => {
  it("sorts by display name and never mutates the input", () => {
    const input = [
      view({ name: "c", display: "Zeta" }),
      view({ name: "a", display: "alpha" }),
      view({ name: "b", display: "Beta" }),
    ];
    const before = names(input);
    expect(names(sortTasks(input, "name", NOW, "en"))).toEqual(["a", "b", "c"]);
    expect(names(input)).toEqual(before);
  });

  it("breaks a display-name tie on the unique name", () => {
    const input = [
      view({ name: "b", display: "同名" }),
      view({ name: "a", display: "同名" }),
    ];
    expect(names(sortTasks(input, "name", NOW, "zh-CN"))).toEqual(["a", "b"]);
  });
});

describe("sortTasks: 按最近运行", () => {
  it("puts the newest first and the never-run last", () => {
    const input = [
      view({ name: "old", lastRun: new Date(2026, 8, 1, 9, 0) }),
      view({ name: "never" }),
      view({ name: "new", lastRun: new Date(2026, 8, 10, 11, 0) }),
      view({ name: "mid", lastRun: new Date(2026, 8, 9, 11, 0) }),
    ];
    expect(names(sortTasks(input, "last_run", NOW, "en"))).toEqual([
      "new",
      "mid",
      "old",
      "never",
    ]);
  });

  it("orders several never-run tasks by name rather than at random", () => {
    const input = [
      view({ name: "b" }),
      view({ name: "c" }),
      view({ name: "a" }),
    ];
    expect(names(sortTasks(input, "last_run", NOW, "en"))).toEqual(["a", "b", "c"]);
  });
});

describe("sortTasks: 按状态", () => {
  it("ranks failed, then running, then everything else by name", () => {
    expect(statusRank("failed")).toBeLessThan(statusRank("running"));
    expect(statusRank("running")).toBeLessThan(statusRank("ok"));
    for (const s of ["ok", "never", "disabled", "stopped"] as TaskStatus[]) {
      expect(statusRank(s)).toBe(statusRank("ok"));
    }
    const input = [
      view({ name: "d-ok", status: "ok" }),
      view({ name: "b-running", status: "running" }),
      view({ name: "a-failed", status: "failed" }),
      view({ name: "c-disabled", status: "disabled", enabled: false }),
      view({ name: "e-never", status: "never" }),
    ];
    expect(names(sortTasks(input, "status", NOW, "en"))).toEqual([
      "a-failed",
      "b-running",
      "c-disabled",
      "d-ok",
      "e-never",
    ]);
  });
});

describe("sortTasks: 按下次触发", () => {
  it("puts the soonest first and everything without a next run last", () => {
    const input = [
      // 每天 21:00 -> 今天 21:00
      view({
        name: "evening",
        trigger: { kind: "calendar", entries: [{ hour: 21, minute: 0, weekday: null, day: null }] },
      }),
      // 服务：没有下次
      view({ name: "service", trigger: { kind: "manual" } }),
      // 每小时一次，上次 11:30 -> 12:30
      view({
        name: "hourly",
        trigger: { kind: "interval", seconds: 3600 },
        lastRun: new Date(2026, 8, 10, 11, 30),
      }),
      // 登录时：没有下次
      view({ name: "login", trigger: { kind: "at_login" } }),
      // 每天 13:00 -> 今天 13:00
      view({
        name: "afternoon",
        trigger: { kind: "calendar", entries: [{ hour: 13, minute: 0, weekday: null, day: null }] },
      }),
    ];
    expect(names(sortTasks(input, "next_run", NOW, "en"))).toEqual([
      "hourly",
      "afternoon",
      "evening",
      // 算不出来的两个按名字排在最后
      "login",
      "service",
    ]);
  });

  it("treats a disabled task as having no next run", () => {
    const daily: Trigger = {
      kind: "calendar",
      entries: [{ hour: 13, minute: 0, weekday: null, day: null }],
    };
    const input = [
      view({ name: "off", trigger: daily, enabled: false, status: "disabled" }),
      view({
        name: "on",
        trigger: { kind: "calendar", entries: [{ hour: 21, minute: 0, weekday: null, day: null }] },
      }),
    ];
    // 未启用的那个时间上更早，但它根本不会跑，所以排在后面。
    expect(names(sortTasks(input, "next_run", NOW, "en"))).toEqual(["on", "off"]);
  });
});

/**
 * 测试跑在 node 环境里，没有 `localStorage`（也没装 jsdom，为一个键值对引一份
 * DOM 实现不值当）。这里塞一个 Map 撑起用到的三个方法，用完撤掉。
 */
function withStorage<T>(store: Map<string, string> | null, body: () => T): T {
  const had = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    get() {
      if (store === null) throw new Error("SecurityError: storage is disabled");
      return {
        getItem: (k: string) => store.get(k) ?? null,
        setItem: (k: string, v: string) => void store.set(k, v),
        removeItem: (k: string) => void store.delete(k),
      };
    },
  });
  try {
    return body();
  } finally {
    if (had) Object.defineProperty(globalThis, "localStorage", had);
    else delete (globalThis as Record<string, unknown>).localStorage;
  }
}

describe("localStorage 兜底", () => {
  it("round-trips a mode and ignores a corrupt value", () => {
    const store = new Map<string, string>();
    withStorage(store, () => {
      // 从没存过和存了个坏值不是一回事：没存过时调用方还要去看设置里那份。
      expect(readListSort()).toBeNull();
      writeListSort("status");
      expect(store.get(LIST_SORT_KEY)).toBe("status");
      expect(readListSort()).toBe("status");
      store.set(LIST_SORT_KEY, "nonsense");
      expect(readListSort()).toBe(DEFAULT_SORT);
    });
  });

  it("survives a localStorage that throws", () => {
    // 隐私模式、被策略关掉的 webview：读写都不该把列表带崩。
    withStorage(null, () => {
      expect(readListSort()).toBeNull();
      expect(() => writeListSort("name")).not.toThrow();
    });
  });
});
