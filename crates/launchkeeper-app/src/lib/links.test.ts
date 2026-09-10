import { describe, expect, it } from "vitest";
import { LINKS, coffeeReady } from "./links";

describe("links", () => {
  it("hides the coffee button while the handle is a placeholder", () => {
    expect(coffeeReady("https://buymeacoffee.com/<TODO-handle>")).toBe(false);
    expect(coffeeReady("https://buymeacoffee.com/someone")).toBe(true);
  });
  it("all links are https", () => {
    for (const url of Object.values(LINKS)) expect(url.startsWith("https://")).toBe(true);
  });
});
