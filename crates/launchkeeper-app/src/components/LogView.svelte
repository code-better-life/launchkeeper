<script lang="ts">
  // 单个 run 的一个流（stdout/stderr）：展示已加载内容，运行中时每秒轮询。
  import type { LogStream, RunHandle } from "../lib/bindings";
  import { api } from "../lib/api";
  import { t } from "../lib/i18n";

  let {
    runId,
    stream,
    running,
  }: { runId: RunHandle; stream: LogStream; running: boolean } = $props();

  let content = $state("");
  let totalBytes = $state(0);
  let truncated = $state(false);
  let loading = $state(true);
  let errorMsg = $state<string | null>(null);
  let preEl: HTMLPreElement | undefined = $state();

  const TAIL_BYTES = 64 * 1024;

  /** 距离底部多少像素以内算"钉在底部"。 */
  const PIN_SLACK = 24;

  async function load() {
    // 先看写入前是否钉在底部：滚上去看历史输出的人不该被下一次轮询拽回去。
    const pinned = isPinnedToBottom();
    try {
      const chunk = await api.readLog(runId, stream, TAIL_BYTES);
      content = chunk.content;
      totalBytes = chunk.total_bytes;
      truncated = chunk.truncated;
      errorMsg = null;
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
      if (pinned) scrollToBottom();
    }
  }

  function isPinnedToBottom(): boolean {
    if (!preEl) return true; // 首次渲染：从底部开始看。
    return preEl.scrollHeight - preEl.scrollTop - preEl.clientHeight <= PIN_SLACK;
  }

  function scrollToBottom() {
    queueMicrotask(() => {
      if (preEl) preEl.scrollTop = preEl.scrollHeight;
    });
  }

  // 只在运行中轮询。running 由 RunHistory 从刷新后的运行记录里重新取，所以
  // 这条运行一结束（finished_at 有值）轮询就停，不会一直每秒读一个不再变化
  // 的文件。
  $effect(() => {
    void runId;
    void stream;
    loading = true;
    void load();

    if (!running) return;
    const timer = setInterval(() => {
      void load();
    }, 1000);
    return () => clearInterval(timer);
  });
</script>

<div class="log">
  {#if loading}
    <p class="muted">{t("common.loading")}</p>
  {:else if errorMsg}
    <p class="error">{errorMsg}</p>
  {:else}
    {#if truncated}
      <p class="notice">{t("log.truncated", { tail: TAIL_BYTES, total: totalBytes })}</p>
    {/if}
    <pre bind:this={preEl}>{content || t("log.empty")}</pre>
  {/if}
</div>

<style>
  .log {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .notice {
    margin: 0 0 6px;
    color: var(--muted);
    font-size: 12px;
  }
  pre {
    margin: 0;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 10px;
    font-size: 12px;
    line-height: 1.5;
    max-height: 260px;
    overflow: auto;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .muted {
    color: var(--muted);
  }
  .error {
    color: #d7443e;
  }
</style>
