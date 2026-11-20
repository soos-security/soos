## Audit Constraints — GitHub #487 (batch 10: #574, #575, #581)

| # | Constraint | Applies to (file::fn) | Verified by (test / lint / invariant / grep) |
|---|---|---|---|
| 1 | No `unwrap`/`expect`/`panic!`/indexing/unchecked arithmetic in production code | `crates/admin-cli/src/pam_check.rs`, `gdm.rs`, `main.rs` | `cargo clippy --all-targets --all-features -- -D warnings` (workspace lints) |
| 2 | Every PAM file read goes through `pam_stack::read_bounded_utf8` (64 KiB, regular file, `O_NONBLOCK`, UTF-8) | `pam_check::Flattener::flatten_stack`, `check_pam_services` | code review; `grep -n "read_to_string" crates/admin-cli/src/pam_check.rs` is empty |
| 3 | Bounded analysis: include depth `MAX_PAM_INCLUDE_DEPTH`, `MAX_PAM_STACK_READS` opens per analysis, `MAX_PAM_CHECK_RULES` flattened rules, `MAX_PAM_CHECK_SERVICES` services, `MAX_PAM_CHECK_BRANCHES` forked branch points; exceeding any bound fails closed (refusal or "unanalysed") | `pam_check` | `test_pstk01_pam_check_reports_unanalysable_services`, unit tests in `pam_check` |
| 4 | Include targets and service names are flag-safe names (`[A-Za-z0-9._-]`, no leading `.`), never paths; `--pam-dir` entries that are not regular files are skipped | `pam_check::valid_name`, `check_pam_services` | `test_pstk01_pam_check_ignores_services_without_a_done_soos_rule` |
| 5 | Fail closed: an unanalysable `gdm enable` tail refuses; an unknown simulation falls back to the static enforcing-rule check; `pam check` never reports an unanalysable service as clean (exit 2) | `gdm::plan_gdm_enable`, `pam_check::gdm_tail_skips` | `test_pstk01_gdm_enable_refuses_unresolvable_later_delegation`, `..._reports_unanalysable_services` |
| 6 | `gdm enable` refusal writes nothing: no backup, PAM file byte-identical, disable flag kept | `gdm::plan_gdm_enable` (runs before any write) | `assert_enable_refused` in `pam_success_done_tests.rs` |
| 7 | `gdm disable` writes only a flag `pam_soos.so` reads for the service of `--pam-file`; the CLI refuses a flag outside `/etc/soos`; status never reports `Disabled` for a file the module ignores | `gdm::configure_gdm_with_options`, `get_gdm_status`, `main.rs` | `gdm_flag_path_tests.rs` (PFL1-PFL4), contract against `PamConfig::is_disabled` |
| 8 | Output prints only file names, line numbers, control and module words; no module arguments that could carry secrets, no user data; table output strips control characters | `pam_check::PamCheckReport::format_table` | code review; names are validated, rules rendered from parsed tokens |
| 9 | `crates/pam` unchanged; no new external dependency (`soos-pam` is a workspace dev-dependency) | `Cargo.toml` | `git diff --stat crates/pam` empty; `cargo deny --locked check` |
| 10 | Installer: the shell function is defined at top level, quotes every expansion, prints no command on sssd/winbind/nis | `scripts/install.sh::authselect_activation_hint` | `pam_success_done_contract.rs` (PSK9) |

### Pre-existing violations found (not introduced by this change)
- `get_gdm_status` checks the hard-coded `/etc/soos/disabled` in addition to the flag directory
  of `disable_file`: kept (production flag directory is `/etc/soos`).

### Clearance: CLEARED
