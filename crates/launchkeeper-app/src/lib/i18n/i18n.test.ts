// 两件事只有测试能盯住：两份词典的键必须一模一样，以及 .svelte 里不许再出现
// 写死的中文（M3 §5）。类型上 `en.ts` 已经被声明成 `Record<TranslationKey,
// string>`，但那只在 `pnpm build`（svelte-check）里生效；这里再从运行时确认
// 一遍，`pnpm test` 单跑也拦得住。

import { describe, expect, it } from "vitest";

import { en } from "./en";
import { zhCN } from "./zh-CN";
import {
  interpolate,
  languageSetting,
  plural,
  resolveLocale,
  translate,
  type TranslationKey,
} from "./translate";

describe("dictionaries", () => {
  it("have exactly the same keys", () => {
    const zhKeys = Object.keys(zhCN).sort();
    const enKeys = Object.keys(en).sort();
    // 分开断言，报错时直接说出是哪几个键，而不是甩一份 150 行的 diff。
    expect(enKeys.filter((k) => !(k in zhCN))).toEqual([]);
    expect(zhKeys.filter((k) => !(k in en))).toEqual([]);
    expect(enKeys).toEqual(zhKeys);
  });

  it("has no empty Chinese string", () => {
    // 英文那边 `trigger.run_once` 是**故意**空的（中文把"运行一次"放在句尾，
    // 英文放在句首），所以只对中文这一份要求非空。
    const empty = Object.entries(zhCN)
      .filter(([, v]) => v.trim() === "")
      .map(([k]) => k);
    expect(empty).toEqual([]);
  });

  it("keeps the {placeholders} of a key identical in both languages", () => {
    const holders = (s: string) =>
      (s.match(/\{(\w+)\}/g) ?? []).sort().join(",");
    const mismatched = (Object.keys(zhCN) as TranslationKey[]).filter(
      (k) => holders(zhCN[k]) !== holders(en[k]),
    );
    expect(mismatched).toEqual([]);
  });
});

describe("translate", () => {
  it("interpolates named parameters", () => {
    expect(translate("zh-CN", "row.pid", { pid: 42 })).toBe("pid 42");
    expect(translate("en", "confirm.delete.message", { name: "sync" })).toContain(
      "“sync”",
    );
  });

  it("leaves an unknown placeholder in place instead of printing undefined", () => {
    expect(interpolate("a {x} b", {})).toBe("a {x} b");
  });

  it("falls back to English for a key the locale is missing", () => {
    // 词典是类型安全的，所以这条只能靠临时抠掉一个键来验：缺键回退到英文，
    // 两边都没有才把键本身还回去（界面上难看，但一眼看得出是哪条漏了）。
    const dict = zhCN as unknown as Record<string, string>;
    const key = "fmt.exit.success";
    const saved = dict[key];
    delete dict[key];
    try {
      expect(translate("zh-CN", key as TranslationKey)).toBe("Succeeded");
      expect(translate("zh-CN", "nope.not.a.key" as TranslationKey)).toBe(
        "nope.not.a.key",
      );
    } finally {
      dict[key] = saved;
    }
  });

  it("picks the singular form only for exactly one", () => {
    expect(plural("en", "fmt.rel.hour", 1)).toBe("1 hour ago");
    expect(plural("en", "fmt.rel.hour", 2)).toBe("2 hours ago");
    expect(plural("zh-CN", "fmt.rel.hour", 1)).toBe("1 小时前");
  });
});

describe("resolveLocale", () => {
  it("returns an explicit choice unchanged", () => {
    expect(resolveLocale("zh-CN", "en-US")).toBe("zh-CN");
    expect(resolveLocale("en", "zh-Hans-CN")).toBe("en");
  });

  it("follows the system language for 'system'", () => {
    expect(resolveLocale("system", "zh-Hans-CN")).toBe("zh-CN");
    expect(resolveLocale("system", "zh")).toBe("zh-CN");
    expect(resolveLocale("system", "en-GB")).toBe("en");
    // 没有词典的语言落到英文，而不是落到中文。
    expect(resolveLocale("system", "fr-FR")).toBe("en");
    expect(resolveLocale("system", "")).toBe("en");
  });
});

describe("languageSetting", () => {
  it("treats null, a missing field and garbage as 'follow system'", () => {
    expect(languageSetting(null)).toBe("system");
    expect(languageSetting(undefined)).toBe("system");
    expect(languageSetting("system")).toBe("system");
    expect(languageSetting("zh")).toBe("system");
    expect(languageSetting("zh-CN")).toBe("zh-CN");
    expect(languageSetting("en")).toBe("en");
  });
});

/**
 * 新写死的中文进不了 .svelte（M3 §5）。
 *
 * 注释不算——把每一行说明为什么这么写的中文注释也翻成英文，代价远大于收益，
 * 所以扫描之前先把 `<!-- -->`、`/* *\/` 和 `//` 三种注释去掉。真的需要在
 * 标记里写一个中文字面量（比如某个键的语言名）时，在那一行写
 * `i18n-ignore` 放行。
 */
describe("no hard-coded CJK in components", () => {
  // 用 Vite 自己的 glob 读源码，不走 node:fs：这个包没装 @types/node，
  // svelte-check 会因为 `import "node:fs"` 报错，而为了一个 lint 装一份类型
  // 声明不值当。`?raw` 拿到的是文件原文，编译前的样子。
  const sources = import.meta.glob("/src/**/*.svelte", {
    query: "?raw",
    import: "default",
    eager: true,
  }) as Record<string, string>;

  // CJK 统一表意文字 + 中日韩符号标点（、。「」）+ 全角字母数字标点（（）：）。
  const CJK = /[\u3000-\u303f\u3400-\u4dbf\u4e00-\u9fff\uff00-\uffef]/;

  /** 去掉注释；行号靠把每段注释换成等量的换行保住。 */
  function stripComments(src: string): string {
    const keepNewlines = (m: string) => m.replace(/[^\n]/g, "");
    return src
      .replace(/<!--[\s\S]*?-->/g, keepNewlines)
      .replace(/\/\*[\s\S]*?\*\//g, keepNewlines)
      .replace(/\/\/[^\n]*/g, "");
  }

  it("finds at least the components we know are there", () => {
    expect(Object.keys(sources).length).toBeGreaterThan(10);
  });

  it("has no Chinese left outside lib/i18n", () => {
    const offenders: string[] = [];
    for (const [file, src] of Object.entries(sources)) {
      const lines = stripComments(src).split("\n");
      lines.forEach((line, i) => {
        if (!CJK.test(line)) return;
        if (line.includes("i18n-ignore")) return;
        offenders.push(`${file}:${i + 1}: ${line.trim()}`);
      });
    }
    expect(offenders).toEqual([]);
  });
});
