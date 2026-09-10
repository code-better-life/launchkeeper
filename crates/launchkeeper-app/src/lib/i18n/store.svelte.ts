// 当前语言（runes）。组件里的 `t()` 从这里取语言，所以切换语言会像改任何
// 别的 $state 一样重画整个界面——不需要重启，也不需要每个组件自己订阅。
//
// runes 不能作为普通模块级绑定跨文件保持响应性（导出的是值不是引用），所以
// 和 `lib/stores/tasks.svelte.ts` 一样用一个类把 $state 包起来，对外只暴露
// 单例的 getter。

import {
  resolveLocale,
  translate,
  type LanguageSetting,
  type Locale,
  type Params,
  type TranslationKey,
} from "./translate";

class LocaleStore {
  /** 正在渲染的语言。 */
  current = $state<Locale>(resolveLocale("system"));
  /** 设置里存的那个值；`system` 时 `current` 由系统语言决定。 */
  setting = $state<LanguageSetting>("system");
}

/** 当前语言。读 `locale.current` 会建立响应式依赖。 */
export const locale = new LocaleStore();

/**
 * 切到某个设置值（`system` / `zh-CN` / `en`）并立刻生效。
 *
 * 同时更新 `<html lang>`：拼写检查、朗读、`:lang()` 选择器和浏览器的断行规则
 * 都看它，而 index.html 里写死的 `zh-CN` 在切到英文之后就是错的。
 */
export function setLocale(setting: LanguageSetting) {
  locale.setting = setting;
  locale.current = resolveLocale(setting);
  if (typeof document !== "undefined") {
    document.documentElement.lang = locale.current;
  }
}

/** 当前语言下的一条文案。 */
export function t(key: TranslationKey, params?: Params): string {
  return translate(locale.current, key, params);
}

export type { LanguageSetting, Locale, Params, TranslationKey };
