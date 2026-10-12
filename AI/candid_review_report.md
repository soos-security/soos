# Candid Review Report

- **Date**: 2026-10-12
- **Target Branch**: `fix/remote-security-hardening`
- **Base (merge-base)**: `f9234ea3c`
- **Reviewed-Diff-Fingerprint**: `22e433a73f8d63621f2a89c29fc416230afb46e3ea3c6d359faaf04f3423b695`
- **Audited Files**: `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_remote_security_hardening.md`,
  `AI/auditor_constraints_remote_security_hardening.md`, `AI/tester_contract_remote_security_hardening.md`,
  `AI/walkthroughs/194_remote_security_hardening.md`, `Cargo.lock`, `Docs/DAEMON.md`, `Docs/IPC_PROTOCOL.md`,
  `Docs/REMOTE_COMPANION.md`, `crates/daemon/Cargo.toml`, `crates/daemon/src/dispatcher.rs`,
  `crates/daemon/src/limits.rs`, `crates/daemon/src/preview.rs`, `crates/daemon/src/preview_image.rs`,
  `crates/daemon/src/session_policy.rs`, `crates/daemon/tests/peer_limits_tests.rs`,
  `crates/daemon/tests/preview_authorization_tests.rs`, `crates/daemon/tests/preview_downscale_tests.rs`,
  `crates/daemon/tests/preview_placement_tests.rs`, `crates/daemon/tests/preview_remote_view_tests.rs`,
  `crates/push-sender/src/lib.rs`, `crates/push-sender/src/main.rs`, `crates/push-sender/tests/sandbox_tests.rs`,
  `crates/remote/assets/sw.js`, `crates/remote/src/camera_ipc.rs`, `crates/remote/src/http.rs`,
  `crates/remote/src/lib.rs`, `crates/remote/src/push.rs`, `crates/remote/src/server.rs`,
  `crates/remote/tests/camera_server_tests.rs`, `crates/remote/tests/http_tests.rs`,
  `crates/remote/tests/push_server_tests.rs`, `packaging/soos-push-sender.service`, `scripts/install_remote.sh`,
  `tests/invariants/src/lib.rs`, `tests/invariants/src/remote_push_contract.rs`,
  `tests/invariants/src/remote_security_hardening_contract.rs`

## 1. Executive Summary

Fresh review of the full raw diff `origin/main...` working tree (7093 patch lines, 40 files) after the
traceability commit `d597d42` changed documentation since the previous review (fingerprint `a7fc77f5...`).
The code hunks are unchanged since that review and were re-audited from scratch; the documentation hunks
(`AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, walkthrough 194, `Docs/DAEMON.md`,
`Docs/IPC_PROTOCOL.md`, `Docs/REMOTE_COMPANION.md`) were checked claim by claim against the code.

The diff implements S-1 to S-8d of the 2026-10-12 soos-remote security review: push sender unit hides
`/var/log` and the coredump directory and self-checks its sandbox (exit 78); single-sourced Tailscale
minimum 1.102.4; one auth-reserved daemon connection per unprivileged UID (`Auth` only, whole first frame
within 200 ms); preview peer placement rule (step 6c-2a); HSTS/COOP/CORP/Permissions-Policy on every
response head; symlink-safe installer write; stop re-checked before the camera head and first part;
in-place push subscription renewal; daemon-side conversion to Grey/YUYV/RGB24 within 640x480 for the
remote companion origin only.

`crates/pam` is untouched, no path reaches `Allow`/`PAM_SUCCESS`, every new branch fails closed, all
allocations are bounded before use. Test changes are additive except the owner-approved CM-SR1
strengthening. Re-run locally: `soos-daemon` (`peer_limits_tests` 28, `preview_placement_tests` 17,
`preview_downscale_tests` 19, `preview_remote_view_tests` 15, `preview_authorization_tests` 10),
`soos-remote` (`http_tests` 22, `camera_server_tests` 15, `push_server_tests` 22), `soos-push-sender`
`sandbox_tests` 14, `soos-invariants` 564: all passed. No CRITICAL or MAJOR finding; two MINOR
documentation-accuracy findings and one MINOR code finding remain (section 4).

## 2. Test Changes

Mechanical listing (skill step 3) over `target/candid_diff.patch`:

- Removed lines matching an assertion pattern: one hit, patch line 2064 `// Step 3: Bounded memory
  safety assertion` in `crates/daemon/src/dispatcher.rs`: a production comment; the size check it labelled
  moved verbatim into `read_client_frame` (`0 < size <= MAX_MESSAGE_SIZE` before allocation). Not a test.
- New escape hatches: one hit, patch line 3298, a comment "within JPEG tolerance" in the new
  `preview_downscale_tests.rs` (bounded per-channel comparison on a lossy MJPEG fixture). Justified; no
  `#[ignore]`, no `should_panic`.
- Inline `mod tests` changes: none.
- Removed lines in existing test files (`git diff origin/main...HEAD` over test paths): exactly one,
  `const SENDER_UNIT_LINES: [&str; 37]` -> `[&str; 38]` in `tests/invariants/src/remote_push_contract.rs`
  with one inserted exact line `InaccessiblePaths=-/var/log -/var/lib/systemd/coredump` (**CM-SR1**,
  owner-approved, spec §11.3): exact-set assertion kept, strengthening.
- Setup-only additions inside existing tests: `preview_remote_view_tests.rs::test_rlc_local_peer_unaffected_by_remote_view`
  (session record `"4"`, **AM-SR1**) and `preview_authorization_tests.rs::test_preview_frame_allowed_for_configured_uid_serves_frames`
  (own session record when `/proc/self/cgroup` names a scope, **AM-SR2**). Assertions untouched; both are
  required by the new S-5 rule and do not loosen what is asserted.
- Every other test hunk is a pure addition (new modules or files).

## 3. Deep Reasoning Audit

### Logic & Architecture
- S-3 admission (`dispatcher.rs` ~431-461, `limits.rs`): reserve tried only after `PerUidCap`. Tried:
  root peer (reserve returns `PerUidCap`, root has no per-UID cap anyway), global and unprivileged
  capacity full (checked first, state untouched), second reserve of the same UID (refused), drop
  accounting (class-specific map, totals restored), `tracked_uids` union (reserve map only holds UIDs that
  hit the regular cap, bounded). PASS.
- S-3 dispatch: the 200 ms window wraps the whole `read_client_frame` (no slow-loris on the body),
  nested in `connection_timeout`; non-`Auth` (`Request` of another kind or any event) is refused before
  `handle_request`/`handle_event`; the loop breaks after one request (`one_shot || AuthReserve`). PASS.
- S-5 placement: placement read from the same cgroup content as the origin; session scope first, only
  the first unit component counts, so a `session-N.scope` created inside the delegated `user@UID.service`
  subtree is `UserManager`. Session peer whose own record is absent or remote -> refused; user manager
  with a remote or UID-less non-`REMOTE=0` record -> refused; `Other` keeps 6c-2 (an unprivileged process
  cannot leave its scope/manager for `Other`). Placement cached per connection, rule re-evaluated per
  request. PASS.
- S-8d conversion: only for `PreviewPeerOrigin::RemoteCompanion`; IR recursion terminates (output Grey);
  `step = max(ceil(w/640), ceil(h/480), 1)` gives `w/step <= 640`, `h/step <= 480` (640x512 -> 320x256, as
  documented); pass-through slices exactly `w*h*bpp` with checked arithmetic, short buffer -> empty image;
  GUI origin keeps `preview_image_for_frame`. PASS.
- S-8c renewal: one locked `update`, removal before the capacity count, `TooMany` writes nothing,
  unparsable or equal old endpoint = plain upsert; `deny_unknown_fields` + `camelCase` accept exactly
  `replacesEndpoint`; same authenticated route as unsubscribe, so no new capability. PASS.
- S-8b: stop re-checked after `mark_streaming` (refusal path audits, no head) and after the head (audit
  refused, `finish_view`, never counted shown). PASS.
- S-7 sandbox: `/run` must be a ro tmpfs; home (derived from `current_exe`, never env) must lie under a
  ro tmpfs that is not `/`; any direct child of that tmpfs covering or inside the home other than the exact
  binary bind refuses (a stacked mount at the same point is such a child); `/var/log` listable refuses.
  Tried `BindPaths=%h`, a tmpfs at `/`, `rw` tmpfs, `ro` only as a substring (`errors=remount-ro`): all
  refused. PASS.

### PAM Concurrency & Deadlines
- `crates/pam` not in the diff. The reserve is daemon-side; a PAM client missing the window sees EOF and
  falls back to `PAM_IGNORE` through the unchanged client path. PASS.

### Panic Safety & Fail-Closed
- No `unwrap/expect/panic!/todo!/unreachable!` or raw indexing added in production hunks (`get`, checked
  arithmetic, `unwrap_or_default` on a slice only). Errors map to `refuse_preview`, `Malformed`,
  `LogindUnavailable`, exit 78, or the empty image. No path to `Allow`/`PAM_SUCCESS`. PASS.

### Test Integrity & Anti-Weakening
- See section 2. New tests distinguish plausible wrong implementations (any request vs `Auth` only on the
  reserve; any local session of the UID vs the peer's own session; non-exact `ro`; mount stacked on the
  home). PASS.

### Memory, Bounds & Secrets
- Frame body allocated only after the size check; mountinfo read capped at 1 MiB + 1 and refused above;
  decoded MJPEG in `Zeroizing`, IR intermediate frame zeroized by `Drop for Frame`; sandbox errors are fixed
  strings; `/var/log` opened, never iterated; new log lines carry `peer_uid` and constant text only;
  installer refuses a symlink and writes with `set -C` under `umask 077`. PASS.

### Supply Chain & Automation
- `jpeg-encoder` added as a daemon dev-dependency only (already a workspace dependency, already allowed in
  `deny.toml`); `Cargo.lock` gains only the edge. No workflow change. Installer change is defensive and
  still never runs `tailscale`. PASS.

### English-Only Policy
- No non-English word in added lines (accented-character scan over `+` lines: no hit), commit subjects or
  docs. PASS.

### Documentation Accuracy (docs changed since the last review)
- Verified against code: 6c-2a wording in `Docs/IPC_PROTOCOL.md` (absent record refuses, disabled
  validator disables), admission step 2, remote frame-size bullet and step formula, 640x512 -> 320x256,
  installer behaviour, sender self-check list, ADR item texts, matrix RSH1-RSH17 test names and per-file
  counts (12/10/10/2/1/1/6/14/14; every cited test exists and the matrix citation invariants pass).
  Two inaccuracies found (section 4, MINOR).

## 4. Detailed Findings & Action Items

- **[MINOR]** `Docs/REMOTE_COMPANION.md:1205-1206` and `AI/walkthroughs/194_remote_security_hardening.md:197` —
  "a process of your UID can occupy that reserve for up to 200 ms per connection" is inaccurate: 200 ms
  bounds only an idle or partial first frame; a same-UID process that sends a complete `Auth` request holds
  the reserve for the whole `Auth` run (bounded by `connection_timeout` and the rate limiter), as the spec
  §13 item 3 states correctly. Correction: reword both places to "for up to 200 ms per connection, or for
  the duration of one `Auth` request when it sends one".
- **[MINOR]** `AI/walkthroughs/194_remote_security_hardening.md:45` — the S-5 design row names the
  `PeerPlacement` variants `SessionScope`, `UserManager`, `Other`; the code
  (`crates/daemon/src/session_policy.rs`) defines `Session(String)`, `UserManager(u32)`, `Other`.
  Correction: replace `SessionScope` with `Session`.
- **[MINOR]** `crates/daemon/src/preview_image.rs:376-382` (`sample_rgb`, lenient chroma) — for an
  odd-width YUYV frame the macropixel index `(y*width + (x & !1))*2` ignores row boundaries, so interior
  pixels of odd rows take chroma from a misaligned pair; only the final incomplete macropixel falls back to
  neutral chroma. Colour fidelity only (odd-width YUYV is not a real V4L2 layout); bounds and safety are
  unaffected. Already listed as a follow-up in walkthrough 194 §9; no change required for merge.
- **[SUGGESTION]** `crates/daemon/src/dispatcher.rs:441` — every reserve refusal is mapped to
  `ConnectionRejection::PerUidCap`, so a reserve refused for `GlobalCapacity` or `ReservedForPrivileged`
  is logged with the wrong reason. Behaviour is correct (closed, fail-safe); keeping the reserve's own
  rejection would make the log accurate.
- **[SUGGESTION]** `Docs/DAEMON.md:151` — "no remote (or UID-less) session" can be read as "no UID-less
  session at all"; the rule (and `Docs/IPC_PROTOCOL.md` 6c-2a) is "no record of the UID, or without UID,
  other than `REMOTE=0`". Align the wording.

## 5. Final Verdict

**VERDICT: APPROVED**
