// RPW3: the phone tour opens once on a fresh profile, is remembered after Skip, and with a
// blocked storage it simply shows again without a page error (matrix RPW3; ADR 2026-10-17).
import { test, expect, openApp, blockStorage, TOUR_STORAGE_KEY, TOUR_STORAGE_SEEN } from "../fixtures.mjs";

test("the tour shows once, then is remembered", async ({ page, app }) => {
  await openApp(page, app, { tourSeen: false });
  const tour = page.locator("section.tour");
  await expect(tour).toBeVisible();
  await expect(tour.locator("h2")).toHaveText("Welcome to soos");
  await tour.getByRole("button", { name: "Skip tour" }).click();
  await expect(tour).toBeHidden();
  expect(await page.evaluate((key) => window.localStorage.getItem(key), TOUR_STORAGE_KEY)).toBe(
    TOUR_STORAGE_SEEN,
  );
  await page.reload();
  await expect(page.locator("#state")).toHaveText("Unlocked");
  await expect(page.locator("section.tour")).toBeHidden();
  // The footer button still reopens it on demand.
  await page.getByRole("button", { name: "Feature tour" }).click();
  await expect(page.locator("section.tour")).toBeVisible();
});

test("with a blocked storage the tour shows again after a reload", async ({ page, app }) => {
  await blockStorage(page);
  await openApp(page, app, { tourSeen: false });
  const tour = page.locator("section.tour");
  await expect(tour).toBeVisible();
  await tour.getByRole("button", { name: "Skip tour" }).click();
  await expect(tour).toBeHidden();
  await page.reload();
  await expect(page.locator("#state")).toHaveText("Unlocked");
  await expect(page.locator("section.tour")).toBeVisible();
  await expect(page.locator("section.tour h2")).toHaveText("Welcome to soos");
});
