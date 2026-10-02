# Plan Evaluation Report (round 3)
- **Date**: 2026-10-02
- **Issue**: GitHub #325 — presence auto-unlock review follow-ups (#324) (GitHub-only, no `AI/BACKLOG.md` entry)
- **Branch**: `fix/presence-review-followups`
- **Base commit**: `35a708a`
- **Plan**: `AI/architect_spec_presence_followups.md` (revision 3)

## 0. Findings of Earlier Rounds and Resolution
| Round | Finding | Resolution | Status |
|---|---|---|---|
| 1 | F1 MAJOR token scan under-detects (NBSP + unterminated `[`, `\` before `#`) | §2.3 substring superset rule; PFU4 counter-example tests | Resolved (verified against `pam_line.c`, `pam_misc.c`, `pam_handlers.c:71` in round 2) |
| 1 | F2 MAJOR PAU8 "not enrolled" breaks under the probe | §9 setup-only migration (enroll UID 1001); diff limited to the enrolled vector | Resolved |
| 1 | F3 MINOR `Closes #325` vs pending item 6 | PFU7 follow-up issue | Resolved, on condition that the follow-up issue exists and is linked before merge |
| 1 | F4 MINOR probe edge tests, connect counter | PFU6 wording | Resolved |
| 1 | F5 MINOR "under the lock" not testable | PFU1 + §9 lock-holder hook | Resolved |
| 1 | F6 MINOR connect-only build not testable | `presence_followups_contract::test_pfu_only_connect_opens_a_bus_connection` | Resolved |
| 1 | O1 / O2 | §2.5 rationale / drafts revised before hand-off | Resolved / noted |
| 2 | R2-F1 MAJOR `include` / `substack` of a path not followed | §2.3 option (a): a token equal to `include` / `substack` (ASCII case-insensitive) or `@include`, followed by a token containing `/`, is a policy hit (`Undeterminable`); tests `test_pfu_pam_scan_include_of_a_path_is_detected`, `test_pfu_pam_scan_plain_name_include_stays_usable` | Resolved (analysis below) |
| 2 | R2-F2 MINOR name-only module match | `scan_pam_faillock_options` doc sentence + ADR amendment wording (accepted risk (d)) | Resolved |

## 1. Coverage Matrix
| Acceptance line (issue #325) | Spec element | Status |
|---|---|---|
| 1 fresh stamp under the lock, `ClockUnavailable` | §2.1, PFU1 | Covered |
| 2 reachable connect bound, documented 500 ms coverage | §2.2, PFU2, F6 invariant | Covered |
| 3a document deliberate over-detection | §2.3, PFU4 (superset per file + include-of-path refusal + name-only limit) | Covered |
| 3b `/etc/pam.conf` without PAM directory ⇒ `Undeterminable` | §2.3, PFU3 | Covered |
| 4 one account check in flight | §2.4, PFU5 | Covered |
| 5 no logind polling without templates | §2.5, PFU6, §9 migration | Covered |
| 6 flaky clamp test | §8, PFU7 (diagnosis, owner follow-up) | Covered |

## 2. Facts Verified Against Code (round 3)
| Fact cited by plan | Source | Actual value | Match |
|---|---|---|---|
| libpam opens absolute, nested and `..` include/substack targets | `pam_handlers.c` v1.7.1 `_pam_open_config_file:307-345` | `service[0] == '/'` opened as is; otherwise `"<dir>/%s"` for the three directories | Yes |
| A plain-name target resolves inside the scanned directories | same, `pamd_dirs` = `/etc/pam.d`, `/usr/lib/pam.d`, `VENDORDIR/pam.d` | matches `DEFAULT_PAM_DIRS` (assuming `VENDORDIR=/usr/etc`, see O3) | Yes |
| Existing tests contain no include line whose verdict would flip | `presence_account_tests.rs`, `presence_candid_review_tests.rs`, `presence_logging_tests.rs`, `presence_worker_tests.rs` | no `include` / `substack` line at all | Yes |
| New tests exercise the rule's edges | `presence_followups_tests.rs:636-669` | `/abs`, `sub/file`, `../x`, `INCLUDE`, `Substack ./local`, `@include /path`, pam.conf form, NBSP separator; negatives: plain names, `envfile=/etc/environment`, an absolute module path, a commented include | Yes |

## 3. Pillar Analysis
### Pillar 1 — Architecture & threat model
- Failure scenario considered: a symlink named like a template, an enrollment racing the probe, synchronous `getdents`. Unchanged since round 2, and excluded by the design.
- Result: PASS.

### Pillar 2 — PAM deadline & concurrency
- Failure scenario considered: a connect under the call bound, or a hung connect. Both are rejected by the PFU2 tests and the worker invariant. PAM is untouched.
- Result: PASS.

### Pillar 3 — Panic safety & fail-closed
- Failure scenario considered: an include libpam follows that the rule misses. I checked these cases:
  - A bracketed control `[include]`: for libpam this is an action list, not an include, so there is nothing to miss.
  - A target split by Unicode whitespace: the `/` stays in one guard token.
  - A bracketed target `[/x y]`: the guard token `[/x` contains `/`.
  - An include and its target joined by a `\` continuation: the guard joins those lines too.
  - A `\` before `#`: the guard joins more lines than libpam, which only causes false positives.
  - The pam.conf form `login auth include /x`: tested.
  - Plain-name includes such as `.` or `..`: these open a directory, which libpam fails to parse.

  None of these lets libpam read a file the guard has not scanned. False positives (a module argument literally equal to `include` followed by a path) only refuse presence.
- Result: PASS.

### Pillar 4 — Dependencies
- Failure scenario considered: a new crate or `unsafe` code. Neither is added.
- Result: PASS.

### Pillar 5 — Data confidentiality
- Failure scenario considered: a new log or read that exposes content. There is none. Reads still go through `read_source`/`Zeroizing`, and the probe returns a bool.
- Result: PASS.

### Pillar 6 — Test integrity
- Failure scenario considered: the include rule flips an existing assertion. No existing test has an include line. The PAU25 subdirectory test has no include, so it stays `Usable`. The only migrations remain the setup-only ones in §9.
- Result: PASS.

## 4. Findings (round 3)
- **[MINOR] O3 (observation, pre-existing, out of scope)**: the guard assumes `VENDORDIR=/usr/etc`. A libpam built with another `VENDORDIR` (for example `/usr/share`) reads `<VENDORDIR>/pam.d`, which the guard does not scan. This applies to every stack file, not only to includes. — No plan change required. A follow-up could note the assumption in `Docs/DAEMON.md` §6 next to the directory list.
- **[MINOR] O4 (observation)**: whether Debian's `@include` is case-sensitive was not verified against the Debian patch. The rule matches `@include` exactly. — No plan change required. The tester may add `@INCLUDE /x` as an extra positive if the developer chooses ASCII case-insensitive matching for it too.

No CRITICAL or MAJOR finding remains.

## 5. Verdict
VALIDATION_VERDICT: APPROVED
