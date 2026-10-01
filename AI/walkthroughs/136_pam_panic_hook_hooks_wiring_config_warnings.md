# Walkthrough 136 — PAM Panic Hook Chaining, Production `PamHooks` Path and Configuration Warnings

- **Date**: 2026-09-30
- **Issues**: GitHub #263 (PAM-09), #264 (PAM-16), #265 (PAM-17); #221 (PAM-08) reviewed, remaining part blocked
- **Branch**: `fix/p3-pam-hygiene`
- **Matrix criteria**: PHY1–PHY9 (new component `pam-hygiene`); PHS7 / PHS10 unchanged (pending)
- **ADR**: 2026-09-30 "Chained Once-Installed Panic Hook, Production `PamHooks` Path and Logged PAM Argument Rejections"

---

## 1. Findings

| Issue | Finding | Location (before) |
|---|---|---|
| #263 | `init_panic_hook` replaced the process panic hook once (`Once`) and never chained or restored it: every panic of the host (same libstd copy) was swallowed silently | `crates/pam/src/syslog.rs` |
| #264 | Tautological `total_received != total_capacity` branch; client re-implemented `Response::matches_request` / `Verdict::should_ignore` inline; `impl PamHooks for SoosPam` not reachable from the exported symbols; unreachable `argv.add(i).is_null()`; `build.rs` never re-ran after `libpam0g-dev` was installed | `ipc.rs`, `lib.rs`, `config.rs`, `build.rs` |
| #265 | Invalid `timeout_ms=`, arguments of 256 bytes or more and a `service=` cut inside a multibyte character silently fell back to defaults, without any syslog line | `config.rs` |
| #221 | `PAM_SILENT` and the post-connect "Looking for face..." were already done (PHS5/PHS6). The daemon-absent case still prints one generic "unavailable" line | `lib.rs` |

## 2. Specification

- `syslog::catch_entry<F, R>(f) -> std::thread::Result<R>`: installs the hook if needed, marks
  the thread as inside an entry point (thread-local depth, restored by a drop guard even on
  panic) and runs `catch_unwind`. `syslog::panic_summary(payload, fallback)` is shared by the
  three catch sites. The hook chains to the hook captured with `std::panic::take_hook()`.
- `syslog::log_warning` (`LOG_AUTHPRIV | LOG_WARNING`), sharing one `log_with_priority` helper
  with `log_info`.
- `config::ConfigWarning` (`ArgumentTooLong`, `NullArgument`, `TooManyArguments`, `NotUtf8`,
  `InvalidValue`, `EmptyValue`, `TimeoutClamped`, `ServiceTruncated`, `UnknownArgument`) with a
  value-free `Display`. New `parse_cstrs_with_warnings`, `parse_argv_with_warnings`,
  `collect_argv`, `collect_argv_logged`; `parse_cstrs` / `parse_argv` log the warnings and keep
  their signatures.
- `pam_sm_authenticate`: `collect_argv_logged` then `SoosPam::sm_authenticate` for a non-null
  handle, detached flow otherwise.
- `ipc`: dead branch removed, `resp.matches_request(&req.request_id)`; `lib.rs` maps the
  verdict with `Verdict::should_ignore`.
- `build.rs`: `rerun-if-changed` for the found `libpam.so`, or the `libpam.so.0` fallback and
  its directory.

## 3. Red Evidence

Tests written first, on the unchanged code:

```
panic_hook_chain_tests: test_panic_outside_entry_point_still_reaches_prior_hook FAILED
config_service_boundary_tests: test_argv_service_cut_inside_multibyte_char_keeps_prefix FAILED
                               test_cstr_service_cut_inside_multibyte_char_keeps_prefix FAILED
pam_hygiene_contract: 6 of 6 tests FAILED
error[E0432]: unresolved import `pam_soos::syslog::catch_entry`
error[E0432]: unresolved imports `pam_soos::config::parse_argv_with_warnings`,
              `pam_soos::config::parse_cstrs_with_warnings`, `pam_soos::config::ConfigWarning`
```

The chain test failure printed no panic message at all: the module hook had swallowed the
test harness output, which is exactly the finding.

## 4. Implementation Notes

- The per-call take/set/restore swap suggested by the review was not used: with concurrent
  PAM calls the swaps interleave and can leave the module hook installed forever. One chained
  installation has no race and still returns every foreign panic to its owner.
- The panic-hook tests live in two separate test binaries with one test each: the hook is
  process-global and installed at most once, so the prior hook must be set before anything
  else in the binary calls the module.
- `parse_cstrs` now enforces the same 256-byte argument bound as `parse_argv` (both paths share
  `parse_cstrs_with_warnings`). Warning indices from `collect_argv` are `argv` positions; the
  ones from the parser are positions in the list handed to it.
- `build.rs` watches a directory only in the fallback path (no `libpam.so` on the host), so
  normal builds track a single file.

## 5. #221 Status (decision needed)

The remaining #221 item is PHS7 "no message at all when the daemon connection fails". It
contradicts the pre-existing `pam_handle_tests::test_unavailable_feedback_is_a_single_generic_text`,
which requires the offline case to produce the same generic text as `Unavailable`. The test was
not modified (test integrity); the fail-quiet option is submitted as a test-change proposal.
PHS10 (Docker `PAM_TEXT_INFO` count) stays a follow-up: it needs a `pam_test_runner` flags option
and a new T-number coordinated with the parallel matrix work.

## 6. Audit

- No `unwrap` / `expect` / `panic!` added in PAM production code; the hook uses `try_with` and
  `try_borrow_mut` so it cannot panic during thread-local teardown or re-entrancy.
- Every `unsafe` block added in `config.rs` / `lib.rs` has a `// SAFETY:` comment; argument reads
  stay bounded by `MAX_ARG_LEN` and `MAX_ARGC`.
- Warnings never log a rejected value; the syslog format string stays `"%s"`.
- Verdict mapping unchanged: `PAM_SUCCESS` only for `Verdict::Allow`, every panic still returns
  `PAM_IGNORE`.

## 7. Follow-ups

- User decision on PHS7 (fail-quiet on `IpcError::Connect`) and the matching test change.
- PHS10 Docker case.

## Follow-up: fail quiet when the daemon is absent (#221, 2026-10-01)

The user chose the fail-quiet option. `authenticate_flow` records whether the connect callback fired; without a connection (`IpcError::Connect`: daemon not installed or stopped) no outcome text is sent, so the next module prompts as if soos were absent. A connected daemon still produces "Looking for face..." and one outcome line. The offline case of `pam_handle_tests::test_unavailable_feedback_is_a_single_generic_text` now asserts an empty conversation (user-approved change); new tests: `crates/pam/tests/pam_fail_quiet_tests.rs`. Matrix row PHS7 is ✅ Verified.
