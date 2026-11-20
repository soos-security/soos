# Architect Spec — GitHub #487 (security review 2026-11-10, batch 10): `success=done` and the PAM stack

- **Branch**: `fix/pam-success-done` (GitHub-only review findings, not registered in `BRANCH_TO_ISSUE`)
- **Findings**: #574 `PSTK-01` (MEDIUM), #575 `PSTK-02` (LOW), #581 `PSTKC-01` (LOW)
- **Base**: `origin/main` `0cce947`

## Scope & Blast Radius

| Finding | Crate / file | Change |
|---|---|---|
| PSTK-01 | `crates/admin-cli/src/pam_check.rs` (new, `pub mod pam_check`) | Bounded flattener + two-scenario simulator of an `auth` chain; `check_pam_services` (read-only report) and `gdm_tail_skips` (used by `gdm enable`). |
| PSTK-01 | `crates/admin-cli/src/gdm.rs` `plan_gdm_enable` | Refuse the `Insert` plan when the managed `success=done` rule would skip an enforcing auth rule that the password path still runs after the anchor (edited file tail and tail delegations). |
| PSTK-01 | `crates/admin-cli/src/args.rs`, `main.rs` | New read-only subcommand `soos-admin pam check [--pam-dir DIR] [SERVICE...]`. |
| PSTKC-01 | `gdm.rs` (`get_gdm_status`, `configure_gdm_with_options`), `args.rs`, `main.rs` | Disable flag derived from what `pam_soos.so` reads; status computed with the module rule. |
| PSTK-02 | `scripts/install.sh`, `packaging/rpm/soos.spec`, `packaging/pam/fedora/soos/README` | Activation hint only for `local`/`minimal`; local-only warning otherwise. |
| all | `Docs/DISTRIBUTION_DEPLOYMENT.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `AI/DECISIONS.md`, matrix, walkthrough 264 | Documentation and traceability. |

Consumers: `soos_admin_cli::gdm::*` is used only by `crates/admin-cli` tests and `main.rs`. No
other crate depends on `soos-admin-cli`. `crates/pam` is not modified. `soos-pam` becomes a
**dev-dependency** of `soos-admin-cli` (path dependency) so a contract test can compare the CLI's
flag rule with `soos_pam::config::PamConfig::is_disabled` (one rule, two implementations, pinned
equal by a test).

Not changed (owner question, see "Deferred"): the Debian pam-config control (`success=done`
pinned by `test_debian_notify_profile_sits_between_pam_unix_and_pam_deny`) and `GDM_PAM_LINE`
(pinned by `test_gdm_pam_line_includes_timeout_ms_2500`).

## Types & Signatures

```rust
// crates/admin-cli/src/pam_check.rs
pub const MAX_PAM_CHECK_SERVICES: usize = 256;   // service files examined by one `pam check`
pub const MAX_PAM_CHECK_RULES: usize = 1024;     // flattened auth rules per service analysis
const DEBIAN_NON_AUTH_STACKS: &[&str] = &[       // `@include` targets skipped when missing
    "common-account", "common-password", "common-session", "common-session-noninteractive",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkippedRule { pub stack: String, pub line: usize, pub rule: String }

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServiceFinding { pub service: String, pub soos_stack: String, pub soos_line: usize,
                            pub skipped: Vec<SkippedRule> }

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnanalysedService { pub service: String, pub reason: String }

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PamCheckReport { pub pam_dir: PathBuf, pub services_checked: usize,
                            pub findings: Vec<ServiceFinding>,
                            pub unanalysed: Vec<UnanalysedService> }
impl PamCheckReport {
    pub fn exit_code(&self) -> i32;      // 0 clean, 1 >= 1 finding, 2 no finding but >= 1 unanalysed
    pub fn format_table(&self) -> String;
    pub fn to_json(&self) -> String;     // serde_json
}
/// `services` empty = every regular file of `pam_dir` (sorted, at most MAX_PAM_CHECK_SERVICES;
/// names must be `[A-Za-z0-9._-]`, not starting with `.`, not ending with `.soos-backup`).
pub fn check_pam_services(pam_dir: &Path, services: &[String]) -> Result<PamCheckReport, AdminCliError>;

/// `gdm enable`: first enforcing rule the managed block would skip, `None` when none.
pub(crate) fn gdm_tail_skips(dir: &Path, edited: &str, lines: &[&str], anchor_index: usize)
    -> Result<Option<SkippedRule>, String>;   // Err = refusal reason (unanalysable tail)
```

```rust
// crates/admin-cli/src/gdm.rs
pub const DEFAULT_FLAG_DIR: &str = "/etc/soos";            // equals soos_pam::config::DEFAULT_FLAG_DIR
pub fn pam_service_name(pam_file: &Path) -> Option<&str>;   // basename, flag-safe or None
pub fn module_flag_names(service: &str) -> Vec<String>;     // "disabled", "gdm.disable" (gdm*), "<service>.disable"
pub fn default_disable_file(flag_dir: &Path, service: &str) -> PathBuf; // gdm.disable for gdm*, else <service>.disable
pub fn module_disabled(flag_dir: &Path, service: Option<&str>) -> bool; // same rule as PamConfig::is_disabled
```

`GdmArgs::disable_file` becomes `Option<PathBuf>` (no clap default); `main.rs` resolves it to
`default_disable_file(DEFAULT_FLAG_DIR, service)`.

## Semantics

### Two-scenario simulation (PSTK-01)
Flattening: `include` and `@include` splice the target's auth rules inline; `substack` yields a
nested frame. Every other phase is skipped. Bounds: `MAX_PAM_INCLUDE_DEPTH` (4), a fresh
`MAX_PAM_STACK_READS` (32) budget per analysis, `MAX_PAM_CHECK_RULES`; line continuations, invalid
include names, unreadable/oversized/non-UTF-8 files, and missing targets (except the four Debian
non-auth stacks above, which are skipped when missing) make the analysis fail.

Simulation: every module returns `PAM_SUCCESS`, except primary soos rules (`pam_soos.so`, no
`event=`), which return `PAM_SUCCESS` in scenario A (face match) and `PAM_IGNORE` in scenario B
(the password path). Actions: keyword `required`/`requisite`/`optional` continue, `sufficient`
ends the frame on success; bracket controls use the value of the return key, else `default`, else
`bad`; `ignore`/`ok`/`bad`/`reset` continue, `done`/`die` end the frame (a substack frame only),
`N` skips N following nodes of the frame. Anything else (unknown keyword or value, a jump over a
substack, a jump past the end, an unanalysable delegated stack) marks the run *unknown*.

A **skipped rule** is an auth rule visited in B and not in A whose control can fail the stack
(`required`, `requisite`, any unknown keyword; a bracket without `default=`, or whose non-success
keys use `bad`, `die`, `ok`, `done`, `reset` or an unknown value) and whose module is not exempt
(credential modules — the factor face replaces —, `pam_soos.so`, `pam_permit.so`, `pam_env.so`,
`pam_faildelay.so`). `optional`, `sufficient`, `[success=... default=ignore]` and `-auth` with a
non-failing control never count; `-auth required pam_u2f.so` counts.

### `gdm enable` (PSTK-01)
Only the `Insert` plan is checked (the managed block is at the top level of the edited file, so
scenario A visits nothing after it). Scenario B runs from the anchor: the anchor delegation's
subtree is tagged `in_anchor` (its own ordering, ADR 2026-10-05) and becomes an *unknown* node if
it cannot be analysed; the tail after the anchor must be analysable or the enable is refused.
Refusal when a skipped rule outside the anchor subtree exists; when the run is unknown, refusal
when the tail holds any enforcing non-exempt rule (fail closed, reachability assumed).
Error: `AdminCliError::GdmConfig("... would skip the auth rule '<module>' (control '<control>')
in <stack> line <n>, which the password path still runs ...")`. Nothing is written; the disable
flag stays.

### Flags (PSTKC-01)
`service` = basename of `--pam-file` when flag-safe (`[A-Za-z0-9._-]`, not starting with `.`,
1..=64 bytes). The module reads `<flag_dir>/disabled`, `<flag_dir>/gdm.disable` when the service
contains `gdm`, and `<flag_dir>/<service>.disable`.
- Library: `flag_dir` = parent of `disable_file`. `get_gdm_status` reports `enabled = installed &&
  !module_disabled(flag_dir, service) && !/etc/soos/disabled` — a file not read by the module
  never disables. `Disable` refuses (nothing written) when the file name is not
  `gdm.disable`-for-a-gdm-service or `<service>.disable`, or when the service name is not flag-safe.
- CLI: an explicit `--disable-file` outside `/etc/soos` is refused for `disable`; for the other
  actions a warning is printed on stderr and `Status` is computed from `/etc/soos` (what the
  module reads).

### Activation hint (PSTK-02)
`authselect_activation_hint <recorded profile>` in `scripts/install.sh` (top-level function):
- id `local` or `minimal`: `info "Activate with: authselect select custom/soos with-faillock <features> && authselect check"` (features carried over, `with-faillock` once, no `--force`);
- id `custom/soos*`: already selected, nothing to activate;
- empty: no authselect-managed profile: hint with `--force` prefixed by "only on a host with local accounts only";
- anything else (`sssd`, `winbind`, `nis`, ...): `warn` that `custom/soos` is local-accounts-only and would remove the directory identity source and its features; no activation command printed.

## Error Taxonomy
Admin CLI only (no PAM path touched): `AdminCliError::GdmConfig` for refusals; `pam check`
never errors on a single service (reported as unanalysed), only on an unreadable `--pam-dir`.

## Latency Budget
Not on the authentication path (admin CLI and installer only). `crates/pam` unchanged.

## Invariants Touched
- ADR 2026-09-30 "GDM PAM Stack Placement" and 2026-10-05: gates before the anchor unchanged;
  new: no enforcing rule after the anchor is skipped by the managed rule.
- Fail closed: an unanalysable tail refuses; `pam check` reports unanalysed services (exit 2).
- Flag rule: one rule, pinned against `PamConfig::is_disabled` by a test.

## Test Hooks for Tester
All functions take directories (`pam_dir`, include dir, flag dir) so tests run in `tempfile`
directories; the shell function is extracted from `scripts/install.sh` with stub `info`/`warn`.

## Deferred / Owner Questions
- Debian pam-config `[success=end default=ignore]` (pam-auth-update rewrites `end` into a jump onto
  the success block): changes the asserted control of
  `test_debian_notify_profile_sits_between_pam_unix_and_pam_deny` (invariant) and the pinned
  `success=done` regexes of the Docker/distro harnesses — needs owner approval of a contract
  migration.
- `GDM_PAM_LINE` as a jump or `substack`: pinned by `test_gdm_pam_line_includes_timeout_ms_2500`;
  not needed once `gdm enable` refuses the skipping layouts.
- Docker matrix regression (stub second factor after `@include common-auth`): needs the Debian
  control change above to pass; covered meanwhile by `pam check` unit/integration tests.
- An sssd-derived authselect profile (`custom/soos-sssd`): L effort, out of this batch.

## Revision 1 (plan evaluation, MAJOR finding)
Conditional rules are forked instead of assumed to succeed: a non-credential, non-soos rule
whose success action differs from its failure action (bracket: value of `success` versus value
of `default`, missing = `bad`; keyword `sufficient`) is a branch point. Both scenarios run under
every assignment of the branch points (`MAX_PAM_CHECK_BRANCHES` = 8, i.e. at most 256
assignments); more branch points make the run unknown. Credential modules always return
`PAM_SUCCESS` in both scenarios (scenario B is the correct-password path). `required`,
`requisite` and `optional` rules are not branch points (assumed to succeed; an enforcing rule
visited only in B is reported whatever it returns).
