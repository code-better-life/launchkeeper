<script lang="ts">
  import { describeTrigger } from "../lib/describeTrigger";
  // 别人写的 LaunchAgent 的只读详情（docs/M3-design.md §3.4，设计稿画板
  // 「接管现有 LaunchAgent」）。
  //
  // 只读是刻意的：Launchkeeper 只写自己的 plist（CLAUDE.md）。这里能做的三件事
  // ——运行、启用/禁用、接管——前两件只经过 launchctl，一个字节都不落到别人的
  // 文件上；第三件会改文件，所以它要过确认框、要先备份 .bak。
  import type { ExternalAgentView } from "../lib/bindings";
  import { taskStore } from "../lib/stores/tasks.svelte";
  import {
    adoptTooltip,
    externalHeadline,
    externalLogText,
    foreignDisableWarning,
    isForeignAgent,
  } from "../lib/external";
  import { locale, t } from "../lib/i18n";
  import ConfirmDialog from "./ConfirmDialog.svelte";

  let { agent }: { agent: ExternalAgentView } = $props();

  const canAdopt = $derived(agent.adoptable && !agent.managed);

  /** 禁用别人的 LaunchAgent 之前的确认框（M4，和 ExternalRow 是同一句话）。 */
  let confirmingDisable = $state(false);

  function toggleLoaded() {
    if (agent.loaded && isForeignAgent(agent)) {
      confirmingDisable = true;
      return;
    }
    void taskStore.externalSetEnabled(agent.label, !agent.loaded);
  }
</script>

<div class="detail">
  <header>
    <div class="title">
      <h2>{agent.label}</h2>
      <span class="sub">{externalHeadline(agent, locale.current)}</span>
    </div>
    <div class="actions">
      <button
        class="btn"
        disabled={!agent.loaded}
        title={agent.loaded
          ? t("external.detail.run_tip")
          : t("external.detail.run_disabled_tip")}
        onclick={() => void taskStore.externalRun(agent.label)}
      >
        {t("menu.run_now")}
      </button>
      <button
        class="btn"
        title={t("external.enable_tip")}
        onclick={toggleLoaded}
      >
        {agent.loaded ? t("menu.disable") : t("menu.enable")}
      </button>
      <button
        class="btn primary"
        disabled={!canAdopt}
        title={adoptTooltip(agent, locale.current)}
        onclick={() => taskStore.startAdopt(agent.label)}
      >
        {t("menu.adopt")}
      </button>
    </div>
  </header>
  <div class="body">
    <p class="lead">{t("external.detail.lead")}</p>
    {#if !canAdopt}
      <p class="why">{adoptTooltip(agent, locale.current)}</p>
    {/if}
    <div class="grid">
      <div class="field">
        <span class="label">{t("external.detail.program")}</span>
        <div class="ro">{agent.command || t("common.placeholder")}</div>
      </div>
      <div class="field">
        <span class="label">{t("external.detail.trigger")}</span>
        <div class="ro">{describeTrigger(agent.trigger, locale.current, t("external.trigger_unsupported"))}{agent.keep_alive ? " · KeepAlive" : ""}</div>
      </div>
      <div class="field">
        <span class="label">{t("external.detail.working_dir")}</span>
        <div class="ro">{agent.working_dir ?? t("external.detail.working_dir_unset")}</div>
      </div>
      <div class="field">
        <span class="label">{t("external.detail.log")}</span>
        <div class="ro">{externalLogText(agent, locale.current)}</div>
      </div>
      <div class="field">
        <span class="label">{t("external.detail.plist")}</span>
        <div class="ro">{agent.path}</div>
      </div>
      <div class="field">
        <span class="label">{t("external.detail.launchd")}</span>
        <div class="ro">
          {agent.loaded
            ? t("external.headline.loaded")
            : t("external.headline.unloaded")}
          {#if agent.pid !== null}· {t("row.pid", { pid: agent.pid })}{/if}
          {#if agent.last_exit !== null}
            · {t("external.detail.last_exit", { code: agent.last_exit })}
          {/if}
        </div>
      </div>
    </div>
  </div>
</div>

{#if confirmingDisable}
  <ConfirmDialog
    title={t("confirm.external_disable.title")}
    message={foreignDisableWarning(agent, locale.current)}
    confirmLabel={t("confirm.external_disable.confirm")}
    danger
    onconfirm={() => {
      confirmingDisable = false;
      void taskStore.externalSetEnabled(agent.label, false);
    }}
    oncancel={() => (confirmingDisable = false)}
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
    gap: 12px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
  }
  .title {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  h2 {
    margin: 0;
    font-size: 15px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub {
    color: var(--muted);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .actions {
    display: flex;
    gap: 6px;
    flex-shrink: 0;
  }
  .body {
    flex: 1;
    overflow-y: auto;
    padding: 14px 16px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .lead {
    margin: 0;
    color: var(--muted);
    font-size: 13px;
    line-height: 1.6;
  }
  .why {
    margin: 0;
    font-size: 12px;
    color: var(--muted);
    border-left: 2px solid var(--border);
    padding-left: 10px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 12px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .label {
    font-size: 12px;
    color: var(--muted);
  }
  /* 只读格：和表单的输入框同一个盒子，但底色更闷、光标不变，一眼看得出改不了。 */
  .ro {
    min-height: 28px;
    padding: 5px 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface-hover);
    color: var(--muted);
    font-size: 13px;
    overflow-wrap: anywhere;
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
  .btn:disabled {
    opacity: 0.5;
  }
</style>
