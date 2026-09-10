<script lang="ts">
  // 一个开关。轨道 + 滑块，开着是强调色。
  //
  // 原来列表行里是一个光秃秃的 `<input type="checkbox">`：没有标签、没有
  // tooltip，谁也猜不出点下去会发生什么。`label` 同时当 aria-label 和 title，
  // 写的是「当前状态 + 点击后果」（「已启用，点击停用」），而不是一个名词。
  //
  // 用 `role="switch"` 的按钮而不是 checkbox：读屏软件会念"开/关"，而
  // "选中/未选中"说的是另一回事。
  let {
    checked,
    label,
    small = false,
    selected = false,
    disabled = false,
    ontoggle,
  }: {
    checked: boolean;
    /** aria-label 和 title，两者用同一句话。 */
    label: string;
    /** 详情页里那个稍小一号的。 */
    small?: boolean;
    /** 放在高亮（强调色底）的行里：轨道要换成白的才看得见。 */
    selected?: boolean;
    disabled?: boolean;
    ontoggle: () => void;
  } = $props();

  function onclick(e: MouseEvent) {
    // 列表行整行可点（点行 = 选中任务），开关不能顺带把行也点了。
    e.stopPropagation();
    ontoggle();
  }
</script>

<button
  type="button"
  class="switch"
  class:on={checked}
  class:small
  class:selected
  role="switch"
  aria-checked={checked}
  aria-label={label}
  title={label}
  {disabled}
  {onclick}
>
  <span class="knob"></span>
</button>

<style>
  .switch {
    flex-shrink: 0;
    width: 34px;
    height: 20px;
    padding: 0;
    border-radius: 10px;
    border: 1px solid var(--border);
    background: var(--surface);
    cursor: default;
    display: inline-flex;
    align-items: center;
    transition:
      background 120ms ease,
      border-color 120ms ease;
  }
  .knob {
    width: 14px;
    height: 14px;
    margin-left: 2px;
    border-radius: 50%;
    background: var(--status-gray);
    transition:
      transform 120ms ease,
      background 120ms ease;
  }
  .switch.on {
    background: var(--accent);
    border-color: var(--accent);
  }
  .switch.on .knob {
    background: #fff;
    transform: translateX(14px);
  }
  .switch.small {
    width: 30px;
    height: 18px;
  }
  .switch.small .knob {
    width: 12px;
    height: 12px;
  }
  .switch.small.on .knob {
    transform: translateX(12px);
  }
  /* 高亮行（强调色底）里反过来：白轨道、强调色滑块。 */
  .switch.selected {
    border-color: rgba(255, 255, 255, 0.6);
  }
  .switch.selected.on {
    background: #fff;
    border-color: #fff;
  }
  .switch.selected.on .knob {
    background: var(--accent);
  }
  .switch:disabled {
    opacity: 0.5;
  }
</style>
