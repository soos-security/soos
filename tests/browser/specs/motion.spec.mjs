// RPW4: motion follows prefers-reduced-motion on every engine (matrix RPW4; ADR 2026-10-13
// "Motion System for soos-gui and soos-remote" item 6): with reduce, no element animates or
// transitions; without it, the state numeral animates and the buttons transition.
import { test, expect, openApp } from "../fixtures.mjs";

// Every element whose computed animation or transition is not neutral.
function movingElements() {
  const moving = [];
  for (const node of document.querySelectorAll("*")) {
    const style = window.getComputedStyle(node);
    const animated = style.animationName.split(",").some((n) => n.trim() !== "none");
    const transitioned = style.transitionDuration
      .split(",")
      .some((d) => Number.parseFloat(d) !== 0);
    if (animated || transitioned) {
      moving.push(`${node.tagName.toLowerCase()}#${node.id}.${node.className}`);
    }
  }
  return moving;
}

test.describe("reduced motion", () => {
  test.use({ reducedMotion: "reduce" });

  test("no element animates or transitions", async ({ page, app }) => {
    await openApp(page, app);
    await expect(page.locator("#state")).toHaveText("Unlocked");
    expect(await page.evaluate(() => matchMedia("(prefers-reduced-motion: reduce)").matches)).toBe(true);
    expect(await page.evaluate(movingElements)).toEqual([]);
  });
});

test.describe("full motion", () => {
  test.use({ reducedMotion: "no-preference" });

  test("the state numeral animates and the buttons transition", async ({ page, app }) => {
    await openApp(page, app);
    await expect(page.locator("#state")).toHaveText("Unlocked");
    const state = await page.locator("#state").evaluate((n) => getComputedStyle(n).animationName);
    expect(state).toBe("soos-state-unlocked");
    const lock = await page
      .locator("#lock")
      .evaluate((n) => getComputedStyle(n).transitionDuration);
    expect(lock.split(",").some((d) => Number.parseFloat(d) > 0)).toBe(true);
  });
});
