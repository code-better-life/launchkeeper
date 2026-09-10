<script lang="ts">
  // 一个「?」小圆点，鼠标停上去（或键盘聚焦）显示一段会折行的说明。
  //
  // 为什么不只用原生 `title`：这里要说的那句话（不可接管的完整理由）经常有
  // 三四十个字，macOS 的 tooltip 一行放不下就直接截断，而"被截断的解释"正是
  // 这个控件要解决的问题本身。所以两个都给——`title` 让不看屏幕的人和不 hover
  // 的人也拿得到，自己画的这个负责把长句折行显示完整。
  //
  // 是个 `<button>`（而不是带 tabindex 的 span）：这样它自带焦点、回车/空格
  // 不会触发任何事，也不用手写 role。点击只做一件事——别让事件冒到外面那一行
  // 去（列表行整行可点，点一下说明就把那一行选中了会很怪）。

  let {
    text,
    label,
  }: {
    /** 完整说明。同时进 `title` 和自画的 tooltip。 */
    text: string;
    /** 屏幕阅读器读到的名字，例如「为什么不能接管」。 */
    label: string;
  } = $props();
</script>

<button
  type="button"
  class="help"
  title={text}
  aria-label={label}
  onclick={(e) => e.stopPropagation()}
>
  <span class="mark" aria-hidden="true">?</span>
  <span class="tip">{text}</span>
</button>

<style>
  .help {
    position: relative;
    flex-shrink: 0;
    width: 16px;
    height: 16px;
    padding: 0;
    border-radius: 50%;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--muted);
    font-size: 11px;
    line-height: 1;
    cursor: default;
  }
  .help:hover,
  .help:focus-visible {
    color: var(--fg);
    border-color: var(--muted);
  }
  .mark {
    display: block;
  }
  /* 折行的那一版说明。默认收着（`visibility` 而不是 `display`，这样它一直
     参与布局计算，不会在第一次 hover 时闪一下）。 */
  .tip {
    visibility: hidden;
    opacity: 0;
    position: absolute;
    z-index: 40;
    /* 右对齐：这个点总在 320 px 侧栏的右边，往左展开才不会被切掉。 */
    top: calc(100% + 6px);
    right: 0;
    width: max-content;
    max-width: 230px;
    padding: 6px 8px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--fg);
    font-size: 12px;
    line-height: 1.45;
    text-align: left;
    white-space: normal;
    word-break: break-word;
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.28);
    transition: opacity 100ms ease;
  }
  .help:hover .tip,
  .help:focus-visible .tip {
    visibility: visible;
    opacity: 1;
  }
</style>
