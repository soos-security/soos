/* soos remote companion page (GitHub #339, spec section 2.11; Funnel and passkey ADR
 * 2026-10-06, spec section 8).
 *
 * Server data only ever reaches the DOM through textContent. The page keeps no state
 * beyond the live EventSource: no script-readable cookie (the web session cookie is
 * HttpOnly), no storage, no service worker. Staleness is measured with the browser's own
 * clock from the arrival of the last event, never by comparing checked_unix_ms with the
 * phone's clock.
 *
 * Passkeys: every WebAuthn ceremony is modal (a tap, then Face ID / Touch ID), always with
 * user verification required, and never names a credential: the server only answers with a
 * challenge, so the phone offers the passkey it holds for this site.
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

let source = null;
let latest = null;
let lastEventAt = 0;
let lockRequestedAt = 0;
let lockConfirmTimer = null;
let unlockRequestedAt = 0;
let unlockConfirmTimer = null;
let signedOut = false;

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
      }
    })
    .catch(function () {
      if (markUnreachableOnFailure && !signedOut) {
        render(latest === null ? { state: "unreachable" } : latest, false);
      }
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
    })
    .catch(function () {
      render(latest === null ? { state: "unreachable" } : latest, false);
    });
}

setText(lockButton, LOCK_LABEL);
setText(unlockButton, UNLOCK_LABEL);
lockButton.addEventListener("click", requestLock);
unlockButton.addEventListener("click", requestUnlock);
loginButton.addEventListener("click", requestLogin);
logoutButton.addEventListener("click", requestLogout);
enrollButton.addEventListener("click", requestEnroll);
document.addEventListener("visibilitychange", function () {
  if (document.visibilityState === "visible") {
    resume();
  } else {
    closeStream();
  }
});
window.addEventListener("pageshow", resume);
setInterval(renderUpdated, TICK_MS);
resume();
