// 界面语言的对外入口（PRD M3 项 5，docs/M3-design.md §5）。
//
// 组件只从这里 import：`t` 和 `locale` 来自带 runes 的 `store.svelte.ts`
// （文件名必须以 `.svelte.ts` 结尾，编译器才会把里面的 $state 当 rune 处理），
// 其余是 `translate.ts` 里的纯函数。分成两个文件而不是一个，是为了让
// `lib/format.ts`、`lib/external.ts` 和它们的单测能只依赖纯的那一半。

export { locale, setLocale, t } from "./store.svelte";
export {
  FALLBACK_LOCALE,
  LANGUAGE_SETTINGS,
  interpolate,
  languageSetting,
  plural,
  resolveLocale,
  translate,
} from "./translate";
export type {
  LanguageSetting,
  Locale,
  Params,
  TranslationKey,
} from "./translate";
export { zhCN } from "./zh-CN";
export { en } from "./en";
