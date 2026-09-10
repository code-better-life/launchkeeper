// 只读概览（lib/summary.ts）的例子。两种语言各测一遍，重点在那几个"机器味的
// 字段翻成人话"的地方：拼好的 script_path + args 拆回脚本和参数、秒数变成表单里
// 用户填的那个单位、环境变量一行一个。

import { describe, expect, it } from "vitest";
import type { TaskInput } from "./bindings";
import {
  argsText,
  envText,
  runModeText,
  scriptText,
  summaryRows,
  timeoutText,
} from "./summary";

function task(over: Partial<TaskInput> = {}): TaskInput {
  return {
    name: "report-sync",
    display_name: "夜间报告同步",
    description: "每天 21:00 从对象存储拉新文件",
    script_path: "/Users/demo/.local/bin/uv",
    args: ["run", "/Users/demo/projects/report/sync.py", "--once"],
    working_dir: "/Users/demo/projects/report",
    env: [{ key: "TZ", value: "Asia/Shanghai" }],
    trigger: { kind: "calendar", entries: [{ hour: 21, minute: 0, weekday: null, day: null }] },
    keep_alive: false,
    timeout_secs: 600,
    tags: ["同步", "定时"],
    favorite: true,
    notify_on_fail: true,
    ...over,
  };
}

describe("runModeText / scriptText / argsText", () => {
  it("把 uv run 那条命令拆成运行方式、脚本、脚本自己的参数", () => {
    const t = task();
    expect(runModeText(t, "zh-CN")).toBe("uv run");
    expect(scriptText(t)).toBe("/Users/demo/projects/report/sync.py");
    expect(argsText(t, "zh-CN")).toBe("--once");
  });

  it("解释器自己的参数跟在运行方式后面，不混进脚本参数里", () => {
    const t = task({
      script_path: "/usr/bin/python3",
      args: ["-u", "/Users/demo/a.py", "--verbose"],
    });
    expect(runModeText(t, "zh-CN")).toBe("python3 -u");
    expect(scriptText(t)).toBe("/Users/demo/a.py");
    expect(argsText(t, "zh-CN")).toBe("--verbose");
  });

  it("直接执行的脚本说「直接执行」，英文说 Run directly", () => {
    const t = task({ script_path: "/Users/demo/scripts/cleanup.sh", args: [] });
    expect(runModeText(t, "zh-CN")).toBe("直接执行");
    expect(runModeText(t, "en")).toBe("Run directly");
    expect(scriptText(t)).toBe("/Users/demo/scripts/cleanup.sh");
    expect(argsText(t, "zh-CN")).toBe("无");
    expect(argsText(t, "en")).toBe("None");
  });

  it("拆不开的命令（解释器后面没有脚本）没有运行方式可说，脚本格照原样", () => {
    const t = task({ script_path: "/usr/bin/python3", args: ["--version"] });
    expect(runModeText(t, "zh-CN")).toBe("—");
    expect(scriptText(t)).toBe("/usr/bin/python3");
    expect(argsText(t, "zh-CN")).toBe("--version");
  });
});

describe("envText", () => {
  it("一行一个 KEY=value", () => {
    expect(
      envText(
        [
          { key: "TZ", value: "Asia/Shanghai" },
          { key: "LOG", value: "/tmp/a.log" },
        ],
        "zh-CN",
      ),
    ).toBe("TZ=Asia/Shanghai\nLOG=/tmp/a.log");
  });

  it("一个都没有就说「无」", () => {
    expect(envText([], "zh-CN")).toBe("无");
    expect(envText([], "en")).toBe("None");
  });
});

describe("timeoutText", () => {
  it("按表单里那个单位说，600 秒是 10 分钟", () => {
    expect(timeoutText(600, "zh-CN")).toBe("10 分钟");
    expect(timeoutText(600, "en")).toBe("10 minutes");
    expect(timeoutText(7200, "zh-CN")).toBe("2 小时");
    expect(timeoutText(90, "zh-CN")).toBe("90 秒");
  });

  it("没设超时说「无限制」", () => {
    expect(timeoutText(null, "zh-CN")).toBe("无限制");
    expect(timeoutText(null, "en")).toBe("No limit");
  });
});

describe("summaryRows", () => {
  const extras = {
    triggerText: "每天 21:00",
    logDir: "/Users/demo/Library/Application Support/Launchkeeper/logs/report-sync",
    adoptedFrom: null,
  };

  it("按界面上的顺序给出每一行", () => {
    const rows = summaryRows(task(), extras, "zh-CN");
    expect(rows.map((r) => r.label)).toEqual([
      "form.display_name",
      "form.description",
      "form.script",
      "form.run_with",
      "form.args",
      "form.working_dir",
      "form.env",
      "form.trigger",
      "form.timeout",
      "form.tags",
      "form.notify_on_fail",
      "summary.log_dir",
    ]);
  });

  it("描述、日志目录、接管来源没有值时整行不出现", () => {
    const rows = summaryRows(
      task({ description: null }),
      { triggerText: "每天 21:00" },
      "zh-CN",
    );
    expect(rows.map((r) => r.label)).not.toContain("form.description");
    expect(rows.map((r) => r.label)).not.toContain("summary.log_dir");
    expect(rows.map((r) => r.label)).not.toContain("detail.adopted_from");
  });

  it("接管来的任务多一行「接管自」", () => {
    const rows = summaryRows(
      task(),
      { ...extras, adoptedFrom: "~/Library/LaunchAgents/com.x.y.plist" },
      "zh-CN",
    );
    const adopted = rows.find((r) => r.label === "detail.adopted_from");
    expect(adopted?.value).toBe("~/Library/LaunchAgents/com.x.y.plist");
  });

  it("没设工作目录时说的是 $HOME，而且那一格不是等宽的路径", () => {
    const rows = summaryRows(task({ working_dir: null }), extras, "zh-CN");
    const dir = rows.find((r) => r.label === "form.working_dir")!;
    expect(dir.value).toBe("$HOME（未设置）");
    expect(dir.mono).toBe(false);
  });

  it("常驻服务的 KeepAlive 挂在触发规则那一行后面", () => {
    const rows = summaryRows(
      task({ trigger: { kind: "manual" }, keep_alive: true }),
      { triggerText: "手动启停" },
      "zh-CN",
    );
    expect(rows.find((r) => r.label === "form.trigger")!.value).toBe(
      "手动启停 · 保持常驻",
    );
  });

  it("英文那边说的是同一件事", () => {
    const rows = summaryRows(task({ tags: [], notify_on_fail: false }), extras, "en");
    expect(rows.find((r) => r.label === "form.tags")!.value).toBe("None");
    expect(rows.find((r) => r.label === "form.notify_on_fail")!.value).toBe("Off");
  });
});
