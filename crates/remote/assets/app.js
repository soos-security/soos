/* soos remote companion page (GitHub #339, spec section 2.11; Funnel and passkey ADR
 * 2026-10-06, spec section 8).
 *
 * Server data only ever reaches the DOM through textContent. The page keeps no state
 * beyond the live EventSource: no script-readable cookie (the web session cookie is
 * HttpOnly), no storage. The only service worker (/sw.js) shows push notifications and
 * does nothing else. Staleness is measured with the browser's own
 * clock from the arrival of the last event, never by comparing checked_unix_ms with the
 * phone's clock.
 *
 * Passkeys: every WebAuthn ceremony is modal (a tap, then Face ID / Touch ID), always with
 * user verification required, and never names a credential: the server only answers with a
 * challenge, so the phone offers the passkey it holds for this site.
 *
 * Failed-password alerts (ADR 2026-10-06 "Failed-Password Alerts in soos-remote From the
 * System Journal"): the server only ever sends time, source class, account class, kind and
 * count, never the typed password. The page shows them with textContent, says "No failed
 * password attempts" only while the feature is active, and acknowledges exactly the view it
 * displayed (epoch and through headers; a stale view is re-fetched, never acknowledged).
 *
 * Push notifications (ADR 2026-10-06 "Web Push Notifications for Failed-Password Alerts
 * Through a Separate Sender Unit"): from the home-screen app, "Enable notifications" asks
 * the permission from the tap itself, subscribes with the PC's public VAPID key fetched
 * before the tap, and hands the subscription to the PC. A notification carries counts and
 * classes only, never the typed password.
 *
 * Live camera view (ADR 2026-10-07 "Live Camera View in soos-remote Through the Daemon
 * Preview Channel"): the card is built at runtime (no markup id). Every view needs a tap,
 * a confirmation and a fresh Face ID assertion; the multipart JPEG stream is read with
 * fetch and a ReadableStream into a bounded buffer, each part decoded with
 * createImageBitmap and drawn on a canvas, then released. No image element, no object
 * address, no storage; the view stops when the page is hidden or left. A tap on the live
 * image toggles full screen (element fullscreen where the browser has it, a fixed overlay
 * otherwise, as on iPhone Safari); Rotate turns the image by 90° steps through CSS classes
 * only. The rotation lives in memory for the page lifetime and is never stored. Every end
 * of a view leaves full screen. A new view may start right after the previous one ended
 * (owner request 2026-10-07), always after a fresh Face ID assertion.
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

// Results shared by every passkey ceremony.
const PASSKEY_REASONS = {
  passkey_rejected: "The passkey was not accepted, try again",
  passkey_required: "Face ID is required for every unlock",
  passkeys_not_configured: "Passkeys are not configured (rp_id in remote.toml)",
  no_passkey: "No passkey is registered yet",
  login_required: "Please sign in again",
  rate_limited: "Too many attempts, please wait a moment",
  too_many_challenges: "Too many pending requests, please wait a moment",
  too_many_sessions: "Too many signed-in devices, sign out elsewhere first",
  store_unavailable: "The passkey store is unavailable on the PC",
  busy: "The PC is busy, try again",
  registration_conflict:
    "Enrollment changed meanwhile: get a new code and retry; remove the extra passkey from the phone's Passwords app",
  body_too_large: "Request too large",
  bad_request: "Request refused (malformed)",
  enroll_code_rejected: "The enrollment code is wrong or expired",
  passkey_limit: "The maximum number of passkeys is registered",
  already_registered: "This passkey is already registered",
  unavailable: "The PC is unavailable",
  forbidden: "Request refused",
};

const stateNode = document.getElementById("state");
const activityNode = document.getElementById("activity");
const updatedNode = document.getElementById("updated");
const feedbackNode = document.getElementById("feedback");
const lockButton = document.getElementById("lock");
const unlockButton = document.getElementById("unlock");
const statusCard = document.getElementById("status-card");
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

let source = null;
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

function setText(node, text) {
  node.textContent = text;
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

// A modal passkey assertion over a server challenge (no credential is named).
function getAssertion(options) {
  return navigator.credentials.get({
    publicKey: {
      challenge: fromBase64Url(options.challenge),
      rpId: options.rp_id,
      timeout: options.timeout_ms,
      userVerification: "required",
    },
  });
}

// --- screens ------------------------------------------------------------------------

function showLogin(message) {
  signedOut = true;
  closeStream();
  latest = null;
  loginSection.hidden = false;
  statusCard.hidden = true;
  logoutButton.hidden = true;
  enrollSection.hidden = true;
  alertsSection.hidden = true;
  alertsView = null;
  pushSection.hidden = true;
  pushView = null;
  stopCameraView(null);
  buildCameraCard().section.hidden = true;
  setText(loginFeedback, message || " ");
}

function showApp(state) {
  signedOut = false;
  loginSection.hidden = true;
  statusCard.hidden = false;
  logoutButton.hidden = state.mode !== "funnel";
  enrollSection.hidden = !(state.mode === "tailnet" && state.enrollment === true);
}

function render(view, reachable) {
  const state = reachable ? view.state : "unreachable";
  const label = Object.prototype.hasOwnProperty.call(LABELS, state) ? LABELS[state] : LABELS.unknown;
  setText(stateNode, label);
  stateNode.dataset.state = state;
  stateNode.className = "state state-" + state.replace(/[^a-z_]/g, "");

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
  unlockButton.disabled = !(reachable && view.state === "locked");
  if (reachable && view.state === "locked" && lockRequestedAt !== 0) {
    confirmLock();
  }
  if (reachable && view.state === "unlocked" && unlockRequestedAt !== 0) {
    confirmUnlock();
  }
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
  source.onerror = function () {
    // EventSource reconnects on its own; one fetch decides between a transient blip, an
    // expired web session and an unreachable PC.
    fetchStatus(true);
  };
}

function closeStream() {
  if (source !== null) {
    source.close();
    source = null;
  }
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

// --- unlock (a fresh passkey assertion with Face ID every time) ------------------------

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
  // A deliberate second tap, then Face ID: a stray tap must never open the PC.
  if (!window.confirm("Unlock the PC now?")) {
    return;
  }
  unlockButton.disabled = true;
  setText(feedbackNode, "Confirm with Face ID…");

  postJson(UNLOCK_OPTIONS_PATH, "unlock-options")
    .then(function (options) {
      if (options.status !== 200) {
        unlockFailed(options.status, options.result);
        return null;
      }
      return getAssertion(options.body).then(function (credential) {
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
    .catch(function () {
      unlockFailed(0, "cancelled");
      setText(feedbackNode, "Unlock cancelled or failed");
    });
}

// --- sign in / sign out (internet access through Tailscale Funnel) ---------------------

function requestLogin() {
  loginButton.disabled = true;
  setText(loginFeedback, "Confirm with Face ID…");
  postJson(LOGIN_OPTIONS_PATH, "login-options")
    .then(function (options) {
      if (options.status !== 200) {
        throw new Error(reasonFor(options.result, {}, "Sign-in refused (" + options.status + ")"));
      }
      return getAssertion(options.body);
    })
    .then(function (credential) {
      return postJson(LOGIN_PATH, "login", assertionBody(credential));
    })
    .then(function (outcome) {
      if (outcome.status !== 200) {
        throw new Error(reasonFor(outcome.result, {}, "Sign-in refused (" + outcome.status + ")"));
      }
      setText(loginFeedback, " ");
      resume();
    })
    .catch(function (error) {
      setText(loginFeedback, error && error.message ? error.message : "Sign-in cancelled");
    })
    .finally(function () {
      loginButton.disabled = false;
    });
}

function requestLogout() {
  postJson(LOGOUT_PATH, "logout").finally(function () {
    showLogin("Signed out");
  });
}

// --- passkey enrollment (tailnet only, with a code from `soos-remote enroll-code`) -----

function requestEnroll() {
  const code = enrollCode.value.trim();
  if (code.length === 0) {
    setText(enrollFeedback, "Type the code printed by soos-remote enroll-code");
    return;
  }
  enrollButton.disabled = true;
  setText(enrollFeedback, "Confirm with Face ID…");
  postJson(REGISTER_OPTIONS_PATH, "register-options", { code: code })
    .then(function (options) {
      if (options.status !== 200) {
        throw new Error(reasonFor(options.result, {}, "Enrollment refused (" + options.status + ")"));
      }
      const o = options.body;
      return navigator.credentials.create({
        publicKey: {
          rp: { id: o.rp_id, name: "soos" },
          user: { id: fromBase64Url(o.user_id), name: "soos", displayName: "soos" },
          challenge: fromBase64Url(o.challenge),
          pubKeyCredParams: [{ type: "public-key", alg: -7 }],
          authenticatorSelection: {
            residentKey: "required",
            requireResidentKey: true,
            userVerification: "required",
          },
          attestation: "none",
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
      });
    })
    .then(function (outcome) {
      if (outcome.status !== 200) {
        throw new Error(reasonFor(outcome.result, {}, "Enrollment refused (" + outcome.status + ")"));
      }
      enrollCode.value = "";
      setText(enrollFeedback, "Passkey added");
    })
    .catch(function (error) {
      setText(enrollFeedback, error && error.message ? error.message : "Enrollment cancelled");
    })
    .finally(function () {
      enrollButton.disabled = false;
    });
}

// --- push notifications (home-screen app, iOS 16.4 or later) ---------------------------

const PUSH_REASONS = {
  unsupported_push_service: "This browser's push service is not supported",
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
  google: "Google",
  mozilla: "Mozilla",
};

function pushSupported() {
  return "serviceWorker" in navigator && "PushManager" in window && "Notification" in window;
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

// True when the subscription was made with the PC's current public key.
function sameServerKey(subscription, publicKey) {
  const own = subscription.options ? subscription.options.applicationServerKey : null;
  if (!own || typeof publicKey !== "string") {
    return false;
  }
  const a = new Uint8Array(own);
  const b = new Uint8Array(fromBase64Url(publicKey));
  if (a.length !== b.length) {
    return false;
  }
  for (let i = 0; i < a.length; i += 1) {
    if (a[i] !== b[i]) {
      return false;
    }
  }
  return true;
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
    setText(pushStateNode, "Notifications need the home-screen app (iOS 16.4 or later)");
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
// key (after soos-remote push reset) is dropped locally and must be enabled again.
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
      if (sameServerKey(subscription, view.public_key)) {
        return postJson(PUSH_SUBSCRIBE_PATH, "push-subscribe", subscription.toJSON());
      }
      return subscription.unsubscribe().then(function () {
        setText(pushFeedback, "Notifications must be re-enabled on this phone");
        pushEnableButton.hidden = false;
      });
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

// The permission request is the first step of the tap (Safari requires a user gesture).
async function enableNotifications() {
  if (!pushSupported()) {
    setText(pushFeedback, "Notifications need the home-screen app (iOS 16.4 or later)");
    return;
  }
  if (pushView === null || pushView.state !== "active" || typeof pushView.public_key !== "string") {
    setText(pushFeedback, "Notifications are unavailable on the PC");
    return;
  }
  const publicKey = pushView.public_key;
  pushEnableButton.disabled = true;
  try {
    const permission = await Notification.requestPermission();
    if (permission !== "granted") {
      setText(pushFeedback, "Notifications are blocked in the iPhone settings");
      return;
    }
    const registration = pushRegistration !== null ? pushRegistration : await navigator.serviceWorker.ready;
    const subscription = await registration.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: fromBase64Url(publicKey),
    });
    const outcome = await postJson(PUSH_SUBSCRIBE_PATH, "push-subscribe", subscription.toJSON());
    if (outcome.status !== 200) {
      throw new Error(reasonFor(outcome.result, PUSH_REASONS, "Refused (" + outcome.status + ")"));
    }
    setText(pushFeedback, "Notifications enabled on this phone");
  } catch (error) {
    setText(pushFeedback, error && error.message ? error.message : "Notifications not enabled");
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
// Rotation classes of the stage, in 90° steps (index 0 = upright, no class).
const CAMERA_ROTATIONS = ["", "camera-rot-90", "camera-rot-180", "camera-rot-270"];

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
// Index into CAMERA_ROTATIONS; kept for the page lifetime, never stored.
let cameraRotation = 0;
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
  const rotateButton = document.createElement("button");
  rotateButton.type = "button";
  rotateButton.className = "camera-tool";
  rotateButton.setAttribute("aria-label", "Rotate the image by 90 degrees");
  setText(rotateButton, "Rotate");
  const closeButton = document.createElement("button");
  closeButton.type = "button";
  closeButton.className = "camera-tool";
  closeButton.hidden = true;
  closeButton.setAttribute("aria-label", "Exit full screen");
  setText(closeButton, "✕");
  tools.appendChild(rotateButton);
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
  rotateButton.addEventListener("click", rotateCamera);
  closeButton.addEventListener("click", exitCameraFullscreen);
  stopButton.addEventListener("click", function () {
    stopCameraView("Camera view stopped");
  });
  camera = {
    section: section,
    stateLine: stateLine,
    stage: stage,
    canvas: canvas,
    rotateButton: rotateButton,
    closeButton: closeButton,
    startButton: startButton,
    stopButton: stopButton,
    feedback: feedback,
  };
  applyCameraRotation(camera);
  return camera;
}

function applyCameraRotation(c) {
  CAMERA_ROTATIONS.forEach(function (name, index) {
    if (name !== "") {
      c.stage.classList.toggle(name, index === cameraRotation);
    }
  });
}

function rotateCamera() {
  cameraRotation = (cameraRotation + 1) % CAMERA_ROTATIONS.length;
  applyCameraRotation(buildCameraCard());
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
  if (cameraDecoding) {
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
  const reader = response.body.getReader();
  const opener = asciiBytes(CAMERA_BOUNDARY + "\r\n");
  const closer = asciiBytes(CAMERA_BOUNDARY + "--");
  let buffer = new Uint8Array(0);

  function append(chunk) {
    if (buffer.length + chunk.length > CAMERA_MAX_BUFFER_BYTES) {
      throw new Error("The camera stream exceeded its bound");
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
        throw new Error("The camera stream is malformed");
      }
      const headEnd = headEndOf(buffer, opener.length);
      if (headEnd < 0) {
        if (buffer.length > 1024) {
          throw new Error("The camera stream is malformed");
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
        throw new Error("The camera stream is malformed");
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
  c.startButton.disabled = false;
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
            throw new Error(cameraReason(body.result, response.status));
          });
      }
      const type = response.headers.get("Content-Type") || "";
      if (type.indexOf("application/octet-stream") !== 0 || !response.body) {
        throw new Error("The camera stream is malformed");
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
        endCameraView(error && error.message ? error.message : "The camera view ended");
      }
    });
}

function startCameraView() {
  const c = buildCameraCard();
  // A deliberate second tap, then Face ID: a stray tap must never turn the camera on.
  if (!window.confirm("Start the live camera view of the PC? Its camera light turns on.")) {
    return;
  }
  c.startButton.disabled = true;
  setText(c.feedback, "Confirm with Face ID…");
  postJson(CAMERA_OPTIONS_PATH, "camera-options")
    .then(function (options) {
      if (options.status !== 200) {
        throw new Error(cameraReason(options.result, options.status));
      }
      return getAssertion(options.body);
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
        throw new Error(cameraReason(outcome.body.result, outcome.status));
      }
      const path = outcome.body.stream_path;
      if (typeof path !== "string" || !CAMERA_STREAM_PATTERN.test(path)) {
        throw new Error("The PC answered an unexpected camera address");
      }
      return openCameraStream(path);
    })
    .catch(function (error) {
      setText(c.feedback, error && error.message ? error.message : "Camera view cancelled");
      fetchCamera();
    })
    .finally(function () {
      c.startButton.disabled = false;
    });
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
document.addEventListener("keydown", function (event) {
  if (event.key === "Escape" && cameraFull) {
    exitCameraFullscreen();
  }
});
window.addEventListener("pageshow", resume);
setInterval(renderUpdated, TICK_MS);
registerServiceWorker();
resume();
