// 项目链接集中在一处，改账号只改这里。
// `<TODO-handle>` 是待填的 Buy Me a Coffee 账号；README 里用的是同一个占位符。
export const LINKS = {
  repo: "https://github.com/code-better-life/launchkeeper",
  issues: "https://github.com/code-better-life/launchkeeper/issues",
  coffee: "https://buymeacoffee.com/<TODO-handle>",
} as const;

/** 占位符没换成真账号之前不显示请喝咖啡按钮，免得点出去 404。 */
export function coffeeReady(url: string = LINKS.coffee): boolean {
  return !url.includes("<TODO-handle>");
}
