<script lang="ts" module>
  export interface MenuItem {
    label: string;
    action: () => void;
    disabled?: boolean;
    title?: string;
    danger?: boolean;
    separatorBefore?: boolean;
  }
</script>

<script lang="ts">
  let {
    x,
    y,
    items,
    onclose,
  }: { x: number; y: number; items: MenuItem[]; onclose: () => void } = $props();

  let menuEl: HTMLDivElement | undefined = $state();

  function handleClick(item: MenuItem) {
    if (item.disabled) return;
    item.action();
    onclose();
  }

  function onWindowKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
  }

  function onWindowMousedown(e: MouseEvent) {
    if (menuEl && !menuEl.contains(e.target as Node)) onclose();
  }
</script>

<svelte:window onkeydown={onWindowKeydown} onmousedown={onWindowMousedown} />

<div class="menu" style={`left:${x}px; top:${y}px;`} bind:this={menuEl} role="menu">
  {#each items as item (item.label)}
    {#if item.separatorBefore}
      <div class="sep"></div>
    {/if}
    <button
      class="item"
      class:danger={item.danger}
      disabled={item.disabled}
      title={item.title}
      onclick={() => handleClick(item)}
    >
      {item.label}
    </button>
  {/each}
</div>

<style>
  .menu {
    position: fixed;
    z-index: 1000;
    min-width: 180px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.2);
    padding: 4px;
    display: flex;
    flex-direction: column;
  }
  .item {
    all: unset;
    box-sizing: border-box;
    padding: 6px 10px;
    border-radius: 5px;
    font-size: 13px;
    color: var(--fg);
    cursor: default;
  }
  .item:hover:not(:disabled) {
    background: var(--accent);
    color: #fff;
  }
  .item:disabled {
    color: var(--muted);
    cursor: not-allowed;
  }
  .item.danger {
    color: #d7443e;
  }
  .item.danger:hover:not(:disabled) {
    background: #d7443e;
    color: #fff;
  }
  .sep {
    height: 1px;
    background: var(--border);
    margin: 4px 6px;
  }
</style>
