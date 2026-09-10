import { describe, expect, it } from "vitest";
import { THEME_ATTR, THEME_SETTINGS, applyTheme, themeSetting } from "./theme";

/** `applyTheme` 需要的最小 `<html>`：记下当前属性值，方便断言。 */
function fakeRoot() {
  let value: string | null = null;
  return {
    get value() {
      return value;
    },
    setAttribute(name: string, v: string) {
      expect(name).toBe(THEME_ATTR);
      value = v;
    },
    removeAttribute(name: string) {
      expect(name).toBe(THEME_ATTR);
      value = null;
    },
  };
}

describe("themeSetting", () => {
  it("认得三个合法值", () => {
    expect(themeSetting("light")).toBe("light");
    expect(themeSetting("dark")).toBe("dark");
    expect(themeSetting("system")).toBe("system");
  });

  it("缺失和不认识的值都退回跟随系统", () => {
    expect(themeSetting(null)).toBe("system");
    expect(themeSetting(undefined)).toBe("system");
    expect(themeSetting("")).toBe("system");
    expect(themeSetting("Dark")).toBe("system");
    expect(themeSetting("solarized")).toBe("system");
  });
});

describe("applyTheme", () => {
  it("浅色和深色写属性", () => {
    const root = fakeRoot();
    applyTheme("dark", root);
    expect(root.value).toBe("dark");
    applyTheme("light", root);
    expect(root.value).toBe("light");
  });

  it("跟随系统删掉属性，而不是写 system", () => {
    const root = fakeRoot();
    applyTheme("dark", root);
    applyTheme("system", root);
    // CSS 里的守卫是 `:not([data-theme="light"])`，属性必须真的不存在。
    expect(root.value).toBeNull();
  });

  it("从跟随系统开始也不会留下属性", () => {
    const root = fakeRoot();
    applyTheme("system", root);
    expect(root.value).toBeNull();
  });
});

describe("THEME_SETTINGS", () => {
  it("是分段控件的三格，顺序固定", () => {
    expect(THEME_SETTINGS).toEqual(["system", "light", "dark"]);
    // 每一格都能被 themeSetting 原样认回来。
    for (const s of THEME_SETTINGS) expect(themeSetting(s)).toBe(s);
  });
});
