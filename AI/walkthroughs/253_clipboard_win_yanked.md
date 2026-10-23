# Walkthrough 253 — Replace the Yanked `clipboard-win` 5.4.1

- **Date**: 2026-10-23
- **Issue**: no backlog id; `cargo deny check` (CI "Security (cargo-deny)") fails on every branch
- **Branch**: `fix/clipboard-win-yanked`
- **Decision**: `deny.toml` unchanged (`yanked = "deny"`, `ignore = []`); same lockfile-only
  approach as walkthrough 247 (webpki advisories).
- **Matrix criteria**: none added; the existing cargo-deny gate is the acceptance check.

## 1. Context & Objectives

On 2026-10-23 the maintainer yanked `clipboard-win` 5.2.0 through 5.4.1 from crates.io and
published 5.4.2 and 5.4.3 (upstream issue DoumanAsh/clipboard-win#42: `raw::get_html` could read
past the end of the clipboard buffer when the HTML fragment offsets of a crafted "HTML Format"
payload point beyond the data; 5.4.3 replaces the raw copy with a bounds-checked slice). `Cargo.lock` pinned 5.4.1, so `cargo deny check` reported
`error[yanked]: detected yanked crate` and failed CI on every pull request (first seen on PR #470).
Goal: a non-yanked version with a lockfile-only change.

## 2. Architect (Phase 1)

- `clipboard-win` reaches the workspace only through `soos-gui` → `eframe` 0.35.0 →
  `egui-winit` 0.35.0 → `arboard` 3.6.1, and only for Windows targets. soos builds and ships for
  Linux only, so the crate is never compiled; it is still part of the lockfile that cargo-deny
  audits for every target.
- `cargo update -p clipboard-win` moves it from 5.4.1 to 5.4.3 (latest compatible, not yanked,
  license BSL-1.0, already allowed). No `Cargo.toml` change, no other lockfile entry moves.

## 3. Tester, Auditor and Developer (Phases 2–4)

No code changes and no new tests: the failing gate is `cargo deny check` itself. The 7-day
adoption cooldown applies to Dependabot action bumps and is not waited for (owner decision of
2026-10-21); the version is the minimal one that clears the gate.

## 4. Verification

- `cargo deny check`: `advisories ok, bans ok, licenses ok, sources ok` (was `advisories FAILED`).
- `git diff --stat`: `Cargo.lock` only (2 lines) plus this walkthrough.

## 5. Not Verified

A Windows build (no Windows target installed; soos does not target Windows).
