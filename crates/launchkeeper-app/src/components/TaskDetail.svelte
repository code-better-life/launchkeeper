<script lang="ts">
  import { describeTrigger } from "../lib/describeTrigger";
  import { untrack } from "svelte";
  import type { InsightView, TaskDetail as TaskDetailType } from "../lib/bindings";
  import { api } from "../lib/api";
  import { formatUptime, relativeTime } from "../lib/format";
  import { locale, t } from "../lib/i18n";
  import { renderMarkdown } from "../lib/markdown";
  import { runModeText } from "../lib/summary";
  import { taskStore } from "../lib/stores/tasks.svelte";
  import TaskForm from "./TaskForm.svelte";
  import TaskSummary from "./TaskSummary.svelte";
  import RunHistory from "./RunHistory.svelte";
  import ConfirmDialog from "./ConfirmDialog.svelte";

  let {
    mode,
    taskName,
    onclose,
    oncreated,
    onsettings,
  }: {
    mode: "view" | "create";
    taskName: string | null;
    onclose: () => void;
    oncreated: (name: string) => void;
    /**
     * 打开设置面板（M4 §4）。「AI 解读」失败时多半是没配 Key，报错文案已经
     * 说了"去设置里填"，再给一个直接过去的按钮，省得用户自己找顶栏。
     */
    onsettings: () => void;
  } = $props();

  let detail = $state<TaskDetailType | null>(null);
  /** `task_log_dir` 的结果，只读概览里那一行「日志目录」。 */
  let logDir = $state<string | null>(null);
  let loading = $state(false);
  let loadError = $state<string | null>(null);
  let tab = $state<"config" | "history" | "insight">("config");
  // ---- M4 §1 / §4：AI 解读 ----
  /** 库里那条解读（一个任务只有一条，M4 §1.3），没有就是 null。 */
  let insight = $state<InsightView | null>(null);
  /** 正在读库里那条（纯读，不联网，通常一瞬间）。 */
  let insightLoading = $state(false);
  /** 正在调模型（可能几十秒）。 */
  let explaining = $state(false);
  /** 上一次「解读」失败的原因，就地留一行 + 一个「打开设置」的入口。 */
  let insightError = $state<string | null>(null);
  /** 「撤销接管」的确认框（M3 §3.4）。 */
  let confirmingUnadopt = $state(false);
  let formKey = $state(0); // 递增以强制 TaskForm 重新挂载，从而丢弃未保存的编辑。

  async function loadDetail(name: string) {
    loading = true;
    loadError = null;
    try {
      detail = await api.getTask(name);
      // 日志目录不在 TaskDetail 里（它是按数据目录拼出来的），单独问一次。
      // 问不到就不显示那一行——概览少一行，比在上面写一个"—"强。
      logDir = await api.taskLogDir(name).catch(() => null);
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
      detail = null;
      logDir = null;
    } finally {
      loading = false;
    }
  }

  /**
   * 读库里那条解读。
   *
   * 走 `get_task_insight` 而不是 `explain_task(name, false)`：后者语义上一样
   * （不刷新就直接返回库里那份，M4 §1.7），但它要 Key、名字也叫"解读"——
   * 换一个任务就悄悄发一次请求出去，不是这一页该做的事。这个是纯读，没有
   * Key、断网都能显示上周那份。
   */
  async function loadInsight(name: string) {
    insightLoading = true;
    insightError = null;
    try {
      insight = await api.getTaskInsight(name);
    } catch (e) {
      insight = null;
      insightError = e instanceof Error ? e.message : String(e);
    } finally {
      insightLoading = false;
    }
  }

  /** 「解读」/「重新解读」：真的调模型，可能等几十秒。 */
  async function explain(refresh: boolean) {
    if (!taskName || explaining) return;
    explaining = true;
    insightError = null;
    try {
      insight = await api.explainTask(taskName, refresh);
    } catch (e) {
      // 两处都报：toast 是"刚才那一下没成"，就地那一行是"现在这一页为什么
      // 还是空的"，后者还带着去设置面板的入口。
      insightError = e instanceof Error ? e.message : String(e);
      taskStore.showError(e);
    } finally {
      explaining = false;
    }
  }

  $effect(() => {
    // 只依赖 mode / taskName；formKey 的读-改-写不能发生在这个响应式作用域
    // 里，否则它自己写回的 formKey 会被当成依赖，造成无限重跑（Svelte 5
    // 的 effect_update_depth_exceeded）。用 untrack 包起来即可。
    if (mode === "view" && taskName) {
      tab = "config";
      untrack(() => {
        formKey += 1;
      });
      void loadDetail(taskName);
      // 解读跟着选中的任务走（M4 §4）：换一个任务就重新读库里那份，否则这一页
      // 会拿上一个任务的解读当成这一个的。
      insight = null;
      void loadInsight(taskName);
    } else {
      detail = null;
      insight = null;
      insightError = null;
    }
  });

  /** 保存成功：回到只读概览，并把刚存进去的东西重新读出来。 */
  async function onSaved() {
    taskStore.setEditing(false);
    formKey += 1;
    if (!taskName) return;
    await taskStore.refresh();
    await loadDetail(taskName);
  }

  /** 取消：丢弃改动（{#key} 重新挂载表单），回到只读概览。 */
  function onCancelEdit() {
    formKey += 1;
    taskStore.setEditing(false);
  }

  // 头部（显示名）跟着 store 走：后台扫描（§3.4）刷新了任务，这里就跟着变，
  // 不必重新拉一次 get_task，也不会停在打开详情页那一刻的名字上。
  const header = $derived(
    taskStore.tasks.find((t) => t.name === taskName) ?? detail?.view ?? null,
  );

  // 服务型任务的头部换成 启动 / 停止 / 重启，并显示 pid 和运行时长。
  const isService = $derived(header?.is_service ?? false);
  const running = $derived(header?.status === "running");
  const busy = $derived(taskName != null && taskStore.isBusy(taskName));

  async function onCreateSaved(name: string) {
    await taskStore.refresh();
    oncreated(name);
  }
</script>

<div class="detail">
  {#if mode === "create"}
    <header>
      <h2>{t("detail.create_title")}</h2>
    </header>
    <div class="body">
      <TaskForm
        mode="create"
        initial={null}
        onsaved={(input) => void onCreateSaved(input.name)}
        oncancel={onclose}
      />
    </div>
  {:else if !taskName}
    <div class="empty">
      <p>{t("detail.empty")}</p>
    </div>
  {:else if loading && !detail}
    <div class="empty muted">{t("common.loading")}</div>
  {:else if loadError}
    <div class="empty error">{loadError}</div>
  {:else if detail}
    <header>
      <div class="title">
        <h2>{header?.display_name ?? detail.view.display_name}</h2>
        <span class="name">{detail.task.name}</span>
        {#if runModeText(detail.task, locale.current)}
          <!-- 「怎么跑的」：python3 / uv run / 直接执行，按当前语言由前端从
               script_path + args 反推（lib/summary.ts），和摘要那一行一致。 -->
          <span class="name">{runModeText(detail.task, locale.current)}</span>
        {/if}
        {#if isService && running}
          <span class="live">
            {t("status.running")}
            {#if header?.loaded_pid != null}· {t("row.pid", { pid: header.loaded_pid })}{/if}
            · {formatUptime(header?.uptime_secs, locale.current)}
          </span>
        {:else if isService}
          <span class="live muted">{t("status.stopped")}</span>
        {/if}
      </div>
      {#if isService}
        <div class="actions">
          <button
            class="btn primary"
            disabled={running || busy}
            onclick={() => void taskStore.start(detail!.task.name)}
          >
            {t("menu.start")}
          </button>
          <button
            class="btn"
            disabled={!running || busy}
            onclick={() => void taskStore.stop(detail!.task.name)}
          >
            {t("menu.stop")}
          </button>
          <button
            class="btn"
            disabled={!running || busy}
            onclick={() => void taskStore.restart(detail!.task.name)}
          >
            {t("menu.restart")}
          </button>
        </div>
      {:else}
        <button class="btn primary" onclick={() => void taskStore.runNow(detail!.task.name)}>
          {t("menu.run_now")}
        </button>
      {/if}
    </header>
    {#if detail.adopted}
      <!-- 接管来的任务：说清楚它的 plist 是谁的、在哪儿，以及怎么还回去
           （M3 §3.4）。撤销会把 .bak 按字节移回原处，任务行随之消失。 -->
      <div class="adopted">
        <span>
          {t("detail.adopted_from")}
          <code>{detail.plist_path ?? t("detail.unknown_path")}</code>
        </span>
        <span class="sep">·</span>
        <button class="link" onclick={() => (confirmingUnadopt = true)}>
          {t("detail.undo_adopt")}
        </button>
      </div>
    {/if}
    <div class="tabs">
      <button class="tab" class:active={tab === "config"} onclick={() => (tab = "config")}>{t("detail.tab.config")}</button>
      <button class="tab" class:active={tab === "history"} onclick={() => (tab = "history")}>{t("detail.tab.history")}</button>
      <button class="tab" class:active={tab === "insight"} onclick={() => (tab = "insight")}>{t("detail.tab.insight")}</button>
      <span class="spacer"></span>
      {#if tab === "config" && !taskStore.editing}
        <!-- 「编辑」是一个明确的动作。点开任务直接掉进表单里，用户不知道自己
             碰了什么、会不会存下去（M3 §3.4 追补）。 -->
        <button
          class="edit"
          title={t("detail.edit_title")}
          onclick={() => taskStore.setEditing(true)}
        >
          {t("detail.edit")}
        </button>
      {/if}
    </div>
    <div class="body">
      {#if tab === "config"}
        {#if taskStore.editing}
          {#key formKey}
            <TaskForm mode="edit" initial={detail.task} onsaved={onSaved} oncancel={onCancelEdit} />
          {/key}
        {:else}
          <!-- 服务型任务的启用开关在这里（列表行上只有启动/停止，两个都长得
               像"停掉它"的控件不该摆在一起）。 -->
          <TaskSummary
            task={detail.task}
            triggerText={describeTrigger((header ?? detail.view).trigger, locale.current, (header ?? detail.view).trigger_text)}
            logDir={logDir}
            adoptedFrom={detail.adopted ? detail.plist_path : null}
            service={isService}
            enabled={(header ?? detail.view).enabled}
          />
        {/if}
      {:else if tab === "history"}
        <RunHistory taskName={detail.task.name} />
      {:else}
        <!-- ---- AI 解读（M4 §1 / §4）----
             显示的是**库里那一条**：一次解读要花钱、要等几秒，打开这一页不
             是重付一次的理由（M4 §1.7）。想要新的就点「重新解读」。 -->
        <div class="insight">
          <div class="insight-head">
            <button
              class="btn primary"
              disabled={explaining || insightLoading}
              onclick={() => void explain(insight != null)}
            >
              {#if explaining}
                <span class="spinner" aria-hidden="true"></span>
                {t("insight.running")}
              {:else}
                {insight ? t("insight.refresh") : t("insight.explain")}
              {/if}
            </button>
            {#if insight}
              <span class="insight-meta">
                {t("insight.meta", {
                  time: relativeTime(insight.created_at, new Date(), locale.current),
                  model: insight.model,
                })}
              </span>
            {/if}
          </div>

          {#if insightError}
            <p class="insight-error">
              {insightError}
              <button class="link" onclick={onsettings}>{t("insight.open_settings")}</button>
            </p>
          {/if}

          {#if insightLoading && !insight}
            <p class="muted">{t("common.loading")}</p>
          {:else if insight}
            <!-- `content` 是远端模型写的字符串，Markdown 允许内联 HTML——
                 `renderMarkdown` 里过了一遍 DOMPurify，这里才敢用 {@html}。 -->
            <article class="markdown">{@html renderMarkdown(insight.content)}</article>
          {:else if !explaining}
            <div class="insight-empty">
              <p class="empty-title">{t("insight.empty.title")}</p>
              <p>{t("insight.empty.body")}</p>
              <p class="muted">{t("insight.empty.kept")}</p>
              <p class="muted">{t("insight.empty.privacy")}</p>
            </div>
          {/if}
        </div>
      {/if}
    </div>
  {/if}
</div>

{#if confirmingUnadopt && detail}
  <ConfirmDialog
    title={t("confirm.unadopt.title")}
    message={t("confirm.unadopt.message", {
      path: detail.plist_path ?? t("confirm.unadopt.fallback_path"),
    })}
    confirmLabel={t("confirm.unadopt.confirm")}
    danger
    onconfirm={() => {
      confirmingUnadopt = false;
      void taskStore.unadopt(detail!.task.name);
    }}
    oncancel={() => (confirmingUnadopt = false)}
  />
{/if}

<style>
  .detail {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
  }
  .title {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  h2 {
    margin: 0;
    font-size: 15px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .name {
    color: var(--muted);
    font-size: 12px;
  }
  .live {
    font-size: 12px;
    color: var(--status-running);
    white-space: nowrap;
  }
  .live.muted {
    color: var(--muted);
  }
  .actions {
    display: flex;
    gap: 6px;
    flex-shrink: 0;
  }
  .btn:disabled {
    opacity: 0.5;
  }
  .adopted {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 16px;
    font-size: 12px;
    color: var(--muted);
    background: var(--surface-hover);
    border-bottom: 1px solid var(--border);
    overflow-wrap: anywhere;
  }
  .adopted code {
    font-family: "SF Mono", Menlo, monospace;
  }
  .link {
    all: unset;
    color: var(--accent);
    cursor: default;
  }
  .sep {
    color: var(--muted);
  }
  .tabs {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 8px 16px 0;
    border-bottom: 1px solid var(--border);
  }
  .spacer {
    flex: 1;
  }
  .edit {
    margin-bottom: 6px;
    padding: 4px 12px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--fg);
    font-size: 12px;
    cursor: default;
  }
  .edit:hover {
    background: var(--surface-hover);
  }
  .tab {
    all: unset;
    padding: 6px 12px;
    font-size: 13px;
    color: var(--muted);
    cursor: default;
    border-bottom: 2px solid transparent;
  }
  .tab.active {
    color: var(--fg);
    border-bottom-color: var(--accent);
  }
  .body {
    flex: 1;
    overflow-y: auto;
    padding: 14px 16px;
    min-width: 0;
  }
  .empty {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--muted);
  }
  .error {
    color: #d7443e;
  }
  .btn {
    padding: 6px 14px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--fg);
    font-size: 13px;
    cursor: default;
    flex-shrink: 0;
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }

  /* ---- AI 解读（M4 §4）---- */
  .insight {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .insight-head {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .insight-head .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .insight-meta {
    font-size: 12px;
    color: var(--muted);
  }
  .spinner {
    width: 11px;
    height: 11px;
    border: 2px solid rgba(255, 255, 255, 0.4);
    border-top-color: #fff;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation-duration: 3s;
    }
  }
  .insight-error {
    margin: 0;
    font-size: 12px;
    color: var(--status-failed);
    overflow-wrap: anywhere;
  }
  .insight-empty {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 60ch;
    font-size: 13px;
    line-height: 1.6;
  }
  .insight-empty p {
    margin: 0;
  }
  .empty-title {
    font-weight: 600;
  }
  .muted {
    color: var(--muted);
    font-size: 12px;
  }
  /* 模型写的 Markdown：只给它排版，不给它颜色以外的任何主动权。 */
  .markdown {
    font-size: 13px;
    line-height: 1.65;
    max-width: 76ch;
    overflow-wrap: anywhere;
  }
  .markdown :global(h1),
  .markdown :global(h2),
  .markdown :global(h3) {
    font-size: 13px;
    font-weight: 600;
    margin: 16px 0 6px;
  }
  .markdown :global(h1:first-child),
  .markdown :global(h2:first-child),
  .markdown :global(h3:first-child) {
    margin-top: 0;
  }
  .markdown :global(p) {
    margin: 0 0 10px;
  }
  .markdown :global(ul),
  .markdown :global(ol) {
    margin: 0 0 10px;
    padding-left: 20px;
  }
  .markdown :global(li) {
    margin-bottom: 4px;
  }
  .markdown :global(code) {
    font-family: "SF Mono", Menlo, monospace;
    font-size: 12px;
    background: var(--surface);
    border-radius: 4px;
    padding: 1px 4px;
  }
  .markdown :global(pre) {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 10px 12px;
    overflow-x: auto;
  }
  .markdown :global(pre code) {
    background: none;
    padding: 0;
  }
  .markdown :global(blockquote) {
    margin: 0 0 10px;
    padding-left: 10px;
    border-left: 3px solid var(--border);
    color: var(--muted);
  }
  .markdown :global(table) {
    border-collapse: collapse;
    font-size: 12px;
  }
  .markdown :global(th),
  .markdown :global(td) {
    border: 1px solid var(--border);
    padding: 4px 8px;
    text-align: left;
  }
  .markdown :global(a) {
    color: var(--accent);
  }
</style>
