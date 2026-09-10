<script lang="ts">
  // 两组：Launchkeeper 自己管的任务，和机器上别人写的 LaunchAgents
  // （docs/M3-design.md §3.4，设计稿画板「主界面」）。搜索和标签对两组都生效
  // ——外部 agent 没有标签，所以一选标签它们就整组让位（lib/external.ts）。
  import { taskStore } from "../lib/stores/tasks.svelte";
  import {
    groupExternal,
    readExpanded,
    shouldForceExpand,
    writeExpanded,
  } from "../lib/external";
  import { locale, t } from "../lib/i18n";
  import { SORT_LABEL, SORT_MODES, sortMode, sortTasks } from "../lib/sort";
  import TaskRow from "./TaskRow.svelte";
  import ExternalRow from "./ExternalRow.svelte";
  import HelpDot from "./HelpDot.svelte";

  let { onedit }: { onedit: (name: string) => void } = $props();

  // 排序（M4 §2）在这里做，不在 store 里：排序键里有语言（名字的字母序）和
  // "现在几点"（下次触发），前者是 `locale.current`、后者每次重算都该重新取，
  // 而这两样正好都是"画这一屏时"的事。列表每 2 秒被后端换一次，跟着重算一次
  // 几十条的排序不值一提。
  const tasks = $derived(
    sortTasks(taskStore.filtered, taskStore.listSort, new Date(), locale.current),
  );
  const agents = $derived(taskStore.filteredExternal);
  const nothingAtAll = $derived(
    taskStore.tasks.length === 0 && taskStore.externalAgents.length === 0,
  );

  // 「其他 LaunchAgents」分两堆：能接管的在上面，不支持接管的收进一行
  // 「还有 N 个不支持接管 ▸」里，默认收起（M3 §3.4 追补）。这一组多半是各种
  // App 装的更新器，一屏灰字会把用户自己手写的那几个 plist 埋掉。
  const groups = $derived(groupExternal(agents));
  // 用户自己点开/收起的状态，记在 localStorage 里（读写都 try/catch）。
  let userExpanded = $state(readExpanded());
  // 搜索命中的全在折叠的那一堆里时强制展开，否则搜索看上去什么也没找到。
  const forceExpanded = $derived(shouldForceExpand(taskStore.search, groups));
  const othersExpanded = $derived(userExpanded || forceExpanded);

  function toggleOthers() {
    userExpanded = !othersExpanded;
    writeExpanded(userExpanded);
  }

  // 分组标题上的条目数（M4 §3）。搜索或标签筛选生效时管理组写「匹配 x / 共 y」
  // ——只写一个数字的话，用户看到的是一个变小了的列表和一个不知道从哪儿来的
  // 数，说不清是"只有这么多"还是"筛掉了一些"。
  const managedCount = $derived(
    taskStore.filtering
      ? t("list.group.count_filtered", {
          n: tasks.length,
          total: taskStore.tasks.length,
        })
      : t("list.group.count", { n: tasks.length }),
  );

  function chooseSort(e: Event & { currentTarget: HTMLSelectElement }) {
    void taskStore.setListSort(sortMode(e.currentTarget.value));
  }
</script>

<div class="list">
  {#if taskStore.loading && taskStore.tasks.length === 0}
    <p class="empty muted">{t("common.loading")}</p>
  {:else if nothingAtAll}
    <div class="empty">
      <p>{t("list.empty.title")}</p>
      <p class="muted">{t("list.empty.hint")}</p>
    </div>
  {:else if tasks.length === 0 && agents.length === 0}
    <p class="empty muted">{t("list.empty.no_match")}</p>
  {:else}
    {#if tasks.length > 0}
      <div class="group">
        <span>{t("list.group.managed")}</span>
        <span class="count">{managedCount}</span>
        <!-- 排序下拉框（M4 §2）：四项、每项两三个字，用原生 <select> 就够，
             它自带键盘操作和 VoiceOver 的读法。外面套一层只为那个小三角：
             `appearance: none` 把系统自带的箭头也去掉了，而没有箭头的一行灰字
             看不出是可以点的。 -->
        <span class="sort-wrap">
          <select
            class="sort"
            value={taskStore.listSort}
            onchange={chooseSort}
            title={t("list.sort.title")}
            aria-label={t("list.sort.label")}
          >
            {#each SORT_MODES as mode (mode)}
              <option value={mode}>{t(SORT_LABEL[mode])}</option>
            {/each}
          </select>
        </span>
        <span class="spacer"></span>
      </div>
      {#each tasks as task (task.name)}
        <TaskRow
          {task}
          selected={taskStore.selectedName === task.name}
          onselect={() => taskStore.select(task.name)}
          onedit={() => onedit(task.name)}
        />
      {/each}
    {/if}
    {#if agents.length > 0}
      <div class="group">
        <span>{t("list.group.external")}</span>
        <span class="count">{t("list.group.count", { n: agents.length })}</span>
        <span class="note">{t("list.group.external_note")}</span>
        <span class="spacer"></span>
        <!-- 这一组不在后台扫描里（列一次要读一遍 ~/Library/LaunchAgents），
             所以给它一个自己的刷新。 -->
        <button
          class="reload"
          onclick={() => void taskStore.loadExternal()}
          disabled={taskStore.externalLoading}
          title={t("list.group.rescan_title")}
        >
          {taskStore.externalLoading ? t("list.group.scanning") : t("common.refresh")}
        </button>
      </div>
      {#each groups.adoptable as agent (agent.label)}
        <ExternalRow {agent} selected={taskStore.selectedExternal === agent.label} />
      {/each}
      {#if groups.others.length > 0}
        <!-- 折叠行 + 一个「?」：「不支持接管」说的是结果，不是原因，而原因
             （plist 用了表达不了的触发方式，或者程序在 App 包/系统目录里）
             是用户唯一能拿来判断"那我该怎么办"的东西。两个控件，所以外面套
             一层，不能把按钮塞进按钮里。 -->
        <div class="more-row">
          <button
            class="more"
            aria-expanded={othersExpanded}
            title={othersExpanded ? t("list.group.more_collapse") : t("list.group.more_expand")}
            onclick={toggleOthers}
          >
            <span class="caret" class:open={othersExpanded}>▸</span>
            <span>{t("list.group.more_collapsed", { n: groups.others.length })}</span>
          </button>
          <HelpDot
            text={t("list.group.more_help")}
            label={t("external.why_not_adoptable")}
          />
        </div>
        {#if othersExpanded}
          {#each groups.others as agent (agent.label)}
            <ExternalRow {agent} selected={taskStore.selectedExternal === agent.label} />
          {/each}
        {/if}
      {/if}
    {/if}
  {/if}
</div>

<style>
  .list {
    overflow-y: auto;
    height: 100%;
  }
  .group {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 12px 12px 6px;
    font-size: 11px;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .note {
    text-transform: none;
    letter-spacing: 0;
  }
  /* 条目数（M4 §3）：和分组标题同一行，但不跟着它变大写。 */
  .count {
    text-transform: none;
    letter-spacing: 0;
    font-variant-numeric: tabular-nums;
  }
  /* 排序下拉框（M4 §2）：压到和分组标题一样的分量，不喧宾夺主。 */
  .sort-wrap {
    position: relative;
    display: inline-flex;
    align-items: center;
  }
  .sort-wrap::after {
    content: "▾";
    position: absolute;
    right: 5px;
    font-size: 9px;
    color: var(--muted);
    pointer-events: none;
  }
  .sort {
    appearance: none;
    border: 1px solid transparent;
    border-radius: 5px;
    background: transparent;
    color: var(--muted);
    font: inherit;
    text-transform: none;
    letter-spacing: 0;
    padding: 1px 15px 1px 4px;
    cursor: default;
  }
  .sort:hover {
    border-color: var(--border);
    background: var(--surface);
    color: var(--fg);
  }
  .sort:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .spacer {
    flex: 1;
  }
  .reload {
    all: unset;
    text-transform: none;
    letter-spacing: 0;
    color: var(--accent);
    cursor: default;
  }
  .reload:disabled {
    color: var(--muted);
  }
  /* 折叠行：一行灰字加一个转向的三角，点哪儿都能开合。右端另有一个「?」，
     所以边框和 hover 归外面这一层，按钮自己只占左边那一片。 */
  .more-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-right: 12px;
    border-bottom: 1px solid var(--border);
  }
  .more-row:hover {
    background: var(--surface-hover);
  }
  .more {
    all: unset;
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
    box-sizing: border-box;
    padding: 7px 12px;
    font-size: 12px;
    color: var(--muted);
    cursor: default;
  }
  .caret {
    display: inline-block;
    font-size: 10px;
    transition: transform 120ms ease;
  }
  .caret.open {
    transform: rotate(90deg);
  }
  .empty {
    padding: 30px 16px;
    text-align: center;
  }
  .muted {
    color: var(--muted);
  }
</style>
