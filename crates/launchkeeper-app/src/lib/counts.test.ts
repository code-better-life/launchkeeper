// `lib/counts.ts`：标签 chip 上的数字（M4 §3）。

import { describe, expect, it } from "vitest";

import { tagCounts } from "./counts";
import type { TaskView } from "./bindings";

function view(name: string, tags: string[]): TaskView {
  return {
    name,
    display_name: name,
    description: null,
    trigger: { kind: "at_login" },
    trigger_text: "",
    interpreter_label: null,
    is_service: false,
    keep_alive: false,
    enabled: true,
    loaded_pid: null,
    uptime_secs: null,
    last_run: null,
    tags,
    favorite: false,
    notify_on_fail: false,
    status: "ok",
  };
}

describe("tagCounts", () => {
  it("counts the tasks carrying each tag", () => {
    const counts = tagCounts([
      view("a", ["同步", "定时"]),
      view("b", ["同步"]),
      view("c", ["清理"]),
      view("d", []),
    ]);
    expect(counts.get("同步")).toBe(2);
    expect(counts.get("定时")).toBe(1);
    expect(counts.get("清理")).toBe(1);
    // 没人用过的标签不在表里，界面上按 0 显示。
    expect(counts.get("服务")).toBeUndefined();
    expect(counts.size).toBe(3);
  });

  it("counts a task once even if it carries the same tag twice", () => {
    expect(tagCounts([view("a", ["同步", "同步"])]).get("同步")).toBe(1);
  });

  it("has an entry for nothing when there are no tasks", () => {
    expect(tagCounts([]).size).toBe(0);
  });

  it("is not confused by a tag named like an Object property", () => {
    // 标签名是用户自己起的，用普通对象存计数会和原型上的名字撞车。
    const counts = tagCounts([
      view("a", ["constructor", "__proto__", "toString"]),
      view("b", ["constructor"]),
    ]);
    expect(counts.get("constructor")).toBe(2);
    expect(counts.get("__proto__")).toBe(1);
    expect(counts.get("toString")).toBe(1);
  });

  it("reflects exactly the list it is given", () => {
    // 调用方传的是"只按搜索过滤过"的那一份（taskStore.searchFiltered）：
    // 数字跟着搜索走，不跟着标签筛选走。这里把那个约定钉一下——传进去什么
    // 就数什么，这个函数自己不做任何过滤。
    const all = [view("a", ["同步"]), view("b", ["同步"]), view("c", ["清理"])];
    expect(tagCounts(all).get("同步")).toBe(2);
    const searched = all.filter((t) => t.name === "a");
    expect(tagCounts(searched).get("同步")).toBe(1);
    expect(tagCounts(searched).get("清理")).toBeUndefined();
  });
});
