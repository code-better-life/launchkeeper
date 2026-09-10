<script lang="ts">
  // 「配置」页的只读概览（docs/M3-design.md §3.4 追补）。
  //
  // 点开一个任务先看到的是这一页：一列键/值，没有一个输入框。要改再点上面的
  // 「编辑」，那时才换成 TaskForm。行的内容全部由 lib/summary.ts 的纯函数
  // 拼好（那边有单测），这里只负责画。
  import type { TaskInput } from "../lib/bindings";
  import { summaryRows } from "../lib/summary";
  import { locale, t } from "../lib/i18n";
  import { taskStore } from "../lib/stores/tasks.svelte";
  import ToggleSwitch from "./ToggleSwitch.svelte";

  let {
    task,
    triggerText,
    logDir,
    adoptedFrom,
    service = false,
    enabled = false,
  }: {
    task: TaskInput;
    triggerText: string;
    logDir: string | null;
    adoptedFrom: string | null;
    /** 常驻服务（Trigger::Manual）。 */
    service?: boolean;
    /** plist 装进 launchd 了没有。 */
    enabled?: boolean;
  } = $props();

  // 常驻服务的行上没有启用开关（那一行的启动/停止按钮管的是这次进程，两个
  // 控件摆在一起像是两个"停止"），所以它挪到这里来：这是唯一一个既有地方
  // 说清楚"这个 job 还在不在 launchd 里"，又有地方放开关的位置。
  const toggleLabel = $derived(enabled ? t("row.toggle_on") : t("row.toggle_off"));

  const rows = $derived(
    summaryRows(task, { triggerText, logDir, adoptedFrom }, locale.current),
  );
</script>

<dl class="summary">
  {#if service}
    <dt>{t("summary.start_at_login")}</dt>
    <dd class="switch-cell">
      <ToggleSwitch
        checked={enabled}
        label={toggleLabel}
        small
        ontoggle={() => void taskStore.setEnabled(task.name, !enabled)}
      />
      <span>{enabled ? t("summary.on") : t("summary.off")}</span>
      <span class="note">{t("summary.start_at_login_note")}</span>
    </dd>
  {/if}
  {#each rows as row (row.label)}
    <dt>{t(row.label)}</dt>
    <dd class:mono={row.mono} class:multiline={row.multiline}>{row.value}</dd>
  {/each}
</dl>

<style>
  .summary {
    display: grid;
    grid-template-columns: 108px 1fr;
    gap: 10px 14px;
    margin: 0;
    padding: 2px 0 20px;
    align-items: baseline;
  }
  dt {
    font-size: 12px;
    color: var(--muted);
    text-align: right;
    overflow-wrap: anywhere;
  }
  dd {
    margin: 0;
    font-size: 13px;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .switch-cell {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .note {
    color: var(--muted);
    font-size: 12px;
  }
  .mono {
    font-family: "SF Mono", Menlo, monospace;
    font-size: 12px;
  }
  /* 环境变量一行一个：值里带的换行要留住。 */
  .multiline {
    white-space: pre-wrap;
  }
</style>
