// Shared fixtures of the soos-remote browser bench (GitHub #464, part of #355).
//
// Every test gets its own loopback-only fixture server (tests/fixtures, example
// readme_web_fixture_server: synthetic scenes, fixed clock, no identity), started from the
// binary named by SOOS_FIXTURE_SERVER and stopped when the test ends. The page clock is frozen
// at the fixture's capture instant. Expected user texts are read from app.js itself, so the
// bench checks which fixed sentence is shown, never a copy of it. Every test also fails on an
// uncaught page error (an unguarded browser API call throws one).
import { test as base, expect } from "@playwright/test";
import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
export const ASSETS_DIR = resolve(HERE, "../../crates/remote/assets");
const APP_JS = readFileSync(resolve(ASSETS_DIR, "app.js"), "utf8");

// FIXED_CAPTURE_UNIX_MS of tests/fixtures/mod.rs (2026-10-01T09:41:00Z).
export const FIXED_CAPTURE_UNIX_MS = 1790847660000;
// Bounds of one fixture server: lifetime and start-up wait.
const SERVE_MS = 60000;
const START_TIMEOUT_MS = 5000;
// The tour flag of app.js (TOUR_STORAGE_KEY = TOUR_STORAGE_SEEN).
export const TOUR_STORAGE_KEY = "soos-remote-tour-v1";
export const TOUR_STORAGE_SEEN = "seen";

// Synthetic API bodies for the two states the README scenes do not cover (the shapes of
// AuthStateView and PushView in crates/remote/src); fulfilled in the browser, no identity.
export const SIGN_IN_AUTH_STATE = {
  mode: "funnel",
  authenticated: false,
  passkeys: true,
  unlock_enabled: true,
  enrollment: false,
};
export const ACTIVE_PUSH_VIEW = {
  state: "active",
  reason: null,
  public_key: "B" + "A".repeat(86),
  subscriptions: 0,
  devices: [],
  last_delivery: null,
  sender: "reachable",
};

// The value of `const NAME = "...";` in app.js (fails the test when it is missing).
export function appText(name) {
  const match = new RegExp(`\\bconst ${name} =\\s*"([^"\\\\]*)";`).exec(APP_JS);
  if (match === null) {
    throw new Error(`app.js has no string constant ${name}`);
  }
  return match[1];
}

function startFixtureServer(scene) {
  const binary = process.env.SOOS_FIXTURE_SERVER;
  if (!binary) {
    throw new Error("SOOS_FIXTURE_SERVER must name the readme_web_fixture_server binary");
  }
  const child = spawn(
    binary,
    ["--scene", scene, "--assets", ASSETS_DIR, "--serve-ms", String(SERVE_MS), "--clock", "fixed"],
    { stdio: ["ignore", "pipe", "ignore"] },
  );
  return new Promise((resolvePort, reject) => {
    let output = "";
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      reject(new Error("the fixture server did not print its port in time"));
    }, START_TIMEOUT_MS);
    child.on("error", (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.stdout.on("data", (chunk) => {
      output += String(chunk);
      const match = /^PORT=(\d{1,5})$/m.exec(output);
      if (match !== null) {
        clearTimeout(timer);
        resolvePort({ url: `http://127.0.0.1:${match[1]}`, child });
      }
    });
  });
}

export const test = base.extend({
  // The fixture scene: "unlocked" or "locked-alerts".
  scene: ["unlocked", { option: true }],
  // The base address of a fresh loopback fixture server for this test.
  app: async ({ scene }, use) => {
    const server = await startFixtureServer(scene);
    try {
      await use({ url: server.url });
    } finally {
      server.child.kill("SIGKILL");
    }
  },
  // Every uncaught page error fails the test.
  pageErrors: [
    async ({ page }, use) => {
      const errors = [];
      page.on("pageerror", (error) => errors.push(error.message));
      await use(errors);
      expect(errors, "uncaught page errors").toEqual([]);
    },
    { auto: true },
  ],
});

export { expect };

// Removes browser features before any page script runs. `features` names entries of the
// capability probe of app.js: webauthn, eventSource, push, serviceWorker.
export async function removeFeatures(page, features) {
  await page.addInitScript((names) => {
    for (const name of names) {
      if (name === "webauthn") {
        delete window.PublicKeyCredential;
      } else if (name === "eventSource") {
        delete window.EventSource;
      } else if (name === "push") {
        delete window.PushManager;
      } else if (name === "serviceWorker") {
        delete Navigator.prototype.serviceWorker;
      }
    }
  }, features);
}

// Gives the page the WebAuthn surface the probe looks for where the engine build lacks it
// (the Linux WebKit build of Playwright has no navigator.credentials). The stub never answers
// a ceremony: get() and create() reject with NotAllowedError. Engines with WebAuthn keep theirs.
export async function ensureWebAuthnSurface(page) {
  await page.addInitScript(() => {
    if (typeof window.PublicKeyCredential === "undefined") {
      window.PublicKeyCredential = function PublicKeyCredential() {};
    }
    if (!window.navigator.credentials) {
      const refuse = () => Promise.reject(new DOMException("stub", "NotAllowedError"));
      const store = { get: refuse, create: refuse };
      Object.defineProperty(Navigator.prototype, "credentials", {
        configurable: true,
        get: () => store,
      });
    }
  });
}

// Makes every localStorage access throw, as when the browser blocks site data.
export async function blockStorage(page) {
  await page.addInitScript(() => {
    Object.defineProperty(window, "localStorage", {
      configurable: true,
      get() {
        throw new DOMException("blocked", "SecurityError");
      },
    });
  });
}

// Answers one API path with a synthetic JSON body in the browser.
export async function fulfillJson(page, path, body) {
  await page.route(`**${path}`, (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(body) }),
  );
}

// Opens the page with the clock frozen at the capture instant; `tourSeen` marks the feature
// tour as closed first so it does not cover the cards.
export async function openApp(page, app, { tourSeen = true } = {}) {
  await page.clock.setFixedTime(FIXED_CAPTURE_UNIX_MS);
  if (tourSeen) {
    await page.addInitScript(
      ([key, value]) => {
        try {
          window.localStorage.setItem(key, value);
        } catch (_) {
          // A blocked storage is the business of the tour tests.
        }
      },
      [TOUR_STORAGE_KEY, TOUR_STORAGE_SEEN],
    );
  }
  await page.goto(`${app.url}/`);
}
