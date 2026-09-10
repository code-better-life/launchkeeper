<script lang="ts">
  // 接管确认框（docs/M3-design.md §3.4，设计稿画板「接管现有 LaunchAgent」）。
  //
  // 这个框存在的理由只有一个：接管会改写用户自己写的文件，所以在动手之前，
  // 要改成什么必须一行一行摆出来。diff 由后端算（core 的 AdoptionPlan：拿
  // 现有 plist 和将要生成的 plist 逐键对比），前端不猜。
  import type { AdoptionPlanView } from "../lib/bindings";
  import { api } from "../lib/api";
  import { taskStore } from "../lib/stores/tasks.svelte";
  import { diffLines } from "../lib/external";
  import { locale, t } from "../lib/i18n";

  let { label }: { label: string } = $props();

  let plan = $state<AdoptionPlanView | null>(null);
  let loadError = $state<string | null>(null);
  /** 界面稿上默认勾着：接管完就让新定义生效，是绝大多数人要的。 */
  let reload = $state(true);
  let busy = $state(false);

  $effect(() => {
    const want = label;
    plan = null;
    loadError = null;
    void api
      .adoptionPlan(want)
      .then((p) => {
        if (want === label) plan = p;
      })
      .catch((e) => {
        if (want === label) loadError = e instanceof Error ? e.message : String(e);
      });
  });

  const lines = $derived(plan ? diffLines(plan, locale.current) : []);

  async function confirm() {
    if (!plan || busy) return;
    busy = true;
    try {
      await taskStore.adopt(label, reload);
    } catch (e) {
      // 失败就把框留在原地并把后端原话写在里面：接管失败的原因（.bak 已存在、
      // 任务名已被占用）多半要用户先去处理一件事，关掉框只会让他忘了是什么。
      loadError = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  function onWindowKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") taskStore.cancelAdopt();
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="backdrop" onmousedown={() => taskStore.cancelAdopt()}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="dialog"
    onmousedown={(e) => e.stopPropagation()}
    role="alertdialog"
    aria-modal="true"
    aria-labelledby="adopt-title"
    tabindex="-1"
  >
    <h2 id="adopt-title">{t("adopt.title", { label })}</h2>
    {#if plan}
      <p class="lead">
        {t("adopt.lead_before")}
        <code>{plan.backup_path}</code>{t("adopt.lead_after")}
      </p>
      <div class="diff">
        {#each lines as line, i (i)}
          <div class="line" class:muted={line.muted}>
            <span
              class="sign"
              class:minus={line.sign === "−"}
              class:plus={line.sign === "+"}>{line.sign}</span
            >
            <span class="text">{line.text}</span>
          </div>
        {/each}
      </div>
      <label class="check">
        <input type="checkbox" bind:checked={reload} />
        <span>{t("adopt.reload_check")}</span>
      </label>
      {#if !reload}
        <p class="hint">{t("adopt.no_reload_hint")}</p>
      {/if}
    {:else if loadError}
      <p class="error">{loadError}</p>
    {:else}
      <p class="lead">{t("adopt.loading")}</p>
    {/if}
    {#if plan && loadError}
      <p class="error">{loadError}</p>
    {/if}
    <div class="actions">
      <button class="btn" onclick={() => taskStore.cancelAdopt()}>
        {t("common.cancel")}
      </button>
      <button class="btn primary" disabled={!plan || busy} onclick={() => void confirm()}>
        {busy
          ? t("adopt.busy")
          : reload
            ? t("adopt.confirm_reload")
            : t("adopt.confirm")}
      </button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.28);
    z-index: 1500;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .dialog {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 22px 24px;
    width: 560px;
    max-width: calc(100vw - 40px);
    max-height: calc(100vh - 60px);
    overflow-y: auto;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.3);
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  h2 {
    margin: 0;
    font-size: 15px;
    overflow-wrap: anywhere;
  }
  .lead {
    margin: 0;
    color: var(--muted);
    font-size: 13px;
    line-height: 1.6;
  }
  code {
    font-family: "SF Mono", Menlo, monospace;
    font-size: 12px;
    overflow-wrap: anywhere;
  }
  /* 等宽框：−/+/= 三种记号，和 CLI 的 `adopt --dry-run` 打的是同一份 diff。 */
  .diff {
    display: flex;
    flex-direction: column;
    gap: 8px;
    font-size: 12px;
    font-family: "SF Mono", Menlo, monospace;
    background: var(--surface-hover);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 12px;
  }
  .line {
    display: flex;
    gap: 10px;
  }
  .line.muted .text {
    color: var(--muted);
  }
  .sign {
    width: 12px;
    flex-shrink: 0;
    color: var(--muted);
  }
  .sign.minus {
    color: #d7443e;
  }
  .sign.plus {
    color: var(--status-ok);
  }
  .text {
    overflow-wrap: anywhere;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }
  .hint {
    margin: 0;
    font-size: 12px;
    color: var(--muted);
  }
  .error {
    margin: 0;
    font-size: 13px;
    color: #d7443e;
    overflow-wrap: anywhere;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
  .btn {
    padding: 6px 14px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--fg);
    font-size: 13px;
    cursor: default;
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .btn:disabled {
    opacity: 0.5;
  }
</style>
