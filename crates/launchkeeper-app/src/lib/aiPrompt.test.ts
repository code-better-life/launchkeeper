import { describe, expect, it } from "vitest";
import { aiPrompt } from "./aiPrompt";

describe("aiPrompt", () => {
  it("两种语言都先让助手去读 launchkeeper docs", () => {
    // 这一句是整段提示词存在的理由：CLI 参考不进 prompt，助手自己去读。
    for (const locale of ["zh-CN", "en"] as const) {
      expect(aiPrompt(locale)).toContain("launchkeeper docs");
    }
  });

  it("中文版是中文，英文版不含中文", () => {
    const zh = aiPrompt("zh-CN");
    const en = aiPrompt("en");
    expect(zh).toContain("Launchkeeper");
    expect(zh).toMatch(/[一-鿿]/);
    expect(en).not.toMatch(/[一-鿿]/);
    expect(zh).not.toBe(en);
  });

  it("两种语言都说到 --json、add 和确认破坏性操作", () => {
    for (const locale of ["zh-CN", "en"] as const) {
      const text = aiPrompt(locale);
      expect(text).toContain("--json");
      expect(text).toContain("launchkeeper add");
      expect(text).toContain("launchkeeper show <name> --json");
      expect(text).toContain("launchkeeper agents");
      // 破坏性操作那一条：三个子命令都点名。
      expect(text).toContain("rm");
      expect(text).toContain("unadopt");
    }
  });

  it("是多行文本，不是一句话", () => {
    expect(aiPrompt("zh-CN").split("\n").length).toBeGreaterThan(5);
    expect(aiPrompt("en").split("\n").length).toBeGreaterThan(5);
  });
});
