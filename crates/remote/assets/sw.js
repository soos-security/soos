// soos remote companion: Web Push service worker (ADR 2026-10-06 "Web Push Notifications
// for Failed-Password Alerts Through a Separate Sender Unit").
//
// It shows notifications and, on pushsubscriptionchange only, re-sends the renewed
// subscription once to /api/push/subscribe (ADR 2026-10-09 "Android Support for the
// `soos-remote` Phone Companion and Web Push"). The worker makes no request on push. Every
// push shows one notification (Safari revokes the permission after a push without a visible
// notification); an absent or unreadable payload shows a generic text. A notification
// carries counts and classes only, never a typed password. A tap focuses the open app or
// opens the start page of this origin.
"use strict";

const FALLBACK_TITLE = "soos";
const FALLBACK_BODY = "New security alert — open soos";
// Same-origin images the browser loads when it displays a notification (Android).
const ICON_PATH = "/icon-192.png";
const BADGE_PATH = "/badge-96.png";
// The page's subscription route and action (PUSH_SUBSCRIBE_PATH, ACTION_PUSH_SUBSCRIBE).
const SUBSCRIBE_PATH = "/api/push/subscribe";
const SUBSCRIBE_ACTION = "push-subscribe";
// One attempt, bounded: no retry, no timer left behind.
const RESUBSCRIBE_TIMEOUT_MS = 10000;

function readNotification(event) {
  let title = FALLBACK_TITLE;
  let body = FALLBACK_BODY;
  let tag = "soos-alerts";
  try {
    if (event.data) {
      const data = event.data.json();
      if (data && data.notification) {
        if (typeof data.notification.title === "string" && data.notification.title) {
          title = data.notification.title;
        }
        if (typeof data.notification.body === "string" && data.notification.body) {
          body = data.notification.body;
        }
      }
      if (data && data.soos && data.soos.kind === "test") {
        tag = "soos-test";
      }
    }
  } catch (_) {
    title = FALLBACK_TITLE;
    body = FALLBACK_BODY;
  }
  return { title, body, tag };
}

self.addEventListener("push", (event) => {
  const shown = readNotification(event);
  event.waitUntil(
    self.registration.showNotification(shown.title, {
      body: shown.body,
      tag: shown.tag,
      renotify: true,
      icon: ICON_PATH,
      badge: BADGE_PATH,
      lang: "en",
    })
  );
});

// The renewed subscription: the browser's own, else one made with the old key; null when the
// old key is unknown (the page re-subscribes on its next "Enable notifications").
function renewedSubscription(event) {
  if (event.newSubscription) {
    return Promise.resolve(event.newSubscription);
  }
  const old = event.oldSubscription;
  const key = old && old.options ? old.options.applicationServerKey : null;
  if (!key) {
    return Promise.resolve(null);
  }
  return self.registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key });
}

// The exact request shape of the page's postJson(PUSH_SUBSCRIBE_PATH, "push-subscribe", ...);
// the response is ignored and the timer is cleared on both outcomes.
function postSubscription(subscription) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), RESUBSCRIBE_TIMEOUT_MS);
  return fetch(SUBSCRIBE_PATH, {
    method: "POST",
    headers: { "X-Soos-Action": SUBSCRIBE_ACTION, "Content-Type": "application/json" },
    body: JSON.stringify(subscription.toJSON()),
    credentials: "same-origin",
    cache: "no-store",
    signal: controller.signal,
  }).finally(() => clearTimeout(timer));
}

// Silent: a refusal (off-tailnet, expired Funnel session, full store) is swallowed; the page
// re-sends the subscription on its next open.
self.addEventListener("pushsubscriptionchange", (event) => {
  event.waitUntil(
    renewedSubscription(event)
      .then((subscription) => (subscription === null ? null : postSubscription(subscription)))
      .catch(() => null)
  );
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  event.waitUntil(
    self.clients.matchAll({ type: "window", includeUncontrolled: true }).then((windows) => {
      for (const client of windows) {
        if ("focus" in client) {
          return client.focus();
        }
      }
      return self.clients.openWindow("/");
    })
  );
});
