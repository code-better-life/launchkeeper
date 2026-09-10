<script lang="ts">
  import { taskStore } from "../lib/stores/tasks.svelte";
  import { t } from "../lib/i18n";

  let { oncreate, onsettings }: { oncreate: () => void; onsettings: () => void } = $props();

  function toggleTag(tag: string) {
    taskStore.selectedTags = taskStore.selectedTags.includes(tag)
      ? taskStore.selectedTags.filter((t) => t !== tag)
      : [...taskStore.selectedTags, tag];
  }
</script>

<div class="topbar">
  <div class="row">
    <input
      class="search"
      type="text"
      placeholder={t("topbar.search_placeholder")}
      bind:value={taskStore.search}
    />
    <div class="spacer"></div>
    <button
      class="btn"
      onclick={() => void taskStore.refreshAll()}
      title={t("topbar.refresh_title")}
    >
      {t("common.refresh")}
    </button>
    <button class="btn primary" onclick={oncreate}>{t("topbar.new_task")}</button>
    <button class="btn icon" onclick={onsettings} title={t("topbar.settings")} aria-label={t("topbar.settings")}>
      <!-- 齿轮画成 SVG，不用 ⚙ 字形：那个字符的实际大小由系统 emoji 字体
           决定，在 14px 的按钮里小得像个句号，而且颜色不跟着 currentColor 走。
           这里是描边图形，线宽和颜色都随按钮。轮廓那条 path 是八颗齿的多边形
           （半径 10.1 / 6.9，齿宽 20°），拐角 round 之后看着就是齿，不是太阳
           ——中间再加一个孔，跟"发光"彻底分开。 -->
      <svg
        class="gear"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.5"
        stroke-linejoin="round"
        stroke-linecap="round"
        aria-hidden="true"
        focusable="false"
      >
        <path
          d="M10 5.4L10.2 2.1L13.8 2.1L14 5.4L15.2 5.9L17.8 3.7L20.3 6.2L18.1 8.8L18.6 10L21.9 10.2L21.9 13.8L18.6 14L18.1 15.2L20.3 17.8L17.8 20.3L15.2 18.1L14 18.6L13.8 21.9L10.2 21.9L10 18.6L8.8 18.1L6.2 20.3L3.7 17.8L5.9 15.2L5.4 14L2.1 13.8L2.1 10.2L5.4 10L5.9 8.8L3.7 6.2L6.2 3.7L8.8 5.9Z"
        />
        <circle cx="12" cy="12" r="3.2" />
      </svg>
    </button>
  </div>
  {#if taskStore.allTags.length > 0}
    <div class="tags">
      {#each taskStore.allTags as tag (tag)}
        <button
          class="chip"
          class:active={taskStore.selectedTags.includes(tag)}
          onclick={() => toggleTag(tag)}
        >
          {tag}
          <!-- 各标签的任务数（M4 §3）。跟着搜索走，不跟着标签筛选走，
               理由见 lib/counts.ts。 -->
          <span class="n">{taskStore.tagCounts.get(tag) ?? 0}</span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .topbar {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .search {
    padding: 5px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
    color: var(--fg);
    font-size: 13px;
    width: 220px;
  }
  .spacer {
    flex: 1;
  }
  /* 三个按钮一样高：文字按钮的高度本来由行高决定，图标按钮由图标决定，两者
     差个两三像素，摆在一行里一眼就看得出来。所以统一钉死高度，内容居中。 */
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    box-sizing: border-box;
    height: 28px;
    padding: 0 12px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--fg);
    font-size: 13px;
    line-height: 1;
    cursor: default;
  }
  .btn:hover {
    background: var(--surface-hover);
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .btn.icon {
    width: 28px;
    padding: 0;
    color: var(--muted);
  }
  .btn.icon:hover {
    color: var(--fg);
  }
  .gear {
    width: 18px;
    height: 18px;
    display: block;
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip {
    padding: 2px 10px;
    border-radius: 12px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--fg);
    font-size: 12px;
    cursor: default;
  }
  .chip.active {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  /* 数字比标签名淡一档：先读到的应该是标签，数量是它的注脚。 */
  .n {
    margin-left: 5px;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }
  .chip.active .n {
    color: rgba(255, 255, 255, 0.75);
  }
</style>
