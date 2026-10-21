// RPW2: the status, alerts, offline history and battery cards render the synthetic scenes of
// the fixture server on every engine (matrix RPW2, ADR 2026-10-13 browser support policy).
import { test, expect, openApp } from "../fixtures.mjs";

test.describe("locked scene with alerts", () => {
  test.use({ scene: "locked-alerts" });

  test("status card shows the locked state and the idle session", async ({ page, app }) => {
    await openApp(page, app);
    await expect(page.locator("#state")).toHaveText("Locked");
    await expect(page.locator("#state")).toHaveAttribute("data-state", "locked");
    await expect(page.locator(".stat-tile")).toHaveAttribute("data-state", "locked");
    await expect(page.locator("#activity")).toHaveText("Active session, idle for 12 min");
    await expect(page.locator("#updated")).toHaveText(/^Updated \d+ s ago$/);
    await expect(page.locator("#lock")).toBeDisabled();
  });

  test("battery line comes from the event stream", async ({ page, app }) => {
    await openApp(page, app);
    await expect(page.locator(".battery")).toBeVisible();
    await expect(page.locator(".battery")).toHaveText("Battery 64%, on battery");
  });

  test("alerts card lists the failed passwords", async ({ page, app }) => {
    await openApp(page, app);
    const card = page.locator("#alerts");
    await expect(card).toBeVisible();
    await expect(card).toHaveClass(/alerts-attention/);
    await expect(page.locator("#alerts-summary")).toHaveText(
      /^2 failed password attempts, last at .+ \(sudo\)$/,
    );
    await expect(page.locator("#alerts-history li")).toHaveCount(1);
    await expect(page.locator("#alerts-history li")).toHaveText(
      /^.+ sudo, your account, 2 wrong passwords$/,
    );
    await expect(page.locator("#alerts-ack")).toBeVisible();
  });

  test("offline history card lists both periods, newest first", async ({ page, app }) => {
    await openApp(page, app);
    const card = page.locator("section.offline");
    await expect(card).toBeVisible();
    await expect(card.locator("h2")).toHaveText("Offline history");
    await expect(card.locator("p.detail")).toHaveText("2 offline periods recorded");
    const rows = card.locator("li");
    await expect(rows).toHaveCount(2);
    await expect(rows.nth(0)).toHaveText(/ · 12 min · Network lost$/);
    await expect(rows.nth(1)).toHaveText(/ · 40 min · Shutdown or crash$/);
  });
});

test.describe("unlocked scene", () => {
  test.use({ scene: "unlocked" });

  test("status, battery, alerts and offline cards", async ({ page, app }) => {
    await openApp(page, app);
    await expect(page.locator("#state")).toHaveText("Unlocked");
    await expect(page.locator(".stat-tile")).toHaveAttribute("data-state", "unlocked");
    await expect(page.locator("#activity")).toHaveText("Active session");
    await expect(page.locator("#lock")).toBeEnabled();
    await expect(page.locator(".battery")).toHaveText("Battery 82%, charging");
    await expect(page.locator("#alerts-summary")).toHaveText("No failed password attempts");
    await expect(page.locator("#alerts-history li")).toHaveCount(0);
    await expect(page.locator("#alerts-ack")).toBeHidden();
    const offline = page.locator("section.offline");
    await expect(offline.locator("p.detail")).toHaveText("1 offline period recorded");
    await expect(offline.locator("li")).toHaveText(/ · 8 h · Sleep$/);
  });
});
