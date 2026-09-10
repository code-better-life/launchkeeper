<script lang="ts">
  // 「运行方式」下拉框（设计稿画板「运行方式选择器」）。
  //
  // 原生 <select> 撑不住这个形状：每行是「名字 + 版本 + 出处 + 路径」两级文字，
  // 还要分组、要有一行灰掉的占位说明、最后要有一个「自定义路径…」。所以自己画
  // 一个 listbox，但键盘行为按 listbox 的规矩来：上下键移动、回车选中、Esc 关闭
  // 并把焦点还给触发按钮、点外面也关。
  import type { InterpreterView } from "../lib/bindings";
  import { customInterpreter, groupInterpreters, interpreterDetail, interpreterLabel } from "../lib/interpreter";
  import { locale, t } from "../lib/i18n";

  let {
    value = $bindable(),
    options,
    loading = false,
    labelledby,
    onpicked,
  }: {
    value: InterpreterView | null;
    options: InterpreterView[];
    loading?: boolean;
    /** 表单里那行「运行方式」文字的 id，用来给触发按钮和列表做无障碍标注。 */
    labelledby?: string;
    /** 用户自己选了一项（不是扫描结果自动填的）。 */
    onpicked?: () => void;
  } = $props();

  let open = $state(false);
  let active = $state(-1);
  let customOpen = $state(false);
  let customPath = $state("");
  let triggerEl = $state<HTMLButtonElement | null>(null);
  let listEl = $state<HTMLDivElement | null>(null);
  let customEl = $state<HTMLInputElement | null>(null);

  // 每行要有一个稳定的 id：aria-activedescendant 靠它告诉读屏软件"现在停在
  // 哪一行"，滚动也靠它把那一行找出来。一个表单里可能有多个 picker，所以带上
  // 一个实例前缀。
  const uid = `interp-${Math.random().toString(36).slice(2, 8)}`;
  const rowId = (i: number) => `${uid}-row-${i}`;

  const groups = $derived(groupInterpreters(options));
  // 「项目内」那一格是否有东西可放。推荐项会被挪进「推荐」，所以不能只看
  // groups.project——项目自己的 .venv 恰好就是推荐项时，那一格会空着，再显示
  // 「此项目没有 .venv」就成了睁眼说瞎话。
  const hasProject = $derived(options.some((o) => o.origin === "project_venv"));
  /**
   * 键盘上下键走的顺序。必须和真正渲染出来的顺序一致——渲染是按分组来的，而
   * `options` 里当前选中项可能被塞在最前面（它不在扫描结果里时），两者并不
   * 相同；直接用 `options` 会让 ↓ 跳到看上去不相邻的一行上。
   */
  const flat = $derived([
    ...groups.recommended,
    ...groups.project,
    ...groups.path,
    null, // 自定义路径…
  ]);

  function toggle() {
    if (open) {
      close();
      return;
    }
    open = true;
    active = Math.max(
      0,
      flat.findIndex((o) => o !== null && o.id === value?.id),
    );
    // 列表挂载后再抓焦点，键盘事件才有地方落。
    queueMicrotask(() => listEl?.focus());
  }

  function close(focusTrigger = false) {
    open = false;
    active = -1;
    if (focusTrigger) triggerEl?.focus();
  }

  function choose(o: InterpreterView | null) {
    if (o === null) {
      close(false);
      customOpen = true;
      // 按出处而不是 kind 判断"这是手填的"：手填一个 uv 的路径会被认成
      // uv run（`customInterpreter`），kind 就不是 custom 了。
      customPath = value?.origin === "custom" ? value.program : "";
      queueMicrotask(() => customEl?.focus());
      return;
    }
    value = o;
    customOpen = false;
    onpicked?.();
    close(true);
  }

  function applyCustom() {
    const p = customPath.trim();
    if (!p) {
      customOpen = false;
      return;
    }
    value = customInterpreter(p, locale.current);
    customOpen = false;
    onpicked?.();
    triggerEl?.focus();
  }

  function onKeydown(e: KeyboardEvent) {
    switch (e.key) {
      case "Escape":
        e.preventDefault();
        close(true);
        break;
      case "ArrowDown":
        e.preventDefault();
        active = (active + 1) % flat.length;
        break;
      case "ArrowUp":
        e.preventDefault();
        active = (active - 1 + flat.length) % flat.length;
        break;
      case "Home":
        e.preventDefault();
        active = 0;
        break;
      case "End":
        e.preventDefault();
        active = flat.length - 1;
        break;
      case "Enter":
      case " ":
        e.preventDefault();
        if (active >= 0) choose(flat[active]);
        break;
      case "Tab":
        // 焦点这会儿在列表上，列表马上要消失：不还给触发按钮的话，Tab 之后
        // 焦点会掉到 <body>，再按一下要从整个页面的开头重新走。
        e.preventDefault();
        close(true);
        break;
    }
  }

  function onTriggerKeydown(e: KeyboardEvent) {
    if (!open && (e.key === "ArrowDown" || e.key === "Enter" || e.key === " ")) {
      e.preventDefault();
      toggle();
    }
  }

  // 点到别处就收起来。用 pointerdown 而不是 click：click 会先在按钮上触发一次
  // toggle，再被这里关掉，看着像没反应。
  $effect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      const t = e.target as Node;
      if (listEl?.contains(t) || triggerEl?.contains(t)) return;
      close();
    };
    window.addEventListener("pointerdown", onDown, true);
    return () => window.removeEventListener("pointerdown", onDown, true);
  });

  // 键盘走到的那一行要能看见：列表最多 320px 高，PATH 里的解释器多起来
  // 一屏放不下。`block: "nearest"` 只在需要时滚动，不会每按一下都把列表
  // 甩到中间。
  $effect(() => {
    if (!open || active < 0) return;
    document.getElementById(rowId(active))?.scrollIntoView({ block: "nearest" });
  });

  function index(o: InterpreterView): number {
    return flat.findIndex((x) => x !== null && x.id === o.id);
  }
</script>

{#snippet rows(list: InterpreterView[])}
  {#each list as o (o.id)}
    <button
      type="button"
      class="row"
      class:active={active === index(o)}
      id={rowId(index(o))}
      role="option"
      aria-selected={value?.id === o.id}
      tabindex="-1"
      onclick={() => choose(o)}
      onmousemove={() => (active = index(o))}
    >
      <!-- label / detail 是后端拼好的（scan_interpreters），本期仍然只有中文
           （docs/M3-design.md §5）。 -->
      <span class="name">{interpreterLabel(o, locale.current)}</span>
      <span class="detail">{interpreterDetail(o, locale.current)}</span>
      {#if o.recommended}<span class="badge">{t("interp.badge_default")}</span>{/if}
    </button>
  {/each}
{/snippet}

<div class="picker">
  <button
    type="button"
    class="trigger"
    class:open
    bind:this={triggerEl}
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-labelledby={labelledby ? `${labelledby} ${uid}-value` : undefined}
    aria-controls={open ? `${uid}-list` : undefined}
    onclick={toggle}
    onkeydown={onTriggerKeydown}
  >
    {#if value}
      <span class="chosen" id="{uid}-value">
        <span class="name">{interpreterLabel(value, locale.current)}</span>
        <span class="detail">{interpreterDetail(value, locale.current)}</span>
      </span>
    {:else if loading}
      <span class="detail" id="{uid}-value">{t("interp.scanning")}</span>
    {:else}
      <span class="detail" id="{uid}-value">{t("interp.choose")}</span>
    {/if}
    <svg width="10" height="6" viewBox="0 0 10 6" aria-hidden="true">
      <path
        d={open ? "M1 5l4-4 4 4" : "M1 1l4 4 4-4"}
        stroke="currentColor"
        stroke-width="1.5"
        fill="none"
      />
    </svg>
  </button>

  {#if open}
    <div
      class="menu"
      id="{uid}-list"
      role="listbox"
      aria-label={labelledby ? undefined : t("form.run_with")}
      aria-labelledby={labelledby}
      aria-activedescendant={active >= 0 ? rowId(active) : undefined}
      tabindex="-1"
      bind:this={listEl}
      onkeydown={onKeydown}
    >
      {#if loading && options.length === 0}
        <div class="empty" role="presentation">{t("interp.scanning")}</div>
      {/if}

      {#if groups.recommended.length > 0}
        <div class="group" role="presentation">{t("interp.group.recommended")}</div>
        {@render rows(groups.recommended)}
      {/if}

      <!-- 「项目内」这一格空着的时候，说明写在分组标题里，而不是摆一行假的
           option：listbox 里除了 option 不该有别的东西，键盘也不该在一行
           点不动的东西上停下来。 -->
      {#if groups.project.length > 0}
        <div class="group" role="presentation">{t("interp.group.project")}</div>
        {@render rows(groups.project)}
      {:else if !hasProject}
        <div class="group empty-group" role="presentation">
          {t("interp.group.project_empty")}
        </div>
      {/if}

      {#if groups.path.length > 0}
        <div class="group" role="presentation">{t("interp.group.path")}</div>
        {@render rows(groups.path)}
      {/if}

      <!-- 「自定义路径…」是个动作（展开一个输入框），不是一个值，所以
           aria-selected 恒为 false：当前选中的是什么，由上面的触发按钮报。 -->
      <button
        type="button"
        class="row custom"
        class:active={active === flat.length - 1}
        id={rowId(flat.length - 1)}
        role="option"
        aria-selected={false}
        tabindex="-1"
        onclick={() => choose(null)}
        onmousemove={() => (active = flat.length - 1)}
      >
        <span class="link">{t("interp.custom")}</span>
      </button>
    </div>
  {/if}
</div>

{#if customOpen}
  <div class="custom-row">
    <input
      type="text"
      bind:this={customEl}
      bind:value={customPath}
      placeholder={t("interp.custom_placeholder")}
      onkeydown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          applyCustom();
        } else if (e.key === "Escape") {
          e.preventDefault();
          customOpen = false;
          triggerEl?.focus();
        }
      }}
    />
    <button type="button" class="mini" onclick={applyCustom}>
      {t("interp.custom_confirm")}
    </button>
  </div>
{/if}

<style>
  .picker {
    position: relative;
    width: 100%;
    max-width: 420px;
  }
  .trigger {
    width: 100%;
    height: 30px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    color: var(--fg);
    font-size: 13px;
    cursor: default;
    text-align: left;
  }
  .trigger.open,
  .trigger:focus-visible {
    border-color: var(--accent);
    outline: none;
  }
  .chosen {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .name {
    font-weight: 500;
    white-space: nowrap;
  }
  .detail {
    color: var(--muted);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .menu {
    position: absolute;
    z-index: 20;
    top: calc(100% + 4px);
    left: 0;
    width: 100%;
    min-width: 420px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.18);
    overflow: hidden auto;
    max-height: 320px;
    outline: none;
  }
  .group {
    padding: 6px 12px;
    font-size: 11px;
    color: var(--muted);
    background: var(--surface);
  }
  .row {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 12px;
    border: 0;
    background: transparent;
    color: var(--fg);
    font-size: 13px;
    text-align: left;
    cursor: default;
  }
  .row .name {
    width: 90px;
    flex-shrink: 0;
  }
  .row .detail {
    flex: 1;
  }
  .row.active {
    background: var(--accent);
    color: #fff;
  }
  .row.active .detail,
  .row.active .link {
    color: #fff;
  }
  .group.empty-group {
    opacity: 0.7;
  }
  .row.custom {
    border-top: 1px solid var(--border);
  }
  .link {
    color: var(--accent);
  }
  .badge {
    font-size: 11px;
    border: 1px solid currentColor;
    border-radius: 4px;
    padding: 0 6px;
    opacity: 0.8;
    flex-shrink: 0;
  }
  .empty {
    padding: 10px 12px;
    color: var(--muted);
    font-size: 12px;
  }
  .custom-row {
    display: flex;
    gap: 6px;
    margin-top: 6px;
    max-width: 420px;
  }
  .custom-row input {
    flex: 1;
    padding: 5px 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    color: var(--fg);
    font-size: 13px;
    font-family: inherit;
  }
  .mini {
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--fg);
    border-radius: 5px;
    padding: 3px 10px;
    font-size: 12px;
    cursor: default;
  }
</style>
