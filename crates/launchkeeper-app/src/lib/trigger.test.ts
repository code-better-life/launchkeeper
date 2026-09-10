import { describe, expect, it } from "vitest";
import {
  defaultTriggerForm,
  triggerFormError,
  formToTrigger,
  intervalToSeconds,
  secondsToInterval,
  triggerToForm,
  type TriggerForm,
} from "./trigger";
import type { Trigger } from "./bindings";

describe("at_login round trip", () => {
  it("form -> trigger -> form", () => {
    const form: TriggerForm = { ...defaultTriggerForm(), kind: "at_login" };
    const trigger = formToTrigger(form);
    expect(trigger).toEqual({ kind: "at_login" });
    const back = triggerToForm(trigger);
    expect(back.kind).toBe("at_login");
  });
});

describe("interval round trip", () => {
  it("converts value+unit to seconds and back", () => {
    expect(intervalToSeconds(2, "hours")).toBe(7200);
    expect(intervalToSeconds(30, "minutes")).toBe(1800);
    expect(intervalToSeconds(45, "seconds")).toBe(45);

    expect(secondsToInterval(7200)).toEqual({ value: 2, unit: "hours" });
    expect(secondsToInterval(1800)).toEqual({ value: 30, unit: "minutes" });
    expect(secondsToInterval(45)).toEqual({ value: 45, unit: "seconds" });
  });

  it("round-trips through formToTrigger/triggerToForm preserving seconds", () => {
    const form: TriggerForm = {
      ...defaultTriggerForm(),
      kind: "interval",
      intervalValue: 6,
      intervalUnit: "hours",
    };
    const trigger = formToTrigger(form);
    expect(trigger).toEqual({ kind: "interval", seconds: 21600 });
    const back = triggerToForm(trigger);
    expect(back.kind).toBe("interval");
    expect(intervalToSeconds(back.intervalValue, back.intervalUnit)).toBe(21600);
  });

  it("enforces a minimum of 1 second", () => {
    expect(intervalToSeconds(0, "seconds")).toBe(1);
  });
});

describe("calendar trigger", () => {
  it("expands a multi-weekday entry into one CalendarEntry per weekday, never day 7", () => {
    const form: TriggerForm = {
      ...defaultTriggerForm(),
      kind: "calendar",
      calendarEntries: [{ hour: 9, minute: 30, weekdays: [1, 3, 5], day: null }],
    };
    const trigger = formToTrigger(form);
    expect(trigger.kind).toBe("calendar");
    if (trigger.kind !== "calendar") throw new Error("unreachable");
    expect(trigger.entries).toHaveLength(3);
    for (const e of trigger.entries) {
      expect(e.hour).toBe(9);
      expect(e.minute).toBe(30);
      expect(e.day).toBeNull();
      expect(e.weekday).not.toBeNull();
      expect(e.weekday).toBeGreaterThanOrEqual(0);
      expect(e.weekday).toBeLessThanOrEqual(6);
    }
    // never emits day 7, even if a caller tries to sneak it in via weekdays
    const noneAreSeven = trigger.entries.every((e) => e.weekday !== 7);
    expect(noneAreSeven).toBe(true);
  });

  it("treats weekday 7 in the input the same as 0 and never re-emits 7", () => {
    const form: TriggerForm = {
      ...defaultTriggerForm(),
      kind: "calendar",
      calendarEntries: [{ hour: 0, minute: 0, weekdays: [7], day: null }],
    };
    const trigger = formToTrigger(form);
    if (trigger.kind !== "calendar") throw new Error("unreachable");
    expect(trigger.entries).toHaveLength(1);
    expect(trigger.entries[0].weekday).toBe(0);
  });

  it("emits a single weekday:null entry when no weekday is selected", () => {
    const form: TriggerForm = {
      ...defaultTriggerForm(),
      kind: "calendar",
      calendarEntries: [{ hour: 21, minute: 0, weekdays: [], day: null }],
    };
    const trigger = formToTrigger(form);
    if (trigger.kind !== "calendar") throw new Error("unreachable");
    expect(trigger.entries).toEqual([{ hour: 21, minute: 0, weekday: null, day: null }]);
  });

  it("carries day-of-month through", () => {
    const form: TriggerForm = {
      ...defaultTriggerForm(),
      kind: "calendar",
      calendarEntries: [{ hour: 8, minute: 0, weekdays: [], day: 1 }],
    };
    const trigger = formToTrigger(form);
    if (trigger.kind !== "calendar") throw new Error("unreachable");
    expect(trigger.entries).toEqual([{ hour: 8, minute: 0, weekday: null, day: 1 }]);
  });

  it("round-trips backend entries with multiple weekdays sharing a time back into one form entry", () => {
    const trigger: Trigger = {
      kind: "calendar",
      entries: [
        { hour: 9, minute: 0, weekday: 1, day: null },
        { hour: 9, minute: 0, weekday: 3, day: null },
        { hour: 9, minute: 0, weekday: 5, day: null },
      ],
    };
    const form = triggerToForm(trigger);
    expect(form.kind).toBe("calendar");
    expect(form.calendarEntries).toHaveLength(1);
    expect(form.calendarEntries[0].weekdays.sort()).toEqual([1, 3, 5]);
    expect(form.calendarEntries[0].hour).toBe(9);
    expect(form.calendarEntries[0].minute).toBe(0);
  });

  it("round-trips distinct hour/day groups into separate form entries", () => {
    const trigger: Trigger = {
      kind: "calendar",
      entries: [
        { hour: 9, minute: 0, weekday: null, day: null },
        { hour: 21, minute: 30, weekday: null, day: 1 },
      ],
    };
    const form = triggerToForm(trigger);
    if (form.kind !== "calendar") throw new Error("unreachable");
    expect(form.calendarEntries).toHaveLength(2);
    expect(form.calendarEntries[0]).toEqual({ hour: 9, minute: 0, weekdays: [], day: null });
    expect(form.calendarEntries[1]).toEqual({ hour: 21, minute: 30, weekdays: [], day: 1 });
  });

  it("normalizes weekday 7 from the backend to 0 when reading it back", () => {
    const trigger: Trigger = {
      kind: "calendar",
      entries: [{ hour: 0, minute: 0, weekday: 7, day: null }],
    };
    const form = triggerToForm(trigger);
    if (form.kind !== "calendar") throw new Error("unreachable");
    expect(form.calendarEntries[0].weekdays).toEqual([0]);
  });

  it("full round trip: form -> trigger -> form preserves the effective schedule", () => {
    const form: TriggerForm = {
      ...defaultTriggerForm(),
      kind: "calendar",
      calendarEntries: [
        { hour: 9, minute: 15, weekdays: [1, 2], day: null },
        { hour: 18, minute: 0, weekdays: [], day: 15 },
      ],
    };
    const trigger = formToTrigger(form);
    const back = triggerToForm(trigger);
    const trigger2 = formToTrigger(back);
    expect(trigger2).toEqual(trigger);
  });
});

describe("triggerFormError", () => {
  it("accepts the defaults", () => {
    expect(triggerFormError(defaultTriggerForm())).toBeNull();
  });

  it("rejects an empty or non-positive interval instead of clamping it", () => {
    const form: TriggerForm = { ...defaultTriggerForm(), kind: "interval" };
    expect(
      triggerFormError({
        ...form,
        intervalValue: null as unknown as number,
      }),
    ).toBe("form.error.interval_number");
    expect(triggerFormError({ ...form, intervalValue: 0 })).toBe("form.error.interval_number");
    expect(
      triggerFormError({ ...form, intervalValue: Number.NaN }),
    ).toBe("form.error.interval_number");
    expect(triggerFormError({ ...form, intervalValue: 5 })).toBeNull();
  });

  it("rejects out-of-range calendar fields", () => {
    const at = (hour: number, minute: number, day: number | null): TriggerForm => ({
      ...defaultTriggerForm(),
      kind: "calendar",
      calendarEntries: [{ hour, minute, weekdays: [], day }],
    });
    expect(triggerFormError(at(9, 0, null))).toBeNull();
    expect(triggerFormError(at(24, 0, null))).toBe("form.error.hour_range");
    expect(triggerFormError(at(9, 60, null))).toBe("form.error.minute_range");
    expect(triggerFormError(at(9, 0, 32))).toBe("form.error.day_range");
    expect(
      triggerFormError(at(null as unknown as number, 0, null)),
    ).toBe("form.error.hour_range");
  });
});

describe("manual (service) round trip", () => {
  it("form -> trigger -> form", () => {
    const form: TriggerForm = { ...defaultTriggerForm(), kind: "manual" };
    const trigger = formToTrigger(form);
    expect(trigger).toEqual({ kind: "manual" });
    expect(triggerToForm(trigger).kind).toBe("manual");
  });

  it("carries no schedule fields into the backend shape", () => {
    // 手动启停的 plist 里三个触发键一个都不该有，所以表单里的间隔/日历
    // 设置必须被完全丢掉，而不是跟着一起发过去。
    const form: TriggerForm = {
      kind: "manual",
      intervalValue: 30,
      intervalUnit: "minutes",
      calendarEntries: [{ hour: 21, minute: 0, weekdays: [1], day: 3 }],
    };
    expect(Object.keys(formToTrigger(form))).toEqual(["kind"]);
  });

  it("keeps the other kinds' defaults available after switching back", () => {
    const back = triggerToForm({ kind: "manual" } as Trigger);
    expect(back.calendarEntries.length).toBeGreaterThan(0);
    expect(back.intervalValue).toBeGreaterThan(0);
  });

  it("never reports a form error for manual", () => {
    expect(triggerFormError({ ...defaultTriggerForm(), kind: "manual" })).toBeNull();
  });
});
