// RPW5: per-feature degradation when WebAuthn, EventSource or push is missing (matrix RPW5;
// ADR 2026-10-13 "Browser Support Policy for the `soos-remote` Web App" item 4). The features
// are removed before any page script runs; every other control keeps working.
import {
  test,
  expect,
  openApp,
  removeFeatures,
  ensureWebAuthnSurface,
  fulfillJson,
  appText,
  SIGN_IN_AUTH_STATE,
  ACTIVE_PUSH_VIEW,
  FIXED_CAPTURE_UNIX_MS,
} from "../fixtures.mjs";

test.describe("without WebAuthn", () => {
  test("sign-in is disabled with the fixed sentence", async ({ page, app }) => {
    await removeFeatures(page, ["webauthn"]);
    await fulfillJson(page, "/api/auth/state", SIGN_IN_AUTH_STATE);
    await openApp(page, app);
    await expect(page.locator("#login")).toBeVisible();
    await expect(page.locator("#login-button")).toBeDisabled();
    await expect(page.locator("#login-feedback")).toHaveText(appText("PASSKEY_UNSUPPORTED_TEXT"));
  });

  test.describe("on a locked PC", () => {
    test.use({ scene: "locked-alerts" });

    test("unlock is disabled with the fixed sentence, status still works", async ({ page, app }) => {
      await removeFeatures(page, ["webauthn"]);
      await openApp(page, app);
      await expect(page.locator("#state")).toHaveText("Locked");
      await expect(page.locator("#unlock")).toBeDisabled();
      await expect(page.locator("#feedback")).toHaveText(appText("PASSKEY_UNSUPPORTED_TEXT"));
      const camera = page.locator("section.camera");
      await expect(camera).toBeVisible();
      await expect(camera.getByRole("button", { name: "Start camera view" })).toBeDisabled();
    });
  });

  test("lock keeps working on an unlocked PC", async ({ page, app }) => {
    await removeFeatures(page, ["webauthn"]);
    await openApp(page, app);
    await expect(page.locator("#state")).toHaveText("Unlocked");
    await expect(page.locator("#lock")).toBeEnabled();
    await expect(page.locator("#feedback")).toHaveText(appText("PASSKEY_UNSUPPORTED_TEXT"));
  });

  test("with WebAuthn no passkey sentence is shown", async ({ page, app }) => {
    await ensureWebAuthnSurface(page);
    await openApp(page, app);
    await expect(page.locator("#state")).toHaveText("Unlocked");
    await expect(page.locator("#feedback")).not.toHaveText(appText("PASSKEY_UNSUPPORTED_TEXT"));
  });
});

test.describe("without EventSource", () => {
  test.use({ scene: "locked-alerts" });

  test("the status is polled every STATUS_POLL_MS instead", async ({ page, app }) => {
    await removeFeatures(page, ["eventSource"]);
    const statusFetches = [];
    const streams = [];
    page.on("request", (request) => {
      const path = new URL(request.url()).pathname;
      if (path === "/api/status") {
        statusFetches.push(path);
      } else if (path === "/api/events") {
        streams.push(path);
      }
    });
    await page.clock.install({ time: FIXED_CAPTURE_UNIX_MS });
    await page.goto(`${app.url}/`);
    await expect(page.locator("#state")).toHaveText("Locked");
    await expect(page.locator("#alerts-summary")).toHaveText(/^2 failed password attempts/);
    // The battery line exists only on the event stream.
    await expect(page.locator(".battery")).toBeHidden();
    // The page loads its status on load and on pageshow; wait until that count settles.
    let settled = -1;
    await expect
      .poll(
        () => {
          const stable = settled === statusFetches.length;
          settled = statusFetches.length;
          return stable && settled > 0;
        },
        { intervals: [500] },
      )
      .toBe(true);
    // Each STATUS_POLL_MS (15 s) of page time fetches the status exactly once more.
    await page.clock.runFor(15000);
    await expect.poll(() => statusFetches.length).toBe(settled + 1);
    await page.clock.runFor(15000);
    await expect.poll(() => statusFetches.length).toBe(settled + 2);
    expect(streams).toEqual([]);
  });
});

test.describe("without push", () => {
  test("the push card names the missing feature and hides its buttons", async ({ page, app }) => {
    await removeFeatures(page, ["push"]);
    await fulfillJson(page, "/api/push", ACTIVE_PUSH_VIEW);
    await openApp(page, app);
    await expect(page.locator("#push")).toBeVisible();
    await expect(page.locator("#push-state")).toHaveText(appText("PUSH_UNSUPPORTED_TEXT"));
    for (const id of ["#push-enable", "#push-test", "#push-disable"]) {
      await expect(page.locator(id)).toBeHidden();
    }
    await expect(page.locator("#state")).toHaveText("Unlocked");
  });

  test("without a service worker the card asks for a normal window", async ({ page, app }) => {
    await removeFeatures(page, ["serviceWorker"]);
    await fulfillJson(page, "/api/push", ACTIVE_PUSH_VIEW);
    await openApp(page, app);
    await expect(page.locator("#push-state")).toHaveText(appText("PUSH_NO_WORKER_TEXT"));
    await expect(page.locator("#push-enable")).toBeHidden();
  });

  test("with push the card offers Enable notifications", async ({ page, app }) => {
    await fulfillJson(page, "/api/push", ACTIVE_PUSH_VIEW);
    await openApp(page, app);
    await expect(page.locator("#push-state")).toHaveText("No device receives notifications yet");
    await expect(page.locator("#push-enable")).toBeVisible();
    await expect(page.locator("#push-enable")).toHaveText(appText("PUSH_ENABLE_LABEL"));
  });
});
