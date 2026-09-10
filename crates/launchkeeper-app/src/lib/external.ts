// 「其他 LaunchAgents」这一组的纯展示函数（docs/M3-design.md §3.4）。
//
// 后端已经把能算的字符串都算好了（command / trigger_text / adopt_reason），
// 这里只负责把它们拼成界面稿上的那几行，以及把 AdoptionPlan 的三组 diff 排成
// 确认框里那个等宽框的行。没有副作用，全部单测。
//
// 和 `lib/format.ts` 一样收一个显式的 `locale`（M3 §5）。注意后端给的那几个
// 字符串（`trigger_text`、`command`、`adopt_reason`）本期仍然只有中文，这里
// 只翻译自己拼的那部分。

import type { AdoptionPlanView, ExternalAgentView } from "./bindings";
import { translate, type Locale } from "./i18n/translate";

/**
 * 列表行的第二行。
 *
 * 可接管的写"手写 plist · 无运行历史"——这是接管的理由本身：它现在没有历史，
 * 接管之后就有了。不可接管的写理由 + "仅展示"，和灰掉的行、tooltip 一致。
 */
export function externalSubtitle(a: ExternalAgentView, locale: Locale): string {
  if (a.managed) return translate(locale, "external.subtitle.managed");
  if (!a.adoptable) {
    return translate(locale, "external.subtitle.not_adoptable", {
      reason: a.adopt_reason ?? translate(locale, "external.reason.fallback"),
    });
  }
  return translate(locale, "external.subtitle.adoptable");
}

/** 状态点的 tooltip：加载了没有，跑着没有。 */
export function externalStatusTitle(
  a: ExternalAgentView,
  locale: Locale,
): string {
  if (!a.loaded) return translate(locale, "external.status.unloaded");
  if (a.pid !== null) {
    return translate(locale, "external.status.running", { pid: a.pid });
  }
  return translate(locale, "external.status.loaded");
}

/** 详情页头部第二行：plist 路径 · 手写 plist · 已加载/未加载。 */
export function externalHeadline(a: ExternalAgentView, locale: Locale): string {
  const parts = [
    a.path,
    translate(
      locale,
      a.managed ? "external.headline.managed" : "external.headline.handwritten",
    ),
  ];
  parts.push(
    a.loaded
      ? a.pid !== null
        ? translate(locale, "external.status.running", { pid: a.pid })
        : translate(locale, "external.headline.loaded")
      : translate(locale, "external.headline.unloaded"),
  );
  return parts.join(" · ");
}

/**
 * 详情页「日志」那一格。
 *
 * 两个流指到同一个文件时写「（合并）」——launchd 里这是最常见的写法，分两行
 * 报同一个路径只会让人以为有两个文件。一个都没有就直说：launchd 会把输出丢掉。
 */
export function externalLogText(a: ExternalAgentView, locale: Locale): string {
  const { stdout_path: out, stderr_path: err } = a;
  if (out && err && out === err) {
    return translate(locale, "external.log.merged", { path: out });
  }
  if (out && err) return translate(locale, "external.log.split", { out, err });
  if (out) return out;
  if (err) return translate(locale, "external.log.err_only", { err });
  return translate(locale, "external.log.none");
}

/** 接管按钮/菜单项的 tooltip：能接管就说做什么，不能就说为什么不能。 */
export function adoptTooltip(a: ExternalAgentView, locale: Locale): string {
  if (a.managed) return translate(locale, "external.adopt.managed_tip");
  if (!a.adoptable) {
    return a.adopt_reason ?? translate(locale, "external.reason.fallback");
  }
  return translate(locale, "external.adopt.tip");
}

/** 确认框里等宽框的一行。 */
export interface DiffLine {
  /** 界面稿上的三个记号。 */
  sign: "−" | "+" | "=";
  /** 行文本。 */
  text: string;
  /** `=` 那一行是灰的。 */
  muted: boolean;
}

/**
 * AdoptionPlan → 确认框里的 diff 行。
 *
 * 删掉的和加上的一条一行（用户要看清楚自己的 ProgramArguments 被换成了什么），
 * 保持不变的收成一行「A、B 保持不变」：那些键正是"没有变化"的证据，一条一行
 * 反而会把真正的改动淹掉。Label 不在这行里单独强调——它本来就不变，标题上
 * 已经写着了。
 */
export function diffLines(plan: AdoptionPlanView, locale: Locale): DiffLine[] {
  const lines: DiffLine[] = [];
  for (const e of plan.removed) {
    lines.push({ sign: "−", text: `${e.key}: ${e.value}`, muted: false });
  }
  for (const e of plan.added) {
    lines.push({ sign: "+", text: `${e.key}: ${e.value}`, muted: false });
  }
  if (plan.kept.length > 0) {
    const keys = plan.kept
      .map((e) => e.key)
      .join(translate(locale, "external.diff.join"));
    lines.push({
      sign: "=",
      text: translate(locale, "external.diff.kept", { keys }),
      muted: true,
    });
  }
  return lines;
}

/**
 * 搜索框对「其他 LaunchAgents」的过滤。
 *
 * 外部 agent 没有标签，所以只要用户选了标签，这一组就整体让位——列表这时问的是
 * "带这个标签的任务有哪些"，一组答不上来的行留在那儿只是噪音。
 */
export function filterExternal(
  list: ExternalAgentView[],
  search: string,
  selectedTags: string[],
): ExternalAgentView[] {
  if (selectedTags.length > 0) return [];
  const q = search.trim().toLowerCase();
  if (!q) return list;
  return list.filter((a) =>
    `${a.label} ${a.path} ${a.command}`.toLowerCase().includes(q),
  );
}

// ---- 「其他 LaunchAgents」的分组与折叠（M3 §3.4 追补）------------------

/** 这一组分成的两堆：能接管的，和只能看的。 */
export interface ExternalGroups {
  /** 可以接管的（也就是这个列表存在的理由），按 label 排序。 */
  adoptable: ExternalAgentView[];
  /** 不支持接管的（第三方 App 装的、已经归 Launchkeeper 管的），按 label 排序。 */
  others: ExternalAgentView[];
}

/**
 * 排序 + 分组。
 *
 * 可接管的排在前面：一台机器上 `~/Library/LaunchAgents` 里躺着的多半是各种
 * App 装的更新器，用户来这一组是为了找自己手写的那几个 plist，让那几个排在
 * 十几行灰字后面等于没有这一组。组内按 label 字母序，好从一堆
 * `com.<厂商>.<东西>` 里扫到自己要的那个。
 *
 * 已接管的（`managed`）归到"不支持接管"那一堆：它已经在上面那一组里以任务的
 * 形式出现了，这里再摆一行只是提醒它的 plist 还在原处。
 */
export function groupExternal(list: ExternalAgentView[]): ExternalGroups {
  const byLabel = (a: ExternalAgentView, b: ExternalAgentView) =>
    a.label.localeCompare(b.label, "en");
  const adoptable = list.filter((a) => a.adoptable && !a.managed).sort(byLabel);
  const others = list.filter((a) => !(a.adoptable && !a.managed)).sort(byLabel);
  return { adoptable, others };
}

/**
 * 搜索命中的全在折叠着的那一堆里时，自动展开。
 *
 * 否则搜一个第三方 agent 的名字得到的是一个空列表加一行「还有 3 个不支持
 * 接管」——搜索说"没找到"，而东西其实就在那行后面。
 */
export function shouldForceExpand(
  search: string,
  groups: ExternalGroups,
): boolean {
  if (!search.trim()) return false;
  return groups.adoptable.length === 0 && groups.others.length > 0;
}

/** 折叠状态记在 localStorage 里的键。 */
export const EXTERNAL_EXPANDED_KEY = "launchkeeper.externalOthersExpanded";

/**
 * 上次是展开还是收起，默认收起。
 *
 * localStorage 在 webview 里可能整个不可用（隐私模式、被策略关掉），读写
 * 一律 try/catch：记不住一个折叠状态是小事，为它抛异常把列表画不出来是大事。
 */
export function readExpanded(): boolean {
  try {
    return localStorage.getItem(EXTERNAL_EXPANDED_KEY) === "1";
  } catch {
    return false;
  }
}

export function writeExpanded(expanded: boolean) {
  try {
    localStorage.setItem(EXTERNAL_EXPANDED_KEY, expanded ? "1" : "0");
  } catch {
    // 记不住就算了。
  }
}

/**
 * 这个 agent 是不是别的软件自己装的（M4）。
 *
 * 判据借的是「能不能接管」：拒绝接管的那些理由——label 是 Apple / Homebrew 的
 * 前缀、程序在某个 .app 里、WatchPaths / MachServices——正好也是「这个 plist
 * 是某个产品的安装程序放进来的，不是用户手写的」的判据。Launchkeeper 自己写的
 * 那些（`managed`）不算：它们不可接管是另一个理由，而且本来就是我们的。
 */
export function isForeignAgent(a: ExternalAgentView): boolean {
  return !a.adoptable && !a.managed;
}

/** 禁用一个别人的 LaunchAgent 之前，确认框里的那段警告（M4）。 */
export function foreignDisableWarning(a: ExternalAgentView, locale: Locale): string {
  return translate(locale, "confirm.external_disable.message", {
    label: a.label,
    reason: a.adopt_reason ?? translate(locale, "external.reason.fallback"),
  });
}
