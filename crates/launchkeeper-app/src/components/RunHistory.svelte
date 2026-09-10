<script lang="ts">
  import { untrack } from "svelte";
  import type { RunView } from "../lib/bindings";
  import { api } from "../lib/api";
  import { taskStore } from "../lib/stores/tasks.svelte";
  import { formatDuration, formatExit, relativeTime, stopReasonText } from "../lib/format";
  import { locale, t } from "../lib/i18n";
  import LogView from "./LogView.svelte";

  let { taskName }: { taskName: string } = $props();

  let runs = $state<RunView[]>([]);
  let loading = $state(true);
  let expandedId = $state<number | null>(null);

  async function load(showSpinner: boolean) {
    if (showSpinner) loading = true;
    try {
      runs = await api.listRuns(taskName, 50);
    } catch (e) {
      taskStore.showError(e);
    } finally {
      loading = false;
    }
  }

  // 换任务时清空展开状态并显示加载中。
  $effect(() => {
    void taskName;
    untrack(() => {
      expandedId = null;
    });
    void load(true);
  });

  // 后台扫描（§3.4）每刷新一次任务列表就重新拉一次运行历史：立即运行产生的
  // 新记录、正在运行的那条跑完，都会在这里跟上，而不是停在打开这个 tab 时的
  // 快照上。run.running 由此变 false，LogView 的轮询也随之停下。
  let seenRevision = taskStore.revision;
  $effect(() => {
    const rev = taskStore.revision;
    if (rev === seenRevision) return;
    seenRevision = rev;
    void load(false);
  });

  function toggle(id: number) {
    expandedId = expandedId === id ? null : id;
  }

  function fmtTime(iso: string): string {
    return new Date(iso).toLocaleString(locale.current);
  }

  /**
   * 这一条算不算"失败"（结果列标红）。
   *
   * 被用户停掉的服务退出码是空的（子进程被信号打死），但那是它该有的样子，
   * 不是故障；标红只会让人以为常驻服务每次停止都出了问题。
   */
  function isFailure(run: RunView): boolean {
    if (run.running) return false;
    if (run.stop_reason === "stopped") return false;
    return run.exit_code !== 0;
  }
</script>

<div class="history">
  {#if loading}
    <p class="muted">{t("common.loading")}</p>
  {:else if runs.length === 0}
    <p class="muted">{t("history.empty")}</p>
  {:else}
    <table>
      <thead>
        <tr>
          <th>{t("history.col.id")}</th>
          <th>{t("history.col.started")}</th>
          <th>{t("history.col.duration")}</th>
          <th>{t("history.col.result")}</th>
          <th>{t("history.col.stop")}</th>
          <th>{t("history.col.source")}</th>
        </tr>
      </thead>
      <tbody>
        {#each runs as run (run.id)}
          <tr class="row" onclick={() => toggle(run.id)}>
            <td>{run.id}</td>
            <td title={fmtTime(run.started_at)}>
              {relativeTime(run.started_at, new Date(), locale.current)}
            </td>
            <td>{formatDuration(run.duration_ms, locale.current)}</td>
            <!-- 手动停止的服务退出码是空的，那不是失败，别标红。 -->
            <td class:ok={run.exit_code === 0} class:failed={isFailure(run)}>
              {formatExit(run.exit_code, run.running, run.finished_at, locale.current, run.stop_reason)}
            </td>
            <td>
              {#if stopReasonText(run.stop_reason, locale.current)}
                <span class="badge" class:warn={run.stop_reason === "timeout"}>
                  {stopReasonText(run.stop_reason, locale.current)}
                </span>
              {:else}
                <span class="muted-cell">{t("common.placeholder")}</span>
              {/if}
            </td>
            <td>
              {run.trigger_kind === "manual"
                ? t("history.source.manual")
                : t("history.source.scheduled")}
            </td>
          </tr>
          {#if expandedId === run.id}
            <tr class="detail-row">
              <td colspan="6">
                <div class="detail">
                  <div>
                    <h4>{t("history.stdout")}</h4>
                    <LogView runId={run.id} stream="stdout" running={run.running} />
                  </div>
                  <div>
                    <h4>{t("history.stderr")}</h4>
                    <LogView runId={run.id} stream="stderr" running={run.running} />
                  </div>
                </div>
              </td>
            </tr>
          {/if}
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .history {
    overflow-x: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
  }
  th {
    text-align: left;
    color: var(--muted);
    font-weight: 500;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
  }
  td {
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
  }
  .row {
    cursor: default;
  }
  .row:hover {
    background: var(--surface-hover);
  }
  .ok {
    color: var(--status-ok);
  }
  .failed {
    color: var(--status-failed);
  }
  .detail-row td {
    background: var(--surface);
  }
  .detail {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
    padding: 8px 4px;
  }
  h4 {
    margin: 0 0 6px;
    font-size: 12px;
    color: var(--muted);
    font-weight: 500;
  }
  .muted {
    color: var(--muted);
    padding: 8px 2px;
  }
  .muted-cell {
    color: var(--muted);
  }
  .badge {
    display: inline-block;
    padding: 1px 7px;
    border-radius: 10px;
    border: 1px solid var(--border);
    background: var(--surface);
    font-size: 11.5px;
    white-space: nowrap;
  }
  .badge.warn {
    color: var(--status-failed);
    border-color: var(--status-failed);
  }
</style>
