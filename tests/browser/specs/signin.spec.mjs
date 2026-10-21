// RPW1: the sign-in screen of a Funnel visit renders on every engine and hides every card
// (matrix RPW1). The auth state is a synthetic body fulfilled in the browser.
import {
  test,
  expect,
  openApp,
  fulfillJson,
  ensureWebAuthnSurface,
  SIGN_IN_AUTH_STATE,
  appText,
} from "../fixtures.mjs";

test("sign-in screen renders with an enabled passkey button", async ({ page, app }) => {
  await ensureWebAuthnSurface(page);
  await fulfillJson(page, "/api/auth/state", SIGN_IN_AUTH_STATE);
  await openApp(page, app, { tourSeen: false });
  const login = page.locator("#login");
  await expect(login).toBeVisible();
  await expect(login.locator("h2")).toHaveText("Sign in");
  await expect(page.locator("#login-button")).toBeVisible();
  await expect(page.locator("#login-button")).toBeEnabled();
  await expect(page.locator("#login-feedback")).not.toHaveText(appText("PASSKEY_UNSUPPORTED_TEXT"));
  for (const hidden of ["#status-card", "#alerts", "#push", "#enroll", "#logout"]) {
    await expect(page.locator(hidden)).toBeHidden();
  }
  await expect(page.locator("section.offline")).toBeHidden();
  // The feature tour never covers the sign-in screen.
  await expect(page.locator("section.tour")).toBeHidden();
});
