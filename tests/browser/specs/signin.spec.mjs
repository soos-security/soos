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

// RTI5 (review RWEB-02, GitHub #616): Sign out on a Funnel session first drops this phone's push
// subscription (browser unsubscribe, then the PC record), then ends the session. The push
// surface is a synthetic registration installed before any page script runs; both API calls are
// answered in the browser.
const SIGNED_IN_AUTH_STATE = { ...SIGN_IN_AUTH_STATE, authenticated: true };

test("sign out drops this phone's notifications before ending the session", async ({ page, app }) => {
  await ensureWebAuthnSurface(page);
  await page.addInitScript(() => {
    const record = { unsubscribed: 0 };
    window.soosPushRecord = record;
    const subscription = {
      endpoint: "endpoint-of-this-phone",
      unsubscribe: () => {
        record.unsubscribed += 1;
        return Promise.resolve(true);
      },
      toJSON: () => ({ endpoint: "endpoint-of-this-phone", keys: {} }),
    };
    const registration = {
      active: {},
      pushManager: {
        getSubscription: () => Promise.resolve(subscription),
        subscribe: () => Promise.reject(new DOMException("stub", "NotAllowedError")),
      },
    };
    const container = {
      register: () => Promise.resolve(registration),
      ready: Promise.resolve(registration),
      addEventListener: () => {},
    };
    Object.defineProperty(Navigator.prototype, "serviceWorker", {
      configurable: true,
      get: () => container,
    });
    if (!("PushManager" in window)) {
      window.PushManager = function PushManager() {};
    }
    if (!("Notification" in window)) {
      window.Notification = { permission: "default", requestPermission: () => Promise.resolve("default") };
    }
  });
  await fulfillJson(page, "/api/auth/state", SIGNED_IN_AUTH_STATE);
  const order = [];
  let unsubscribeBody = null;
  await page.route("**/api/push/unsubscribe", (route) => {
    order.push("push-unsubscribe");
    unsubscribeBody = route.request().postDataJSON();
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ result: "unsubscribed" }),
    });
  });
  await page.route("**/api/auth/logout", (route) => {
    order.push("logout");
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ result: "logged_out" }),
    });
  });
  await openApp(page, app);
  const logout = page.locator("#logout");
  await expect(logout).toBeVisible();
  await logout.click();
  await expect(page.locator("#login")).toBeVisible();
  await expect(page.locator("#login-feedback")).toHaveText("Signed out");
  expect(order).toEqual(["push-unsubscribe", "logout"]);
  expect(unsubscribeBody).toEqual({ endpoint: "endpoint-of-this-phone" });
  expect(await page.evaluate(() => window.soosPushRecord.unsubscribed)).toBe(1);
});

// RTI5b (independent review of #616): Sign out never waits on a worker registration. Here
// register() never settles and no registration exists, so this phone has no subscription to
// drop: the logout is still posted at once and the sign-in screen shows "Signed out".
test("sign out still ends the session when the worker registration never settles", async ({ page, app }) => {
  await ensureWebAuthnSurface(page);
  await page.addInitScript(() => {
    const record = { registerCalls: 0 };
    window.soosPushRecord = record;
    const container = {
      register: () => {
        record.registerCalls += 1;
        return new Promise(() => {});
      },
      getRegistration: () => Promise.resolve(undefined),
      ready: new Promise(() => {}),
      addEventListener: () => {},
    };
    Object.defineProperty(Navigator.prototype, "serviceWorker", {
      configurable: true,
      get: () => container,
    });
    if (!("PushManager" in window)) {
      window.PushManager = function PushManager() {};
    }
    if (!("Notification" in window)) {
      window.Notification = { permission: "default", requestPermission: () => Promise.resolve("default") };
    }
  });
  await fulfillJson(page, "/api/auth/state", SIGNED_IN_AUTH_STATE);
  const order = [];
  await page.route("**/api/push/unsubscribe", (route) => {
    order.push("push-unsubscribe");
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ result: "unsubscribed" }),
    });
  });
  await page.route("**/api/auth/logout", (route) => {
    order.push("logout");
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ result: "logged_out" }),
    });
  });
  await openApp(page, app);
  const logout = page.locator("#logout");
  await expect(logout).toBeVisible();
  const registerCallsBefore = await page.evaluate(() => window.soosPushRecord.registerCalls);
  await logout.click();
  await expect(page.locator("#login")).toBeVisible();
  await expect(page.locator("#login-feedback")).toHaveText("Signed out");
  expect(order).toEqual(["logout"]);
  // Sign out started no new registration.
  expect(await page.evaluate(() => window.soosPushRecord.registerCalls)).toBe(registerCallsBefore);
});
