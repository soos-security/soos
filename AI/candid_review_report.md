# Candid Review Report

- **Date**: 2026-10-02
- **Target Branch**: `chore/swaylock-pam-doc`
- **Base (merge-base)**: `be1dd95`
- **Reviewed-Diff-Fingerprint**: `7917341f5a4e11853c1805c7c5ba17a604b9fe34a2221a3bc2ba2f971d1e0c19`
- **Audited Files**: `Docs/DISTRIBUTION_DEPLOYMENT.md`, `AI/walkthroughs/172_swaylock_pam_behaviour_doc.md`

## 1. Executive Summary

Documentation-only change. It rewrites `Docs/DISTRIBUTION_DEPLOYMENT.md` §5.3 (Wayland screen
locker PAM behaviour), removes the false "Instant Unlock" guarantee (< 150 ms, no Enter needed),
and adds walkthrough 172. Every factual claim of the new §5.3 was checked against the repository,
the host PAM configuration (Arch Linux, `swaylock 1.8.6-1`, `pambase 20260616-1`, `pam 1.7.3-1`)
and the swaylock 1.8.7 source tree (`password.c`, `pam.c`). No claim was found to be wrong. One
MINOR finding: a stale copy of the removed claim remains outside the diff, in
`tests/physical/screensaver_test.md`. No CRITICAL or MAJOR finding.

## 2. Test Changes

Mechanical listing on `target/candid_diff.patch`:

- Test, fixture or invariant files touched: none.
- Removed or changed assertions (`assert`, `#[test]`, `proptest!`, `should_panic`): none.
- New escape hatches (`#[ignore]`, `cfg(any())`, tolerance, epsilon): none.
- Inline `mod tests` changes: none.

These invariants read `Docs/DISTRIBUTION_DEPLOYMENT.md`: `arch_faillock_ci_contract.rs`,
`packaging_ownership_contract.rs` (both read only §5.2, split at `### 5.3 `, a heading this diff
keeps unchanged), `quality_followups.rs` (§2.1), and `lib.rs` (the topic list, which still finds
`swaylock` and `pam_faillock`; the Fedora `authselect` activation; `pam_deny`; the rollback
paths). `cargo test -p soos-invariants` passed: 378 passed, 0 failed.

## 3. Deep Reasoning Audit

### Logic & Architecture
Each claim was checked against a source:
- **Arch chain**: on the host, `/etc/pam.d/swaylock` (owned by `swaylock 1.8.6-1`) is
  `auth include login`. The chain `login` → `system-local-login` → `system-login` →
  `system-auth` matches the host files, and the host `system-auth` is identical to
  `packaging/pam/arch/system-auth`. PASS.
- **Submit-only verification**: in swaylock 1.8.x `password.c`, `submit_password` returns early
  when `ignore_empty && password.len == 0`. In `pam.c`, `pam_authenticate` runs only for each
  password request it reads. `PAM_TEXT_INFO` and `PAM_ERROR_MSG` are dropped (`break`). The
  walkthrough's naming of the function is correct. PASS.
- **`pam_soos.so` output**: the module writes nothing to stdout or stderr. Its feedback goes
  through the `PAM_TEXT_INFO` conversation, which swaylock ignores (`crates/pam/src/lib.rs`
  `PamFeedback`), so "nothing is shown on screen" holds. PASS.
- **Empty-password failure path**: on the §5.2 stack, a face miss returns `PAM_IGNORE` and takes
  `default=ignore`. `pam_unix.so` then fails on the empty password for an account that has a
  password. The event line sends `PasswordFailed` (`lib.rs` lines 277-287), and `authfail`
  records the failure. PASS.
- **Fedora `with-faillock`**: `packaging/pam/fedora/soos/system-auth` places `preauth` before
  `pam_soos.so` and `authfail` after `pam_unix.so`, so the counting and lockout claims hold
  there too. PASS.
- **Faillock defaults**: `/etc/security/faillock.conf` lists `# deny = 3` and
  `# unlock_time = 600`, which are the built-in defaults. `preauth` comes before `pam_soos.so`
  on the Arch stack, so a locked account blocks both face and password. This matches the §5.2
  text and the `arch_linux_test.sh` assertions. PASS.
- **Timeout**: `crates/pam/src/config.rs` has `DEFAULT_TIMEOUT_MS = 1000`, clamped to
  `MIN_TIMEOUT_MS = 10` ..= `MAX_TIMEOUT_MS = 5000`. The §5.2 primary line
  `auth  [success=4 default=ignore]  pam_soos.so` has no arguments, so the default applies.
  PASS.
- **"Three consecutive live captures"**: matches `DEFAULT_PAD_CONSENSUS_REQUIRED = 3`, whose
  passing frames must be consecutive (`crates/policy/src/pad_consensus.rs`). The 0.3–0.5 s
  figure is labelled as a hardware measurement, which is acceptable. PASS.
- **Session policy**: `/usr/bin/swaylock` is `0755 root:root`, not setuid. An unprivileged peer
  therefore falls under rule (2) of ADR 2026-09-30 "Local Session Binding for Facial `Auth`":
  same UID through `SO_PEERCRED`, and the target must own an active session that is not flagged
  `REMOTE=1` (`session_policy.rs` `is_active_non_remote_of`). The doc's wording "active,
  non-remote logind session" is faithful. PASS.
- **Debian/Ubuntu and Fedora/RHEL chains**: the doc describes these only as "equivalent chains",
  ending in `common-auth` or `system-auth`, which the soos packaging profiles target. Plausible,
  and not presented as verified detail. PASS.
- **Removed claim still elsewhere**: the removed "Instant Unlock" claim survives outside the
  diff. FINDING 1 (MINOR).

### PAM Concurrency & Deadlines
No code change. The documented deadline matches the clamped `timeout_ms` contract. PASS.

### Panic Safety & Fail-Closed
No code change. The new "Graceful Fallback" wording (a failure returns `PAM_IGNORE`) matches
`lib.rs`. No text suggests an error path leading to `PAM_SUCCESS`. PASS.

### Test Integrity & Anti-Weakening
No test or invariant changed, and the invariant suite is green. PASS.

### Memory, Bounds & Secrets
No code change. The documentation exposes no credentials and no biometric data. PASS.

### Supply Chain & Automation
No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change. Not applicable.

### English-Only Policy
Both files are in English only. No added line exceeds 100 columns, which matches the wrapping
of the surrounding §5.2 text. Walkthrough number 172 follows 171. `sync_issue.py --check`
passes, and the unregistered `chore/` branch is accepted. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `tests/physical/screensaver_test.md:42-57` and `:176` (outside the diff). The
  physical validation plan still shows a `/etc/pam.d/swaylock` that includes `system-auth` and
  adds its own event line. It also still expects "Screen unlocks instantly (< 150ms) without
  prompting for a password" after "Press any key (e.g. `Space` or `Enter`)", and its matrix row
  says "Unlocks session instantly (<= 150ms)". Each of these contradicts the corrected §5.3:
  swaylock verifies only on Enter, the latency is about 0.3–0.5 s, and `Space` types a
  character instead of submitting. Required correction, in a follow-up or in this branch:
  - Replace the PAM block with Arch's `auth include login`, and point to §5.3 for the chain.
  - Change step 3 to "Press `Enter` on an empty field" with "Expected Result: the screen
    unlocks within about 0.5 s (bounded by `timeout_ms`)".
  - Change line 176 to "Unlocks after Enter within the `timeout_ms` budget".

## 5. Final Verdict

**VERDICT: APPROVED**
