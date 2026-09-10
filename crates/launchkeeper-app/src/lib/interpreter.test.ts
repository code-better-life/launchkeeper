import { describe, expect, it } from "vitest";
import {
  basename,
  composeTaskCommand,
  customInterpreter,
  detectFromTask,
  displayName,
  groupInterpreters,
  interpreterId,
  viewOf,
  type RunWith,
} from "./interpreter";
import type { InterpreterView } from "./bindings";

const python: RunWith = {
  kind: "python",
  program: "/usr/bin/python3",
  prefix_args: [],
};
const uv: RunWith = {
  kind: "uv",
  program: "/Users/x/.local/bin/uv",
  prefix_args: ["run"],
};
const direct: RunWith = {
  kind: "direct",
  program: "/p/backup.sh",
  prefix_args: [],
};

describe("composeTaskCommand", () => {
  it("puts the interpreter first and the script in front of its own args", () => {
    expect(composeTaskCommand("/p/sync.py", ["--once"], python)).toEqual({
      script_path: "/usr/bin/python3",
      args: ["/p/sync.py", "--once"],
    });
  });

  it("keeps the prefix args of uv run", () => {
    expect(composeTaskCommand("/p/sync.py", [], uv)).toEqual({
      script_path: "/Users/x/.local/bin/uv",
      args: ["run", "/p/sync.py"],
    });
  });

  it("leaves the script alone for direct execution", () => {
    expect(composeTaskCommand("/p/backup.sh", ["-n"], direct)).toEqual({
      script_path: "/p/backup.sh",
      args: ["-n"],
    });
  });

  it("falls back to direct execution when nothing is selected", () => {
    expect(composeTaskCommand("/p/backup.sh", [], null)).toEqual({
      script_path: "/p/backup.sh",
      args: [],
    });
  });

  it("puts the interpreter's own flags after its prefix args", () => {
    expect(composeTaskCommand("/p/sync.py", ["--once"], python, ["-u"])).toEqual({
      script_path: "/usr/bin/python3",
      args: ["-u", "/p/sync.py", "--once"],
    });
    expect(composeTaskCommand("/p/sync.py", [], uv, ["--frozen"])).toEqual({
      script_path: "/Users/x/.local/bin/uv",
      args: ["run", "--frozen", "/p/sync.py"],
    });
  });
});

describe("detectFromTask", () => {
  it("round trips with composeTaskCommand", () => {
    for (const interp of [python, uv, direct]) {
      const script = interp.kind === "direct" ? "/p/backup.sh" : "/p/sync.py";
      const args = ["--once", "-v"];
      const composed = composeTaskCommand(script, args, interp);
      const back = detectFromTask(composed.script_path, composed.args);
      expect(back).not.toBeNull();
      expect(back!.script).toBe(script);
      expect(back!.args).toEqual(args);
      expect(back!.interpreter.kind).toBe(interp.kind);
      expect(back!.interpreter.program).toBe(interp.program);
      expect(back!.interpreter.prefix_args).toEqual(interp.prefix_args);
      expect(back!.interp_args).toEqual([]);
    }
  });

  it("keeps the interpreter's own flags out of the script's arguments", () => {
    const back = detectFromTask("/usr/bin/python3", [
      "-u",
      "/p/sync.py",
      "--once",
    ]);
    expect(back!.interp_args).toEqual(["-u"]);
    expect(back!.script).toBe("/p/sync.py");
    expect(back!.args).toEqual(["--once"]);

    // `--` ends them; whatever follows is the script even if it looks like
    // a flag.
    const dashed = detectFromTask("/usr/bin/python3", [
      "-u",
      "--",
      "/p/-weird.py",
    ]);
    expect(dashed!.interp_args).toEqual(["-u", "--"]);
    expect(dashed!.script).toBe("/p/-weird.py");

    // And they survive a round trip through compose.
    const composed = composeTaskCommand(
      back!.script,
      back!.args,
      back!.interpreter,
      back!.interp_args,
    );
    expect(composed).toEqual({
      script_path: "/usr/bin/python3",
      args: ["-u", "/p/sync.py", "--once"],
    });
  });

  it("does not mistake a script named after an interpreter for one", () => {
    for (const name of ["/p/node.js", "/p/python.py", "/p/bash.sh"]) {
      const got = detectFromTask(name, ["--flag"]);
      expect(got?.interpreter.kind, name).toBe("direct");
      expect(got?.script, name).toBe(name);
      expect(got?.args, name).toEqual(["--flag"]);
    }
    // A real version suffix is still an interpreter.
    for (const name of ["/usr/bin/python3.13", "/usr/bin/ruby2.7"]) {
      const got = detectFromTask(name, ["/p/a"]);
      expect(got?.interpreter.kind, name).not.toBe("direct");
      expect(got?.script, name).toBe("/p/a");
    }
  });

  it("treats an unknown program as direct execution", () => {
    const got = detectFromTask("/usr/bin/true", ["x"]);
    expect(got?.interpreter.kind).toBe("direct");
    expect(got?.script).toBe("/usr/bin/true");
    expect(got?.args).toEqual(["x"]);
  });

  it("understands a versioned interpreter name", () => {
    const got = detectFromTask("/opt/homebrew/bin/python3.13", ["/p/a.py"]);
    expect(got?.interpreter.kind).toBe("python");
    expect(got?.script).toBe("/p/a.py");
    expect(got?.args).toEqual([]);
  });

  it("returns null when an interpreter has no script to run", () => {
    expect(detectFromTask("/usr/bin/python3", [])).toBeNull();
    expect(detectFromTask("/Users/x/.local/bin/uv", ["run"])).toBeNull();
    expect(detectFromTask("", [])).toBeNull();
  });
});

/**
 * TaskForm 在挂载和保存时做的那两步，压缩成一个函数：进来 `detectFromTask`
 * 拆开，出去 `composeTaskCommand` 拼回。打开一个已有任务却什么都不改，存下来的
 * 命令必须和原来一模一样——包括 detect 拆不开（返回 null）的那些，那时表单里
 * 「脚本」格放的是整条 script_path、运行方式留空，而运行方式留空就是"直接
 * 执行"，只有钉住选择（TaskForm 里按 `initial != null` 判断，不是
 * `initialSplit != null`）才不会被第一次扫描的推荐项顶掉。
 */
describe("opening an existing task and saving it unchanged", () => {
  const roundTrip = (scriptPath: string, args: string[]) => {
    const split = detectFromTask(scriptPath, args);
    return composeTaskCommand(
      split?.script ?? scriptPath,
      split ? split.args : args,
      split ? viewOf(split.interpreter, "zh-CN") : null,
      split?.interp_args ?? [],
    );
  };

  it("never rewrites the command", () => {
    const cases: [string, string[]][] = [
      // 拆得开的
      ["/usr/bin/python3", ["/p/sync.py", "--once"]],
      ["/usr/bin/python3", ["-u", "/p/sync.py"]],
      ["/Users/x/.local/bin/uv", ["run", "/p/sync.py"]],
      ["/p/backup.sh", ["-n"]],
      // 拆不开的：解释器后面根本没有脚本，detectFromTask 返回 null
      ["/usr/bin/python3", []],
      ["/Users/x/.local/bin/uv", ["run"]],
    ];
    for (const [scriptPath, args] of cases) {
      expect(roundTrip(scriptPath, args), scriptPath).toEqual({
        script_path: scriptPath,
        args,
      });
    }
  });
});

describe("labels", () => {
  it("names an interpreter the way the Rust side does", () => {
    expect(displayName(python, "zh-CN")).toBe("python3");
    expect(displayName(uv, "zh-CN")).toBe("uv run");
    // 只有「直接执行」这一条是本地化的文案，其余是程序自己的名字。
    expect(displayName(direct, "zh-CN")).toBe("直接执行");
    expect(displayName(direct, "en")).toBe("Run directly");
    expect(interpreterId(python)).toBe("/usr/bin/python3");
    expect(interpreterId(uv)).toBe("/Users/x/.local/bin/uv run");
    expect(basename("/a/b/c.sh")).toBe("c.sh");
    expect(basename("bare")).toBe("bare");
  });

  it("wraps a bare selection into a renderable row", () => {
    const view = viewOf(uv, "zh-CN");
    expect(view.id).toBe(interpreterId(uv));
    expect(view.label).toBe("uv run");
    expect(view.group).toBe("path");
    // A hand-typed path is matched against KNOWN, so it gets the right kind
    // and the prefix args that go with it.
    expect(customInterpreter("/opt/x/bin/python3", "zh-CN").kind).toBe("python");
    const customUv = customInterpreter("/opt/x/bin/uv", "zh-CN");
    expect(customUv.kind).toBe("uv");
    expect(customUv.prefix_args).toEqual(["run"]);
    expect(customUv.label).toBe("uv run");
    // Something we have never heard of stays custom.
    expect(customInterpreter("/opt/x/bin/frobnicate", "zh-CN").kind).toBe("custom");
  });
});

describe("groupInterpreters", () => {
  const row = (
    id: string,
    group: InterpreterView["group"],
  ): InterpreterView => ({
    ...viewOf({ kind: "python", program: id, prefix_args: [] }, "zh-CN"),
    group,
  });

  it("splits the list into the three sections of the design", () => {
    const groups = groupInterpreters([
      row("a", "recommended"),
      row("b", "project"),
      row("c", "path"),
      row("d", "path"),
    ]);
    expect(groups.recommended.map((i) => i.id)).toEqual(["a"]);
    expect(groups.project.map((i) => i.id)).toEqual(["b"]);
    expect(groups.path.map((i) => i.id)).toEqual(["c", "d"]);
  });
});

describe("localised interpreter labels", () => {
  const uv = {
    id: "uv", kind: "uv_run", program: "/Users/demo/.local/bin/uv", prefix_args: ["run"],
    version: "0.10.2", origin: "uv", label: "", detail: "", recommended: true, reason: null, group: "recommended",
  } as const;
  it("label and detail follow the locale", async () => {
    const { interpreterDetail, interpreterLabel } = await import("./interpreter");
    expect(interpreterLabel(uv as never, "zh-CN")).toBe("uv run · uv 安装");
    expect(interpreterLabel(uv as never, "en")).toBe("uv run · installed by uv");
    expect(interpreterDetail(uv as never, "en")).toBe("0.10.2 · installed by uv ~/.local/bin/uv");
  });
});
