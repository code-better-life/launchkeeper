<script lang="ts">
  import { t } from "../lib/i18n";

  // `kind` 只换底色（M4 §4）：报错是红的，「已复制到剪贴板」这类确认是绿的。
  // 默认 `error`，所以老的调用点一个字都不用改。
  let {
    message,
    kind = "error",
    onclose,
  }: { message: string; kind?: "error" | "ok"; onclose: () => void } = $props();
</script>

<div class="toast" class:ok={kind === "ok"} role="alert">
  <span class="msg">{message}</span>
  <button class="close" onclick={onclose} aria-label={t("common.close")}>✕</button>
</div>

<style>
  .toast {
    position: fixed;
    left: 50%;
    bottom: 20px;
    transform: translateX(-50%);
    z-index: 2000;
    max-width: 560px;
    display: flex;
    align-items: flex-start;
    gap: 10px;
    background: #d7443e;
    color: #fff;
    padding: 10px 14px;
    border-radius: 8px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.3);
    font-size: 13px;
  }
  .toast.ok {
    background: #29a745;
  }
  .msg {
    white-space: pre-wrap;
    word-break: break-word;
  }
  .close {
    all: unset;
    cursor: default;
    opacity: 0.85;
    flex-shrink: 0;
  }
  .close:hover {
    opacity: 1;
  }
</style>
