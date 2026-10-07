// soos remote companion: Web Push service worker (ADR 2026-10-06 "Web Push Notifications
// for Failed-Password Alerts Through a Separate Sender Unit").
//
// It only shows notifications. Every push shows one (Safari revokes the permission after a
// push without a visible notification); an absent or unreadable payload shows a generic
// text. A notification carries counts and classes only, never a typed password. A tap
// focuses the open app or opens the start page of this origin. A live camera view start
// (ADR 2026-10-07) shows a fixed text under its own tag.
"use strict";

const FALLBACK_TITLE = "soos";
const FALLBACK_BODY = "New security alert — open soos";

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
      } else if (data && data.soos && data.soos.kind === "camera") {
        tag = "soos-camera";
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
      lang: "en",
    })
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
