# Candid Review Report

- **Date**: 2026-09-30
- **Target Branch**: `fix/p1-daemon-pam-batch`
- **Base (merge-base)**: `067c68c`
- **Reviewed-Diff-Fingerprint**: `c7008aa175d77d2ed6d5d5a8f4f33b87efe386ad9ce287f42cd3b8ccc2b6a70f`
- **Review Round**: 2. Round 1 (fingerprint `c86aab9e…`) was CHANGES_REQUESTED on finding 1.
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `.github/workflows/ci.yml`, `AI/ARCHITECTURE.md`, `AI/BACKLOG.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/94_pam_deadline_feedback_and_service.md`, `AI/walkthroughs/95_daemon_peer_limits.md`, `AI/walkthroughs/96_daemon_inference_budget.md`, `AI/walkthroughs/97_daemon_local_session_binding.md`, `Docs/CI_CD_AND_SECURITY.md`, `Docs/IPC_PROTOCOL.md`, `Docs/PAM_DOCKER_TEST_MATRIX.md`, `Docs/PAM_MODULE.md`, `Docs/POLICY_CRATE.md`, `crates/daemon/src/{config,dispatcher,inference,lib,limits,main,session,session_policy}.rs`, `crates/daemon/tests/{inference_budget_tests,peer_limits_tests,pipeline_integration_tests,session_policy_tests}.rs`, `crates/pam/src/{config,ipc,lib}.rs`, `crates/pam/tests/{config_tests,ipc_tests,pam_handle_tests}.rs`, `run_tests.sh`, `tests/docker/test_suite.sh`

## 1. Executive Summary

Round 2 re-reviews the whole batch diff (#157, #158, #159, #160, #173, #174, #175, #176). It focuses on the rework in commits `939b724` (tests), `8d26e96` (`session_policy.rs`) and `532ad17` (docs and minors). The rework touched no dispatcher, PAM, inference or limiter code, so the round-1 conclusions for those seams still hold. I re-ran the daemon test suite (all green, `session_policy_tests` 47/47), `cargo clippy -p soos-daemon --all-features --all-targets -D warnings` (clean) and `cargo fmt --check` (clean).

Round-1 finding 1 (MAJOR) is resolved, and the code matches the user decision of 2026-09-30. A root peer with no session scope is allowed only when:
- its systemd-managed cgroup lines (`0::` and/or `name=systemd`) all agree on `/user.slice/user-<uid>.slice/user@<uid>.service/<non-empty child>...`;
- both UIDs are strict decimals, equal to each other and to the target;
- the target owns an active, `REMOTE=0`, seat-attached, `CLASS=user` session;
- the target owns no session that is remote or has an unknown remote flag, in any state (records without `UID=` count against it).

Every IO or parse error denies. The new `session-<id>.scope` rule (first unit below the slices, as in `sd_pid_get_session`) cannot be bypassed by a user-created transient `session-N.scope` inside `user@.service`. Real TTY, GDM and login scopes still resolve. Minors 2 and 3 are fixed, and suggestions 4 and 5 are recorded as follow-ups in walkthroughs 96 and 95. I found no CRITICAL or MAJOR issue. Two MINOR findings and two SUGGESTIONS remain.

## 2. Test Changes (mechanical listing from step 3)

- Test files touched: `crates/daemon/tests/{inference_budget_tests,peer_limits_tests,session_policy_tests}.rs` (new on this branch), `crates/pam/tests/pam_handle_tests.rs` (new), `crates/pam/tests/{config_tests,ipc_tests}.rs` (additions only), `crates/daemon/tests/pipeline_integration_tests.rs`, `tests/docker/test_suite.sh`, `run_tests.sh` (comments).
- Removed or changed assertions (`^-.*assert|#[test]|…`): **none** in the frozen patch.
- New escape hatches (`#[ignore]`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` changes: **none**.
- `pipeline_integration_tests.rs`: the only removed line is `let target_uid = fixture.current_uid.saturating_add(42);`, replaced by `fixture.current_uid`. This is setup only, with no assertion changed. It is the single user-approved #175 contract migration (`test_12_4`).
- Round-2 rework of `session_policy_tests.rs` (diff `939b724~1..HEAD`):
  - The only removed lines are the `use soos_daemon::session_policy::{…}` import, rewritten to add `parse_user_manager_uid_from_cgroup`.
  - The `MockLogind` helper gained a `pid_cgroups` map, a `with_cgroup` builder and a `cgroup_of_pid` override (it honours `fail`). The existing helpers `with_pid`, `with_session`, `failing` and `record` are unchanged.
  - The test count went from 32 to 47. All 32 original tests are byte-identical, and the 15 new tests are pure additions.
  - The trait default `cgroup_of_pid → Ok(None)` means that original mocks without cgroups still take the fail-closed path (`CallerSessionUnresolved`), exactly as before.
- New tests can fail against plausible wrong implementations:
  - a lax `parse::<u32>` (the `+1000`, `01000` and ` 1000` vectors);
  - a missing slice/service equality check;
  - an "anywhere in path" session-scope match (`test_cgroup_session_scope_nested_in_user_manager_is_not_a_session`);
  - a remote check limited to active sessions (the closing SSH variant);
  - a missing `UID ==` check (`su alice` from Bob's terminal);
  - an error mapped to allow (`SessionsFail` mock);
  - a truncating reader (the oversized cgroup file).

## 3. Deep Reasoning Audit

### Logic & Architecture

**Finding 1 resolution** (`session_policy.rs:239-356`, `:550-582`). Scenarios attempted:

- **`su alice` from Bob's GNOME terminal**: cgroup `user@1001.service`, so `manager_uid(1001) != target(1000)` and the request is denied with `user_manager_uid_mismatch`. PASS.
- **`sudo` from Alice's GNOME terminal, Konsole or gnome-shell polkit agent, Alice seated, no SSH session**: allowed. The real layouts are covered by both the mock and the `SystemLogind` tempdir tests. PASS.
- **Alice seated and also SSH'd in (active, closing, or `REMOTE` absent)**: denied with `user_manager_caller_remote_session_active`. Bob's SSH session does not affect Alice. PASS.
- **UID parsing**: `+1000`, `-1000`, ` 1000`, `01000`, the empty string, `1000x`, `4294967296` and mismatched slice/service UIDs all give `UserManagerCgroupMalformed`. `0` is accepted as the single digit, but root is never a user-manager target in practice, and a target of 0 would still need a root seat session. PASS.
- **Path edge cases**:
  - A trailing slash (`user@1000.service/`) or the manager with no child is malformed. The manager itself lives in `init.scope`, so real processes always have a child.
  - `user@1000.service.d` fails `strip_suffix(".service")` and is malformed.
  - A path not rooted at `/` (for example `/../../user.slice/...` if the daemon ever ran in a private cgroup namespace) gives `Ok(None)` and a deny.
  - `..` as a component is not produced by the kernel. A `..` below the manager is only a child name and cannot change the resolved UID.
  - Unicode components are compared by exact prefix/suffix with no indexing, so there is no panic.
  - `splitn(3, ':')` keeps colons inside the path.
  - PASS.
- **Hierarchy selection**:
  - Only `0::` and `name=systemd` are considered. `cpu,cpuacct` and other v1 controllers are ignored, and elogind's `name=elogind` is ignored (fail closed, since there is no `user@` there).
  - If the systemd lines disagree (two managers, or session scope vs manager), the result is malformed.
  - PASS.
- **Oversized input**: `read_bounded` caps the cgroup file at 16 KiB and a file at the bound is an error, so the result is `LogindUnavailable`. PASS.
- **Tightened session-scope rule** (`first_unit_component`, `:239-243`):
  - `user@1000.service/app.slice/session-2.scope`, `user@1000.service/session-2.scope` (both creatable with `systemd-run --user --scope --unit=session-2`) and `system.slice/foo.service/session-2.scope` resolve to no session.
  - `user.slice/user-1000.slice/session-2.scope` and its sub-cgroups resolve to `2`. TTY `login`, GDM (`session-c1.scope` for the greeter, `session-N.scope` for the user) and `sshd` sessions keep their layout.
  - An unprivileged user cannot create a `*.slice` above their manager: `/user.slice` and `user-<uid>.slice` are root-owned.
  - PASS.
- **Fallthrough safety**: `session_id_of_pid` returns `None` both for "no session scope" and for "malformed or disagreeing session lines", and both now reach the user-manager path. I checked whether this can widen access. It cannot: that path independently requires every systemd line to agree on `user@<target>.service`, and a line with a session-scope first unit is never a `user@` line, so a disagreement becomes `UserManagerCgroupMalformed` or `CallerSessionUnresolved`. A resolved session whose record vanished still gives `CallerSessionUnresolved` and does not fall back. PASS.
- **Precedence**: a caller inside an SSH session scope keeps the session-scope rules (`test_root_peer_session_scope_takes_precedence_over_user_manager`). PASS.
- **Root vs unprivileged rules**: `authorize_auth` still branches on `peer.uid == 0`, then `peer.uid == target`, and denies everything else. The unprivileged rule `authorize_same_uid_peer` is unchanged. PASS.
- **TOCTOU**: the cgroup file is read twice (`session_id_of_pid`, then `cgroup_of_pid`). A migration between the two reads is judged against whichever position is read, and each position is safe on its own. The PID-reuse residual is unchanged and documented. PASS.
- **Documentation consistency**: `Docs/IPC_PROTOCOL.md` §10 and `Docs/PAM_MODULE.md` item 5 now describe both root rows and the new denial codes. The ADR amendment in `AI/DECISIONS.md` records the user decision and the accepted residual risk ("malware already running as the target user"). Minor 2 is fixed: §10 and §11 are distinct and no numbers are duplicated. Minor 3 is fixed: the comment at `tests/docker/test_suite.sh:185` now reads T12. PASS.
- The dispatcher, admission, deadline, event-trust, inference and PAM `service=` logic are untouched since round 1. I spot-checked for regressions and found none. PASS.

### PAM Concurrency & Deadlines
- There is no change in `crates/pam` since round 1: no Tokio or threads, the cumulative `Deadline` re-arms every read and write, and zero budget gives `Timeout`. The new daemon work is two bounded file reads plus one bounded directory scan, all inside the request deadline and as costly as the existing same-UID path. PASS.

### Panic Safety & Fail-Closed
- The new code has no `unwrap`, `expect`, indexing or slicing. `parse_strict_uid` checks the bytes before calling `parse`.
- Every error in `authorize_user_manager_caller` maps to a `SessionDenial` (`LogindUnavailable`, `CallerSessionUnresolved`, `UserManagerCgroupMalformed`, `UserManagerUidMismatch`, `UserManagerCallerRemoteSessionActive`, `UserManagerNoLocalSeatSession`), and each one becomes `ProtocolError/UidMismatch`, then `PAM_IGNORE`.
- The trait default `cgroup_of_pid → Ok(None)` fails closed.
- One residual non-denying error path exists in the shared `sessions()` scan. See Finding 1 below.

### Test Integrity & Anti-Weakening
- See §2. No assertion was removed or relaxed. The only setup migration is the approved `test_12_4` change, and the mock extension is purely additive. PASS.

### Memory, Bounds & Secrets
- The bounds are unchanged: 16 KiB cgroup file, 4 KiB record and 1024 directory entries.
- The new denial codes are static strings. Logs carry UIDs and reason codes only, and the cgroup content (which can contain app names) is never logged. PASS.

### Supply Chain & Automation
- The rework has no `Cargo.*`, `deny.toml` or workflow changes. The `test_suite.sh` change is a comment. PASS.

### English-Only Policy
- All added code, comments, docs and walkthrough text are in English. PASS.

## 4. Detailed Findings & Action Items

1. **[MINOR]** `crates/daemon/src/session_policy.rs:440`: the `sessions()` scan skips `read_dir` entries that fail (`let Ok(entry) = entry else { continue; }`).
   - **Why it matters now**: `authorize_user_manager_caller` (`:566-572`) uses this scan to prove the absence of a remote session for the target. So an I/O error on one entry silently removes that record, and it can remove Alice's SSH session from the check. The user decision requires that "any error denies".
   - **Failure scenario**: a transient `EIO` or `ENOMEM` during `readdir` on `/run/systemd/sessions` drops the entry of Alice's live SSH session. `sudo` launched from that SSH session through `systemd-run --user` is then face-authorized by whoever sits at the desk.
   - **Severity**: this is not attacker-triggerable (the directory is a root-owned tmpfs, and per-entry `readdir` errors are practically nonexistent), hence MINOR.
   - **Correction**: return `Err(LogindError(..))` for a failed entry instead of `continue`. This is also safe for the unprivileged path, where it can only deny more.
2. **[MINOR]** `AI/DECISIONS.md:29` (ADR amendment) and `Docs/PAM_MODULE.md` item 5: condition (c) ("no remote session") only sees remote footholds that logind registers.
   - **Gap**: code running as the target through a channel with no logind session also reaches `user@<uid>.service` through the `/run/user/<uid>/bus` socket and bypasses (c). Examples: `sshd` with `UsePAM no` or no `pam_systemd`, a service or daemon running as the user, a container with the user's UID.
   - **Why it is not covered**: before the amendment such a foothold had no face path at all. The accepted residual is phrased as "malware already running as the target user inside the desktop session", which is narrower than this.
   - **Correction**: widen that sentence to "any process running as the target user, including remote access not registered as a logind session (e.g. sshd without `pam_systemd`)". This is documentation only. The behavior matches the user decision.
3. **[SUGGESTION]** `crates/daemon/src/session_policy.rs:251-272`: `parse_session_id_from_cgroup` still considers every hierarchy line, including v1 resource controllers. `parse_user_manager_uid_from_cgroup` restricts itself to `0::` and `name=systemd`.
   - **Effect**: on a legacy cgroup-v1 host, a `cpu`/`pids` line that names a session scope different from the `name=systemd` line would be used for the session lookup.
   - **Why it is only a suggestion**: this is not exploitable (the result is still checked against the record's `UID == target`, and v1 controllers are not delegated to users).
   - **Proposal**: restrict both parsers to the systemd-managed hierarchies for one consistent source of truth.
4. **[SUGGESTION]** polkit ≥ 126 can run `polkit-agent-helper-1` socket-activated as `system.slice/.../polkit-agent-helper@N.service` instead of as a child of gnome-shell. On such hosts polkit prompts would fail closed to the password (`caller_session_unresolved`). Record this for hardware validation alongside the existing lock-screen note. It is not a security issue.

Carried from round 1 (resolved in this round):
- Finding 1 (MAJOR, user-manager callers): **resolved** as specified by the user decision.
- Finding 2 (MINOR, duplicate `## 10.`): **resolved**.
- Finding 3 (MINOR, stale T11 comment): **resolved**.
- Suggestions 4 (inference warm-up) and 5 (evidence-store global cap): **recorded as follow-ups** in `AI/walkthroughs/96_daemon_inference_budget.md` and `AI/walkthroughs/95_daemon_peer_limits.md`.

## 5. Final Verdict

**VERDICT: APPROVED**
