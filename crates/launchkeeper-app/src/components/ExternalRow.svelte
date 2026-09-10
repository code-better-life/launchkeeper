<script lang="ts">
  import { describeTrigger } from "../lib/describeTrigger";
  // 列表里「其他 LaunchAgents」那一组的行（docs/M3-design.md §3.4，设计稿
  // 画板「主界面」）。和 TaskRow 长得像但不是同一件东西：这里没有运行历史、
  // 没有开关（启用/禁用只在右键菜单和详情页里，因为它改的是别人的 job），
  // 多了一个行内的「接管」按钮。
  import type { ExternalAgentView } from "../lib/bindings";
  import { api } from "../lib/api";
  import { taskStore } from "../lib/stores/tasks.svelte";
  import {
    adoptTooltip,
    externalStatusTitle,
    externalSubtitle,
    foreignDisableWarning,
    isForeignAgent,
  } from "../lib/external";
  import { locale, t } from "../lib/i18n";
  import ConfirmDialog from "./ConfirmDialog.svelte";
  import ContextMenu, { type MenuItem } from "./ContextMenu.svelte";
  import HelpDot from "./HelpDot.svelte";

  let {
    agent,
    selected,
  }: {
    agent: ExternalAgentView;
    selected: boolean;
  } = $props();

  /** 接管确认框由 App 统一渲染（详情页的「接管」按钮走的是同一条路）。 */
  function onadopt() {
    taskStore.startAdopt(agent.label);
  }

  let menuPos = $state<{ x: number; y: number } | null>(null);
  /** 禁用别人的 LaunchAgent 之前的确认框（M4）。 */
  let confirmingDisable = $state(false);

  const canAdopt = $derived(agent.adoptable && !agent.managed);

  /**
   * 启用不用问，禁用要问——而且只在这个 agent 是别的软件装的时候问。
   *
   * plist 一个字节都不会动，但那个软件会从此不再被 launchd 拉起来，这是用户
   * 在别人的产品上按下的开关，不该一次误点就生效（M4 §3）。
   */
  function toggleLoaded() {
    if (agent.loaded && isForeignAgent(agent)) {
      confirmingDisable = true;
      return;
    }
    void taskStore.externalSetEnabled(agent.label, !agent.loaded);
  }

  function onselect() {
    taskStore.selectExternal(agent.label);
  }

  function onContextMenu(e: MouseEvent) {
    e.preventDefault();
    onselect();
    menuPos = { x: e.clientX, y: e.clientY };
  }

  function menuItems(): MenuItem[] {
    return [
      {
        label: t("menu.run_now"),
        action: () => void taskStore.externalRun(agent.label),
        disabled: !agent.loaded,
        title: agent.loaded ? undefined : t("external.run_disabled_tip"),
      },
      {
        label: agent.loaded ? t("menu.disable") : t("menu.enable"),
        action: toggleLoaded,
        title: t("external.enable_tip"),
      },
      {
        label: t("menu.adopt"),
        action: onadopt,
        disabled: !canAdopt,
        title: adoptTooltip(agent, locale.current),
        separatorBefore: true,
      },
      {
        label: t("menu.reveal_plist"),
        action: () =>
          void api.revealInFinder(agent.full_path).catch((e) => taskStore.showError(e)),
      },
    ];
  }

  function onAdoptClick(e: MouseEvent) {
    e.stopPropagation();
    onadopt();
  }
</script>

<div
  class="row"
  class:selected
  class:dim={!canAdopt}
  title={canAdopt ? undefined : adoptTooltip(agent, locale.current)}
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
  <!-- 加载了是实心绿点，没加载是空心圈——和任务行的"未启用"用同一个形状。 -->
  <span
    class="dot"
    class:loaded={agent.loaded}
    title={externalStatusTitle(agent, locale.current)}
  ></span>
  <div class="main">
    <div class="line1">
      <span class="name">{agent.label}</span>
      <span class="trigger">{describeTrigger(agent.trigger, locale.current, t("external.trigger_unsupported"))}</span>
    </div>
    <div class="line2">{externalSubtitle(agent, locale.current)}</div>
  </div>
  {#if canAdopt}
    <button class="adopt" onclick={onAdoptClick} title={adoptTooltip(agent, locale.current)}>
      {t("menu.adopt")}
    </button>
  {:else}
    <!-- 接管按钮的位置上，不可接管的行放一个「?」：第二行那句理由是被省略号
         截断的，而理由正是这一行为什么在这儿的全部内容（M4 §3 追补）。 -->
    <HelpDot
      text={adoptTooltip(agent, locale.current)}
      label={agent.managed
        ? t("external.subtitle.managed")
        : t("external.why_not_adoptable")}
    />
  {/if}
</div>

{#if menuPos}
  <ContextMenu x={menuPos.x} y={menuPos.y} items={menuItems()} onclose={() => (menuPos = null)} />
{/if}

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
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    cursor: default;
    border-bottom: 1px solid var(--border);
  }
  /* 不可接管的行灰掉，但仍然可点可选：它是只读展示，不是禁用控件。
     灰的是内容（状态点 + 两行字），不是整行：`opacity` 会把子树整个合成一遍，
     行尾那个「?」和它弹出来的说明也会跟着变淡——而那段说明正是这一行为什么
     灰着的解释，它必须是清楚的（M4 §3 追补）。 */
  .row.dim .dot,
  .row.dim .main {
    opacity: 0.6;
  }
  .row:hover {
    background: var(--surface-hover);
  }
  .row.selected {
    background: var(--accent);
  }
  .row.selected .dot,
  .row.selected .main {
    opacity: 1;
  }
  .row.selected .name,
  .row.selected .trigger,
  .row.selected .line2 {
    color: #fff;
  }
  .row.selected .dot {
    border-color: #fff;
  }
  .row.selected .dot.loaded {
    background: #fff;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
    background: transparent;
    border: 1.5px solid var(--status-gray);
  }
  .dot.loaded {
    background: var(--status-ok);
    border-color: var(--status-ok);
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
  .trigger,
  .line2 {
    color: var(--muted);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .adopt {
    flex-shrink: 0;
    height: 22px;
    padding: 0 10px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--fg);
    font-size: 12px;
    cursor: default;
  }
  .adopt:hover {
    background: var(--surface-hover);
  }
</style>
