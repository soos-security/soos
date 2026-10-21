// Playwright configuration of the soos-remote browser bench (GitHub #464, part of #355; ADR
// 2026-10-13 "Browser Support Policy for the `soos-remote` Web App", amendment of 2026-10-21).
//
// Dev-only, never packaged. Runs inside the official Playwright image pinned by digest (it
// carries Node and the three browser builds), started by scripts/remote_browser_tests.sh with
// no network: every page is served by the loopback-only fixture server of tests/fixtures
// (synthetic scenes, fixed clock), one server per test. No trace, screenshot or video is kept.
import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./specs",
  testMatch: "*.spec.mjs",
  fullyParallel: true,
  forbidOnly: true,
  retries: 0,
  workers: 2,
  timeout: 30000,
  expect: { timeout: 5000 },
  reporter: [["list"]],
  outputDir: "test-results",
  use: {
    trace: "off",
    screenshot: "off",
    video: "off",
    locale: "en-US",
    timezoneId: "UTC",
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    { name: "firefox", use: { ...devices["Desktop Firefox"] } },
    { name: "webkit", use: { ...devices["Desktop Safari"] } },
  ],
});
