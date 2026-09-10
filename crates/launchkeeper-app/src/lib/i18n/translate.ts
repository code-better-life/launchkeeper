// 词典查表、插值和语言解析。纯函数，不含 runes——`lib/format.ts`、
// `lib/external.ts` 这些纯展示模块直接用它（收一个显式的 `locale` 参数），
// 组件用 `store.svelte.ts` 里那个跟着当前语言走的 `t()`。
//
// 没有引入 svelte-i18n 之类的库：全部界面文案 150 条上下，需要的只有"查表 +
// {name} 替换 + 缺键回退"，一个库换来的是一份运行时、一套 store 约定和一个
// 异步加载模型，都不值当（docs/M3-design.md §5）。

import { zhCN, type TranslationKey } from "./zh-CN";
import { en } from "./en";

/** 实际渲染出来的语言。 */
export type Locale = "zh-CN" | "en";

/** 设置里存的那个值。`system` = 跟随系统语言。 */
export type LanguageSetting = "system" | "zh-CN" | "en";

/** 插值参数：`t("row.pid", { pid: 42 })` -> `pid 42`。 */
export type Params = Record<string, string | number>;

export type { TranslationKey };

const DICTS: Record<Locale, Record<TranslationKey, string>> = {
  "zh-CN": zhCN,
  en,
};

/** 缺键回退的目标语言，也是解析不出系统语言时的默认值。 */
export const FALLBACK_LOCALE: Locale = "en";

/** 三个可选设置，`Settings.svelte` 的分段控件按这个顺序渲染。 */
export const LANGUAGE_SETTINGS: LanguageSetting[] = ["system", "zh-CN", "en"];

const warned = new Set<string>();

/**
 * 缺键只在开发构建里喊一次。
 *
 * 一个键在渲染里可能每秒被读几十遍（列表每 2 秒重画一次），每次都打一行会把
 * 控制台冲成日志；生产构建里更没必要——那时候能做的只有显示回退语言的文案。
 */
function warnMissing(locale: Locale, key: string) {
  const id = `${locale}:${key}`;
  if (warned.has(id)) return;
  warned.add(id);
  if (import.meta.env?.DEV) {
    console.warn(`[i18n] ${locale} 缺少键 "${key}"，回退到 ${FALLBACK_LOCALE}`);
  }
}

/** 只在测试里用：把"喊过一次了"的记录清掉。 */
export function resetMissingKeyWarnings() {
  warned.clear();
}

/** `{name}` 替换。词典里没写的占位符原样留着，方便一眼看出是哪条文案漏了参数。 */
export function interpolate(template: string, params?: Params): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (whole, name: string) =>
    name in params ? String(params[name]) : whole,
  );
}

/**
 * 查表 + 插值。缺键回退到英文，英文也没有就把键本身还回去——界面上出现一个
 * `list.group.managed` 很难看，但比一片空白更容易被发现和修。
 */
export function translate(
  locale: Locale,
  key: TranslationKey,
  params?: Params,
): string {
  const template = DICTS[locale]?.[key];
  if (template === undefined) {
    warnMissing(locale, key);
    const fallback = DICTS[FALLBACK_LOCALE][key];
    return fallback === undefined ? key : interpolate(fallback, params);
  }
  return interpolate(template, params);
}

/** 英文单复数：`n === 1` 走 `<base>_one`，其余走 `<base>_other`。 */
export function plural(
  locale: Locale,
  base: string,
  n: number,
  params?: Params,
): string {
  const key = `${base}_${n === 1 ? "one" : "other"}` as TranslationKey;
  return translate(locale, key, { n, ...params });
}

/**
 * 设置值 -> 真正要渲染的语言。
 *
 * `system` 看浏览器（= 这个 webview）报的语言标签，`zh` 打头算中文，其余一律
 * 英文——目前只有这两份词典，把 `fr-FR` 读成中文比读成英文更让人摸不着头脑。
 */
export function resolveLocale(setting: LanguageSetting, navLang?: string): Locale {
  if (setting === "zh-CN" || setting === "en") return setting;
  const tag = (
    navLang ??
    (typeof navigator === "undefined" ? "" : navigator.language)
  ).toLowerCase();
  return tag.startsWith("zh") ? "zh-CN" : FALLBACK_LOCALE;
}

/**
 * `AppSettings.language`（后端来的 `Option<String>`）-> 设置值。
 *
 * `null` / 缺字段 / 不认识的值都算"跟随系统"：settings.json 是可以手改的，
 * 一个手滑写成 `"zh"` 的文件不该让界面变成一片键名。
 */
export function languageSetting(
  raw: string | null | undefined,
): LanguageSetting {
  return raw === "zh-CN" || raw === "en" ? raw : "system";
}
