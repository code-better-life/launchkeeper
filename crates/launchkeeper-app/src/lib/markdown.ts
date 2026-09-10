// 模型答复（Markdown）→ 可以塞进 `{@html}` 的 HTML（docs/M4-design.md §4）。
//
// 两步，缺一不可：
//
// 1. `marked` 把 Markdown 渲染成 HTML。
// 2. `DOMPurify` 把结果洗一遍。
//
// 第二步不是形式主义：`content` 是一个**远端模型**写的字符串，Markdown 本来
// 就允许内联 HTML，`marked` 会原样放行。前端把它交给 `{@html}` 的那一刻，
// 一段 `<script>` 或者一个 `<img onerror=…>` 就在应用自己的 origin 里跑起来了，
// 而这个 origin 里有整套 Tauri IPC。所以渲染路径只有这一个函数，测试盯着它。

import DOMPurify from "dompurify";
import { marked } from "marked";

/**
 * 渲染一份解读。
 *
 * `async: false` 让 `marked.parse` 的返回类型确定是 `string`（它默认可能返回
 * Promise，取决于有没有异步扩展；这里一个扩展都没装）。`breaks: true` 是因为
 * 模型爱写单换行的列表说明，按 CommonMark 那样把它们并成一段读起来是糊的。
 */
export function renderMarkdown(source: string): string {
  const html = marked.parse(source, { async: false, gfm: true, breaks: true });
  return DOMPurify.sanitize(html, { USE_PROFILES: { html: true } });
}
