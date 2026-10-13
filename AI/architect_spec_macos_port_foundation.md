# Architect Spec — GitHub #367 (first PR of #360): macOS Port Foundation

- **Date**: 2026-10-13
- **Round**: 3 (round 1 findings: §10; round 2 findings R2-1/R2-3: §4.1, R2-2: §11)
- **Branch**: `chore/macos-port-foundation` (from `origin/main` 1b6d211). GitHub-only issue (#360 / #367 have no
  `AI/BACKLOG.md` id): **not** registered in `scripts/sync_issue.py` `BRANCH_TO_ISSUE`; the commit message carries
  `Closes #367` (dev-workflow Phase 0 rule).
- **Binding inputs**: GitHub #360 (body, owner decisions of 2026-10-12 and 2026-10-13), GitHub #367 (scope = planned
  split items 1–3 in one PR), the orchestrator's owner authorisation of 2026-10-13 (decide sensibly, record decisions
  here and in the ADR, do not stop for questions).
- **Agents never** create the `soos-macos` repository, write macOS code, sign, notarize, install anything, commit or push.
- **Code read**: `Cargo.toml`, `deny.toml`, `crates/camera-v4l/{Cargo.toml,src/{lib,frame,sensor}.rs}`,
  `crates/vision/{Cargo.toml,src/{color,error,pipeline,ir_liveness}.rs}`, every `crates/vision/tests/*.rs` import,
  `tests/fixtures/{Cargo.toml,mod.rs,tests/fixture_generators_tests.rs}`, `crates/{protocol,policy,push-protocol,
  inference-ort,biometric-store,evidence-store}/Cargo.toml`, `crates/biometric-store/src/{crypto,store}.rs` (system
  calls), `tests/invariants/src/{lib,lexing_contract,arch_faillock_ci_contract,distro_matrix,remote_push_contract}.rs`,
  `scripts/{candid_review.sh,prefetch_onnxruntime.sh,pr_loop.sh}`, `.github/workflows/ci.yml`.
- **Test prefixes**: `test_mac_` (behaviour and static invariants, new file
  `tests/invariants/src/portable_crates_contract.rs`), `test_frame_` (new crate `soos-frame`), plus the issue-mandated
  name `test_portable_crates_have_no_linux_only_dependencies`. Matrix rows `MAC1`–`MAC9` (prefix `MAC` is unused).

---

## 0. Decisions

| # | Decision | Why |
|---|---|---|
| D-1 | The macOS port lives in a separate **private** repository `soos-security/soos-macos`; this repository stays Linux-only and every immutable decision of `AGENTS.md` is unchanged | owner decisions 2026-10-12/13; #360 design direction 1 |
| D-2 | Shared crates are consumed through **git dependencies pinned to an immutable annotated tag** `shared-v<MAJOR>.<MINOR>.<PATCH>` of `soos-security/soos` (never a branch; the consumer's `Cargo.lock` records the resolved `rev`). crates.io publication is **deferred** (own ADR; `publish = false` stays) | #360 item 4; owner: crates.io still open |
| D-3 | Portable crate set (P) = `soos-frame`, `soos-protocol`, `soos-policy`, `soos-push-protocol`, `soos-inference-ort`, `soos-vision`, `soos-biometric-store`. Dev-only support crate checked with P: `soos-test-fixtures` (it must stay portable so P's tests build on macOS) | §4 evidence; #360 table |
| D-4 | `soos-biometric-store` is classified **portable (by inspection, compile-verified on macOS by the non-required job)**: it uses only POSIX calls available on macOS through `nix`/`libc` (`geteuid`, `flock` via `nix::fcntl::Flock`, `fstatat`, `unlinkat`, `O_NOFOLLOW`, `O_DIRECTORY`, `O_NONBLOCK`, `S_IFMT`/`S_IFREG`, `std::os::unix` metadata/permissions). Its **tests are not run on macOS yet**: store semantics that differ on macOS (BSD group inheritance of new files, `/var` → `/private/var` temp-dir symlink, `flock` on directories) are unverified without a Mac. The job runs `clippy --all-targets` on it (compiles lib + tests) | #360 AC "classified with evidence"; no Mac available (owner) |
| D-5 | `soos-evidence-store` is **not** in P (not needed by the phase 1 sudo target; same system-call profile as biometric-store; classify when the port needs it) | scope |
| D-6 | New pure crate `crates/frame` (`soos-frame`) owns `Frame`, `PixelFormat`, `SensorType`, moved **byte-for-byte** (same derives, same methods, same `Zeroize` impl, same `Drop` that wipes `data`). No I/O, `#![forbid(unsafe_code)]`, single normal dependency `zeroize` | #360 design 2 |
| D-7 | `soos-camera-v4l` keeps every public path: `crates/camera-v4l/src/frame.rs` becomes `pub use soos_frame::{Frame, PixelFormat};`, `crates/camera-v4l/src/sensor.rs` keeps everything except the `SensorType` definition, replaced by `pub use soos_frame::SensorType;`. `soos_camera_v4l::{Frame, PixelFormat, SensorType}`, `soos_camera_v4l::frame::{Frame, PixelFormat}`, `soos_camera_v4l::sensor::SensorType` and `crate::frame::*` inside the crate stay valid. They are the **same types** (re-exports, not copies) | source compatibility, one source of truth |
| D-8 | `soos-vision` `[dependencies]`: `soos-frame` replaces `soos-camera-v4l`; `soos-camera-v4l` is removed from vision entirely (also not a dev-dependency) so vision's tests build on macOS. `soos-test-fixtures` depends on `soos-frame` instead of `soos-camera-v4l` | #360 AC; macOS tests |
| D-9 | IR-specific types stay **optional**: `soos-frame` carries only the sensor *tag* `SensorType` (`Rgb`, `Infrared`, `Unknown`), which the RGB-only pipeline also needs (`Unknown`/`Rgb` frames). It contains no IR processing type; IR capture (`deep_grey`, sensor classification) stays in `soos-camera-v4l` and IR liveness (`ir_liveness.rs`) in `soos-vision`. Rule recorded in the ADR: any IR-only type added to `soos-frame` later (IR anti-spoofing, #353) goes behind a **non-default** cargo feature `ir`, so an RGB-only consumer (macOS) never compiles it. No empty feature is declared now | owner: keep IR optional; YAGNI |
| D-10 | Portability guard = static invariant over `cargo tree --locked -e normal,build --target all --all-features --prefix none --format {p}` (the resolved graph that `cargo metadata` exposes, already used by `lexing_contract`; the invariants crate stays dependency-free, so no JSON parser). `--target all` makes the guard strict: a forbidden crate reached on **any** platform fails | #360 item 3; precedent |
| D-11 | macOS CI job `portable-macos` ("Portable crates (macOS, non-required)"), `runs-on: macos-latest`, **not** in `ci-success.needs` and not a required branch-protection check; same SHA-pinned actions as the Linux jobs, no secrets, workflow-default `contents: read`. It runs `cargo clippy` (P, `--all-targets --all-features -D warnings`), then `cargo test` for P minus biometric-store (D-4) | owner: non-required job; #360 asked `macos-14`, the orchestrator asked `macos-latest` (arm64 today); chosen `macos-latest` and recorded |
| D-12 | macOS soos positioning (recorded in ADR and `Docs/MACOS_PORT.md`): **RGB-only** (no IR anti-spoofing), a **convenience factor with password fallback, never a password replacement**; first target `sudo` through `/etc/pam.d/sudo_local` (OpenPAM `pam_soos.so`, `PAM_IGNORE` on every failure); login/lock-screen authorization plugin = later, experimental, opt-in; **ad-hoc signing** for the owner's own Mac, no Apple Developer Program, no notarization until distribution is decided; maintainer identity `soos-security` | owner decisions |
| D-13 | Deferred (recorded, not decided): crates.io publication; soos-remote Mac support; reference Mac model/macOS version for physical rows; CoreML execution provider (an ADR of `soos-macos`); README link (only once the repository exists) | owner: still open |

---

## 1. Scope & Blast Radius

| Area | Change |
|---|---|
| `Cargo.toml` | member `crates/frame`; `[workspace.dependencies] soos-frame = { path = "crates/frame", version = "0.1.0" }` |
| `crates/frame/` (new) | `Cargo.toml`, `src/lib.rs`, `src/frame.rs`, `src/sensor.rs`, `tests/frame_contract_tests.rs` |
| `crates/camera-v4l` | `Cargo.toml` + `soos-frame`; `src/frame.rs` → re-export; `src/sensor.rs` → `SensorType` re-export (rest unchanged); new test file `tests/frame_drop_wipe_tests.rs` (adapter crate: a test-only global allocator, documented `unsafe`) |
| `crates/vision` | `Cargo.toml` dep swap; four `use` lines in `src/{color,error,pipeline,ir_liveness}.rs`; 13 test files: import path only (AM-2) |
| `tests/fixtures` | `Cargo.toml` dep swap; two `use` lines in `mod.rs`; `tests/fixture_generators_tests.rs` import (AM-3) |
| `tests/invariants` | `lib.rs`: `"frame"` added to `test_business_crates_forbid_unsafe_code` (AM-1), `mod portable_crates_contract;` |
| `scripts/candid_review.sh` | `BUSINESS_CRATES` + `frame` (must match the invariant list) |
| `.github/workflows/ci.yml` | new job `portable-macos` + header line; `ci-success` untouched |
| `scripts/pr_loop.sh`, `scripts/pr_checks_gate.sh` (new) | advisory gate for the macOS job (§4.1, X-2, R2-1, R2-3) |
| Docs / AI | `Docs/README.md` index rows (M-3); ADR (`AI/DECISIONS.md`, appended last), `Docs/MACOS_PORT.md` (new), `Docs/FRAME_CRATE.md` (new), `Docs/VISION_CRATE.md`, `Docs/CAMERA_V4L_CRATE.md`, `Docs/CI_CD_AND_SECURITY.md`, `AGENTS.md` crate map, `AI/ARCHITECTURE.md` crate tree + forbid list, `.agents/skills/dev-workflow/references/project-facts.md` crate map, `AI/VERIFICATION_MATRIX.md` (section appended last), walkthrough `AI/walkthroughs/198_macos_port_foundation.md` |

Downstream consumers of the moved types (`grep -rn "soos_camera_v4l::{.*Frame\|PixelFormat\|SensorType"`):
`soos-daemon`, `soos-gui`, `soos-enrollment-cli`, `soos-admin-cli`, camera-v4l's own tests — all keep compiling through the
re-exports (D-7). `deny.toml`: no change (no new external crate; `soos-frame` is private, `publish.workspace = true`).

---

## 2. Types & Signatures (`soos-frame`, moved verbatim)

```rust
// crates/frame/src/lib.rs
//! `soos-frame` — pure, I/O-free camera frame types shared by capture backends and vision.
#![forbid(unsafe_code)]
mod frame;
mod sensor;
pub use frame::{Frame, PixelFormat};
pub use sensor::SensorType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat { Yuyv, Rgb24, Grey, Mjpeg, Nv12 }
impl PixelFormat {
    pub const fn fourcc_str(self) -> &'static str;                       // "YUYV" "RGB3" "GREY" "MJPG" "NV12"
    pub const fn expected_buffer_size(self, width: u32, height: u32) -> Option<usize>; // saturating; Mjpeg = None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub data: Vec<u8>, pub width: u32, pub height: u32, pub timestamp_mono_ns: u64,
    pub format: PixelFormat, pub sequence: u64, pub sensor_type: SensorType,
}
impl Frame {
    pub fn new(data: Vec<u8>, width: u32, height: u32, timestamp_mono_ns: u64,
               format: PixelFormat, sequence: u64) -> Self;               // sensor_type = Unknown
    #[must_use] pub fn with_sensor_type(self, sensor_type: SensorType) -> Self;
    pub fn age_ms(&self, now_mono_ns: u64) -> u64;                        // saturating_sub, /1_000_000
    pub fn is_stale(&self, now_mono_ns: u64, max_age_ms: u64) -> bool;   // age_ms > max_age_ms
}
impl zeroize::Zeroize for Frame { fn zeroize(&mut self) { self.data.zeroize(); } }
impl Drop for Frame { fn drop(&mut self) { self.data.zeroize(); } }     // wipes the whole capacity

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SensorType { Rgb, Infrared, Unknown }
```

Bounds (unchanged): `expected_buffer_size` saturates (`usize::MAX` never overflows); `age_ms` saturates to 0 for a
future timestamp; `is_stale` is strict `>`. `width`/`height` are not validated here (decoders in `soos-vision` validate
against the buffer, unchanged). The derived `Debug` of `Frame` prints the buffer exactly as today — **no behaviour
change** in this PR; see §8 drift item 1.

`crates/frame/Cargo.toml`: `name = "soos-frame"`, `description`, `version/edition/license/repository.workspace`,
`publish.workspace = true`, `[dependencies] zeroize = { workspace = true }`, `[lints] workspace = true`. No
`[features]` section (D-9).

---

## 3. Constants & Config (invariants module `portable_crates_contract.rs`)

```rust
/// Crates the soos-macos repository may consume (ADR 2026-10-13). Order irrelevant.
const PORTABLE_CRATES: [&str; 7] = ["soos-frame", "soos-protocol", "soos-policy", "soos-push-protocol",
    "soos-inference-ort", "soos-vision", "soos-biometric-store"];
/// Dev-only crate the portable crates' tests use; must be just as portable.
const PORTABLE_DEV_SUPPORT: [&str; 1] = ["soos-test-fixtures"];
/// Crates whose tests the macOS job runs (P minus soos-biometric-store, D-4).
const MACOS_TESTED_CRATES: [&str; 6] = [/* P without soos-biometric-store */];
/// Package names no portable crate may reach (normal + build edges, any target, all features).
const FORBIDDEN_PORTABLE_DEPENDENCIES: [&str; N] = [ ...§3.1 ];
```

### 3.1 Forbidden set (exact package names)

| Group | Packages | Reason |
|---|---|---|
| V4L2 | `v4l`, `v4l2-sys-mit`, `v4l-sys` | Linux kernel video API (ioctls, bindgen on `<linux/videodev2.h>`) |
| Linux-PAM ABI | `pam-bindings`, `pam-sys`, `pam` | Linux-PAM symbols; macOS uses OpenPAM with a different ABI |
| D-Bus / logind | `zbus`, `zbus_macros`, `zbus_names`, `zvariant` | systemd-logind client stack (daemon presence, soos-remote) |
| udev / systemd / inotify | `udev`, `libudev-sys`, `libsystemd`, `sd-notify`, `inotify`, `inotify-sys` | Linux-only userland (defensive: none is in `Cargo.lock` today) |
| Async runtime | `tokio`, `tokio-macros` | policy: shared crates stay runtime-free so a future OpenPAM module (Tokio-free, like `pam_soos.so`) can embed them |
| Linux desktop session | `wayland-client`, `wayland-sys`, `x11rb`, `x11-dl` | GUI windowing stacks of `soos-gui` |
| Linux-only / application workspace crates | `soos-camera-v4l`, `soos-pam`, `soos-daemon`, `soos-remote`, `soos-push-sender`, `soos-gui`, `soos-admin-cli`, `soos-enrollment-cli`, `soos-test-fixtures` (never a normal dependency) | shared crates never depend upward on adapters or binaries |

**Not forbidden, and why**: `nix` and `libc` (portable POSIX bindings; whether the *used* items exist on macOS is
proven by compiling on macOS, D-4/D-11); `linux-raw-sys` (reached only through `rustix` behind
`cfg(target_os = "linux")` inside `ort-sys`'s build tooling and `tempfile`; a cfg-gated backend, not a Linux-only
dependency of the crate); `windows-*`, `core-foundation`, `security-framework` (platform backends of `native-tls`/`ort`
selected per target). A guard hit on `soos-test-fixtures` for a portable crate means a normal (non-dev) edge, which is
itself a violation.

### 3.2 Query (single helper, fail-closed)

`fn dependency_packages(manifest_dir: &Path, package: &str, locked: bool) -> Vec<String>` runs
`$CARGO tree [--locked | --offline] -p <package> -e normal,build --target all --all-features --prefix none --format {p}`
in `manifest_dir`; asserts exit status 0 (stderr in the message); returns sorted, deduplicated first tokens of each
line **with the queried root package itself removed** (X-1: `cargo tree` prints the root as its first line, so
`soos-test-fixtures` would otherwise always hit its own forbidden entry). `fn forbidden_hits(packages: &[String]) ->
Vec<String>` = the intersection with §3.1 (pure). Unit test `test_mac_forbidden_hits_is_exact` proves: a graph
`[v4l, serde]` reports `v4l`; a graph without forbidden names reports nothing; near-miss names (`v4l-utils-free`,
`tokio-util-free`) are not hits; `strip_root` removes only the exact root name (a graph `[soos-test-fixtures]` queried
for `soos-test-fixtures` reports nothing, while the same graph queried for `soos-vision` still reports it).

---

## 4. CI Job (`.github/workflows/ci.yml`, inserted before the `ci-success` banner)

```yaml
  # ---------------------------------------------------------------------------
  # Job 10: Portable crates on macOS (GitHub #367, ADR 2026-10-13 "macOS Port in
  # a Separate Repository"). NOT a required check and NOT a CI Success
  # dependency: ... no secrets, workflow-default read-only token.
  # ---------------------------------------------------------------------------
  portable-macos:
    name: "Portable crates (macOS, non-required)"
    if: github.event_name != 'schedule'
    runs-on: macos-latest
    timeout-minutes: 45
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1   (persist-credentials: false)
      - Install Rust toolchain (rust-toolchain.toml)   rustup toolchain install --profile minimal
      - uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2 (save-if main)
      - ORT cache location / restore / prefetch (./scripts/prefetch_onnxruntime.sh) / save (main only)  — same pins as Linux
      - cargo clippy --locked -p <each of P> --all-targets --all-features -- -D warnings
      - cargo test --locked -p <each of MACOS_TESTED_CRATES> --all-targets --all-features --no-run
      - cargo test --locked -p <each of MACOS_TESTED_CRATES> --all-targets --all-features
```

Never `--workspace`, never `-p soos-camera-v4l` (or any forbidden crate). `ci-success.needs` is unchanged. Header
comment gains a line `10. portable-macos — ... (not part of CI Success)`. The `cargo check` of the #360 AC is covered by
the clippy step (clippy performs the full `check` of every selected target; recorded in §8). The job must stay on an
**arm64** image: `ort-sys` 2.0.0-rc.13 ships a prebuilt ONNX Runtime for `aarch64-apple-darwin` only (M-6); an x86 image
such as `macos-13` would fail in `prefetch_onnxruntime.sh`.

### 4.1 Advisory check in `scripts/pr_loop.sh` (X-2, revised in Round 3 for R2-1 and R2-3)

`main` has no branch protection; the only merge gate is `scripts/pr_loop.sh`, whose `gh pr checks --watch --fail-fast`
covers every check. `CI Success` alone is **not** sufficient: checks of other workflows (for example "PR Title
(Conventional Commits)" of `pr-title.yml`) are outside it. To keep the macOS job truly non-required without ever merging
while a blocking check is unfinished, the decision is extracted into a new helper `scripts/pr_checks_gate.sh <pr-number>`:

```bash
# Exactly one advisory (non-required) check, ADR 2026-10-13 "macOS Port in a Separate Repository".
readonly ADVISORY_CHECKS=("Portable crates (macOS, non-required)")
```

- It reads `gh pr checks "$PR" --json name,bucket --jq '.[] | [.name, .bucket] | @tsv'` (verified: `gh pr checks
  --json` exits 0 even when checks failed, gh 2.102.0; the plain form exits 1). A non-zero `gh` exit blocks.
- Every line must be exactly `name<TAB>bucket` with a non-empty name and a bucket in `pass|fail|pending|skipping|cancel`;
  anything else (malformed line, unknown bucket, extra field) blocks.
- Advisory checks (exact name match) are only reported on stderr as a warning, whatever their bucket.
- Non-advisory checks: zero of them blocks (empty list); any `fail`/`cancel` blocks; else any `pending` → pending; else
  (all `pass`/`skipping`) → proceed.
- Exit codes: `0` proceed, `1` block, `3` pending; a missing or non-numeric argument blocks (`1`).

`pr_loop.sh`: when the fail-fast watch exits non-zero (failure, cancellation or watch timeout), it loops on the helper:
`0` → `warn` and continue to the unchanged `CI Success` poll (still the final authoritative step); `3` → sleep 15 s and
retry while `$(date +%s) - WATCH_START <= CI_WATCH_TIMEOUT_SECS`, else `exit 1`; any other code → `exit 1` (today's
behaviour). When the watch succeeds, nothing changes.

Pinned by:
- `test_mac_pr_loop_treats_only_the_macos_job_as_advisory` (textual): the allowlist has exactly one entry equal to the
  job's `name:` in `ci.yml`; `pr_loop.sh` keeps `--watch --fail-fast`, calls the helper only after the watch and before
  the `CI Success` poll, keeps `refusing to merge`; the job is absent from `ci-success.needs`.
- `test_mac_pr_checks_gate_decisions` (behavioural, `gh` shim on `PATH` printing a fixture, precedent `run_prefetch`):
  advisory fail + others pass/skipping → 0; advisory fail + another pending → 3; non-advisory fail → 1; non-advisory
  cancel → 1; empty list → 1; only the advisory check → 1; malformed line → 1; unknown bucket → 1; shim exit 1 → 1;
  advisory pass + non-advisory fail → 1; all pass → 0; a name that merely contains the advisory name is not advisory;
  missing argument → 1; the shim receives exactly `pr checks <n> --json name,bucket --jq ...`.

## 5. Error Taxonomy

No runtime error type changes. Fail-closed rule restated for the port in the ADR: on macOS an OpenPAM module returns
`PAM_IGNORE` on every error/panic/timeout, an authorization mechanism returns deny/defer to the password prompt; never
success on error.

## 6. Latency Budget

Not touched (no auth-path code change; types moved verbatim; `soos-vision` keeps `opt-level = 3` in dev; `soos-frame`
only holds trivial methods — the hot loops stay in vision).

## 7. Invariants Touched

ARCHITECTURE §10 memory hygiene (`Frame` wipe on drop, preserved); business-crate `forbid(unsafe_code)` list;
supply chain (workflow pins); "no OpenCV / v4l only for capture" unchanged; matrix rows new `MAC1`–`MAC9`.

## 8. Documentation Drift / ADR

1. #360 "Risks" says the extraction must preserve "`Debug` redaction" of `Frame`; the code has **no** redaction
   (`#[derive(Debug)]` prints the buffer). This PR preserves behaviour exactly (owner: no behaviour change); flagged as a
   follow-up observation in the walkthrough (no current log site prints a `Frame` with `{:?}` — verify with
   `grep -rn '{:?}' ` in Phase 3).
2. #360 asked for a required job on `macos-14`; owner decision of 2026-10-13 makes it non-required, the orchestrator
   asked for `macos-latest`. Recorded in the ADR.
3. #360 asks for an `AI/BACKLOG.md` entry; #360/#367 are GitHub-only issues (like #339–#354), so traceability lives in
   the matrix section, the ADR and the walkthrough; no backlog id is created and the branch is not registered.
4. `AI/ARCHITECTURE.md` lists 10 business crates in prose while the invariant has 12 (+ `frame` = 13): fixed in prose.
5. Workspace `repository` points at `Mysticaly622/soos` while issues live at `soos-security/soos`; the ADR uses
   `soos-security/soos` for the tag scheme and records the field as drift (not changed here).

ADR text (appended as the last bullet of `AI/DECISIONS.md` "Active Decisions"): **"[2026-10-13] macOS Port in a
Separate Repository (GitHub #360 / #367)"** with: separate private repo, this repo Linux-only and immutable decisions
unchanged, portable set P + guard + forbidden set, `shared-v` tag pinning, macOS-specific architecture decided by ADRs in
`soos-macos` (XPC, capture agent, CoreML), fail-closed rule verbatim, RGB-only convenience factor, sudo_local first,
ad-hoc signing, non-required macOS job, IR-optional rule (D-9), deferred items (D-13), AGPL propagation.

---

## 9. Test Hooks for Tester (acceptance mapping)

| # | Acceptance (#360 / #367) | Test |
|---|---|---|
| A1 | ADR added with portable set, tag rule, immutable decisions unchanged, macOS architecture in soos-macos | `portable_crates_contract::test_mac_adr_records_the_separate_repository_decision` |
| A2 | `soos-frame` pure, forbid unsafe, no I/O, holds the 3 types | `test_mac_frame_crate_is_pure_and_forbids_unsafe`, `soos-frame::frame_contract_tests::*` |
| A3 | camera-v4l re-exports; vision no longer depends on camera-v4l | `test_mac_camera_v4l_reexports_the_frame_types`, `test_mac_vision_and_fixtures_do_not_depend_on_camera_v4l`, `soos-camera-v4l::frame_drop_wipe_tests::test_camera_frame_types_are_the_soos_frame_types` |
| A4 | `Frame` wiped on drop (test proves it) | `soos-camera-v4l::frame_drop_wipe_tests::{test_frame_buffer_wiped_on_drop, test_drop_probe_detects_an_unwiped_buffer}` (byte-level evidence), `soos-frame::frame_contract_tests::test_frame_zeroize_empties_data_and_keeps_capacity` |
| A5 | `frame` in business-crate list | `test_business_crates_forbid_unsafe_code` (AM-1), `test_mac_business_lists_include_frame` |
| A6 | Existing tests unchanged except import paths | AM-2, AM-3 (assertions untouched; `git diff` shows `use` lines only) |
| A7 | `test_portable_crates_have_no_linux_only_dependencies` via the resolved graph; a broken manifest proves it can fail | `test_portable_crates_have_no_linux_only_dependencies`, `test_mac_guard_detects_a_forbidden_crate_in_a_broken_manifest`, `test_mac_guard_query_sees_linux_only_crates`, `test_mac_forbidden_hits_is_exact` |
| A8 | biometric-store classified with evidence | D-4; `test_mac_macos_port_doc_covers_the_bootstrap_spec` (doc lists the calls) |
| A9 | macOS CI job, non-required, pinned, no secrets, portable set only | `test_mac_macos_job_is_pinned_secret_free_and_non_required`, `test_mac_pr_loop_treats_only_the_macos_job_as_advisory`, `test_mac_pr_checks_gate_decisions` |
| A10 | `Docs/MACOS_PORT.md` content + owner checklist | `test_mac_macos_port_doc_covers_the_bootstrap_spec` |
| A11 | Matrix rows, walkthrough 198 | Phase 6 (`matrix_citations` invariant) |

Red expectations: `soos-frame` tests and `frame_drop_wipe_tests` fail to compile until the crate exists (missing item =
exactly the specified API); every `portable_crates_contract` test fails on an assertion (manifest still points at
camera-v4l, job/ADR/doc absent, business list lacks `frame`); the broken-manifest test and `forbidden_hits` unit test
are expected **green** at Red (they test the guard itself) — that is their purpose.

---

## 10. Round 2 — Resolution of Plan Evaluation Round 1

| Finding | Resolution |
|---|---|
| X-1 (MAJOR) guard always red for `soos-test-fixtures` | §3.2: the root package is removed from the queried graph before intersecting; `test_mac_forbidden_hits_is_exact` proves self-exclusion never hides a genuine hit |
| X-2 (MAJOR) non-required job gates every autonomous merge | §4.1: `scripts/pr_loop.sh` is in scope with a one-entry advisory allowlist; every other check stays fail-closed, `CI Success` stays authoritative; pinned by `test_mac_pr_loop_treats_only_the_macos_job_as_advisory`; recorded in the ADR |
| M-3 `Docs/README.md` | added to §1 (index rows for `FRAME_CRATE.md` and `MACOS_PORT.md`); trees use the exact `── frame/` form; `crates/frame/Cargo.toml` has the exact line `license.workspace = true` |
| M-4 drop-wipe proof | §11 specifies the probe; the soos-frame zeroize test is renamed `test_frame_zeroize_empties_data_and_keeps_capacity` and asserts only what it can (`data.is_empty()`, capacity kept); the vacuous post-`zeroize` byte loop of the existing `frame_zeroize_tests::test_frame_zeroize_trait` (row H1) is recorded as drift in the walkthrough, test unchanged |
| M-5 guard non-vacuity | broken-manifest test: a temporary crate under `std::env::temp_dir()` (outside the repository), its own empty `[workspace]` table, a single path dependency on a local stub package named `v4l`, `cargo tree --offline`, same `dependency_packages` + `forbidden_hits` path; witness test requires one hit per group present in `Cargo.lock`: `v4l`+`v4l2-sys-mit` (`soos-camera-v4l`), `pam-bindings` (`soos-pam`), `zbus`+`zvariant`+`tokio` (`soos-daemon`), `wayland-client`+`x11rb` (`soos-gui`); the constant's doc comment marks `v4l-sys`, `pam-sys`, `pam`, `udev`, `libudev-sys`, `libsystemd`, `sd-notify`, `inotify`, `inotify-sys` as defensive (absent from `Cargo.lock`) |
| M-6 `ort` macOS fact | `Docs/MACOS_PORT.md` and the ADR state: `ort-sys` rc.13 prebuilt = `aarch64-apple-darwin` only, the `coreml` build (CoreML EP compiled in, never registered by `soos-inference-ort`, CPU EP unchanged); Intel Macs need a source build; the job stays on an arm64 image |
| M-7 AC deviations | §8 gains items 6–8 below; matrix row for macOS store tests is `⬜ Pending`; remediation rule below |
| M-8 walkthrough number | 198: numbers 196 and 197 are reserved by the in-flight `feat/network-lock` worktree and a second parallel branch (orchestrator instruction), so the next free unique number is used; re-checked for uniqueness in Phase 6 |
| M-9 AM-2 enumeration | §12 lists every amended file; the candid reviewer confirms with `git diff -U0` that they only swap `use soos_camera_v4l::` for `use soos_frame::` |

### 8 (continued) — further drift items

6. The #360 AC lists `cargo check`; the job's `cargo clippy` step performs the same full check of every selected target
   (lib, tests, benches) and is the recorded equivalent.
7. The #360 AC asks `cargo test` for the whole portable set; `soos-biometric-store` tests are compiled (clippy
   `--all-targets`) but not run on macOS (D-4). Matrix row `MAC8` is `⬜ Pending (store tests not yet run on macOS)`.
8. **Remediation rule for a red macOS run**: fix portable production code; if `soos-biometric-store` does not compile on
   macOS, apply the AC's alternative branch (classify it Linux-only, list the blocking calls in `Docs/MACOS_PORT.md`,
   drop it from `PORTABLE_CRATES` in the same change) instead of adding untested `cfg` code. Never edit an existing test.

## 11. Drop-Wipe Probe (`crates/camera-v4l/tests/frame_drop_wipe_tests.rs`)

- A test-only `#[global_allocator]` wrapping `std::alloc::System`; every `unsafe` block/impl carries a `// SAFETY:`
  comment (camera-v4l is an adapter crate; business crates stay unsafe-free).
- Keyed on the **recorded buffer address**: a test stores the address and capacity of the watched buffer in atomics
  (`AtomicUsize` `WATCH_PTR`, `WATCH_LEN`), then drops the owner. In `dealloc`, when `ptr == WATCH_PTR`, the probe reads
  `layout.size()` bytes **before** forwarding to `System.dealloc`, stores `SEEN = true` and `ALL_ZERO = <bool>`, and
  clears `WATCH_PTR`. Atomics only: no allocation, no lock, no I/O inside `alloc`/`dealloc`.
- The tests serialise themselves through a `static Mutex<()>` taken **outside** the allocator hooks (other threads'
  deallocations never match the watched address).
- `test_frame_buffer_wiped_on_drop`: `Vec::with_capacity(4096)` filled with 3000 bytes of `0xA5` (capacity > len),
  `Frame::new(...)`, watch the buffer, `drop(frame)`; assert `SEEN` (a leak or `mem::forget` fails) and `ALL_ZERO` over
  the whole 4096-byte capacity.
- `test_drop_probe_detects_an_unwiped_buffer` (non-vacuity twin): a plain `vec![0xA5; 4096]` with `len == capacity`
  (R2-2: the probe never reads uninitialised bytes of an unwiped buffer); assert `SEEN` and `!ALL_ZERO`.
- `test_camera_frame_types_are_the_soos_frame_types`: a value of `soos_camera_v4l::Frame` is passed to a function taking
  `soos_frame::Frame` (likewise `PixelFormat` and `SensorType`, and `soos_camera_v4l::frame::Frame`,
  `soos_camera_v4l::sensor::SensorType`), compile-time proof of the re-exports.

## 12. Contract Amendments (import paths only, assertions untouched)

| ID | File | Change | Mandate |
|---|---|---|---|
| AM-1 | `tests/invariants/src/lib.rs::test_business_crates_forbid_unsafe_code` | `"frame"` appended to `business_crates` (strengthens the test) | #360 AC "soos-frame is added to the business-crate list" |
| AM-2 | `crates/vision/tests/{ir_sensor_policy_tests,pipeline_tests,color_tests,bench_tests,frame_robustness_tests,ir_pad_policy_tests,face_quality_gate_tests,gcv_review_tests,zeroize_tests,pad_crop_real_model_tests,embedding_lfw_evaluation_tests,mjpeg_bounds_tests,pad_ensemble_tests}.rs` | one `use soos_camera_v4l::…` line → `use soos_frame::…` (same item list) | #360 AC "soos-vision no longer depends on soos-camera-v4l" + "import-path updates" exception; D-8 |
| AM-3 | `tests/fixtures/tests/fixture_generators_tests.rs` | `use soos_camera_v4l::PixelFormat;` → `use soos_frame::PixelFormat;` | D-8 (fixtures portable) |

`tests/fixtures/mod.rs` (fixture library, not a test) swaps its two `use` lines the same way.

## 13. Round 3 Approval Constraints (R3-1 to R3-4, binding for tester, auditor and developer)

- **R3-1**: the gate reads every record with `while IFS= read -r line || [[ -n "$line" ]]` and splits each line itself
  on the single tab (never the `IFS=$'\t'` field split); one `gh` call fetches
  `--jq '(length|tostring), (.[] | [.name, .bucket] | @tsv)'` and the gate blocks unless the first line is a decimal
  count equal to the number of parsed records. Tests: `last_pending_without_newline` → 3, `last_fail_without_newline`
  → 1, `count_mismatch` → 1, `count_not_a_number` → 1, `no_output` → 1.
- **R3-2**: advisory matching is `[[ "$name" == "$advisory" ]]` (quoted literal); the advisory name is a job `name:`
  exactly once across `.github/workflows/*.yml`; tests `trailing_space_name` and `regex_like_name` → 1.
- **R3-3**: `pr_loop.sh` captures the status as `gate_rc=0; "$GATE_HELPER" "$PR_NUMBER" || gate_rc=$?` then
  `case "$gate_rc"` (0 / 3 / *).
- **R3-4** (adopted): after `CI Success` is `success`, `pr_loop.sh` runs the gate once more and merges only on `0`
  (`3` or anything else → `exit 1`).
