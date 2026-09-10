<script lang="ts">
  import { t } from "../lib/i18n";

  let {
    title,
    message,
    confirmLabel,
    danger = false,
    onconfirm,
    oncancel,
  }: {
    title: string;
    message: string;
    confirmLabel?: string;
    danger?: boolean;
    onconfirm: () => void;
    oncancel: () => void;
  } = $props();

  function onWindowKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") oncancel();
    if (e.key === "Enter") onconfirm();
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="backdrop" onmousedown={oncancel}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="dialog"
    onmousedown={(e) => e.stopPropagation()}
    role="alertdialog"
    aria-modal="true"
    tabindex="-1"
  >
    <h2>{title}</h2>
    <p>{message}</p>
    <div class="actions">
      <button class="btn" onclick={oncancel}>{t("common.cancel")}</button>
      <button class="btn" class:danger onclick={onconfirm}>{confirmLabel ?? t("common.ok")}</button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.25);
    z-index: 1500;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .dialog {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 18px 20px;
    width: 340px;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.3);
  }
  h2 {
    margin: 0 0 8px;
    font-size: 14px;
  }
  p {
    margin: 0 0 16px;
    color: var(--muted);
    font-size: 13px;
    line-height: 1.5;
    /* 多段的警告（M4 的「禁用别人的 LaunchAgent」）里的换行要留住；
       没有换行的旧消息一个像素都不会变。 */
    white-space: pre-line;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .btn {
    padding: 5px 14px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--fg);
    font-size: 13px;
    cursor: default;
  }
  .btn:hover {
    filter: brightness(0.96);
  }
  .btn.danger {
    background: #d7443e;
    border-color: #d7443e;
    color: #fff;
  }
</style>
