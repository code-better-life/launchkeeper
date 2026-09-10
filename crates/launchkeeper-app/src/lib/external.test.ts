import { describe, expect, it } from "vitest";
import type { AdoptionPlanView, ExternalAgentView } from "./bindings";
import {
  adoptTooltip,
  diffLines,
  externalHeadline,
  externalLogText,
  externalStatusTitle,
  externalSubtitle,
  filterExternal,
  foreignDisableWarning,
  groupExternal,
  isForeignAgent,
  shouldForceExpand,
} from "./external";

function agent(over: Partial<ExternalAgentView> = {}): ExternalAgentView {
  return {
    label: "com.example.backup-notes",
    path: "~/Library/LaunchAgents/com.example.backup-notes.plist",
    full_path: "/Users/demo/Library/LaunchAgents/com.example.backup-notes.plist",
    command: "/bin/zsh /Users/demo/scripts/backup-notes.sh",
    trigger: null,
    trigger_text: "每周日 03:00",
    working_dir: "~/Notes",
    stdout_path: "~/Library/Logs/backup-notes.log",
    stderr_path: "~/Library/Logs/backup-notes.log",
    keep_alive: false,
    loaded: true,
    pid: null,
    last_exit: 0,
    adoptable: true,
    adopt_reason: null,
    managed: false,
    ...over,
  };
}

describe("externalSubtitle", () => {
  it("可接管的说没有历史——那正是接管的理由", () => {
    expect(externalSubtitle(agent(), "zh-CN")).toBe("手写 plist · 无运行历史");
  });

  it("不可接管的把理由摆在行里，不用等 tooltip", () => {
    const a = agent({ adoptable: false, adopt_reason: "程序在 /Library 下" });
    expect(externalSubtitle(a, "zh-CN")).toBe("程序在 /Library 下 · 仅展示");
  });

  it("英文那边说的是同一件事", () => {
    expect(externalSubtitle(agent(), "en")).toBe(
      "Hand-written plist · no run history",
    );
    // 不可接管的理由是后端给的原话，本期仍然只有中文（M3 §5），英文只翻译
    // 它外面那半句。
    const a = agent({ adoptable: false, adopt_reason: "程序在 /Library 下" });
    expect(externalSubtitle(a, "en")).toBe("程序在 /Library 下 · shown only");
  });

  it("已接管的 plist 仍然会被列出来，但说清它归谁", () => {
    const a = agent({ managed: true, adoptable: false, adopt_reason: "已经由 Launchkeeper 管理" });
    expect(externalSubtitle(a, "zh-CN")).toBe("已由 Launchkeeper 管理");
  });
});

describe("externalStatusTitle / externalHeadline", () => {
  it("没加载、加载了、跑着，三种说法各不相同", () => {
    expect(externalStatusTitle(agent({ loaded: false }), "zh-CN")).toBe("未加载到 launchd");
    expect(externalStatusTitle(agent(), "zh-CN")).toBe("已加载，等待触发");
    expect(externalStatusTitle(agent({ pid: 4242 }), "zh-CN")).toBe("运行中 · pid 4242");
  });

  it("详情页头部把路径、来源和 launchd 状态排成一行", () => {
    expect(externalHeadline(agent({ pid: 77 }), "zh-CN")).toBe(
      "~/Library/LaunchAgents/com.example.backup-notes.plist · 手写 plist · 运行中 · pid 77",
    );
    expect(externalHeadline(agent({ loaded: false }), "zh-CN")).toContain("· 未加载");
  });
});

describe("externalLogText", () => {
  it("两个流指到同一个文件时只报一次", () => {
    expect(externalLogText(agent(), "zh-CN")).toBe("~/Library/Logs/backup-notes.log（合并）");
  });

  it("分开写就分开报", () => {
    const a = agent({ stdout_path: "~/a.log", stderr_path: "~/b.log" });
    expect(externalLogText(a, "zh-CN")).toBe("~/a.log · 错误 ~/b.log");
  });

  it("英文的日志格用 stderr 而不是「错误」", () => {
    const a = agent({ stdout_path: "~/a.log", stderr_path: "~/b.log" });
    expect(externalLogText(a, "en")).toBe("~/a.log · stderr ~/b.log");
    expect(externalLogText(agent(), "en")).toBe(
      "~/Library/Logs/backup-notes.log (merged)",
    );
  });

  it("一个都没有就直说输出被丢掉了", () => {
    const a = agent({ stdout_path: null, stderr_path: null });
    expect(externalLogText(a, "zh-CN")).toBe("未设置（输出被 launchd 丢弃）");
  });
});

describe("adoptTooltip", () => {
  it("能接管就说会做什么", () => {
    expect(adoptTooltip(agent(), "zh-CN")).toContain(".bak");
  });

  it("不能接管就原样转达后端给的理由", () => {
    const a = agent({ adoptable: false, adopt_reason: "plist 里有 WatchPaths" });
    expect(adoptTooltip(a, "zh-CN")).toBe("plist 里有 WatchPaths");
  });
});

describe("diffLines", () => {
  const plan: AdoptionPlanView = {
    label: "com.example.backup-notes",
    name: "com.example.backup-notes",
    path: "~/Library/LaunchAgents/com.example.backup-notes.plist",
    backup_path: "~/Library/LaunchAgents/com.example.backup-notes.plist.bak",
    removed: [{ key: "ProgramArguments", value: "/bin/zsh backup.sh" }],
    added: [
      { key: "ProgramArguments", value: "launchkeeper-runner run com.example.backup-notes" },
      { key: "LaunchkeeperManaged", value: "true" },
    ],
    kept: [
      { key: "StartCalendarInterval", value: "Hour=3" },
      { key: "WorkingDirectory", value: "~/Notes" },
    ],
  };

  it("先删后加，最后一行把没变的收成一句", () => {
    const lines = diffLines(plan, "zh-CN");
    expect(lines.map((l) => l.sign)).toEqual(["−", "+", "+", "="]);
    expect(lines[0].text).toBe("ProgramArguments: /bin/zsh backup.sh");
    expect(lines[3].text).toBe("StartCalendarInterval、WorkingDirectory 保持不变");
    expect(lines[3].muted).toBe(true);
  });

  it("英文用逗号连接没变的键", () => {
    const lines = diffLines(plan, "en");
    expect(lines[3].text).toBe(
      "StartCalendarInterval, WorkingDirectory unchanged",
    );
  });

  it("没有保持不变的键时就不写那一行", () => {
    expect(diffLines({ ...plan, kept: [] }, "zh-CN")).toHaveLength(3);
  });
});

describe("filterExternal", () => {
  const list = [agent(), agent({ label: "com.google.keystone.agent", command: "ksadmin --ping" })];

  it("搜索同时看 label、路径和命令", () => {
    expect(filterExternal(list, "keystone", []).map((a) => a.label)).toEqual([
      "com.google.keystone.agent",
    ]);
    expect(filterExternal(list, "backup-notes.sh", [])).toHaveLength(1);
    expect(filterExternal(list, "  ", [])).toHaveLength(2);
  });

  it("一选标签这一组就整体让位——外部 agent 没有标签，答不上来", () => {
    expect(filterExternal(list, "", ["同步"])).toEqual([]);
  });
});

describe("groupExternal", () => {
  const list = [
    agent({ label: "com.google.keystone.agent", adoptable: false, adopt_reason: "程序在 /Library 下" }),
    agent({ label: "com.example.photo-sync" }),
    agent({ label: "com.example.backup-notes" }),
    agent({ label: "com.example.mail-archive", adoptable: false, managed: true }),
    agent({ label: "com.adobe.ccxprocess", adoptable: false, adopt_reason: "程序在 .app 包里" }),
  ];

  it("可接管的排在前面，两堆各自按 label 字母序", () => {
    const { adoptable, others } = groupExternal(list);
    expect(adoptable.map((a) => a.label)).toEqual([
      "com.example.backup-notes",
      "com.example.photo-sync",
    ]);
    expect(others.map((a) => a.label)).toEqual([
      "com.adobe.ccxprocess",
      "com.example.mail-archive",
      "com.google.keystone.agent",
    ]);
  });

  it("已接管的归到折叠那一堆——它已经以任务的身份出现在上面一组里了", () => {
    const { adoptable, others } = groupExternal([
      agent({ label: "com.x.managed", adoptable: true, managed: true }),
    ]);
    expect(adoptable).toEqual([]);
    expect(others).toHaveLength(1);
  });

  it("不改原数组的顺序", () => {
    const original = list.map((a) => a.label);
    groupExternal(list);
    expect(list.map((a) => a.label)).toEqual(original);
  });
});

describe("shouldForceExpand", () => {
  const grouped = (adoptable: number, others: number) => ({
    adoptable: Array.from({ length: adoptable }, (_, i) => agent({ label: `a${i}` })),
    others: Array.from({ length: others }, (_, i) => agent({ label: `o${i}` })),
  });

  it("没在搜索时一律保持折叠", () => {
    expect(shouldForceExpand("", grouped(0, 3))).toBe(false);
    expect(shouldForceExpand("   ", grouped(0, 3))).toBe(false);
  });

  it("搜索只命中折叠里的那几个时自动展开", () => {
    expect(shouldForceExpand("keystone", grouped(0, 2))).toBe(true);
  });

  it("上面那一堆也有命中时不动它——结果已经看得见了", () => {
    expect(shouldForceExpand("com", grouped(1, 2))).toBe(false);
  });

  it("两堆都空就没什么可展开的", () => {
    expect(shouldForceExpand("nothing", grouped(0, 0))).toBe(false);
  });
});

// M4 CLI docs / external disable
describe("isForeignAgent / foreignDisableWarning", () => {
  it("可接管的手写 plist 不算别人的", () => {
    expect(isForeignAgent(agent())).toBe(false);
  });

  it("Launchkeeper 自己管的也不算别人的", () => {
    expect(isForeignAgent(agent({ adoptable: false, managed: true }))).toBe(false);
  });

  it("不可接管、又不是我们的，就是别的软件装的", () => {
    expect(isForeignAgent(agent({ adoptable: false, adopt_reason: "plist 里有 WatchPaths" }))).toBe(
      true,
    );
  });

  it("警告里有 label、后端给的理由，以及「plist 不会被改动」这句", () => {
    const a = agent({ adoptable: false, adopt_reason: "plist 里有 WatchPaths" });
    const zh = foreignDisableWarning(a, "zh-CN");
    expect(zh).toContain(a.label);
    expect(zh).toContain("plist 里有 WatchPaths");
    expect(zh).toContain("plist 文件不会被改动");
    const en = foreignDisableWarning(a, "en");
    expect(en).toContain(a.label);
    expect(en).toContain("not modified");
  });

  it("后端没给理由时退回到通用文案", () => {
    const a = agent({ adoptable: false, adopt_reason: null });
    expect(foreignDisableWarning(a, "zh-CN")).toContain("不可接管");
  });
});
