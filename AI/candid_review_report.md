# Candid Review Report

- **Date**: 2026-10-02
- **Target Branch**: `test/screensaver-matrix-and-locker-stacks`
- **Base (merge-base)**: `b4c9556`
- **Reviewed-Diff-Fingerprint**: `6fe2ca5586521b6a1ec0b55422bfb828816f4aa97219ab2e54e717f30881d98d`
- **Audited Files**:
  - `AI/walkthroughs/174_screensaver_matrix_and_locker_stacks.md` (new)
  - `tests/physical/screensaver_test.md`

## 1. Executive Summary

The diff is documentation only: it rewrites the manual physical procedure for `hyprlock` (§3.2),
GDM (§3.3), the `login` TTY (§3.4) and `sudo` (§3.5), the §4 behaviour matrix and §6 rollback, and
adds walkthrough 174. No Rust, test code, packaging, script or CI file changes. Every changed
expected result was checked against the code, the ADRs and this host: the old expectations
(password-less initial GDM/TTY login, a fixed 250 ms `sudo` fallback, `<= 5 ms` / `<= 2 ms`
figures, explicit `pam_soos.so` lines on top of a base stack that already has one, a hand-written
GDM block) contradict ADR 2026-09-30 "Local Session Binding for Facial `Auth`", ADR "PAM Deadline
Derived From Clamped `timeout_ms`", ADR "GDM PAM Stack Placement" and
`Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1 / §5.3. The new text is consistent with them. No
CRITICAL or MAJOR finding; two SUGGESTIONs.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched: `tests/physical/screensaver_test.md` only (manual procedure, no executable
  test).
- Removed/changed `assert`/`#[test]`/`proptest!`/`should_panic`: none.
- New escape hatches (`#[ignore]`, `cfg(any())`, tolerance, epsilon): none.
- Inline `mod tests` changes: none.
- Changed manual expectations and justification:
  - §3.3 TC1 and §3.4 TC1 (initial greeter / TTY login now expects the password and a
    `caller_session_unresolved` refusal): required by ADR 2026-09-30 "Local Session Binding for
    Facial `Auth`" ("Accepted consequences: the initial GDM/TTY login ... never uses face").
    Code path confirmed: `LocalSessionPolicy::authorize_root_peer` → no session scope →
    `authorize_user_manager_caller` → cgroup not under `user@<uid>.service` →
    `SessionDenial::CallerSessionUnresolved` (`crates/daemon/src/session_policy.rs`), refused at
    dispatcher Step 6b, before the Step 8d-1 camera wake (`crates/daemon/src/dispatcher.rs`),
    so "the camera does not light" holds.
  - §3.4 old TC2 (blocked lens, wait for timeout) dropped: unreachable for an initial TTY login
    under the same ADR (the request is refused before any capture).
  - §3.5 TC2 "within 250ms" → "within the `timeout_ms` budget (default 1000 ms)": the 250 ms
    figure contradicts ADR "PAM Deadline Derived From Clamped `timeout_ms`" and the Arch
    `system-auth` primary line sets no `timeout_ms`.
  - §4 matrix timing figures replaced by the `timeout_ms` budget; daemon-stopped row keeps "at
    once" (no `.socket` unit is packaged, so a stopped daemon refuses the connection).
  - §3.3 TC2 (GDM screen unlock) kept as a recorded hardware result, consistent with LSB8 /
    GSO10 (pending hardware).
- No acceptance criterion of BACKLOG #31.4 or matrix PH4 is removed: the document still covers
  `swaylock`, `hyprlock`, `gdm`, `login`, `sudo`, latency budgets and rollback;
  `test_physical_hardware_validation_suite_spec` and `pam_deadline_contract` still pass.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Journal strings: `Local-session policy refused auth request; password fallback`
  (`dispatcher.rs:728`), `caller_session_unresolved` (`session_policy.rs:177`) and
  `Rendered authentication response` with `verdict=` / `reason=` (`dispatcher.rs:1434`) exist;
  the host journal shows the exact rendered forms `reason="caller_session_unresolved"` and
  `verdict=Allow reason=FaceMatch`. → PASS
- Host chains (`/etc/pam.d`): `sudo` = `auth include system-auth`; `login` = `requisite
  pam_nologin.so` + `include system-local-login`; `gdm-password` = `include system-local-login`
  (no managed block on this host); `system-local-login` → `system-login` (`pam_shells`,
  `pam_nologin`, `include system-auth`); `system-auth` carries
  `[success=4 default=ignore] pam_soos.so` and the `event=password-failed` line, identical to
  `packaging/pam/arch/system-auth`. The chains stated in §3.3 / §3.4 / §3.5 match. → PASS
- "Second attempt" on Arch GDM: the managed block is inserted before the anchor
  `auth include system-local-login` (ADR "GDM PAM Stack Placement" rule 1–2), `GDM_PAM_LINE` is
  `[success=done default=ignore]`, so any non-success falls through the include chain to the
  `system-auth` primary soos line, which issues a second daemon request. No PAM-side dedup
  (`pam_set_data`) exists in `crates/pam/src`. Claim accurate; recording it rather than changing
  it is in scope. → PASS
- `soos-admin gdm status|enable|disable|restore` match `GdmAction` (`crates/admin-cli/src/args.rs`);
  default disable path `/etc/soos/gdm.disable` (`args.rs:165`); the PAM module honours it for
  every service containing `gdm` (`crates/pam/src/config.rs:132`), so `gdm disable` also silences
  the base-stack line under `gdm-password`. → PASS
- Operator walk: §3.1 TC3 restarts the daemon, §3.2 step 1 re-checks it; §3.4 TC1 → TC2 flows
  from the same prompt; §3.5 runs `sudo -k` before each case so cached credentials cannot mask
  the result. → PASS
- `sudo` from a GNOME/KDE terminal is allowed by the user-manager amendment (LSB10) only from a
  local session; the "not SSH" instruction matches. → PASS

### PAM Concurrency & Deadlines
No code change. Documented budgets (1000 ms default, GDM 2500 ms) match `crates/pam/src/config.rs`
and `GDM_PAM_LINE`. → PASS

### Panic Safety & Fail-Closed
No code change; no documented path turns an error into `PAM_SUCCESS`. Every fallback row of §4
still says `PAM_IGNORE`. → PASS

### Test Integrity & Anti-Weakening
Each changed manual expectation contradicted an ADR or the code (see §2); none relaxes a
security criterion (they make the procedure stricter: refusal codes must be observed). → PASS

### Memory, Bounds & Secrets
No logging of frames/embeddings/credentials is introduced; the cited journal lines carry only
UIDs, verdict and reason codes. → PASS

### Supply Chain & Automation
No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change. `cargo test -p
soos-invariants`: 378 passed. `scripts/sync_issue.py --check`: OK. → PASS

### English-Only Policy
All added text is English; the only non-ASCII characters are `§`, dashes and arrows. → PASS

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `tests/physical/screensaver_test.md:155` — `cat /proc/<worker pid>/cgroup`
  does not say how to find the reauthentication worker, which only lives during the
  conversation. Add: "while the shield is raised, find it with
  `pgrep -af 'gdm-session-worker \[pam/gdm-password\]'`".
- **[SUGGESTION]** `tests/physical/screensaver_test.md:242` — the rollback `sed` still lists
  `/etc/pam.d/swaylock`, `/etc/pam.d/hyprlock` and `/etc/pam.d/sudo`, which the revised
  §3.1/§3.2/§3.5 say must carry no soos line (and `hyprlock` may not exist, so `sed` prints an
  error). Pre-existing line; optionally reduce it to the base stacks
  (`/etc/pam.d/common-auth /etc/pam.d/system-auth`) and note that other files only need it if
  the administrator added a line by hand.

## 5. Final Verdict

**VERDICT: APPROVED**
