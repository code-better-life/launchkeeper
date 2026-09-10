// @vitest-environment jsdom
//
// DOMPurify 需要一个真的 DOM 才能解析并遍历 HTML；没有 window 时它的
// `isSupported` 是 false，`sanitize` 会把输入原样交回来——那正是这里要防的
// 情况，所以这个文件单独跑在 jsdom 上（其余单测都是纯函数，仍在 node 上）。
import { describe, expect, it } from "vitest";
import { renderMarkdown } from "./markdown";

describe("renderMarkdown", () => {
  it("渲染普通 Markdown", () => {
    const html = renderMarkdown("# 标题\n\n- 一\n- 二\n\n`code`");
    expect(html).toContain("<h1>标题</h1>");
    expect(html).toContain("<li>一</li>");
    expect(html).toContain("<code>code</code>");
  });

  it("剥掉 <script>", () => {
    const html = renderMarkdown("正常一句\n\n<script>alert(1)</script>\n\n再一句");
    expect(html).not.toContain("<script");
    expect(html).not.toContain("alert(1)");
    expect(html).toContain("正常一句");
    expect(html).toContain("再一句");
  });

  it("剥掉事件处理器和 javascript: 链接", () => {
    const html = renderMarkdown(
      '<img src=x onerror="alert(1)">\n\n<a href="javascript:alert(1)">点我</a>',
    );
    expect(html).not.toContain("onerror");
    expect(html).not.toContain("javascript:");
    // 链接文字留着，只有危险的 href 被摘走。
    expect(html).toContain("点我");
  });

  it("围栏代码块里的标签不会被当成 HTML", () => {
    const html = renderMarkdown("```\n<script>alert(1)</script>\n```");
    expect(html).not.toContain("<script>");
    // 转义之后仍然看得见原文，解读里贴一段配置不该被吞掉。
    expect(html).toContain("&lt;script&gt;");
  });
});
