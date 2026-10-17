/* soos remote companion page (GitHub #339, spec section 2.11; Funnel and passkey ADR
 * 2026-10-06, spec section 8).
 *
 * Server data only ever reaches the DOM through textContent. The page keeps no state
 * beyond the live EventSource: no script-readable cookie (the web session cookie is
 * HttpOnly), and no storage but one fixed flag remembering that the feature tour was closed
 * (the single tour storage block below). The only service worker (/sw.js) shows push notifications and,
 * when the browser renews a push subscription, re-sends it once to the PC. Staleness is measured with the browser's own
 * clock from the arrival of the last event, never by comparing checked_unix_ms with the
 * phone's clock.
 *
 * Passkeys: every WebAuthn ceremony is modal (a tap, then the passkey check:
 * Face ID, fingerprint or screen lock), always with user verification required, and never
 * names a credential: the server only answers with a challenge, so the phone offers the
 * passkey it holds for this site.
 *
 * Failed-password alerts (ADR 2026-10-06 "Failed-Password Alerts in soos-remote From the
 * System Journal"): the server only ever sends time, source class, account class, kind and
 * count, never the typed password. The page shows them with textContent, says "No failed
 * password attempts" only while the feature is active, and acknowledges exactly the view it
 * displayed (epoch and through headers; a stale view is re-fetched, never acknowledged).
 *
 * Push notifications (ADR 2026-10-06 "Web Push Notifications for Failed-Password Alerts
 * Through a Separate Sender Unit"; Android ADR 2026-10-09): from the phone web app (the
 * iPhone home-screen app or a supporting browser), "Enable notifications" asks
 * the permission from the tap itself, subscribes with the PC's public VAPID key fetched
 * before the tap, and hands the subscription to the PC. A notification carries counts and
 * classes only, never the typed password.
 *
 * Live camera view (ADR 2026-10-07 "Live Camera View in soos-remote Through the Daemon
 * Preview Channel"): the card is built at runtime (no markup id). Every view needs a tap,
 * a confirmation and a fresh passkey assertion (Face ID, fingerprint or screen lock); the
 * multipart JPEG stream is read with fetch and a ReadableStream into a bounded buffer, each part decoded with
 * createImageBitmap and drawn on a canvas, then released. No image element, no object
 * address, no storage; the view stops when the page is hidden or left. A tap on the live
 * image toggles full screen (element fullscreen where the browser has it, a fixed overlay
 * otherwise, as on iPhone Safari); one button switches between two modes, landscape (the
 * image as captured) and portrait (the image turned by 90°), through a CSS class only. The
 * mode lives in memory for the page lifetime and is never stored. Every end
 * of a view leaves full screen. A new view may start right after the previous one ended
 * (owner request 2026-10-07), always after a fresh passkey assertion.
 *
 * Battery level (ADR 2026-10-07 "Live Battery Level in soos-remote"): level, charge state
 * and mains presence only, from event: battery on the status stream only, shown with
 * textContent; hidden while the PC is unreachable or the stream is down.
 *
 * Offline history (ADR 2026-10-13 "Offline History and Back-Online Push Notifications in
 * soos-remote"): the PC lists when it was offline, as reason and times only, fetched from
 * /api/offline with the status and shown with textContent in the phone's own time zone; the
 * card is built at runtime (no markup id).
 *
 * Browser support (ADR 2026-10-13 "Browser Support Policy for the soos-remote Web App"):
 * one pure probe, browserCapabilities(), says from feature detection only (never the
 * User-Agent) what this browser can do; every passkey, push, service worker, stream and
 * image-decoding call sits behind it. A missing feature disables or hides only its own
 * control with a fixed explanation; lock and status keep working. The flags are hints for
 * the page only: every server check is the same for every browser. Messages are fixed
 * strings of this file; a browser's own error text is never shown.
 *
 * Feature tour (ADR 2026-10-17 "Phone Onboarding Tour in the soos-remote Web App and Its
 * Single Storage Exception"): a short tour opens once after sign-in, or on the first tailnet
 * visit, and again from Feature tour in the footer. It explains each feature of this page,
 * what it exposes and where to turn it off, with fixed text only; it reads no server data,
 * makes no request and switches nothing. The card is built at runtime (no markup id). The
 * browser remembers only that the tour was closed, through the one guarded helper of the
 * tour storage block; when site data is blocked the tour simply shows again.
 */
"use strict";

// Show "Unreachable" when no event arrived for this long (the server re-sends every 15 s).
const STALE_UI_MS = 45000;
// After a lock request, report "LockedHint unchanged" unless the stream confirmed "locked".
const LOCK_CONFIRM_UI_MS = 5000;
// After an unlock request, report "LockedHint unchanged" unless the stream confirmed
// "unlocked".
const UNLOCK_CONFIRM_UI_MS = 5000;
// Refresh the relative "Updated N s ago" and "idle for N min" lines at this cadence.
const TICK_MS = 1000;

// Button labels (the markup carries the same text for the no-script fallback).
const LOCK_LABEL = "Lock now";
const UNLOCK_LABEL = "Unlock now";
const PUSH_ENABLE_LABEL = "Enable notifications";
const PUSH_TEST_LABEL = "Send test notification";
const PUSH_DISABLE_LABEL = "Disable notifications";

const STATUS_PATH = "/api/status";
const EVENTS_PATH = "/api/events";
const LOCK_PATH = "/api/lock";
const UNLOCK_PATH = "/api/unlock";
const AUTH_STATE_PATH = "/api/auth/state";
const LOGIN_OPTIONS_PATH = "/api/auth/login/options";
const LOGIN_PATH = "/api/auth/login/verify";
const LOGOUT_PATH = "/api/auth/logout";
const UNLOCK_OPTIONS_PATH = "/api/auth/unlock/options";
const REGISTER_OPTIONS_PATH = "/api/auth/register/options";
const REGISTER_PATH = "/api/auth/register/verify";
const ALERTS_PATH = "/api/alerts";
const ALERTS_ACK_PATH = "/api/alerts/ack";
const PUSH_PATH = "/api/push";
const PUSH_SUBSCRIBE_PATH = "/api/push/subscribe";
const PUSH_UNSUBSCRIBE_PATH = "/api/push/unsubscribe";
const PUSH_TEST_PATH = "/api/push/test";
const SERVICE_WORKER_PATH = "/sw.js";
const OFFLINE_PATH = "/api/offline";
// Rows shown in the offline history card (the PC keeps at most 32).
const OFFLINE_HISTORY_SHOWN = 10;
const OFFLINE_REASONS = {
  network_lost: "Network lost",
  wifi_disabled: "Wi-Fi turned off",
  sleep: "Sleep",
  shutdown_or_crash: "Shutdown or crash",
};
// History rows shown (the server keeps at most 32).
const ALERTS_HISTORY_SHOWN = 10;

const ALERT_SOURCES = {
  lock_screen: "lock screen",
  sudo: "sudo",
  login: "login",
  other: "other",
};

const ALERT_ACCOUNTS = {
  owner: "your account",
  root: "root",
  other: "another account",
};

const ALERT_REASONS = {
  no_journal_access: "the account cannot read the system journal",
  journal_reader_failed: "journal reader stopped",
  owner_unresolved: "the owner account could not be resolved",
  overflow: "attempt counter exhausted, restart the service",
  rng_failed: "the random source failed",
};

const LABELS = {
  locked: "Locked",
  unlocked: "Unlocked",
  no_session: "No session",
  unavailable: "Unavailable",
  unreachable: "Unreachable",
  unknown: "Connecting",
};

// Every passkey ceremony (login, enroll, unlock, camera view).
// The six WebAuthn transport names (anything else is dropped).
const PASSKEY_TRANSPORTS = ["internal", "hybrid", "usb", "nfc", "ble", "smart-card"];
// Most credentials named on the unlock and camera ceremonies (the server stores at most 4).
const MAX_NAMED_CREDENTIALS = 4;
// Most transport names sent with a registration (the server refuses more than 8).
const MAX_REGISTER_TRANSPORTS = 8;
const PASSKEY_PROMPT_TEXT = "Confirm with your passkey (Face ID, fingerprint or screen lock)…";
const PUSH_UNSUPPORTED_TEXT =
  "On iPhone, notifications need the home-screen app (iOS 16.4 or later); this browser does not support them";
const PUSH_BLOCKED_TEXT = "Notifications are blocked: allow them for this site in the browser or phone settings";
const PUSH_WORKER_FAILED_TEXT =
  "Notifications could not start: the service worker did not register (reload the page and try again)";
const PUSH_HOME_SCREEN_TEXT =
  "On iPhone and iPad, notifications only work in the home-screen app: tap Share, then Add to Home Screen, and open soos from the new icon (iOS 16.4 or later)";
const PUSH_NO_WORKER_TEXT =
  "Notifications need a service worker, which this window does not allow (private windows block it): open the page in a normal window";
const PUSH_NO_SERVICE_TEXT =
  "This browser has no working push service: in Brave, turn on Use Google services for push messaging; otherwise use Chrome, Firefox, Safari or Samsung Internet";
const PUSH_STALE_TEXT =
  "An older notification subscription is in the way: reload the page, then enable notifications again";
const PUSH_TIMEOUT_TEXT = "The PC did not confirm the subscription in time: check the connection and try again";
const PUSH_UNREACHABLE_TEXT =
  "The PC could not be reached to save the subscription: check the connection and try again";
const PUSH_SLOW_TEXT = "The browser's push service did not answer in time: try again";
// Fixed texts for the browser's push errors, by DOMException name.
const PUSH_ERROR_TEXTS = {
  NotAllowedError: PUSH_BLOCKED_TEXT,
  AbortError: PUSH_NO_SERVICE_TEXT,
  NotSupportedError: PUSH_NO_SERVICE_TEXT,
  InvalidStateError: PUSH_STALE_TEXT,
  SecurityError: PUSH_NO_WORKER_TEXT,
};
const PASSKEY_UNSUPPORTED_TEXT =
  "This browser cannot use passkeys, so sign-in, unlock, passkey enrollment and the camera view are off here; lock and status still work on the tailnet";
// Fixed texts for the browser's passkey errors, by DOMException name.
const PASSKEY_ERROR_TEXTS = {
  NotAllowedError: "The passkey check was cancelled or timed out",
  SecurityError: "This address cannot use the passkey: open the exact address the passkey was made for",
  NotSupportedError: "This browser or device cannot create this kind of passkey",
};
// Longest wait for an active service worker before subscribing; also the whole budget of an
// "Enable notifications" tap from the permission answer on.
const SERVICE_WORKER_READY_TIMEOUT_MS = 10000;
// Status refresh cadence when the browser has no EventSource (the server's keep-alive cadence).
const STATUS_POLL_MS = 15000;

// Results shared by every passkey ceremony.
const PASSKEY_REASONS = {
  passkey_rejected: "The passkey was not accepted, try again",
  passkey_required: "A passkey check (Face ID, fingerprint or screen lock) is required for every unlock",
  passkeys_not_configured: "Passkeys are not configured (rp_id in remote.toml)",
  no_passkey: "No passkey is registered yet",
  login_required: "Please sign in again",
  rate_limited: "Too many attempts, please wait a moment",
  too_many_challenges: "Too many pending requests, please wait a moment",
  too_many_sessions: "Too many signed-in devices, sign out elsewhere first",
  store_unavailable: "The passkey store is unavailable on the PC",
  busy: "The PC is busy, try again",
  registration_conflict:
    "Enrollment changed meanwhile: get a new code and retry; remove the extra passkey from the phone's password manager",
  body_too_large: "Request too large",
  bad_request: "Request refused (malformed)",
  enroll_code_rejected: "The enrollment code is wrong or expired",
  passkey_limit: "The maximum number of passkeys is registered",
  already_registered: "This passkey is already registered",
  unavailable: "The PC is unavailable",
  forbidden: "Request refused",
};
// Enrollment adds the error of a passkey that already exists (create() only).
const ENROLL_ERROR_TEXTS = Object.assign({}, PASSKEY_ERROR_TEXTS, {
  InvalidStateError: PASSKEY_REASONS.already_registered,
});

const stateNode = document.getElementById("state");
const activityNode = document.getElementById("activity");
const updatedNode = document.getElementById("updated");
const feedbackNode = document.getElementById("feedback");
const lockButton = document.getElementById("lock");
const unlockButton = document.getElementById("unlock");
const statusCard = document.getElementById("status-card");
const statTile = statusCard.querySelector(".stat-tile");
const loginSection = document.getElementById("login");
const loginButton = document.getElementById("login-button");
const loginFeedback = document.getElementById("login-feedback");
const logoutButton = document.getElementById("logout");
const enrollSection = document.getElementById("enroll");
const enrollCode = document.getElementById("enroll-code");
const enrollButton = document.getElementById("enroll-button");
const enrollFeedback = document.getElementById("enroll-feedback");
const alertsSection = document.getElementById("alerts");
const alertsSummary = document.getElementById("alerts-summary");
const alertsCoverage = document.getElementById("alerts-coverage");
const alertsHistory = document.getElementById("alerts-history");
const alertsAckButton = document.getElementById("alerts-ack");
const alertsFeedback = document.getElementById("alerts-feedback");
const pushSection = document.getElementById("push");
const pushStateNode = document.getElementById("push-state");
const pushDevices = document.getElementById("push-devices");
const pushHint = document.getElementById("push-hint");
const pushEnableButton = document.getElementById("push-enable");
const pushTestButton = document.getElementById("push-test");
const pushDisableButton = document.getElementById("push-disable");
const pushFeedback = document.getElementById("push-feedback");

// The battery line, built at run time between the activity and the "Updated" lines.
const batteryNode = document.createElement("p");
batteryNode.className = "detail battery";
batteryNode.hidden = true;
updatedNode.parentNode.insertBefore(batteryNode, updatedNode);

// The "Offline history" card, built at run time before the push card (no markup id).
const offlineSection = document.createElement("section");
offlineSection.className = "card offline";
offlineSection.hidden = true;
offlineSection.setAttribute("aria-live", "polite");
const offlineTitle = document.createElement("h2");
const offlineSummary = document.createElement("p");
offlineSummary.className = "detail";
const offlineMonitor = document.createElement("p");
offlineMonitor.className = "banner banner-warn";
offlineMonitor.hidden = true;
const offlineHistory = document.createElement("ul");
offlineHistory.className = "offline-history table";
offlineSection.appendChild(offlineTitle);
offlineSection.appendChild(offlineSummary);
offlineSection.appendChild(offlineMonitor);
offlineSection.appendChild(offlineHistory);
pushSection.parentNode.insertBefore(offlineSection, pushSection);
setText(offlineTitle, "Offline history");

let source = null;
let statusPoll = null;
let latest = null;
let lastEventAt = 0;
let lockRequestedAt = 0;
let lockConfirmTimer = null;
let unlockRequestedAt = 0;
let unlockConfirmTimer = null;
let signedOut = false;
let alertsView = null;
let pushView = null;
let pushRegistration = null;
let batteryView = null;

function setText(node, text) {
  node.textContent = text;
}

// The browser features the page relies on, from feature detection only (never the
// User-Agent). Pure: no request, no DOM write, no storage; recomputed on every call.
function browserCapabilities() {
  const nav = window.navigator;
  const store = nav.credentials;
  const display =
    typeof window.matchMedia === "function" ? window.matchMedia("(display-mode: standalone)") : null;
  return {
    webauthn:
      typeof window.PublicKeyCredential !== "undefined" &&
      !!store &&
      typeof store.get === "function" &&
      typeof store.create === "function",
    serviceWorker: "serviceWorker" in nav,
    push: "PushManager" in window,
    notifications: "Notification" in window,
    standalone: nav.standalone === true || (display !== null && display.matches === true),
    // iPhone and iPad WebKit (Safari and every other iOS browser): wording only.
    homeScreen: "standalone" in nav,
    eventSource: typeof window.EventSource !== "undefined",
    streams:
      typeof window.ReadableStream !== "undefined" &&
      typeof window.AbortController !== "undefined" &&
      typeof window.Response !== "undefined" &&
      "body" in window.Response.prototype,
    imageBitmap: typeof window.createImageBitmap === "function",
  };
}

// An error whose message is one of this file's fixed texts.
function fixedError(text) {
  const error = new Error(text);
  error.fixed = true;
  return error;
}

// The text to show for a failure: a fixed text of this file, the fixed text for the browser's
// error name, or the fallback; a browser's own message is never shown.
function failureText(error, names, fallback) {
  if (error && error.fixed === true && typeof error.message === "string") {
    return error.message;
  }
  if (error && typeof error.name === "string" && Object.prototype.hasOwnProperty.call(names, error.name)) {
    return names[error.name];
  }
  return fallback;
}

// Settles like `promise`, or fails with the fixed `text` at `deadline` (Date.now() ms); the
// timer is cleared on both outcomes.
function beforeDeadline(promise, deadline, text) {
  return new Promise(function (resolve, reject) {
    const timer = setTimeout(function () {
      reject(fixedError(text));
    }, Math.max(0, deadline - Date.now()));
    promise.then(
      function (value) {
        clearTimeout(timer);
        resolve(value);
      },
      function (error) {
        clearTimeout(timer);
        reject(error);
      }
    );
  });
}

function reasonFor(result, extra, fallback) {
  if (Object.prototype.hasOwnProperty.call(extra, result)) {
    return extra[result];
  }
  if (Object.prototype.hasOwnProperty.call(PASSKEY_REASONS, result)) {
    return PASSKEY_REASONS[result];
  }
  return fallback;
}

// --- base64url <-> ArrayBuffer (no padding), written by hand -------------------------

function toBase64Url(buffer) {
  const bytes = new Uint8Array(buffer);
  let binary = "";
  for (let i = 0; i < bytes.length; i += 1) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function fromBase64Url(text) {
  const base64 = text.replace(/-/g, "+").replace(/_/g, "/");
  const padded = base64 + "===".slice((base64.length + 3) % 4);
  const binary = atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes.buffer;
}

// --- JSON requests --------------------------------------------------------------------

function postJson(path, action, body) {
  const headers = { "X-Soos-Action": action };
  const init = { method: "POST", headers: headers, cache: "no-store" };
  if (body !== undefined) {
    headers["Content-Type"] = "application/json";
    init.body = JSON.stringify(body);
  }
  return fetch(path, init).then(function (response) {
    return response
      .json()
      .catch(function () {
        return {};
      })
      .then(function (json) {
        return { status: response.status, body: json, result: json.result };
      });
  });
}

function assertionBody(credential) {
  const response = credential.response;
  const body = {
    id: toBase64Url(credential.rawId),
    client_data_json: toBase64Url(response.clientDataJSON),
    authenticator_data: toBase64Url(response.authenticatorData),
    signature: toBase64Url(response.signature),
  };
  if (response.userHandle) {
    body.user_handle = toBase64Url(response.userHandle);
  }
  return body;
}

// The known transport names of a server or browser list (anything else is dropped).
function knownTransports(list) {
  if (!Array.isArray(list)) {
    return [];
  }
  return list.filter(function (name) {
    return typeof name === "string" && PASSKEY_TRANSPORTS.indexOf(name) !== -1;
  });
}

// A modal passkey assertion over a server challenge. With `nameCredentials`, the
// credentials listed by the server (unlock, camera) are passed as the allow list so the
// phone goes straight to the biometric prompt; sign-in never names one.
function getAssertion(options, nameCredentials) {
  if (!browserCapabilities().webauthn) {
    return Promise.reject(fixedError(PASSKEY_UNSUPPORTED_TEXT));
  }
  const publicKey = {
    challenge: fromBase64Url(options.challenge),
    rpId: options.rp_id,
    timeout: options.timeout_ms,
    userVerification: "required",
  };
  if (nameCredentials === true && Array.isArray(options.allow_credentials)) {
    const named = options.allow_credentials
      .filter(function (entry) {
        return entry !== null && typeof entry === "object" && typeof entry.id === "string";
      })
      .slice(0, MAX_NAMED_CREDENTIALS)
      .map(function (entry) {
        return {
          type: "public-key",
          id: fromBase64Url(entry.id),
          transports: knownTransports(entry.transports),
        };
      });
    if (named.length > 0) {
      publicKey.allowCredentials = named;
    }
  }
  return navigator.credentials.get({ publicKey: publicKey });
}

// --- screens ------------------------------------------------------------------------

// The Sign out button and the feedback-line hook of the :has() fallback in style.css.
function setLogoutHidden(hidden) {
  logoutButton.hidden = hidden;
  feedbackNode.classList.toggle("feedback-last", hidden);
}

function showLogin(message) {
  signedOut = true;
  closeStream();
  clearBattery();
  latest = null;
  loginSection.hidden = false;
  statusCard.hidden = true;
  setLogoutHidden(true);
  enrollSection.hidden = true;
  alertsSection.hidden = true;
  alertsView = null;
  offlineSection.hidden = true;
  clearChildren(offlineHistory);
  pushSection.hidden = true;
  pushView = null;
  stopCameraView(null);
  buildCameraCard().section.hidden = true;
  loginButton.disabled = !browserCapabilities().webauthn;
  setText(loginFeedback, loginButton.disabled ? PASSKEY_UNSUPPORTED_TEXT : message || " ");
  // The feature tour never stays over the sign-in screen (it is not marked as seen).
  if (tour !== null) {
    tour.section.hidden = true;
  }
}

function showApp(state) {
  signedOut = false;
  loginSection.hidden = true;
  statusCard.hidden = false;
  setLogoutHidden(state.mode !== "funnel");
  enrollSection.hidden = !(state.mode === "tailnet" && state.enrollment === true);
  if (!browserCapabilities().webauthn) {
    enrollButton.disabled = true;
    setText(enrollFeedback, PASSKEY_UNSUPPORTED_TEXT);
    setText(feedbackNode, PASSKEY_UNSUPPORTED_TEXT);
  }
  offerTour();
}

function render(view, reachable) {
  const state = reachable ? view.state : "unreachable";
  const label = Object.prototype.hasOwnProperty.call(LABELS, state) ? LABELS[state] : LABELS.unknown;
  setText(stateNode, label);
  stateNode.dataset.state = state;
  stateNode.className = "state state-" + state.replace(/[^a-z_]/g, "");
  // Hook of the :has() fallback in style.css: known states only.
  statTile.dataset.state = Object.prototype.hasOwnProperty.call(LABELS, state) ? state : "unknown";

  if (reachable && (view.state === "locked" || view.state === "unlocked")) {
    let activity = view.active ? "Active session" : "Inactive session";
    if (view.idle) {
      activity += ", idle";
      if (typeof view.idle_since_unix_s === "number") {
        const minutes = Math.max(0, Math.floor((Date.now() / 1000 - view.idle_since_unix_s) / 60));
        activity += " for " + minutes + " min";
      }
    }
    setText(activityNode, activity);
  } else if (!reachable) {
    setText(activityNode, "The PC is off, asleep, or unreachable");
  } else if (view.state === "no_session") {
    setText(activityNode, "No local desktop session for the owner");
  } else {
    setText(activityNode, "logind could not be read");
  }

  lockButton.disabled = !(reachable && view.state === "unlocked");
  unlockButton.disabled = !(reachable && view.state === "locked" && browserCapabilities().webauthn);
  if (reachable && view.state === "locked" && lockRequestedAt !== 0) {
    confirmLock();
  }
  if (reachable && view.state === "unlocked" && unlockRequestedAt !== 0) {
    confirmUnlock();
  }
  showBattery(reachable);
}

// --- battery level ------------------------------------------------------------------

// The battery line of a view, or null to hide it (unknown states fail closed).
function batteryText(view) {
  if (view === null || typeof view !== "object" || typeof view.state !== "string") {
    return null;
  }
  if (view.state === "unavailable") {
    return "Battery: unknown";
  }
  if (view.state === "no_battery") {
    return "No battery";
  }
  if (view.state !== "present") {
    return null;
  }
  const p = view.percent;
  const level = Number.isInteger(p) && p >= 0 && p <= 100 ? "Battery " + p + "%" : "Battery level unknown";
  let suffix = "";
  if (view.charge === "charging") {
    suffix = ", charging";
  } else if (view.charge === "discharging") {
    suffix = ", on battery";
  } else if (view.charge === "full") {
    suffix = ", full";
  } else if (view.charge === "not_charging") {
    suffix = ", plugged in, not charging";
  }
  if (suffix === "" && view.external_power === true) {
    suffix = ", plugged in";
  }
  return level + suffix;
}

function showBattery(reachable) {
  const text = reachable && !signedOut ? batteryText(batteryView) : null;
  batteryNode.hidden = text === null;
  setText(batteryNode, text === null ? " " : text);
}

function renderBattery(view) {
  batteryView = view;
  showBattery(true);
}

function clearBattery() {
  batteryView = null;
  showBattery(false);
}

function renderUpdated() {
  if (signedOut || latest === null || lastEventAt === 0) {
    setText(updatedNode, " ");
    return;
  }
  const seconds = Math.max(0, Math.round((Date.now() - lastEventAt) / 1000));
  setText(updatedNode, "Updated " + seconds + " s ago");
  if (Date.now() - lastEventAt > STALE_UI_MS) {
    render(latest, false);
  } else {
    render(latest, true);
  }
}

function accept(view) {
  latest = view;
  lastEventAt = Date.now();
  render(view, true);
  renderUpdated();
}

function confirmLock() {
  lockRequestedAt = 0;
  if (lockConfirmTimer !== null) {
    clearTimeout(lockConfirmTimer);
    lockConfirmTimer = null;
  }
  setText(feedbackNode, "Locked");
}

function confirmUnlock() {
  unlockRequestedAt = 0;
  if (unlockConfirmTimer !== null) {
    clearTimeout(unlockConfirmTimer);
    unlockConfirmTimer = null;
  }
  setText(feedbackNode, "Unlocked");
}

function openStream() {
  closeStream();
  if (!browserCapabilities().eventSource) {
    // No EventSource: the status is fetched at the server's keep-alive cadence instead.
    statusPoll = setInterval(function () {
      fetchStatus(true);
    }, STATUS_POLL_MS);
    return;
  }
  source = new EventSource(EVENTS_PATH);
  source.addEventListener("status", function (event) {
    try {
      accept(JSON.parse(event.data));
    } catch (_error) {
      // A malformed event is ignored; the next one or the fetch below recovers.
    }
  });
  source.addEventListener("alerts", function (event) {
    try {
      renderAlerts(JSON.parse(event.data));
    } catch (_error) {
      // A malformed event is ignored; the next one or the fetch below recovers.
    }
  });
  source.addEventListener("battery", function (event) {
    try {
      renderBattery(JSON.parse(event.data));
    } catch (_error) {
      // A malformed event is ignored; the next battery frame recovers.
    }
  });
  source.onerror = function () {
    // EventSource reconnects on its own; one fetch decides between a transient blip, an
    // expired web session and an unreachable PC. The battery line stays hidden until the
    // reopened stream sends its first battery frame.
    clearBattery();
    fetchStatus(true);
  };
}

function closeStream() {
  if (source !== null) {
    source.close();
    source = null;
  }
  if (statusPoll !== null) {
    clearInterval(statusPoll);
    statusPoll = null;
  }
  clearBattery();
}

function fetchStatus(markUnreachableOnFailure) {
  fetch(STATUS_PATH, { method: "GET", cache: "no-store" })
    .then(function (response) {
      if (response.status === 403) {
        return response.json().then(function (body) {
          if (body.result === "login_required") {
            showLogin("Your session ended, please sign in again");
            return null;
          }
          throw new Error("status 403");
        });
      }
      if (!response.ok) {
        throw new Error("status " + response.status);
      }
      return response.json();
    })
    .then(function (view) {
      if (view !== null) {
        accept(view);
        fetchAlerts();
        fetchOffline();
      }
    })
    .catch(function () {
      if (markUnreachableOnFailure && !signedOut) {
        render(latest === null ? { state: "unreachable" } : latest, false);
      }
    });
}

// --- failed-password alerts -----------------------------------------------------------

function plural(count, word) {
  return count + " " + word + (count === 1 ? "" : "s");
}

function clockTime(unixMs) {
  if (typeof unixMs !== "number") {
    return "--:--";
  }
  return new Date(unixMs).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function labelOf(table, key) {
  return Object.prototype.hasOwnProperty.call(table, key) ? table[key] : "other";
}

function renderAlerts(view) {
  if (signedOut || view === null || typeof view !== "object") {
    return;
  }
  alertsView = view;
  if (view.state === "disabled") {
    alertsSection.hidden = true;
    return;
  }
  alertsSection.hidden = false;
  const wrong = typeof view.unacknowledged_wrong_password === "number" ? view.unacknowledged_wrong_password : 0;
  const lockedOut = typeof view.unacknowledged_locked_out === "number" ? view.unacknowledged_locked_out : 0;
  const notMonitored = view.lock_screen === "not_configured";
  let counts = "";
  if (wrong > 0 || lockedOut > 0) {
    counts = plural(wrong, "failed password attempt");
    counts += ", last at " + clockTime(view.last_unix_ms);
    counts += " (" + labelOf(ALERT_SOURCES, view.last_source) + ")";
    if (lockedOut > 0) {
      counts += " and " + plural(lockedOut, "attempt") + " while locked out";
    }
  }
  let summary;
  if (view.state === "active") {
    if (counts !== "") {
      summary = counts;
    } else if (notMonitored) {
      summary = "No failed password attempts (sudo, login, other)";
    } else {
      summary = "No failed password attempts";
    }
  } else {
    if (view.state === "unavailable") {
      summary = "Password alerts unavailable";
      if (Object.prototype.hasOwnProperty.call(ALERT_REASONS, view.reason)) {
        summary += ": " + ALERT_REASONS[view.reason];
      }
    } else {
      summary = "Password alerts starting";
    }
    if (counts !== "") {
      summary += ". " + counts;
    }
  }
  setText(alertsSummary, summary);
  alertsSection.classList.toggle("alerts-attention", counts !== "");
  alertsCoverage.hidden = !notMonitored;
  setText(alertsCoverage, notMonitored ? "Lock screen not monitored — see setup" : " ");

  while (alertsHistory.firstChild) {
    alertsHistory.removeChild(alertsHistory.firstChild);
  }
  const history = Array.isArray(view.history) ? view.history.slice(0, ALERTS_HISTORY_SHOWN) : [];
  history.forEach(function (record) {
    const item = document.createElement("li");
    const count = typeof record.count === "number" ? record.count : 0;
    const what =
      record.kind === "locked_out"
        ? plural(count, "attempt") + " while locked out"
        : plural(count, "wrong password");
    setText(
      item,
      clockTime(record.last_unix_ms) +
        " " +
        labelOf(ALERT_SOURCES, record.source) +
        ", " +
        labelOf(ALERT_ACCOUNTS, record.account) +
        ", " +
        what
    );
    alertsHistory.appendChild(item);
  });
  alertsAckButton.hidden = !(wrong > 0 || lockedOut > 0) || typeof view.epoch !== "string";
}

function fetchAlerts() {
  if (signedOut) {
    return;
  }
  fetch(ALERTS_PATH, { method: "GET", cache: "no-store" })
    .then(function (response) {
      if (!response.ok) {
        throw new Error("status " + response.status);
      }
      return response.json();
    })
    .then(renderAlerts)
    .catch(function () {
      // The status line already reports an unreachable PC.
    });
}

// --- offline history ------------------------------------------------------------------

function clearChildren(node) {
  while (node.firstChild) {
    node.removeChild(node.firstChild);
  }
}

function dateTime(unixMs) {
  if (typeof unixMs !== "number") {
    return "--";
  }
  return new Date(unixMs).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function durationText(ms) {
  if (typeof ms !== "number" || ms < 60000) {
    return "less than a minute";
  }
  const minutes = Math.floor(ms / 60000);
  if (minutes < 60) {
    return minutes + " min";
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 48) {
    return minutes % 60 === 0 ? hours + " h" : hours + " h " + (minutes % 60) + " min";
  }
  return Math.floor(hours / 24) + " days";
}

function renderOffline(view) {
  if (signedOut || view === null || typeof view !== "object") {
    return;
  }
  if (view.state === "disabled") {
    offlineSection.hidden = true;
    return;
  }
  offlineSection.hidden = false;
  clearChildren(offlineHistory);
  if (view.state !== "active") {
    setText(
      offlineSummary,
      view.reason === "store_failed"
        ? "Offline alerts unavailable: the history file on the PC was refused"
        : "Offline alerts unavailable"
    );
    offlineMonitor.hidden = true;
    setText(offlineMonitor, " ");
    return;
  }
  const rows = Array.isArray(view.intervals) ? view.intervals : [];
  setText(
    offlineSummary,
    rows.length === 0 ? "No offline period recorded" : plural(rows.length, "offline period") + " recorded"
  );
  const unmonitored = view.network_monitor === "unavailable";
  offlineMonitor.hidden = !unmonitored;
  setText(
    offlineMonitor,
    unmonitored
      ? "Network changes are not monitored on this PC (NetworkManager not found); sleep and shutdown are"
      : " "
  );
  rows.slice(0, OFFLINE_HISTORY_SHOWN).forEach(function (row) {
    const item = document.createElement("li");
    const reason = Object.prototype.hasOwnProperty.call(OFFLINE_REASONS, row.reason)
      ? OFFLINE_REASONS[row.reason]
      : "Other";
    setText(
      item,
      dateTime(row.start_unix_ms) +
        " \u2013 " +
        clockTime(row.end_unix_ms) +
        " \u00b7 " +
        durationText(row.duration_ms) +
        " \u00b7 " +
        reason
    );
    offlineHistory.appendChild(item);
  });
}

function fetchOffline() {
  if (signedOut) {
    return;
  }
  fetch(OFFLINE_PATH, { method: "GET", cache: "no-store" })
    .then(function (response) {
      if (!response.ok) {
        throw new Error("status " + response.status);
      }
      return response.json();
    })
    .then(renderOffline)
    .catch(function () {
      // The status line already reports an unreachable PC or an ended session.
    });
}

function requestAlertsAck() {
  const view = alertsView;
  if (view === null || typeof view.epoch !== "string" || typeof view.through !== "number") {
    return;
  }
  alertsAckButton.disabled = true;
  fetch(ALERTS_ACK_PATH, {
    method: "POST",
    cache: "no-store",
    headers: {
      "X-Soos-Action": "alerts-ack",
      "X-Soos-Alerts-Epoch": view.epoch,
      "X-Soos-Alerts-Through": String(view.through),
    },
  })
    .then(function (response) {
      return response
        .json()
        .catch(function () {
          return {};
        })
        .then(function (body) {
          return { status: response.status, body: body };
        });
    })
    .then(function (outcome) {
      if (outcome.status === 200) {
        setText(alertsFeedback, "Acknowledged");
        renderAlerts(outcome.body);
        return;
      }
      if (outcome.body.result === "login_required") {
        showLogin("Your session ended, please sign in again");
        return;
      }
      if (outcome.body.result === "stale_view") {
        // The PC restarted since this view: show the fresh one, never acknowledge it blindly.
        setText(alertsFeedback, "New attempts since this view, check them and acknowledge again");
        fetchAlerts();
        return;
      }
      const reasons = {
        rate_limited: "Please wait a moment before acknowledging again",
        alerts_disabled: "Password alerts are disabled (password_alerts in remote.toml)",
        unavailable: "Password alerts are unavailable",
        bad_request: "The view changed, refresh and acknowledge again",
        forbidden: "Request refused",
      };
      setText(alertsFeedback, reasonFor(outcome.body.result, reasons, "Acknowledge refused (" + outcome.status + ")"));
      fetchAlerts();
    })
    .catch(function () {
      setText(alertsFeedback, "Acknowledge request failed");
    })
    .finally(function () {
      alertsAckButton.disabled = false;
    });
}

// --- lock -----------------------------------------------------------------------------

function requestLock() {
  lockButton.disabled = true;
  setText(feedbackNode, "Lock requested…");
  lockRequestedAt = Date.now();
  if (lockConfirmTimer !== null) {
    clearTimeout(lockConfirmTimer);
  }
  lockConfirmTimer = setTimeout(function () {
    lockConfirmTimer = null;
    if (lockRequestedAt !== 0) {
      lockRequestedAt = 0;
      setText(feedbackNode, "The desktop did not confirm the lock (LockedHint unchanged)");
      if (latest !== null) {
        render(latest, true);
      }
    }
  }, LOCK_CONFIRM_UI_MS);

  fetch(LOCK_PATH, { method: "POST", headers: { "X-Soos-Action": "lock" } })
    .then(function (response) {
      return response.json().then(function (body) {
        return { status: response.status, result: body.result };
      });
    })
    .then(function (outcome) {
      if (outcome.status === 202) {
        return;
      }
      lockRequestedAt = 0;
      if (lockConfirmTimer !== null) {
        clearTimeout(lockConfirmTimer);
        lockConfirmTimer = null;
      }
      if (outcome.result === "login_required") {
        showLogin("Your session ended, please sign in again");
        return;
      }
      const reasons = {
        no_session: "No session to lock",
        already_locked: "Already locked",
        rate_limited: "Please wait a moment before locking again",
        unavailable: "logind is unavailable",
        forbidden: "Request refused",
      };
      setText(feedbackNode, reasonFor(outcome.result, reasons, "Lock refused (" + outcome.status + ")"));
      fetchStatus(false);
    })
    .catch(function () {
      lockRequestedAt = 0;
      if (lockConfirmTimer !== null) {
        clearTimeout(lockConfirmTimer);
        lockConfirmTimer = null;
      }
      setText(feedbackNode, "Lock request failed");
      fetchStatus(true);
    });
}

// --- unlock (a fresh passkey assertion every time) -------------------------------------

function unlockFailed(status, result) {
  unlockRequestedAt = 0;
  if (unlockConfirmTimer !== null) {
    clearTimeout(unlockConfirmTimer);
    unlockConfirmTimer = null;
  }
  if (result === "login_required") {
    showLogin("Your session ended, please sign in again");
    return;
  }
  const reasons = {
    no_session: "No session to unlock",
    already_unlocked: "Already unlocked",
    rate_limited: "Please wait a moment before unlocking again",
    unavailable: "logind is unavailable",
    unlock_disabled: "Remote unlock is disabled (allow_unlock = false in remote.toml)",
    forbidden: "Request refused",
  };
  setText(feedbackNode, reasonFor(result, reasons, "Unlock refused (" + status + ")"));
  fetchStatus(false);
}

function requestUnlock() {
  if (!browserCapabilities().webauthn) {
    setText(feedbackNode, PASSKEY_UNSUPPORTED_TEXT);
    return;
  }
  // A deliberate second tap, then the passkey check: a stray tap must never open the PC.
  if (!window.confirm("Unlock the PC now?")) {
    return;
  }
  unlockButton.disabled = true;
  setText(feedbackNode, PASSKEY_PROMPT_TEXT);

  postJson(UNLOCK_OPTIONS_PATH, "unlock-options")
    .then(function (options) {
      if (options.status !== 200) {
        unlockFailed(options.status, options.result);
        return null;
      }
      return getAssertion(options.body, true).then(function (credential) {
        setText(feedbackNode, "Unlock requested…");
        unlockRequestedAt = Date.now();
        if (unlockConfirmTimer !== null) {
          clearTimeout(unlockConfirmTimer);
        }
        unlockConfirmTimer = setTimeout(function () {
          unlockConfirmTimer = null;
          if (unlockRequestedAt !== 0) {
            unlockRequestedAt = 0;
            setText(feedbackNode, "The desktop did not confirm the unlock (LockedHint unchanged)");
            if (latest !== null) {
              render(latest, true);
            }
          }
        }, UNLOCK_CONFIRM_UI_MS);
        return fetch(UNLOCK_PATH, {
          method: "POST",
          headers: { "X-Soos-Action": "unlock", "Content-Type": "application/json" },
          body: JSON.stringify(assertionBody(credential)),
        })
          .then(function (response) {
            return response.json().then(function (body) {
              return { status: response.status, result: body.result };
            });
          })
          .then(function (outcome) {
            if (outcome.status !== 202) {
              unlockFailed(outcome.status, outcome.result);
            }
          });
      });
    })
    .catch(function (error) {
      unlockFailed(0, "cancelled");
      setText(feedbackNode, failureText(error, PASSKEY_ERROR_TEXTS, "Unlock cancelled or failed"));
    });
}

// --- sign in / sign out (internet access through Tailscale Funnel) ---------------------

function requestLogin() {
  if (!browserCapabilities().webauthn) {
    setText(loginFeedback, PASSKEY_UNSUPPORTED_TEXT);
    return;
  }
  loginButton.disabled = true;
  setText(loginFeedback, PASSKEY_PROMPT_TEXT);
  postJson(LOGIN_OPTIONS_PATH, "login-options")
    .then(function (options) {
      if (options.status !== 200) {
        throw fixedError(reasonFor(options.result, {}, "Sign-in refused (" + options.status + ")"));
      }
      return getAssertion(options.body, false);
    })
    .then(function (credential) {
      return postJson(LOGIN_PATH, "login", assertionBody(credential));
    })
    .then(function (outcome) {
      if (outcome.status !== 200) {
        throw fixedError(reasonFor(outcome.result, {}, "Sign-in refused (" + outcome.status + ")"));
      }
      setText(loginFeedback, " ");
      resume();
    })
    .catch(function (error) {
      setText(loginFeedback, failureText(error, PASSKEY_ERROR_TEXTS, "Sign-in cancelled"));
    })
    .finally(function () {
      loginButton.disabled = !browserCapabilities().webauthn;
    });
}

function requestLogout() {
  postJson(LOGOUT_PATH, "logout").finally(function () {
    showLogin("Signed out");
  });
}

// --- passkey enrollment (tailnet only, with a code from `soos-remote enroll-code`) -----

// The transports the browser reports for a new passkey (known names only, at most 8);
// an old browser without `getTransports`, or one that throws, sends none.
function credentialTransports(response) {
  if (typeof response.getTransports === "function") {
    try {
      const reported = response.getTransports();
      if (!Array.isArray(reported)) {
        return [];
      }
      return reported
        .filter(function (name) {
          return typeof name === "string" && PASSKEY_TRANSPORTS.indexOf(name) !== -1;
        })
        .slice(0, MAX_REGISTER_TRANSPORTS);
    } catch (error) {
      return [];
    }
  }
  return [];
}

function requestEnroll() {
  if (!browserCapabilities().webauthn) {
    setText(enrollFeedback, PASSKEY_UNSUPPORTED_TEXT);
    return;
  }
  const code = enrollCode.value.trim();
  if (code.length === 0) {
    setText(enrollFeedback, "Type the code printed by soos-remote enroll-code");
    return;
  }
  enrollButton.disabled = true;
  setText(enrollFeedback, PASSKEY_PROMPT_TEXT);
  postJson(REGISTER_OPTIONS_PATH, "register-options", { code: code })
    .then(function (options) {
      if (options.status !== 200) {
        throw fixedError(reasonFor(options.result, {}, "Enrollment refused (" + options.status + ")"));
      }
      if (!browserCapabilities().webauthn) {
        throw fixedError(PASSKEY_UNSUPPORTED_TEXT);
      }
      const o = options.body;
      return navigator.credentials.create({
        publicKey: {
          rp: { id: o.rp_id, name: "soos" },
          user: { id: fromBase64Url(o.user_id), name: "soos", displayName: "soos" },
          challenge: fromBase64Url(o.challenge),
          pubKeyCredParams: [{ type: "public-key", alg: -7 }],
          authenticatorSelection: {
            authenticatorAttachment: "platform",
            residentKey: "required",
            requireResidentKey: true,
            userVerification: "required",
          },
          attestation: "none",
          hints: ["client-device"],
          excludeCredentials: o.exclude_credentials.map(function (id) {
            return { type: "public-key", id: fromBase64Url(id) };
          }),
          timeout: o.timeout_ms,
        },
      });
    })
    .then(function (credential) {
      const response = credential.response;
      return postJson(REGISTER_PATH, "register", {
        id: toBase64Url(credential.rawId),
        client_data_json: toBase64Url(response.clientDataJSON),
        attestation_object: toBase64Url(response.attestationObject),
        transports: credentialTransports(response),
      });
    })
    .then(function (outcome) {
      if (outcome.status !== 200) {
        throw fixedError(reasonFor(outcome.result, {}, "Enrollment refused (" + outcome.status + ")"));
      }
      enrollCode.value = "";
      setText(enrollFeedback, "Passkey added");
    })
    .catch(function (error) {
      setText(enrollFeedback, failureText(error, ENROLL_ERROR_TEXTS, "Enrollment cancelled"));
    })
    .finally(function () {
      enrollButton.disabled = !browserCapabilities().webauthn;
    });
}

// --- push notifications (iPhone home-screen app or a supporting browser) ---------------

const PUSH_REASONS = {
  unsupported_push_service:
    "The PC does not accept this browser's push service (Edge on a computer, for example): use Chrome, Firefox, Safari or Samsung Internet for notifications",
  too_many_subscriptions:
    "Four devices already receive notifications: remove one on the PC (soos-remote push list, then push remove N)",
  no_subscriptions: "No device receives notifications yet",
  push_disabled: "Notifications are off on the PC (push_notifications in remote.toml)",
  store_unavailable: "The notification store is unavailable on the PC",
  rate_limited: "Please wait a moment and try again",
  unavailable: "Notifications are unavailable on the PC",
};

const PUSH_SERVICES = {
  apple: "Apple",
  google: "Android / Chrome (Google)",
  mozilla: "Firefox (Mozilla)",
};

function pushSupported() {
  const caps = browserCapabilities();
  return caps.serviceWorker && caps.push && caps.notifications;
}

// The fixed reason this browser cannot subscribe (wording only; the server checks are the
// same for every browser).
function pushUnsupportedText(fallback) {
  const caps = browserCapabilities();
  if (caps.homeScreen && !caps.standalone) {
    return PUSH_HOME_SCREEN_TEXT;
  }
  if (!caps.serviceWorker) {
    return PUSH_NO_WORKER_TEXT;
  }
  return fallback;
}

function registerServiceWorker() {
  if (!pushSupported()) {
    return Promise.resolve(null);
  }
  if (pushRegistration !== null) {
    return Promise.resolve(pushRegistration);
  }
  const registering = navigator.serviceWorker.register(SERVICE_WORKER_PATH, { scope: "/" });
  return registering
    .then(function (registration) {
      pushRegistration = registration;
      return registration;
    })
    .catch(function () {
      return null;
    });
}

// The registration with an active worker, or null when registration failed or no worker
// was active within SERVICE_WORKER_READY_TIMEOUT_MS of the call (the timer is armed before
// registering, so a register() that never settles is bounded too).
function readyRegistration() {
  if (!pushSupported()) {
    return Promise.resolve(null);
  }
  return new Promise(function (resolve) {
    const timer = setTimeout(function () {
      resolve(null);
    }, SERVICE_WORKER_READY_TIMEOUT_MS);
    function settle(registration) {
      clearTimeout(timer);
      resolve(registration);
    }
    registerServiceWorker()
      .then(function (registration) {
        if (registration === null) {
          settle(null);
          return;
        }
        if (registration.active) {
          settle(registration);
          return;
        }
        navigator.serviceWorker.ready.then(settle, function () {
          settle(null);
        });
      })
      .catch(function () {
        settle(null);
      });
  });
}

// "same" | "different" | "unknown": "unknown" when the browser does not expose
// options.applicationServerKey (or the PC key is missing); such a subscription is kept.
function serverKeyMatch(subscription, publicKey) {
  const own = subscription.options ? subscription.options.applicationServerKey : null;
  if (!own || typeof publicKey !== "string") {
    return "unknown";
  }
  const a = new Uint8Array(own);
  const b = new Uint8Array(fromBase64Url(publicKey));
  if (a.length !== b.length) {
    return "different";
  }
  for (let i = 0; i < a.length; i += 1) {
    if (a[i] !== b[i]) {
      return "different";
    }
  }
  return "same";
}

function renderPush(view) {
  pushView = view;
  if (!view || view.state === "disabled") {
    pushSection.hidden = true;
    return;
  }
  pushSection.hidden = false;
  while (pushDevices.firstChild) {
    pushDevices.removeChild(pushDevices.firstChild);
  }
  pushHint.hidden = true;
  if (!pushSupported()) {
    setText(pushStateNode, pushUnsupportedText(PUSH_UNSUPPORTED_TEXT));
    pushEnableButton.hidden = true;
    pushTestButton.hidden = true;
    pushDisableButton.hidden = true;
    return;
  }
  if (view.state !== "active") {
    if (view.reason === "store_missing") {
      setText(
        pushStateNode,
        "The notification store was removed on the PC — run soos-remote push reset"
      );
    } else {
      setText(pushStateNode, "Notifications are unavailable on the PC");
    }
    pushEnableButton.hidden = true;
    pushTestButton.hidden = true;
    pushDisableButton.hidden = true;
    return;
  }
  const count = typeof view.subscriptions === "number" ? view.subscriptions : 0;
  let text =
    count === 0
      ? "No device receives notifications yet"
      : count === 1
        ? "1 device receives notifications"
        : count + " devices receive notifications";
  if (view.sender === "unavailable") {
    text += ". Push sender not running on the PC";
  } else if (view.last_delivery === "rejected") {
    text += ". The push service refused the last notification — see setup";
  } else if (view.last_delivery === "failed") {
    text += ". The last notification could not be sent";
  }
  setText(pushStateNode, text);
  const devices = Array.isArray(view.devices) ? view.devices : [];
  devices.forEach(function (device) {
    const item = document.createElement("li");
    const service = labelOf(PUSH_SERVICES, device.service);
    const when =
      typeof device.created_unix_s === "number"
        ? new Date(device.created_unix_s * 1000).toLocaleString()
        : "?";
    setText(item, service + " device, enabled " + when);
    pushDevices.appendChild(item);
  });
  pushHint.hidden = devices.length === 0;
  pushEnableButton.hidden = false;
  pushTestButton.hidden = count === 0;
  pushDisableButton.hidden = count === 0;
}

// Keeps the PC in sync with this phone's subscription; a subscription made with an older
// key (after soos-remote push reset) is dropped locally and must be enabled again. A
// subscription whose key the browser does not expose is kept and re-sent.
function syncSubscription(view) {
  if (view.state !== "active" || !pushSupported() || Notification.permission !== "granted") {
    return;
  }
  registerServiceWorker()
    .then(function (registration) {
      return registration === null ? null : registration.pushManager.getSubscription();
    })
    .then(function (subscription) {
      if (subscription === null) {
        return null;
      }
      if (serverKeyMatch(subscription, view.public_key) === "different") {
        return subscription.unsubscribe().then(function () {
          setText(pushFeedback, "Notifications must be re-enabled on this phone");
          pushEnableButton.hidden = false;
        });
      }
      return postJson(PUSH_SUBSCRIBE_PATH, "push-subscribe", subscription.toJSON());
    })
    .catch(function () {
      return null;
    });
}

function fetchPush() {
  fetch(PUSH_PATH, { method: "GET", cache: "no-store" })
    .then(function (response) {
      if (!response.ok) {
        throw new Error("status " + response.status);
      }
      return response.json();
    })
    .then(function (view) {
      renderPush(view);
      syncSubscription(view);
    })
    .catch(function () {
      pushSection.hidden = true;
    });
}

// The permission request is the first step of the tap (browsers require a user gesture).
async function enableNotifications() {
  if (!pushSupported()) {
    setText(pushFeedback, pushUnsupportedText(PUSH_UNSUPPORTED_TEXT));
    return;
  }
  if (pushView === null || pushView.state !== "active" || typeof pushView.public_key !== "string") {
    setText(pushFeedback, "Notifications are unavailable on the PC");
    return;
  }
  const publicKey = pushView.public_key;
  try {
    const permission = await Notification.requestPermission();
    if (permission !== "granted") {
      setText(pushFeedback, PUSH_BLOCKED_TEXT);
      return;
    }
    // The button is off only from a granted permission on, and the rest of the tap ends
    // within SERVICE_WORKER_READY_TIMEOUT_MS (never held by an open permission prompt).
    pushEnableButton.disabled = true;
    const deadline = Date.now() + SERVICE_WORKER_READY_TIMEOUT_MS;
    const registration = await readyRegistration();
    if (registration === null) {
      setText(pushFeedback, PUSH_WORKER_FAILED_TEXT);
      return;
    }
    const subscription = await beforeDeadline(
      registration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: fromBase64Url(publicKey),
      }),
      deadline,
      PUSH_SLOW_TEXT
    );
    const outcome = await beforeDeadline(
      postJson(PUSH_SUBSCRIBE_PATH, "push-subscribe", subscription.toJSON()).catch(function () {
        throw fixedError(PUSH_UNREACHABLE_TEXT);
      }),
      deadline,
      PUSH_TIMEOUT_TEXT
    );
    if (outcome.status !== 200) {
      if (outcome.result === "unsupported_push_service") {
        // The PC refuses this browser's push service (Edge on a computer): drop it here too.
        subscription.unsubscribe().catch(function () {
          return false;
        });
      }
      throw fixedError(reasonFor(outcome.result, PUSH_REASONS, "Refused (" + outcome.status + ")"));
    }
    setText(pushFeedback, "Notifications enabled on this phone");
  } catch (error) {
    setText(pushFeedback, failureText(error, PUSH_ERROR_TEXTS, "Notifications not enabled"));
  } finally {
    pushEnableButton.disabled = false;
    fetchPush();
  }
}

function sendTestNotification() {
  pushTestButton.disabled = true;
  postJson(PUSH_TEST_PATH, "push-test")
    .then(function (outcome) {
      if (outcome.status === 202) {
        setText(pushFeedback, "Test notification sent");
      } else {
        setText(pushFeedback, reasonFor(outcome.result, PUSH_REASONS, "Refused (" + outcome.status + ")"));
      }
    })
    .catch(function () {
      setText(pushFeedback, "The PC is unreachable");
    })
    .finally(function () {
      pushTestButton.disabled = false;
      setTimeout(fetchPush, 3000);
    });
}

function disableNotifications() {
  if (!pushSupported()) {
    setText(pushFeedback, pushUnsupportedText(PUSH_UNSUPPORTED_TEXT));
    return;
  }
  pushDisableButton.disabled = true;
  registerServiceWorker()
    .then(function (registration) {
      return registration === null ? null : registration.pushManager.getSubscription();
    })
    .then(function (subscription) {
      if (subscription === null) {
        setText(pushFeedback, "This phone has no subscription");
        return null;
      }
      const endpoint = subscription.endpoint;
      return subscription.unsubscribe().then(function () {
        return postJson(PUSH_UNSUBSCRIBE_PATH, "push-unsubscribe", { endpoint: endpoint });
      });
    })
    .then(function (outcome) {
      if (outcome && outcome.status === 200) {
        setText(pushFeedback, "Notifications disabled on this phone");
      } else if (outcome) {
        setText(pushFeedback, reasonFor(outcome.result, PUSH_REASONS, "Refused (" + outcome.status + ")"));
      }
    })
    .catch(function () {
      setText(pushFeedback, "Notifications not disabled");
    })
    .finally(function () {
      pushDisableButton.disabled = false;
      fetchPush();
    });
}

// --- live camera view (ADR 2026-10-07) --------------------------------------------------

const CAMERA_PATH = "/api/camera";
const CAMERA_OPTIONS_PATH = "/api/auth/camera/options";
const CAMERA_START_PATH = "/api/camera/start";
const CAMERA_STOP_PATH = "/api/camera/stop";
const CAMERA_BOUNDARY = "--soosframe";
// Largest JPEG part accepted (the server never sends more).
const CAMERA_MAX_PART_BYTES = 524288;
// Largest amount of unparsed stream bytes kept; beyond it the view is aborted.
const CAMERA_MAX_BUFFER_BYTES = 1048576;
const CAMERA_START_LABEL = "Start camera view";
const CAMERA_STOP_LABEL = "Stop camera view";
const CAMERA_STREAM_PATTERN = /^\/api\/camera\/stream\/[A-Za-z0-9_-]{43}$/;
const CAMERA_LIVE_TEXT = "Live view of the PC camera (tap the image for full screen)";
const CAMERA_UNSUPPORTED_TEXT =
  "This browser cannot show the live camera view (no streamed download or image decoding): use Safari, Chrome, Firefox or Samsung Internet";
// Stage class of the portrait mode (the image turned by 90°); landscape has no class.
const CAMERA_PORTRAIT_CLASS = "camera-portrait";

const CAMERA_REASONS = {
  camera_refused:
    "The PC refused the camera view (check the [preview] settings and that you are logged in at the PC)",
  camera_unavailable: "The camera of the PC is unavailable, try again",
  camera_format_unsupported: "The camera format of the PC is not supported",
  view_in_progress: "A camera view is already in progress",
  camera_tailnet_only: "Camera view is available on the tailnet only",
  camera_disabled: "The camera view is off on the PC (camera_view in remote.toml)",
  view_token_rejected: "The camera view expired, start it again",
  passkey_rejected: "The passkey was not accepted, try again",
};

let camera = null;
let cameraController = null;
let cameraDecoding = false;
// True in portrait mode; kept for the page lifetime, never stored.
let cameraPortrait = false;
let cameraFull = false;

function cameraReason(result, status) {
  return reasonFor(result, CAMERA_REASONS, "Camera view refused (" + status + ")");
}

function buildCameraCard() {
  if (camera !== null) {
    return camera;
  }
  const section = document.createElement("section");
  section.className = "card camera";
  section.hidden = true;
  section.setAttribute("aria-live", "polite");
  const title = document.createElement("h2");
  setText(title, "Camera");
  const stateLine = document.createElement("p");
  stateLine.className = "detail";
  setText(stateLine, " ");
  const stage = document.createElement("div");
  stage.className = "camera-stage";
  stage.hidden = true;
  const canvas = document.createElement("canvas");
  canvas.className = "camera-canvas";
  canvas.width = 0;
  canvas.height = 0;
  canvas.tabIndex = 0;
  canvas.setAttribute("role", "button");
  canvas.setAttribute("aria-label", "Live camera image, toggle full screen");
  const tools = document.createElement("div");
  tools.className = "camera-tools";
  const orientationButton = document.createElement("button");
  orientationButton.type = "button";
  orientationButton.className = "camera-tool";
  const closeButton = document.createElement("button");
  closeButton.type = "button";
  closeButton.className = "camera-tool";
  closeButton.hidden = true;
  closeButton.setAttribute("aria-label", "Exit full screen");
  setText(closeButton, "✕");
  tools.appendChild(orientationButton);
  tools.appendChild(closeButton);
  stage.appendChild(canvas);
  stage.appendChild(tools);
  const startButton = document.createElement("button");
  startButton.type = "button";
  setText(startButton, CAMERA_START_LABEL);
  const stopButton = document.createElement("button");
  stopButton.type = "button";
  stopButton.className = "secondary";
  stopButton.hidden = true;
  setText(stopButton, CAMERA_STOP_LABEL);
  const feedback = document.createElement("p");
  feedback.className = "feedback";
  feedback.setAttribute("role", "status");
  setText(feedback, " ");
  section.appendChild(title);
  section.appendChild(stateLine);
  section.appendChild(stage);
  section.appendChild(startButton);
  section.appendChild(stopButton);
  section.appendChild(feedback);
  pushSection.parentNode.insertBefore(section, pushSection.nextSibling);
  startButton.addEventListener("click", startCameraView);
  canvas.addEventListener("click", toggleCameraFullscreen);
  canvas.addEventListener("keydown", function (event) {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      toggleCameraFullscreen();
    }
  });
  orientationButton.addEventListener("click", toggleCameraOrientation);
  closeButton.addEventListener("click", exitCameraFullscreen);
  stopButton.addEventListener("click", function () {
    stopCameraView("Camera view stopped");
  });
  camera = {
    section: section,
    stateLine: stateLine,
    stage: stage,
    canvas: canvas,
    orientationButton: orientationButton,
    closeButton: closeButton,
    startButton: startButton,
    stopButton: stopButton,
    feedback: feedback,
  };
  applyCameraOrientation(camera);
  return camera;
}

// Applies the current mode; the button names the mode it switches to.
function applyCameraOrientation(c) {
  c.stage.classList.toggle(CAMERA_PORTRAIT_CLASS, cameraPortrait);
  setText(c.orientationButton, cameraPortrait ? "Landscape" : "Portrait");
  c.orientationButton.setAttribute(
    "aria-label",
    cameraPortrait ? "Switch to landscape mode" : "Switch to portrait mode"
  );
}

function toggleCameraOrientation() {
  cameraPortrait = !cameraPortrait;
  applyCameraOrientation(buildCameraCard());
}

function fullscreenElement() {
  return document.fullscreenElement || document.webkitFullscreenElement || null;
}

// Full screen for the open view: the stage becomes a fixed overlay (iPhone Safari has no
// element fullscreen) and, where available, the element fullscreen of the browser.
function enterCameraFullscreen() {
  const c = buildCameraCard();
  if (cameraController === null || cameraFull) {
    return;
  }
  cameraFull = true;
  c.stage.classList.add("camera-full");
  document.documentElement.classList.add("camera-lock");
  c.closeButton.hidden = false;
  const request = c.stage.requestFullscreen || c.stage.webkitRequestFullscreen;
  if (typeof request === "function") {
    try {
      const pending = request.call(c.stage);
      if (pending && typeof pending.catch === "function") {
        pending.catch(function () {
          // Refused by the browser: the overlay stays.
        });
      }
    } catch (_) {
      // Refused by the browser: the overlay stays.
    }
  }
}

function exitCameraFullscreen() {
  const c = buildCameraCard();
  cameraFull = false;
  c.stage.classList.remove("camera-full");
  document.documentElement.classList.remove("camera-lock");
  c.closeButton.hidden = true;
  if (fullscreenElement() === c.stage) {
    const exit = document.exitFullscreen || document.webkitExitFullscreen;
    if (typeof exit === "function") {
      try {
        const pending = exit.call(document);
        if (pending && typeof pending.catch === "function") {
          pending.catch(function () {
            // Already left.
          });
        }
      } catch (_) {
        // Already left.
      }
    }
  }
}

function toggleCameraFullscreen() {
  if (cameraFull) {
    exitCameraFullscreen();
  } else {
    enterCameraFullscreen();
  }
}

// The browser left its element fullscreen (Escape, system gesture): leave the overlay too.
function cameraFullscreenChanged() {
  if (cameraFull && fullscreenElement() === null) {
    exitCameraFullscreen();
  }
}

function clearCameraCanvas() {
  const c = buildCameraCard();
  const context = c.canvas.getContext("2d");
  if (context) {
    context.clearRect(0, 0, c.canvas.width, c.canvas.height);
  }
  c.canvas.width = 0;
  c.canvas.height = 0;
}

function renderCamera(view) {
  const c = buildCameraCard();
  if (signedOut || !view || view.enabled !== true) {
    c.section.hidden = true;
    return;
  }
  c.section.hidden = false;
  if (view.reachable === false) {
    setText(c.stateLine, "Camera view is available on the tailnet only");
    c.startButton.hidden = true;
    c.stopButton.hidden = true;
    return;
  }
  const caps = browserCapabilities();
  if (!(caps.streams && caps.imageBitmap)) {
    setText(c.stateLine, CAMERA_UNSUPPORTED_TEXT);
    c.startButton.hidden = true;
    c.stopButton.hidden = true;
    return;
  }
  if (!caps.webauthn) {
    c.startButton.disabled = true;
    setText(c.feedback, PASSKEY_UNSUPPORTED_TEXT);
  }
  const streaming = cameraController !== null;
  c.startButton.hidden = streaming;
  c.stopButton.hidden = !streaming;
  if (streaming) {
    setText(c.stateLine, CAMERA_LIVE_TEXT);
  } else if (view.state === "idle") {
    setText(c.stateLine, "Up to " + view.max_view_s + " s at " + view.fps + " frames per second");
  } else {
    setText(c.stateLine, "A camera view is in progress");
  }
}

function fetchCamera() {
  if (signedOut) {
    return;
  }
  fetch(CAMERA_PATH, { method: "GET", cache: "no-store" })
    .then(function (response) {
      if (!response.ok) {
        throw new Error("status " + response.status);
      }
      return response.json();
    })
    .then(renderCamera)
    .catch(function () {
      buildCameraCard().section.hidden = true;
    });
}

// Draws one JPEG part; a part arriving while the previous one decodes is dropped.
function showCameraFrame(jpeg, controller) {
  if (cameraDecoding || !browserCapabilities().imageBitmap) {
    return;
  }
  cameraDecoding = true;
  createImageBitmap(new Blob([jpeg], { type: "image/jpeg" }))
    .then(function (bitmap) {
      const c = buildCameraCard();
      if (cameraController !== controller) {
        bitmap.close();
        return;
      }
      if (c.canvas.width !== bitmap.width || c.canvas.height !== bitmap.height) {
        c.canvas.width = bitmap.width;
        c.canvas.height = bitmap.height;
      }
      const context = c.canvas.getContext("2d");
      if (context) {
        context.drawImage(bitmap, 0, 0);
      }
      bitmap.close();
    })
    .catch(function () {
      // An undecodable part is skipped; the next one replaces it.
    })
    .finally(function () {
      cameraDecoding = false;
    });
}

function asciiBytes(text) {
  const out = new Uint8Array(text.length);
  for (let i = 0; i < text.length; i += 1) {
    out[i] = text.charCodeAt(i) & 0x7f;
  }
  return out;
}

function startsWithBytes(buffer, prefix) {
  if (buffer.length < prefix.length) {
    return false;
  }
  for (let i = 0; i < prefix.length; i += 1) {
    if (buffer[i] !== prefix[i]) {
      return false;
    }
  }
  return true;
}

function headEndOf(buffer, from) {
  for (let i = from; i + 3 < buffer.length; i += 1) {
    if (buffer[i] === 13 && buffer[i + 1] === 10 && buffer[i + 2] === 13 && buffer[i + 3] === 10) {
      return i;
    }
  }
  return -1;
}

// Reads the multipart stream: `--soosframe`, part headers, `Content-Length` digits only
// (at most CAMERA_MAX_PART_BYTES), the JPEG bytes and a CRLF; anything else aborts.
function readCameraStream(response, controller) {
  if (!browserCapabilities().streams) {
    return Promise.reject(fixedError(CAMERA_UNSUPPORTED_TEXT));
  }
  const reader = response.body.getReader();
  const opener = asciiBytes(CAMERA_BOUNDARY + "\r\n");
  const closer = asciiBytes(CAMERA_BOUNDARY + "--");
  let buffer = new Uint8Array(0);

  function append(chunk) {
    if (buffer.length + chunk.length > CAMERA_MAX_BUFFER_BYTES) {
      throw fixedError("The camera stream exceeded its bound");
    }
    const next = new Uint8Array(buffer.length + chunk.length);
    next.set(buffer, 0);
    next.set(chunk, buffer.length);
    buffer = next;
  }

  // True when the closing boundary arrived.
  function parse() {
    for (;;) {
      if (startsWithBytes(buffer, closer)) {
        return true;
      }
      if (buffer.length < opener.length) {
        return false;
      }
      if (!startsWithBytes(buffer, opener)) {
        throw fixedError("The camera stream is malformed");
      }
      const headEnd = headEndOf(buffer, opener.length);
      if (headEnd < 0) {
        if (buffer.length > 1024) {
          throw fixedError("The camera stream is malformed");
        }
        return false;
      }
      let head = "";
      for (let i = opener.length; i < headEnd; i += 1) {
        head += String.fromCharCode(buffer[i]);
      }
      let length = -1;
      head.split("\r\n").forEach(function (line) {
        const colon = line.indexOf(":");
        if (colon > 0 && line.slice(0, colon).trim().toLowerCase() === "content-length") {
          const value = line.slice(colon + 1).trim();
          if (/^[0-9]{1,7}$/.test(value)) {
            length = Number(value);
          }
        }
      });
      if (length <= 0 || length > CAMERA_MAX_PART_BYTES) {
        throw fixedError("The camera stream is malformed");
      }
      const start = headEnd + 4;
      if (buffer.length < start + length + 2) {
        return false;
      }
      const jpeg = buffer.slice(start, start + length);
      buffer = buffer.slice(start + length + 2);
      showCameraFrame(jpeg, controller);
    }
  }

  function pump() {
    return reader.read().then(function (result) {
      if (result.done) {
        return null;
      }
      append(result.value);
      if (parse()) {
        return null;
      }
      return pump();
    });
  }
  return pump();
}

function endCameraView(message) {
  const c = buildCameraCard();
  const controller = cameraController;
  cameraController = null;
  if (controller !== null) {
    controller.abort();
  }
  exitCameraFullscreen();
  c.stage.hidden = true;
  clearCameraCanvas();
  c.startButton.disabled = !browserCapabilities().webauthn;
  if (message) {
    setText(c.feedback, message);
  }
  fetchCamera();
}

// Ends the view here and asks the PC to stop it (also from a hidden or closing page).
function stopCameraView(message) {
  if (cameraController === null) {
    return;
  }
  fetch(CAMERA_STOP_PATH, {
    method: "POST",
    headers: { "X-Soos-Action": "camera-stop" },
    keepalive: true,
  }).catch(function () {
    // The PC also ends the view when the stream closes.
  });
  endCameraView(message);
}

function openCameraStream(path) {
  const c = buildCameraCard();
  const controller = new AbortController();
  cameraController = controller;
  setText(c.feedback, "Starting the camera…");
  return fetch(path, {
    headers: { "X-Soos-Action": "camera-stream" },
    signal: controller.signal,
    cache: "no-store",
  })
    .then(function (response) {
      if (response.status !== 200) {
        return response
          .json()
          .catch(function () {
            return {};
          })
          .then(function (body) {
            throw fixedError(cameraReason(body.result, response.status));
          });
      }
      const type = response.headers.get("Content-Type") || "";
      if (type.indexOf("application/octet-stream") !== 0 || !response.body) {
        throw fixedError("The camera stream is malformed");
      }
      setText(c.feedback, " ");
      c.startButton.hidden = true;
      c.stopButton.hidden = false;
      c.stage.hidden = false;
      setText(c.stateLine, CAMERA_LIVE_TEXT);
      return readCameraStream(response, controller);
    })
    .then(function () {
      if (cameraController === controller) {
        endCameraView("The camera view ended");
      }
    })
    .catch(function (error) {
      if (cameraController === controller) {
        endCameraView(failureText(error, {}, "The camera view ended"));
      }
    });
}

function startCameraView() {
  const c = buildCameraCard();
  if (!browserCapabilities().webauthn) {
    setText(c.feedback, PASSKEY_UNSUPPORTED_TEXT);
    return;
  }
  // A deliberate second tap, then the passkey check: a stray tap must never turn the camera on.
  if (!window.confirm("Start the live camera view of the PC? Its camera light turns on.")) {
    return;
  }
  c.startButton.disabled = true;
  setText(c.feedback, PASSKEY_PROMPT_TEXT);
  postJson(CAMERA_OPTIONS_PATH, "camera-options")
    .then(function (options) {
      if (options.status !== 200) {
        throw fixedError(cameraReason(options.result, options.status));
      }
      return getAssertion(options.body, true);
    })
    .then(function (credential) {
      return fetch(CAMERA_START_PATH, {
        method: "POST",
        cache: "no-store",
        headers: { "X-Soos-Action": "camera-view", "Content-Type": "application/json" },
        body: JSON.stringify(assertionBody(credential)),
      });
    })
    .then(function (response) {
      return response
        .json()
        .catch(function () {
          return {};
        })
        .then(function (body) {
          return { status: response.status, body: body };
        });
    })
    .then(function (outcome) {
      if (outcome.body.result === "login_required") {
        showLogin("Your session ended, please sign in again");
        return null;
      }
      if (outcome.status !== 200) {
        throw fixedError(cameraReason(outcome.body.result, outcome.status));
      }
      const path = outcome.body.stream_path;
      if (typeof path !== "string" || !CAMERA_STREAM_PATTERN.test(path)) {
        throw fixedError("The PC answered an unexpected camera address");
      }
      return openCameraStream(path);
    })
    .catch(function (error) {
      setText(c.feedback, failureText(error, PASSKEY_ERROR_TEXTS, "Camera view cancelled"));
      fetchCamera();
    })
    .finally(function () {
      c.startButton.disabled = !browserCapabilities().webauthn;
    });
}

// --- feature tour (ADR 2026-10-17, GitHub #414) -----------------------------------------

// Fixed English text only: the tour reads no server data, makes no request and switches
// nothing. The six features shared with the soos-gui tour keep its titles, explanations and
// "Off:" lines (crates/gui/src/onboarding.rs); "where" says where to turn a feature off
// (manual edits name every key that depends on it, which soos-remote would refuse otherwise),
// and "target" names the card of this page that "Show it on this page" moves to ("" for none).
const TOUR_INTRO_TITLE = "Welcome to soos";
const TOUR_INTRO_TEXT =
  "This page shows the lock state of your PC and lets this phone act on it. The next steps explain each feature it offers, what it exposes and where to turn it off. The tour itself changes no setting.";
const TOUR_INTRO_NOTE =
  "This browser only remembers that you closed the tour, nothing else. You can reopen it at any time with Feature tour at the bottom of the page.";
const TOUR_STEPS = [
  {
    title: "Phone companion",
    text: "A small service in your user session shows the lock state of this PC on your phone and lets you lock it remotely. It listens only on a local socket that Tailscale publishes to devices of your tailnet.",
    off: "Off: the service is stopped and disabled; the phone page is unreachable.",
    where:
      "On the PC: Features in soos-gui, or run as your own user: systemctl --user disable --now soos-remote.service",
    target: "",
  },
  {
    title: "Battery level",
    text: "The status card shows the battery level of the PC, whether it is charging and whether it is plugged in. The PC reads it from its power supply information; nothing else about the hardware is sent.",
    off: "Off: the PC stops reading its battery and the line disappears from this page.",
    where:
      "On the PC: set battery_status = false in ~/.config/soos/remote.toml, then run: systemctl --user restart soos-remote.service",
    target: "status",
  },
  {
    title: "Remote lock and unlock",
    text: "Locking from the phone is always available while the companion runs. Unlocking from the phone additionally needs a passkey check (Face ID, fingerprint or screen lock) every time.",
    off: "Off: the phone can still lock this PC, never unlock it.",
    where:
      "Unlock, on the PC: Features in soos-gui, or set allow_unlock = false in ~/.config/soos/remote.toml, then run: systemctl --user restart soos-remote.service. Locking stays available while the companion runs.",
    target: "status",
  },
  {
    title: "Internet access",
    text: "Tailscale Funnel makes the phone page reachable from the Internet without the VPN app on the phone. Every action still needs your passkey, but the page is then exposed beyond your tailnet.",
    off: "Off: the page is reachable only from devices signed in to your tailnet.",
    where:
      "On this phone, Sign out ends this phone's session. On the PC: set allow_funnel = false and camera_view_funnel = false in ~/.config/soos/remote.toml and remove its [android] table if there is one, then run: systemctl --user restart soos-remote.service. Without an [android] table, Features in soos-gui does the same.",
    target: "logout",
  },
  {
    title: "Failed-password history",
    text: "This page lists failed password attempts on the PC, read from the system journal: the time, the source (lock screen, sudo, login), the account class and the count, never the typed text.",
    off: "Off: the PC stops reading the journal and the list disappears; push alerts and offline alerts depend on it and stop too.",
    where:
      "On the PC: set password_alerts = false, push_notifications = false and offline_alerts = false in ~/.config/soos/remote.toml, then run: systemctl --user restart soos-remote.service",
    target: "alerts",
  },
  {
    title: "Push alerts",
    text: "Failed password attempts on this PC are pushed to your phone as notifications. Alerts carry the time, source, account and count, never the typed text.",
    off: "Off: no notification is sent; alerts remain visible on the phone page.",
    where:
      "This phone only: Disable notifications in the Notifications card. Every device, on the PC: Features in soos-gui, or set push_notifications = false and offline_alerts = false in ~/.config/soos/remote.toml, then run: systemctl --user restart soos-remote.service",
    target: "push",
  },
  {
    title: "Live camera view",
    text: "You can watch the PC camera live from the phone, for a limited time and after a passkey check per view. Nothing is recorded or stored.",
    off: "Off: the phone can no longer open the camera of this PC.",
    where:
      "On the PC: Features in soos-gui, or set camera_view = false and camera_view_funnel = false in ~/.config/soos/remote.toml, then run: systemctl --user restart soos-remote.service. A running view stops with Stop camera view or when you leave this page.",
    target: "camera",
  },
  {
    title: "Offline alerts",
    text: "Your phone is told when this PC comes back online after being offline, with how long it was away. It uses the push alerts, so those must stay on.",
    off: "Off: no offline history is recorded and no back-online notification is sent; push alerts keep working.",
    where:
      "On the PC: Features in soos-gui, or set offline_alerts = false in ~/.config/soos/remote.toml, then run: systemctl --user restart soos-remote.service",
    target: "offline",
  },
];
const TOUR_NEXT_LABEL = "Next";
const TOUR_DONE_LABEL = "Done";
const TOUR_BACK_LABEL = "Back";
const TOUR_SKIP_LABEL = "Skip tour";
const TOUR_SHOW_LABEL = "Show it on this page";
const TOUR_REPLAY_LABEL = "Feature tour";
const TOUR_REGION_LABEL = "Feature tour";

// soos-tour-storage:begin
// The only storage of the page (owner decision of 2026-10-17 on GitHub #363): one key whose
// only value is a fixed word, meaning "the tour was closed once in this browser"; nothing
// personal. Every access, the property read included (it throws when site data is blocked),
// sits in the one try block: a blocked or failing storage reads as "not seen", so the tour
// shows again and is dismissed as usual.
const TOUR_STORAGE_KEY = "soos-remote-tour-v1";
const TOUR_STORAGE_SEEN = "seen";

// With markSeen === true, remembers the closed tour; otherwise answers whether it was.
function tourStorage(markSeen) {
  try {
    if (markSeen === true) {
      window.localStorage.setItem(TOUR_STORAGE_KEY, TOUR_STORAGE_SEEN);
      return true;
    }
    return window.localStorage.getItem(TOUR_STORAGE_KEY) === TOUR_STORAGE_SEEN;
  } catch (_) {
    return false;
  }
}
// soos-tour-storage:end

let tour = null;
// The page shown: 0 is the introduction, 1..TOUR_STEPS.length the features.
let tourIndex = 0;
// True once the first-run tour was offered in this page lifetime.
let tourOffered = false;
// True while a tour opened from the Feature tour button is shown (the focus goes back to it).
let tourReplayed = false;

// Builds the tour card (top of the page, hidden) and the Feature tour button of the footer.
function buildTourCard() {
  if (tour !== null) {
    return tour;
  }
  const section = document.createElement("section");
  section.className = "card tour";
  section.hidden = true;
  section.setAttribute("aria-label", TOUR_REGION_LABEL);
  const counter = document.createElement("p");
  counter.className = "tour-step";
  const title = document.createElement("h2");
  title.tabIndex = -1;
  const text = document.createElement("p");
  text.className = "tour-text";
  const off = document.createElement("p");
  off.className = "detail";
  const where = document.createElement("p");
  where.className = "banner tour-where";
  const showButton = document.createElement("button");
  showButton.type = "button";
  showButton.className = "secondary";
  setText(showButton, TOUR_SHOW_LABEL);
  const nav = document.createElement("div");
  nav.className = "tour-nav";
  const backButton = document.createElement("button");
  backButton.type = "button";
  backButton.className = "secondary";
  setText(backButton, TOUR_BACK_LABEL);
  const nextButton = document.createElement("button");
  nextButton.type = "button";
  setText(nextButton, TOUR_NEXT_LABEL);
  nav.appendChild(backButton);
  nav.appendChild(nextButton);
  const skipButton = document.createElement("button");
  skipButton.type = "button";
  skipButton.className = "secondary";
  setText(skipButton, TOUR_SKIP_LABEL);
  section.appendChild(counter);
  section.appendChild(title);
  section.appendChild(text);
  section.appendChild(off);
  section.appendChild(where);
  section.appendChild(showButton);
  section.appendChild(nav);
  section.appendChild(skipButton);
  loginSection.parentNode.insertBefore(section, loginSection);
  const replayButton = document.createElement("button");
  replayButton.type = "button";
  replayButton.className = "tour-replay";
  setText(replayButton, TOUR_REPLAY_LABEL);
  const footer = document.querySelector("footer");
  if (footer !== null) {
    footer.insertBefore(replayButton, footer.firstChild);
  }
  replayButton.addEventListener("click", openTour);
  nextButton.addEventListener("click", tourNext);
  backButton.addEventListener("click", tourBack);
  showButton.addEventListener("click", showTourTarget);
  skipButton.addEventListener("click", closeTour);
  tour = {
    section: section,
    counter: counter,
    title: title,
    text: text,
    off: off,
    where: where,
    showButton: showButton,
    backButton: backButton,
    nextButton: nextButton,
    skipButton: skipButton,
    replayButton: replayButton,
  };
  return tour;
}

// True when neither the node nor any ancestor is hidden.
function shownOnPage(node) {
  for (let n = node; n !== null && n !== document.body; n = n.parentNode) {
    if (n.hidden === true) {
      return false;
    }
  }
  return true;
}

// The card of this page a step points to, or null when it is not shown right now.
function tourTarget(name) {
  let node = null;
  if (name === "status") {
    node = statusCard;
  } else if (name === "logout") {
    node = logoutButton;
  } else if (name === "alerts") {
    node = alertsSection;
  } else if (name === "push") {
    node = pushSection;
  } else if (name === "offline") {
    node = offlineSection;
  } else if (name === "camera" && camera !== null) {
    node = camera.section;
  }
  return node !== null && shownOnPage(node) ? node : null;
}

function renderTour() {
  const t = buildTourCard();
  const index = Math.max(0, Math.min(tourIndex, TOUR_STEPS.length));
  const total = TOUR_STEPS.length + 1;
  tourIndex = index;
  setText(t.counter, "Step " + (index + 1) + " of " + total);
  if (index === 0) {
    setText(t.title, TOUR_INTRO_TITLE);
    setText(t.text, TOUR_INTRO_TEXT);
    setText(t.off, TOUR_INTRO_NOTE);
    setText(t.where, " ");
    t.where.hidden = true;
    t.showButton.hidden = true;
  } else {
    const step = TOUR_STEPS[index - 1];
    setText(t.title, step.title);
    setText(t.text, step.text);
    setText(t.off, step.off);
    setText(t.where, step.where);
    t.where.hidden = false;
    t.showButton.hidden = tourTarget(step.target) === null;
  }
  t.backButton.hidden = index === 0;
  t.skipButton.hidden = index === TOUR_STEPS.length;
  setText(t.nextButton, index === TOUR_STEPS.length ? TOUR_DONE_LABEL : TOUR_NEXT_LABEL);
}

// Opens the tour on its introduction and moves the focus to its title; `event` is set when
// the Feature tour button opened it.
function openTour(event) {
  const t = buildTourCard();
  tourReplayed = event !== undefined;
  tourIndex = 0;
  renderTour();
  t.section.hidden = false;
  t.title.focus();
}

// Skip, Done, Escape or a link: the card is hidden first (the focus goes back to the Feature
// tour button when the tour was opened from it), then the closed tour is remembered.
function closeTour() {
  const t = buildTourCard();
  t.section.hidden = true;
  if (tourReplayed) {
    tourReplayed = false;
    t.replayButton.focus();
  }
  tourStorage(true);
}

function tourNext() {
  if (tourIndex >= TOUR_STEPS.length) {
    closeTour();
    return;
  }
  tourIndex += 1;
  renderTour();
}

function tourBack() {
  const t = buildTourCard();
  if (tourIndex > 0) {
    tourIndex -= 1;
    renderTour();
  }
  if (tourIndex === 0) {
    // The Back button is now hidden: keep the focus in the card.
    t.nextButton.focus();
  }
}

// Closes the tour and moves the focus (and so the scroll position) to the card of the step;
// nothing is clicked or changed.
function showTourTarget() {
  const step = TOUR_STEPS[tourIndex - 1];
  const node = step === undefined ? null : tourTarget(step.target);
  if (node === null) {
    renderTour();
    return;
  }
  closeTour();
  if (node.tabIndex < 0 && !node.hasAttribute("tabindex")) {
    node.setAttribute("tabindex", "-1");
  }
  node.focus();
}

// The first-run tour: once per page lifetime, when this browser does not remember it.
function offerTour() {
  if (tourOffered) {
    return;
  }
  tourOffered = true;
  if (!tourStorage()) {
    openTour();
  }
}

// Escape closes an open tour (a camera full screen takes Escape first).
function tourKeydown(event) {
  if (event.key === "Escape" && !cameraFull && tour !== null && !tour.section.hidden) {
    closeTour();
  }
}

// --- life cycle -----------------------------------------------------------------------

function resume() {
  fetch(AUTH_STATE_PATH, { method: "GET", cache: "no-store" })
    .then(function (response) {
      if (!response.ok) {
        throw new Error("status " + response.status);
      }
      return response.json();
    })
    .then(function (state) {
      if (state.mode === "funnel" && state.authenticated !== true) {
        showLogin();
        return;
      }
      showApp(state);
      openStream();
      fetchStatus(true);
      fetchPush();
      fetchCamera();
    })
    .catch(function () {
      render(latest === null ? { state: "unreachable" } : latest, false);
    });
}

setText(lockButton, LOCK_LABEL);
setText(unlockButton, UNLOCK_LABEL);
setText(pushEnableButton, PUSH_ENABLE_LABEL);
setText(pushTestButton, PUSH_TEST_LABEL);
setText(pushDisableButton, PUSH_DISABLE_LABEL);
lockButton.addEventListener("click", requestLock);
unlockButton.addEventListener("click", requestUnlock);
loginButton.addEventListener("click", requestLogin);
logoutButton.addEventListener("click", requestLogout);
enrollButton.addEventListener("click", requestEnroll);
alertsAckButton.addEventListener("click", requestAlertsAck);
pushEnableButton.addEventListener("click", enableNotifications);
pushTestButton.addEventListener("click", sendTestNotification);
pushDisableButton.addEventListener("click", disableNotifications);
document.addEventListener("visibilitychange", function () {
  if (document.visibilityState === "visible") {
    resume();
  } else {
    closeStream();
    stopCameraView("Camera view stopped (page hidden)");
  }
});
window.addEventListener("pagehide", function () {
  stopCameraView(null);
});
document.addEventListener("fullscreenchange", cameraFullscreenChanged);
document.addEventListener("webkitfullscreenchange", cameraFullscreenChanged);
// The tour listener runs first, so an Escape that leaves the camera full screen keeps the tour.
document.addEventListener("keydown", tourKeydown);
document.addEventListener("keydown", function (event) {
  if (event.key === "Escape" && cameraFull) {
    exitCameraFullscreen();
  }
});
buildTourCard();
window.addEventListener("pageshow", resume);
setInterval(renderUpdated, TICK_MS);
registerServiceWorker();
resume();
