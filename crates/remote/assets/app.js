/* soos remote companion page (GitHub #339, spec section 2.11).
 *
 * Server data only ever reaches the DOM through textContent. The page keeps no state
 * beyond the live EventSource: no cookie, no storage, no service worker. Staleness is
 * measured with the browser's own clock from the arrival of the last event, never by
 * comparing checked_unix_ms with the phone's clock.
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

const LABELS = {
  locked: "Locked",
  unlocked: "Unlocked",
  no_session: "No session",
  unavailable: "Unavailable",
  unreachable: "Unreachable",
  unknown: "Connecting",
};

const stateNode = document.getElementById("state");
const activityNode = document.getElementById("activity");
const updatedNode = document.getElementById("updated");
const feedbackNode = document.getElementById("feedback");
const lockButton = document.getElementById("lock");
const unlockButton = document.getElementById("unlock");

let source = null;
let latest = null;
let lastEventAt = 0;
let lockRequestedAt = 0;
let lockConfirmTimer = null;
let unlockRequestedAt = 0;
let unlockConfirmTimer = null;

function setText(node, text) {
  node.textContent = text;
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
    setText(activityNode, "The PC is off, asleep, or off the tailnet");
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
  if (latest === null || lastEventAt === 0) {
    setText(updatedNode, " ");
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
    // EventSource reconnects on its own; one fetch decides between a transient blip and
    // an unreachable PC.
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
      if (!response.ok) {
        throw new Error("status " + response.status);
      }
      return response.json();
    })
    .then(accept)
    .catch(function () {
      if (markUnreachableOnFailure) {
        render(latest === null ? { state: "unreachable" } : latest, false);
      }
    });
}

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
      const reasons = {
        no_session: "No session to lock",
        already_locked: "Already locked",
        rate_limited: "Please wait a moment before locking again",
        unavailable: "logind is unavailable",
        forbidden: "Request refused",
      };
      const reason = Object.prototype.hasOwnProperty.call(reasons, outcome.result)
        ? reasons[outcome.result]
        : "Lock refused (" + outcome.status + ")";
      setText(feedbackNode, reason);
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

function requestUnlock() {
  // A deliberate second tap: the unlock has no other factor than the Tailscale identity
  // (ADR 2026-10-06), so a stray tap must not open the PC.
  if (!window.confirm("Unlock the PC now?")) {
    return;
  }
  unlockButton.disabled = true;
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

  fetch(UNLOCK_PATH, { method: "POST", headers: { "X-Soos-Action": "unlock" } })
    .then(function (response) {
      return response.json().then(function (body) {
        return { status: response.status, result: body.result };
      });
    })
    .then(function (outcome) {
      if (outcome.status === 202) {
        return;
      }
      unlockRequestedAt = 0;
      if (unlockConfirmTimer !== null) {
        clearTimeout(unlockConfirmTimer);
        unlockConfirmTimer = null;
      }
      const reasons = {
        no_session: "No session to unlock",
        already_unlocked: "Already unlocked",
        rate_limited: "Please wait a moment before unlocking again",
        unavailable: "logind is unavailable",
        unlock_disabled: "Remote unlock is disabled (allow_unlock = false in remote.toml)",
        forbidden: "Request refused",
      };
      const reason = Object.prototype.hasOwnProperty.call(reasons, outcome.result)
        ? reasons[outcome.result]
        : "Unlock refused (" + outcome.status + ")";
      setText(feedbackNode, reason);
      fetchStatus(false);
    })
    .catch(function () {
      unlockRequestedAt = 0;
      if (unlockConfirmTimer !== null) {
        clearTimeout(unlockConfirmTimer);
        unlockConfirmTimer = null;
      }
      setText(feedbackNode, "Unlock request failed");
      fetchStatus(true);
    });
}

function resume() {
  openStream();
  fetchStatus(true);
}

setText(lockButton, LOCK_LABEL);
setText(unlockButton, UNLOCK_LABEL);
lockButton.addEventListener("click", requestLock);
unlockButton.addEventListener("click", requestUnlock);
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
