<script lang="ts">
  import type { TriggerForm } from "../lib/trigger";
  import { t } from "../lib/i18n";

  let { form = $bindable() }: { form: TriggerForm } = $props();

  /** 0-6，周日在前。标签跟着语言走，所以是 $derived 而不是常量。 */
  const weekdayLabels = $derived([0, 1, 2, 3, 4, 5, 6].map((d) => ({
    day: d,
    label: t(`weekday.short.${d as 0 | 1 | 2 | 3 | 4 | 5 | 6}` as const),
  })));

  function addCalendarEntry() {
    form.calendarEntries = [
      ...form.calendarEntries,
      { hour: 9, minute: 0, weekdays: [], day: null },
    ];
  }

  function removeCalendarEntry(idx: number) {
    form.calendarEntries = form.calendarEntries.filter((_, i) => i !== idx);
  }

  function toggleWeekday(idx: number, day: number) {
    const entry = form.calendarEntries[idx];
    const has = entry.weekdays.includes(day);
    entry.weekdays = has
      ? entry.weekdays.filter((d) => d !== day)
      // 数字要显式比较：默认的 sort 按字符串排，多于个位数时会乱（这里虽然
      // 只有 0-6，但排序函数不该依赖这一点）。
      : [...entry.weekdays, day].sort((a, b) => a - b);
    form.calendarEntries = [...form.calendarEntries];
  }

  function setDay(idx: number, value: string) {
    const entry = form.calendarEntries[idx];
    const raw = value === "" ? Number.NaN : Number(value);
    entry.day = Number.isFinite(raw) ? Math.min(31, Math.max(1, raw)) : null;
    form.calendarEntries = [...form.calendarEntries];
  }

  function setTime(idx: number, field: "hour" | "minute", value: string) {
    const entry = form.calendarEntries[idx];
    const max = field === "hour" ? 23 : 59;
    entry[field] = Math.min(max, Math.max(0, Number(value) || 0));
    form.calendarEntries = [...form.calendarEntries];
  }
</script>

<div class="trigger-editor">
  <div class="kind-row">
    <label>
      <input type="radio" name="trigger-kind" value="at_login" bind:group={form.kind} />
      {t("trigger.at_login")}
    </label>
    <label>
      <input type="radio" name="trigger-kind" value="interval" bind:group={form.kind} />
      {t("trigger.interval")}
    </label>
    <label>
      <input type="radio" name="trigger-kind" value="calendar" bind:group={form.kind} />
      {t("trigger.calendar")}
    </label>
    <label>
      <input type="radio" name="trigger-kind" value="manual" bind:group={form.kind} />
      {t("trigger.manual")}
    </label>
  </div>

  {#if form.kind === "manual"}
    <p class="hint">{t("trigger.manual_hint")}</p>
  {/if}

  {#if form.kind === "interval"}
    <div class="interval-row">
      <span>{t("trigger.every")}</span>
      <input
        type="number"
        min="1"
        bind:value={form.intervalValue}
        class="num"
      />
      <select bind:value={form.intervalUnit}>
        {#each ["seconds", "minutes", "hours"] as const as u (u)}
          <option value={u}>{t(`unit.${u}`)}</option>
        {/each}
      </select>
      <span>{t("trigger.run_once")}</span>
    </div>
  {/if}

  {#if form.kind === "calendar"}
    <div class="calendar-list">
      {#each form.calendarEntries as entry, idx (idx)}
        <div class="calendar-entry">
          <div class="time-row">
            <input
              type="number"
              min="0"
              max="23"
              value={entry.hour}
              class="num small"
              oninput={(e) => setTime(idx, "hour", (e.target as HTMLInputElement).value)}
            />
            <span>{t("trigger.hour_suffix")}</span>
            <input
              type="number"
              min="0"
              max="59"
              value={entry.minute}
              class="num small"
              oninput={(e) => setTime(idx, "minute", (e.target as HTMLInputElement).value)}
            />
            <span>{t("trigger.minute_suffix")}</span>
            {#if form.calendarEntries.length > 1}
              <button type="button" class="remove" onclick={() => removeCalendarEntry(idx)}>
                {t("common.remove")}
              </button>
            {/if}
          </div>
          <div class="weekday-row">
            {#each weekdayLabels as { day, label } (day)}
              <button
                type="button"
                class="weekday"
                class:active={entry.weekdays.includes(day)}
                onclick={() => toggleWeekday(idx, day)}
              >
                {label}
              </button>
            {/each}
            <span class="muted">{t("trigger.weekday_hint")}</span>
          </div>
          <div class="day-row">
            <span>{t("trigger.monthly_prefix")}</span>
            <input
              type="number"
              min="1"
              max="31"
              placeholder={t("trigger.day_placeholder")}
              value={entry.day ?? ""}
              class="num small"
              oninput={(e) => setDay(idx, (e.target as HTMLInputElement).value)}
            />
            <span>{t("trigger.monthly_suffix")}</span>
          </div>
        </div>
      {/each}
      <button type="button" class="add" onclick={addCalendarEntry}>
        {t("trigger.add_entry")}
      </button>
    </div>
  {/if}
</div>

<style>
  .trigger-editor {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .kind-row {
    display: flex;
    gap: 16px;
    flex-wrap: wrap;
  }
  .hint {
    margin: 0;
    font-size: 12px;
    line-height: 1.5;
    color: var(--muted);
  }
  .kind-row label {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 13px;
  }
  .interval-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .num {
    width: 70px;
  }
  .num.small {
    width: 52px;
  }
  select,
  input[type="number"] {
    padding: 4px 6px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--bg);
    color: var(--fg);
    font-size: 13px;
  }
  .calendar-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .calendar-entry {
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 8px 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .time-row,
  .day-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .weekday-row {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-wrap: wrap;
  }
  .weekday {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--fg);
    font-size: 12px;
    cursor: default;
    padding: 0;
  }
  .weekday.active {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .muted {
    color: var(--muted);
    font-size: 12px;
    margin-left: 6px;
  }
  .remove,
  .add {
    align-self: flex-start;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--fg);
    border-radius: 5px;
    padding: 3px 10px;
    font-size: 12px;
    cursor: default;
  }
  .remove:hover,
  .add:hover {
    background: var(--surface-hover);
  }
</style>
