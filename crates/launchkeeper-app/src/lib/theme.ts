// 外观（浅色 / 深色 / 跟随系统，docs/M4-design.md §4）。
//
// 一共两件事，分成两个函数是为了让第一件能单测：
//
// 1. 在 `<html>` 上盖一个 `data-theme`（跟随系统时**删掉**它，而不是写
//    `data-theme="system"`）。`app.css` 里的深色 token 写了三遍——媒体查询
//    里一份、`[data-theme="dark"]` 一份、媒体查询那份还带
//    `:not([data-theme="light"])` 的守卫——所以这一个属性就能在两个方向上
//    都压过系统设置。
// 2. 告诉 Tauri 窗口用哪套主题，否则标题栏还是系统那一套，跟正文对不上。
//    这一步在浏览器（`VITE_MOCK=1` 的界面开发、vitest）里没有对应物，
//    所以要守卫，而且失败也只是标题栏不跟着变，不该把设置面板的保存
//    连累成一个红 toast。

/** 设置里存的那个值，也是 `AppSettings.theme` 的三个合法取值。 */
export type ThemeSetting = "system" | "light" | "dark";

/** 分段控件按这个顺序渲染，和 `LANGUAGE_SETTINGS` 一样。 */
export const THEME_SETTINGS: ThemeSetting[] = ["system", "light", "dark"];

/** `<html>` 上那个属性的名字。 */
export const THEME_ATTR = "data-theme";

/**
 * 把 `AppSettings.theme` 里的字符串收敛成三个合法值之一。
 *
 * 手改坏的 settings.json、或者以后新增了一个这个版本还不认识的取值，都退回
 * 「跟随系统」——和 `languageSetting` / `sortMode` 同一条规矩。
 */
export function themeSetting(raw: string | null | undefined): ThemeSetting {
  return raw === "light" || raw === "dark" ? raw : "system";
}

/** `applyTheme` 只用到 `<html>` 的这两个方法，测试因此可以喂一个假对象。 */
export interface ThemeRoot {
  setAttribute(name: string, value: string): void;
  removeAttribute(name: string): void;
}

/**
 * 纯 DOM 那一半：盖属性 / 删属性，不联网、不碰 Tauri。
 *
 * `system` 走 `removeAttribute` 而不是写一个 `data-theme="system"`：CSS 那边
 * 的媒体查询守卫是 `:not([data-theme="light"])`，属性不存在时它自然生效，
 * 多一个取值就要多一条守卫。
 */
export function applyTheme(
  setting: ThemeSetting,
  root: ThemeRoot = document.documentElement,
): void {
  if (setting === "system") root.removeAttribute(THEME_ATTR);
  else root.setAttribute(THEME_ATTR, setting);
}

/**
 * 让 Tauri 窗口（标题栏、原生控件）跟上。
 *
 * 不在 Tauri 里跑（浏览器里的 `VITE_MOCK=1`、vitest）就直接返回：
 * `@tauri-apps/api` 在浏览器里 import 得进来，但 `invoke` 会失败，所以判断
 * 的是 `__TAURI_INTERNALS__` 在不在，而不是 try 一下看看。动态 import 让
 * 这个模块在没有 DOM 的单测里也能加载。
 */
export async function syncWindowTheme(setting: ThemeSetting): Promise<void> {
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) return;
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    // `null` = 跟随系统，正是这个 API 自己的约定。
    await getCurrentWindow().setTheme(setting === "system" ? null : setting);
  } catch (e) {
    // 标题栏没跟上不值得打断用户手上的事，留一行控制台就够。
    console.warn("[theme] 设置窗口主题失败", e);
  }
}

/** 两件事一起做：界面立刻变，标题栏在后台跟上。 */
export function setTheme(setting: ThemeSetting): void {
  applyTheme(setting);
  void syncWindowTheme(setting);
}
