# Plan Evaluation Report (Round 2)
- **Date**: 2026-10-05
- **Issue**: GitHub #331 — fix: install follow-ups, physical procedure fixes, remove GDM double face verification (GitHub-only, no backlog id)
- **Branch**: `fix/install-gdm-followups`
- **Base commit**: `b05477d`
- **Spec evaluated**: `AI/architect_spec_install_gdm_followups.md`
- **Owner decisions taken as final**: no managed block (and removal of a redundant one) when the delegated stack already
  reaches an active primary `pam_soos.so` rule; GDM then uses the 1000 ms module default; one defaulted field added to
  `crates/daemon/tests/common/mod.rs::SpyCamera` is accepted as setup-only (no assertion change).

## 1. Coverage Matrix

| Acceptance line (issue #331) | Spec element | Status |
|---|---|---|
| §1.1 preflight detects read-only files + `chmod` advice for unreadable user-owned subtree | §4.2 checks 2–4, IGF1, IGF2 | Covered |
| §1.2 document the real readiness bound (`T + 7.5 s`) | §5.1, IGF3 | Covered (derivation re-checked, see §2) |
| §1.3 keep last `soos-admin status` stderr; fail fast on 126/127 | §5.2, IGF4, IGF5 | Covered |
| §1.4 presence settle keyed on the camera stream start (PAM-caused wake) | §3.1–§3.5, IGF6–IGF9 | Covered (wording finding 2) |
| §1.5 P-1 relative `CARGO_TARGET_DIR` | §4.1, IGF10 | Covered |
| §1.6 P-2 `systemctl show` timeout | §6, IGF11 | Covered (hardening finding 6) |
| §2.1 screensaver §6 rollback `sed` | §7, IGF12 | Covered |
| §2.2 §3.3 Test Case 2 `pgrep` hint | §7, IGF13 | Covered |
| §3 no managed block when delegated stack reaches a primary soos rule | §2.2–§2.4, IGF14, IGF17, IGF18 | Covered |
| §3 remove an existing redundant managed block | §2.4 `RemoveRedundant`, IGF15 | Covered |
| §3 `gdm status` still reports installed | §2.5 `shared_stack`, IGF16 | Covered |
| §3 1000 ms default accepted (no 2500 for shared case) | §2.6, ADR §9.1 | Covered |
| Docs / ADR / walkthrough | §9, IGF19 | **Incomplete** (finding 1, 3) |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| `GDM_PAM_LINE` keeps `timeout_ms=2500` | `crates/admin-cli/src/gdm.rs:26` | `auth  [success=done default=ignore]  pam_soos.so timeout_ms=2500` | Yes |
| `MAX_PAM_INCLUDE_DEPTH` = 4 | `crates/admin-cli/src/pam_stack.rs:16` | 4 | Yes |
| A `pam_soos.so` rule before the credential module is unclassified today → refusal | `pam_stack.rs::scan_lines` (credential → gate → neutral → refusal) | Confirmed: packaged Arch `system-auth` makes today's `enable` refuse | Yes |
| `delegated_gates` only caller is `gdm.rs` | `gdm.rs:17,336` | Yes | Yes |
| `GdmStatus` built only in `gdm.rs`, no test literal / JSON comparison | `grep -rn GdmStatus crates tests` | only `lib.rs` re-export | Yes |
| No gdm test fixture carries a soos rule in a delegated stack | `crates/admin-cli/tests/gdm_*.rs` | only in-file admin rules (`gdm_stack_order_tests.rs:633`) | Yes |
| `gdm.disable` applies to any service containing `gdm` | `crates/pam/src/config.rs:132` | `self.service.contains("gdm")` | Yes |
| Packaged primary rules | `packaging/pam/arch/system-auth`, `debian/soos`, `fedora/soos/{system,password}-auth` | Arch `[success=4 default=ignore]`, Debian/Fedora `[success=done default=ignore]`, all after `faillock preauth` where present | Yes |
| `DEFAULT_TIMEOUT_MS` = 1000 | `crates/pam/src/config.rs` | 1000 | Yes |
| `PRESENCE_WAKE_SETTLE_MS` = 1000, `woke` sampled before `notify_activity` | `presence/mod.rs:87`, `worker.rs:634` | Yes | Yes |
| `RequestDeadline::compute(now, 0, Instant, Duration)` + `with_not_before` | `crates/daemon/src/inference.rs:87,116` | Yes | Yes |
| Daemon production camera is `V4lCameraManager` directly (no wrapper hiding the override) | `crates/daemon/src/pipeline.rs:582` | `Arc<dyn CameraManager>` = V4L or Mock | Yes |
| Frame stamps use `monotonic_nanos` (CLOCK_MONOTONIC) at dequeue | `v4l_impl.rs` → `run_capture_loop(.., &monotonic_nanos)` | Yes | Yes |
| **Stamp taken "immediately after `start_stream` returns" = "VIDIOC_STREAMON returned"** | `v4l_impl.rs:169` (`mmap_stream_guarded` = REQBUFS + mmap); v4l 0.14 `io/mmap/stream.rs:190-197` | STREAMON is issued **lazily inside the first `next()`** (first `next_buffer` in `capture.rs`), not by `start_stream` | **No** (finding 2) |
| `open_and_stream` has 7 parameters | `v4l_impl.rs:765` | 7 | Yes |
| `SpyCamera` single construction site | `crates/daemon/tests/common/mod.rs:757` | 1 | Yes |
| `inspect_systemd_unit` unbounded `.output()` | `crates/admin-cli/src/status.rs:237` | Yes | Yes |
| `install.sh` resolves `CARGO_TARGET_DIR` twice, relative to cwd | `scripts/install.sh:262,499` | Yes (and `./` prefix at 504) | Yes |
| Check-2 skipped today when check-1 `find` fails → wrong `chown` advice | `install.sh:524-545` | Yes | Yes |
| Readiness loop stop test `SECONDS - start >= TIMEOUT_S + 1`, attempts `timeout --kill-after=1 5` | `scripts/wait_daemon_ready.sh` | Yes; worst case last attempt starts at `< T + 1.5 s`, ends `< T + 7.5 s` | Yes |
| Existing contracts cited in §10 exist | `tests/invariants/src/{install_presence_warmup_contract,arch_faillock_ci_contract,quality_followups,lib}.rs` | all found | Yes |
| Next walkthrough number 179 | `AI/walkthroughs/178_install_readiness_presence_warmup.md` | 178 is the highest | Yes |
| `AI/ARCHITECTURE.md` GDM prose | `AI/ARCHITECTURE.md:130` ("GDM uses `timeout_ms=2500`"), `:177-182` ("managed by `soos-admin gdm enable` with … `timeout_ms=2500` inside a marked block") | Becomes false for the shared-rule case; **not listed in spec §9.2** | **No** (finding 1) |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Failure scenario considered: a delegated stack places a gate (e.g. `requisite pam_nologin.so`) **after** the shared soos
  rule and before `pam_unix`; with no managed block, a face success skips that gate for GDM, whereas the old block copied it
  in front of the face rule.
- Result: PASS with observation. The old design did not guarantee it either: after a miss at the block, the shared rule
  retried and a match there skipped the same gate; and the shared stack's order is what `sudo`, `login` and lockers already
  use. Account-phase gates (ADR 2026-09-30 item 3) are untouched. The ADR text must state this explicitly (finding 4).
- Scenario: `substack` vs `include` — a `success=done` inside a `substack` (Fedora `password-auth`) ends only the substack,
  so the GDM file tail still runs; inside an `include` (Arch, Debian) it ends the whole stack, like the managed block did.
  Both fail closed. PASS.
- UDS / `SO_PEERCRED` / storage paths: untouched. PASS.

### Pillar 2 — PAM deadline & concurrency
- Failure scenario considered: GDM via shared rule now has a 1000 ms deadline; a camera wake from auto-standby exceeds it.
- Result: PASS (owner-accepted; client deadline 1000 − 50 ms margin ≥ `DECISION_BUDGET_MS` 900). `crates/pam` untouched, no
  Tokio, presence settle never reached by the dispatcher (IGF9 static check). Presence worst case 900 + 50 + 1000 = 1950 ms
  ≤ `scan_interval_ms` 2000. PASS.

### Pillar 3 — Panic safety & fail-closed
- Failure scenario considered: a qualifying-looking jump rule `[success=1 default=ignore] pam_soos.so` followed by
  `pam_unix` then `required pam_deny.so`; face success lands on `pam_deny`, so GDM face never succeeds, yet `enable` reports
  success without inserting a block.
- Result: FINDING (MINOR, finding 5) — fail closed (password still works), functional only.
- Scenario: a `[...=N]` jump before the GDM anchor that lands **beyond** the delegation line skips the shared rule; spec
  skips the jump check in the shared case, so `status` reports `installed: true` for a branch that never reaches soos.
- Result: FINDING (MINOR, finding 5). No write occurs, no security impact.
- Scenario: unreadable include / depth exceeded / continuation in status analysis → `installed: false`, never an error.
  PASS. No `unwrap`/`expect` introduced in plan; `Scan` loses `Copy` once it carries a `String` (implementation detail).

### Pillar 4 — Dependencies
- Failure scenario considered: P-2 timeout implemented with a new crate (e.g. `wait-timeout`).
- Result: PASS — spec mandates std `try_wait` polling, no new dependency; camera stamp reuses `monotonic_nanos` (no new
  `unsafe`).

### Pillar 5 — Data confidentiality
- Failure scenario considered: the stderr tail of `soos-admin status` echoes terminal escapes or sensitive data into the
  install log.
- Result: PASS — 1024-char tail, control characters replaced, `printf '%s\n'`, `--admin` through `%q`; status output holds
  no biometric data. Presence `debug!` logs only `settle_ms` and a static `cause`. `RemoveRedundant` keeps atomic writes and
  never creates/alters a backup.

### Pillar 6 — Test integrity
- Failure scenario considered: an implementation that compares `bound < start_ns` instead of `bound <= start_ns`, or
  stamps the stream after the first frame instead of before.
- Result: FINDING (MINOR, finding 7) — IGF6 must pin the exact boundary (`d == settle_ms` → `{0, 0}`;
  `d == settle_ms − 1 ms` → `wait_ms == 1`). The `SpyCamera` extension is additive and owner-approved; no existing assertion
  changes; §10 list of constraining tests is accurate.

## 4. Findings

1. **[MAJOR] `AI/ARCHITECTURE.md` drift omitted from §1 Scope and §9.2.** `AI/ARCHITECTURE.md:130` states "GDM uses
   `timeout_ms=2500`" and `:177-182` states GDM is "managed by `soos-admin gdm enable` with … `timeout_ms=2500` inside a
   marked block". After #331 this is false on every packaged soos install (Arch `system-auth`, Debian profile, Fedora
   `custom/soos`). **Required revision**: add `AI/ARCHITECTURE.md` to the §1 Docs row and §9.2, with the exact replacement
   text for line 130 ("GDM uses `timeout_ms=2500` only when `gdm enable` inserts the managed block; when the delegated stack
   already carries a primary soos rule GDM uses that rule's deadline (module default 1000 ms), ADR 2026-10-05") and for the
   §5 GDM paragraph (no block / redundant block removed / `gdm status` `shared_stack`). Add a needle for it to IGF19.

2. **[MINOR] Wrong fact: `start_stream` does not issue `VIDIOC_STREAMON`.** In v4l 0.14 `mmap_stream_guarded` only does
   REQBUFS + mmap; STREAMON is issued lazily by the first `CaptureStream::next()` (first `next_buffer` in
   `capture.rs::run_capture_loop`). The stamp taken "immediately after `start_stream` returns" precedes STREAMON by
   microseconds (harmless, frames are still stamped later). **Required revision**: in §3.1 trait doc, §3.2 and the ADR §9.1
   item (2), describe the stamp as "taken when the capture stream is set up (buffers mapped), immediately before the first
   dequeue, which issues `VIDIOC_STREAMON` (v4l 0.14 starts the stream lazily)"; keep IGF8's `stamp <= frame.timestamp_mono_ns`.

3. **[MINOR] `Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1 example becomes misleading.** The example "What `gdm enable` writes,
   e.g. on Fedora 40 (`authselect ... with-faillock`)" (line 87) shows a managed block; with the `custom/soos` profile that
   `install.sh` selects, `enable` now writes nothing. **Required revision**: §9.2 must require relabelling the example as a
   stack without a soos rule (e.g. stock `local`/`sssd` profile) and adding the shared-rule output (`Shared soos Rule:
   password-auth`). Also require the screensaver §6 GDM bullet to say that with a shared rule there is no backup (`gdm
   restore` reports "does not exist"); use `gdm disable` or the base-stack loop.

4. **[MINOR] ADR / §8 wording on the gate guarantee.** §8 says "Preserved", but rules after the shared rule are never
   inspected, so for GDM the gate guarantee becomes the shared stack's own ordering. **Required revision**: state in §8 and
   in ADR §9.1 item (1) that gates placed after the shared rule in the delegated stack are governed by that stack (as for
   `sudo`/`login`), and that this was already the effective behavior because the shared rule retried after a block miss.

5. **[MINOR] Known false-positive "installed" cases must be documented.** (a) `is_primary_soos_rule` accepts any jump
   `N >= 1` without checking the landing rule; (b) the shared case skips the jump check, so a pre-anchor jump that lands past
   the delegation line skips the shared rule while `status` reports `installed: true`. Both fail closed (password path
   unchanged, nothing written). **Required revision**: list both in §2.5 "Consequence (documented)" and in the
   `Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1 change; no code change required.

6. **[MINOR] P-2 read-after-exit can still block.** §6 reads stdout after `try_wait` reports exit; if a descendant inherited
   the stdout pipe, `read_to_end` blocks past the 1 s bound. Real `systemctl show` does not fork such a holder, so this is
   hardening. **Required revision**: either bound the read (read on a helper thread joined against the remaining deadline,
   or non-blocking read until EOF/deadline) or document the assumption in §6; IGF11's sleeping fake must use `exec sleep`
   (or the test must not depend on the orphan) so the timeout test is deterministic.

7. **[MINOR] Test power.** **Required revision** of §11: IGF6 must include the exact boundaries (`d == settle_ms` →
   `{0, 0}`; `d = settle_ms − 1 ms` → `wait_ms == 1`, `not_before_ns == stream + settle`; stream stamp later than
   `start_ns` clamped); IGF17 (or `pam_stack.rs` unit tests) must cover every §2.2 edge (`success=0`, `success=-1`,
   `success=`, bracket without `success`, duplicate conflicting `success`, `ignore=ignore` extra key, case-insensitive
   `SUFFICIENT`, path-form `/usr/lib/security/pam_soos.so`); IGF2 must assert that the `chmod -R u+rwX` advice appears
   **instead of** `sudo chown -R` for a user-owned 0300 directory as a non-root runner.

## 5. Round 2 — Re-evaluation of Revision 1

The full revised spec was re-read; each round-1 finding was checked against the revised text and the code.

| # | Round-1 finding | Revision 1 resolution | Status |
|---|---|---|---|
| 1 | MAJOR `AI/ARCHITECTURE.md` drift | §1 Docs row and §9.2 now carry exact replacement text for line ~130 and the §5 GDM paragraph; IGF19 adds needles (`GDM uses \`timeout_ms=2500\` only when`, `shared_stack`) and a negative check on the bare sentence | Resolved |
| 2 | STREAMON wording | §3.1 trait doc, §3.2 and ADR item (2) describe the set-up stamp before the lazy STREAMON at the first dequeue (matches v4l 0.14 `io/mmap/stream.rs:190-197`); IGF8 keeps `stamp <= frame.timestamp_mono_ns` | Resolved |
| 3 | Deployment example / §6 GDM bullet | Fedora example relabelled as stock `local` profile, shared-rule example added; §6 bullet distinguishes block vs shared rule | Resolved (observation R2-1) |
| 4 | Gate guarantee wording | §8 row "Changed scope, not weakened in practice" and ADR item (1) state the shared-stack ordering and why it was already effective | Resolved |
| 5 | False positives | §2.5 lists the false negative and false positives (a) and (b), all fail closed, documented in §2.1 of the deployment doc | Resolved |
| 6 | P-2 read-after-exit | Reader on a helper thread, `mpsc` + `recv_timeout(remaining)`, detached on deadline; IGF11 adds `exec sleep 30` and a pipe-holding descendant case | Resolved (observation R2-2) |
| 7 | Test power | IGF6 exact boundaries (999/1000 ms, 0, clamp, `Some(0)`, max with woke, `u64::MAX`); IGF17 all §2.2 edges plus qualifying forms (upper case, mixed-case bracket, extra `ignore` key, path form); IGF2 asserts `chmod -R u+rwX` and absence of `sudo chown -R` | Resolved |

New issues introduced by the revision (adversarial pass):

- **R2-2 detached reader thread (P-2)**: Failure scenario — a descendant keeps the stdout pipe open, so the reader thread
  outlives the call. Assessment: `inspect_systemd_unit` is only reached from the short-lived `soos-admin status` process
  (no other crate depends on `soos-admin-cli`: `grep soos_admin_cli crates/*/Cargo.toml`), and a Rust process exits when
  `main` returns, whatever detached threads exist. At most one thread per invocation, bounded buffer (≤ 4097 bytes), no
  zombie (the child is reaped by `try_wait`/`wait`). In `cargo test` the descendant case leaves one thread blocked until
  `sleep 30` ends; this does not fail or slow the test. PASS — no leak path in a long-lived process.
- **R2-2b wall-time bound**: `try_wait` polling until the deadline, then `recv_timeout(remaining)` keeps the total at
  ≤ `timeout` + one poll interval; the descendant case cannot block. PASS.
- **R2-1 [MINOR, non-blocking] §6 GDM bullet wording**: "when it uses the shared base-stack rule … there is no backup"
  is not always true. After `RemoveRedundant` the earlier `.soos-backup` is deliberately kept (§2.4), and `gdm restore`
  then succeeds. The developer/traceability agent should write "there may be no backup (`gdm restore` then reports that it
  does not exist)". This is documentation only, so it does not need another spec round.

## 6. Verdict

Round 1: REVISION_REQUIRED (1 MAJOR, 6 MINOR). Round 2: every finding is resolved, and the revision adds no CRITICAL or MAJOR
issue. One non-blocking MINOR wording note (R2-1) is carried to Phase 6.

VALIDATION_VERDICT: APPROVED
