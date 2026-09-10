<script lang="ts">
  import { describeTrigger } from "../lib/describeTrigger";
  import type { TaskView } from "../lib/bindings";
  import { relativeTime, formatExit, formatUptime } from "../lib/format";
  import { taskStore } from "../lib/stores/tasks.svelte";
  import { api } from "../lib/api";
  import { locale, t } from "../lib/i18n";
  import ContextMenu, { type MenuItem } from "./ContextMenu.svelte";
  import ToggleSwitch from "./ToggleSwitch.svelte";
  import ConfirmDialog from "./ConfirmDialog.svelte";

  let {
    task,
    selected,
    onselect,
    onedit,
  }: {
    task: TaskView;
    selected: boolean;
    onselect: () => void;
    onedit: () => void;
  } = $props();

  let menuPos = $state<{ x: number; y: number } | null>(null);
  let confirmingDelete = $state(false);

  const running = $derived(task.status === "running");
  const busy = $derived(taskStore.isBusy(task.name));

  function statusClass(s: TaskView["status"]): string {
    return `dot dot-${s}`;
  }

  /** 状态点的 tooltip。写成函数而不是模块级常量：切换语言要跟着变。 */
  function statusTitle(s: TaskView["status"]): string {
    return t(`status.${s}` as const);
  }

  function onContextMenu(e: MouseEvent) {
    e.preventDefault();
    onselect();
    menuPos = { x: e.clientX, y: e.clientY };
  }

  /** 服务型任务的右键菜单是启动/停止/重启，其余任务是"立即运行"。 */
  function actionItems(): MenuItem[] {
    if (!task.is_service) {
      return [
        { label: t("menu.run_now"), action: () => void taskStore.runNow(task.name) },
      ];
    }
    return [
      {
        label: t("menu.start"),
        action: () => void taskStore.start(task.name),
        disabled: running || busy,
      },
      {
        label: t("menu.stop"),
        action: () => void taskStore.stop(task.name),
        disabled: !running || busy,
      },
      {
        label: t("menu.restart"),
        action: () => void taskStore.restart(task.name),
        disabled: !running || busy,
      },
    ];
  }

  function menuItems(): MenuItem[] {
    return [
      ...actionItems(),
      { label: t("menu.edit"), action: onedit },
      {
        label: task.enabled ? t("menu.disable") : t("menu.enable"),
        action: () => void taskStore.setEnabled(task.name, !task.enabled),
      },
      {
        label: t("menu.reveal_script"),
        action: () =>
          void api
            .getTask(task.name)
            .then((detail) => api.revealInFinder(detail.task.script_path))
            .catch((e) => taskStore.showError(e)),
      },
      {
        label: t("menu.delete"),
        action: () => (confirmingDelete = true),
        danger: true,
        separatorBefore: true,
      },
    ];
  }

  /**
   * 启用开关（`ToggleSwitch`）。
   *
   * 原来这里是一个光秃秃的 `<input type="checkbox">`：没有标签、没有 tooltip，
   * 谁也猜不出点下去会发生什么（用户的原话是"居然有功能"）。
   *
   * **常驻服务的行上没有这个开关**：那一行已经有一个启动/停止按钮，再摆一个
   * 开关就是两个看起来都像"停掉它"的控件，而它们停的其实是两件事（一个是这次
   * 进程，一个是整个 job 还在不在 launchd 里）。服务的启用/禁用留在右键菜单
   * 和详情页概览里那一行「登录时自动启动」。
   */
  const toggleLabel = $derived(task.enabled ? t("row.toggle_on") : t("row.toggle_off"));

  function onPlayStop(e: MouseEvent) {
    e.stopPropagation();
    if (running) void taskStore.stop(task.name);
    else void taskStore.start(task.name);
  }
</script>

<div
  class="row"
  class:selected
  onclick={onselect}
  onkeydown={(e) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      onselect();
    }
  }}
  oncontextmenu={onContextMenu}
  role="button"
  tabindex="0"
>
  <span class={statusClass(task.status)} title={statusTitle(task.status)}></span>
  <div class="main">
    <div class="line1">
      <span class="name">{task.display_name}</span>
      <span class="trigger">{describeTrigger(task.trigger, locale.current, task.trigger_text)}</span>
    </div>
    <div class="line2">
      {#if !task.enabled}
        <!-- 停用是一个状态，不是"最近没跑过"。开关旁边那一格文字是用户确认
             自己刚才点了什么的地方，所以它排在最前面。 -->
        <span class="off">{t("row.disabled_badge")}</span>
        <span class="sep">·</span>
      {/if}
      {#if task.is_service && running}
        <!-- 服务在跑的时候，"上次运行是什么时候"没有意义，运行了多久才有。 -->
        <span>{t("row.uptime", { duration: formatUptime(task.uptime_secs, locale.current) })}</span>
        {#if task.loaded_pid !== null}
          <span class="sep">·</span>
          <span>{t("row.pid", { pid: task.loaded_pid })}</span>
        {/if}
      {:else if task.last_run}
        <span title={new Date(task.last_run.started_at).toLocaleString(locale.current)}>
          {relativeTime(task.last_run.started_at, new Date(), locale.current)}
        </span>
        <span class="sep">·</span>
        <span>
          {formatExit(
            task.last_run.exit_code,
            task.last_run.running,
            task.last_run.finished_at,
            locale.current,
            task.last_run.stop_reason,
          )}
        </span>
      {:else}
        <span class="muted">{t("row.never_run")}</span>
      {/if}
    </div>
  </div>
  {#if task.is_service}
    <button
      class="play"
      class:stop={running}
      onclick={onPlayStop}
      disabled={busy}
      title={running ? t("row.stop_service") : t("row.start_service")}
      aria-label={running ? t("row.stop_service") : t("row.start_service")}
    >
      {running ? "■" : "▶"}
    </button>
  {:else}
    <ToggleSwitch
      checked={task.enabled}
      label={toggleLabel}
      {selected}
      ontoggle={() => void taskStore.setEnabled(task.name, !task.enabled)}
    />
  {/if}
</div>

{#if menuPos}
  <ContextMenu x={menuPos.x} y={menuPos.y} items={menuItems()} onclose={() => (menuPos = null)} />
{/if}

{#if confirmingDelete}
  <ConfirmDialog
    title={t("confirm.delete.title")}
    message={t("confirm.delete.message", { name: task.display_name })}
    confirmLabel={t("menu.delete")}
    danger
    onconfirm={() => {
      confirmingDelete = false;
      void taskStore.deleteTask(task.name);
    }}
    oncancel={() => (confirmingDelete = false)}
  />
{/if}

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    cursor: default;
    border-bottom: 1px solid var(--border);
  }
  .row:hover {
    background: var(--surface-hover);
  }
  .row.selected {
    background: var(--accent);
  }
  .row.selected .name,
  .row.selected .trigger,
  .row.selected .line2,
  .row.selected .muted {
    color: #fff;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .dot-ok {
    background: var(--status-ok);
  }
  .dot-failed {
    background: var(--status-failed);
  }
  .dot-running {
    background: var(--status-running);
    animation: pulse 1.2s ease-in-out infinite;
  }
  .dot-never {
    background: var(--status-gray);
  }
  /* 已停止：实心灰。和"未启用"的空心圈区分——停了但还装着，和没装不是一回事。 */
  .dot-stopped {
    background: var(--status-gray);
  }
  .dot-disabled {
    background: transparent;
    border: 1.5px solid var(--status-gray);
  }
  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.35; }
  }
  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .line1 {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .name {
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .trigger {
    color: var(--muted);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .line2 {
    font-size: 12px;
    color: var(--muted);
  }
  .sep {
    margin: 0 4px;
  }
  .muted {
    color: var(--muted);
  }
  /* 停用标记：不是报错，所以不用红色；但要比旁边的灰字重一点，
     否则"已停用"和"3 小时前"看上去是同一类信息。 */
  .off {
    padding: 1px 5px;
    border-radius: 3px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--fg);
    font-size: 11px;
  }
  .row.selected .off {
    border-color: rgba(255, 255, 255, 0.5);
    background: transparent;
  }
  .play {
    flex-shrink: 0;
    width: 24px;
    height: 24px;
    padding: 0;
    border-radius: 50%;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--fg);
    font-size: 10px;
    line-height: 1;
    cursor: default;
  }
  .play:hover:not(:disabled) {
    background: var(--surface-hover);
  }
  .play.stop {
    color: var(--status-failed);
  }
  .play:disabled {
    opacity: 0.5;
  }
</style>
