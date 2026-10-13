# Plan Evaluation Report (Round 2)
- **Date**: 2026-10-13
- **Issue**: GitHub-only #375: build against the system ONNX Runtime for distribution packages (PR 1 of umbrella #361; no `AI/BACKLOG.md` id)
- **Branch**: `feat/system-onnxruntime`
- **Base commit**: `6f32ef8` (`git rev-parse --short origin/main` = `HEAD` = `6f32ef8`)
- **Plan evaluated**: `AI/architect_spec_system_onnxruntime.md`. "§10 Revision 1" and "§11 Contract Migration" are normative and supersede §0–§9 where they differ. The draft tests and harness in the tree are being updated by the tester. I judged the plan, not how far the drafts lag behind it. No cargo build was run for this round: every fact below comes from source reading.

## 0. Round-1 Findings → Resolution

| Round-1 finding | Revision 1 resolution | Verified against | Status |
|---|---|---|---|
| **M-1** (MAJOR): `dependency_tooling_contract::test_prerelease_workspace_dependencies_are_exact_pinned` breaks, and no migration was planned | §11 lists the test, the old assertion, the new assertion (the `ort = ` line of `[workspace.dependencies]` contains `version = "=2.0.0-rc.`) and the acceptance line that mandates it (#375/#361 require `default-features = false`, which forces an inline table) | `tests/invariants/src/dependency_tooling_contract.rs:340–390`. The generic loop above the assertion already parses `version` inside inline tables, so the `=` pin is still enforced twice. No other invariant pins the literal `ort = "` (grep over `tests/invariants/src`). The intent and strength are kept. | **Resolved** |
| **M-2** (MAJOR): the startup guard could be a no-op without any test failing | §10: a private `ListedProviderProbe(&'static str)` that does not override `is_available`; `#[doc(hidden)] pub fn execution_provider_listed`; `verify` routes through `execution_provider_listed(CPU_EXECUTION_PROVIDER)`; the linked-library test SORT9 checks the real name, an unknown name and the lower-cased name; static SORT-S4 clauses (the `impl`, no `fn is_available`, no `ep::CPU`, the routing) | `ort-2.0.0-rc.13/src/ep/mod.rs:39–62`: the default `is_available` is `is_ep_available(self.name())`. `:268–291`: `GetAvailableProviders` with an exact, case-sensitive `name == avail`, and `Ok(false)` when the name is not listed. So SORT9's `Ok(false)` cases fail against a constant-true probe, and the static clauses block `ep::CPU::is_available` (`Ok(true)`). | **Resolved** |
| **M-3** (MINOR): stale base | Rebased on `6f32ef8`, 18 members including `crates/frame`; SORT-S2 reads `[workspace] members` | `Cargo.toml` `members` has 18 entries. `crates/frame/Cargo.toml` assertions in `portable_crates_contract::test_mac_frame_crate_is_pure_and_forbids_unsafe` are line-membership checks, so adding `rust-version.workspace = true` keeps them green. Only the five cited crates depend on `soos-inference-ort`/`soos-vision`/`soos-enrollment-cli`/`ort` (grep over every member manifest). | **Resolved** |
| **m-4** (MINOR): with both features, pkg-config prints link directives before the API-27 gate rejects the library (mixed link) | Workflow-level `env: LIBONNXRUNTIME_NO_PKG_CONFIG: "1"` in `ci.yml`. The harness passes an explicit environment and asserts the variable is unset in the container. ADR and `Docs/INFERENCE_ORT_CRATE.md` document the hazard; SORT-S5 pins the line. | `pkg-config-0.3.34/src/lib.rs:596–600`: `probe()` checks `format!("{}_NO_PKG_CONFIG", envify(name))` with the plain `env_var_os` (not target-prefixed) **before** running pkg-config or printing any metadata. `envify("libonnxruntime")` = `LIBONNXRUNTIME` (`:1065`). `ort-sys` `build/pkg_config.rs` then takes `Err(_)` → `false` → the download path, which is identical to today. **New-regression check**: no invariant pins the `ci.yml` workflow `env:` block. `portable_crates_contract.rs:611` only searches the header before `\njobs:` for `portable-macos`. `distro_matrix.rs:1576` parses `env:` inside one job body. `config_open_diagnostics_contract.rs:171` checks the job-level `ORT_SKIP_DOWNLOAD` in clippy, which is unaffected. No test greps `CARGO_INCREMENTAL`/`CARGO_PROFILE_*`. The macOS job assertions (`portable_crates_contract.rs:533–612`) look only inside the job body. | **Resolved** (see n-2 on the local gate) |
| **m-5** (MINOR): the deviation from #361 was overstated | §10 gives the real reasons: download code compiled in, API-27 gate excluding Fedora 43/44 and Debian, a silent download when `ORT_SKIP_DOWNLOAD` is unset | `ort-sys` `build/main.rs` order; `version.rs` | **Resolved** |
| **m-6** (MINOR): the §6 command fails, and the SORT3 wording was inconsistent | Harness: one plain `--all-targets` run, then an explicit `SOOS_REQUIRE_REAL_MODELS=1` list; SORT-S5 pins the list; SORT3 wording aligned | `grep -l SOOS_REQUIRE_REAL_MODELS crates/*/tests`: the ten listed targets are exactly every REQUIRE-aware target **except** `embedding_preprocessing_evaluation_tests` (retired ArcFace model, which prints `SKIPPED` without REQUIRE, `:87–92`), plus the two new suites. All files exist. | **Resolved** |
| **m-7** (MINOR): partial MSRV coverage | `cargo check --locked --offline --workspace --all-targets --all-features` under the MSRV in the offline phase; ADR note on `incompatible_msrv` | `cargo check` does not link. With `ORT_SKIP_DOWNLOAD=1`, a rejected or absent `.pc` ends in `link_error_generic`, which `check` tolerates, so the step is deterministic either way. The Ubuntu clippy job compiles the GUI without extra apt packages, so the Arch container needs none either. | **Resolved** |
| **m-8** (MINOR): a rolling release gates every PR | Accepted risk recorded in the ADR and `Docs/CI_CD_AND_SECURITY.md`: versions printed, one re-run, `fix/` PR or digest bump, never `continue-on-error`, never removed from CI Success without an ADR amendment | The rationale (the job exists to detect Arch ABI drift, which an Arch Linux Archive snapshot would hide) is coherent. The digest equals `tests/docker/Dockerfile.arch:9`. | **Resolved (accepted risk)** |
| **m-9** (MINOR): `soos-enroll`/`soos-gui` do not run the check | Both call `verify_cpu_execution_provider()?` before `ModelRegistry::new`; pinned by SORT-S4 | `crates/gui/src/main.rs:22,73`: `main` returns `Box<dyn Error>`, so `?` works, and the real-model branch is the only `ModelRegistry::new`. `crates/enrollment-cli/src/error.rs:72–73`: `Inference(#[from] InferenceError)` exists. `service.rs:1782–1835`: placement is feasible, but see n-1. | **Resolved, with n-1** |
| **m-10** (MINOR): nothing pins "CPU only" in source | SORT-S1 scan: no `with_execution_providers` in `crates/*/src`; `ort::ep` only in `runtime.rs` | Today `with_execution_providers` and `ort::ep` appear nowhere in `crates/*/src`. | **Resolved, with n-3** |

## 1. Coverage Matrix

| Acceptance line (#375 / in-scope #361) | Spec element | Status |
|---|---|---|
| Opt-in feature building `ort` without `download-binaries` | §3; SORT-S1, SORT-S3 | Covered |
| Distro `libonnxruntime.so` through pkg-config | §3 `ort/pkg-config`; §6 `readelf`/`ldd`/`pacman -Qo` | Covered |
| Lowest needed C API level | `api-17`; SORT-S3 forbids `api-18` | Covered |
| CPU EP availability checked at startup, fail closed | §2 + §10 M-2; SORT1–SORT5, SORT9, SORT-S4 | Covered, now with a test that can fail |
| PAM unaffected, `PAM_IGNORE` when the daemon is absent | §5, SORT-S6, `test_ipc_offline_daemon_returns_ignore` | Covered |
| Declared MSRV, not newer than `rust-toolchain.toml` | §4, SORT-S2 (`1.92` ≤ `1.98.1`) | Covered |
| CI builds at exactly the MSRV | §6 + §10 m-7 | Covered |
| Offline Arch job, `onnxruntime-cpu`, no network, no ORT cache | §6, SORT-S5 | Covered |
| `ORT_SKIP_DOWNLOAD=1`, `--locked --offline` after a separate fetch | §6 | Covered |
| Real-model tests pass against the system library | §10 m-6 list, SORT7 | Covered |
| Default dev/CI build unchanged | §3, SORT-S3 default graph, workflow `LIBONNXRUNTIME_NO_PKG_CONFIG` | Covered |
| `--all-features` keeps working | §3 + §10 m-4 | Covered |
| ADR with rejected alternatives and trust change | §9 + §10 m-5/m-7/m-8, SORT-S7 | Covered |
| "ort CPU only" / "no OpenCV" | §8, SORT-S1 + §10 m-10 | Covered |
| Existing tests not weakened | §11 Contract Migration (one test, justified) | Covered |

## 2. Facts Verified Against Code (round 2 additions)

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| `<NAME>_NO_PKG_CONFIG` honoured by pkg-config 0.3.34 | `pkg-config-0.3.34/src/lib.rs:596–600, 682–687, 1065` | Checked first in `probe()`, plain variable name, `LIBONNXRUNTIME` from `envify` | Yes |
| `ort-sys` falls back on any pkg-config `Err` | `ort-sys-2.0.0-rc.13/build/pkg_config.rs` `Err(_) => false` | As stated | Yes |
| The default `is_available` asks the library, exact compare | `ort-2.0.0-rc.13/src/ep/mod.rs:57–59, 268–291` | `name == avail` → `Ok(true)`, otherwise `Ok(false)` | Yes |
| `ExecutionProvider` needs only `name` and `register` | `ep/mod.rs:39–62` | As stated | Yes |
| No invariant pins the `ci.yml` workflow `env:` block | grep `tests/invariants/src` for `CARGO_INCREMENTAL`, `CARGO_PROFILE_DEV_DEBUG`, `"\nenv:"` | Only the new `system_onnxruntime_contract.rs:612` | Yes (no conflict) |
| `[workspace.package] rust-version` cannot be misread as `version` | `scripts/lib/pkg_meta.sh` (awk exact key), `onboarding_packaging_contract.rs:61–78` (`k.trim() == key`) | Exact-key match | Yes |
| `soos-enroll` early mock return precedes the V4L2 camera and models | `crates/enrollment-cli/src/service.rs:1816–1835` | Yes. **But** `MasterKey::load_or_create` and `BiometricStore::new` run before it (`:1813–1814`). | Partial (n-1) |
| GUI real-model branch | `crates/gui/src/main.rs:55–75` | Single `ModelRegistry::new`, `main` returns `Box<dyn Error>` | Yes |
| Only five crates reach ORT | grep over all 18 member manifests | daemon, inference-ort, vision, enrollment-cli, gui | Yes |
| `Dockerfile.arch` digest | `tests/docker/Dockerfile.arch:9` | `sha256:b21322c6…0040` | Yes |
| Daemon order (camera → keys → models) | `crates/daemon/src/pipeline.rs:574–628` | `resolve_pipeline_camera` :581, `MasterKey` :599/:610, `ModelRegistry::new` :628 | Yes |

## 3. Pillar Analysis

### Pillar 1: Architecture & threat model
- Scenario: a planted `.pc` on a CI runner. Workflow-level `LIBONNXRUNTIME_NO_PKG_CONFIG=1` means pkg-config is never run in the `--all-features` jobs, so build-host pkg-config state cannot change which library the CI jobs link. The system job runs in a clean container with the variable explicitly unset.
- Scenario: a GPU variant of `libonnxruntime`. It is still CPU-only by construction (no `with_execution_providers`, no other EP), now pinned by the m-10 scan. That scan has a gap (n-3).
- Result: PASS, with MINOR n-3.

### Pillar 2: PAM deadline & concurrency
- Scenario: the check adds latency to authentication. It does not: it runs once at daemon, `soos-enroll` and `soos-gui` start, before the socket exists. `crates/pam` is untouched (SORT-S6).
- Result: PASS.

### Pillar 3: Panic safety & fail-closed
- Scenario: a constant-true probe. SORT9's `"SoosNoSuchExecutionProvider"` → `Ok(false)` fails against it, and the static SORT-S4 clauses fail against `ep::CPU` or an `is_available` override. Closed.
- Scenario: the handshake panic (`OnceLock::get_or_init` + `expect`). It is caught by `check_cpu_execution_provider`. `OnceLock` does not poison after a panicking initializer, so no later call is made inconsistent.
- Scenario: `soos-enroll` runs against an incompatible library on a fresh host. With the check placed after the mock return (§10), `MasterKey::load_or_create` and `BiometricStore::new` have already created the master key and store directory before the check fails. That is fail closed (no template is written and the exit is non-zero), but it contradicts §10's "nothing written" claim and differs from the daemon's step-0 order (n-1).
- Result: PASS, with MINOR n-1.

### Pillar 4: Dependencies
- Scenario: the env line silently changes the default CI build. It does not: the GitHub runners have no `libonnxruntime.pc` today, so pkg-config already failed and the download was already used. The variable only makes that explicit. `ORT_SKIP_DOWNLOAD` in the i686 step is independent of it.
- Scenario: a developer runs `./save.sh` on an Arch host with `onnxruntime-cpu` installed. `save.sh:239,253` runs the same `--all-features` commands but does not set the variable. If the system library is 1.27 or newer, the local gate links the distro library while CI uses the download. If it is older, the mixed link from m-4 reappears locally. This breaks the "green local predicts green CI" parity claimed in the `ci.yml` header (n-2).
- No new crate. `pkg-config 0.3.34` is already locked.
- Result: PASS, with MINOR n-2.

### Pillar 5: Data confidentiality
- Scenario: secrets in the error detail. The detail is ORT text or a panic payload, bounded to 256 bytes. `execution_provider_listed` returns only `bool`/ORT error text. The GUI and `soos-enroll` print the same bounded `Display`.
- Result: PASS.

### Pillar 6: Test integrity
- Scenario: the developer adapts an existing test ad hoc. §11 now authorizes exactly one migration, with old and new assertions, and the tester has already applied it in a form equivalent to the plan.
- Scenario: SORT9 against a wrong implementation:
  - case-insensitive compare → fails on `"cpuexecutionprovider"`;
  - constant true → fails on the unknown name;
  - `ep::CPU` → static clause fails.
- Scenario: the order check for `soos-enroll` passes while the check still runs after key creation. SORT-S4 as described pins only the presence of the call (n-1).
- Result: PASS, with MINOR n-1.

## 4. Findings (round 2)

- **[MINOR] n-1: the `soos-enroll` check runs after key and store creation, so "nothing written" is not true.** In `build_full_service_with_notes`, `MasterKey::load_or_create(&key_path)` and `BiometricStore::new(bio_dir, key)` (`service.rs:1813–1814`) run before the `if cli.mock` early return. §10 places the check after that return, so on a fresh host with a broken library the master key and store directory are created before the failure. **Required change**: run `soos_inference_ort::verify_cpu_execution_provider()?` guarded by `if !cli.mock` before `MasterKey::load_or_create`, matching the daemon's step 0, and have SORT-S4 pin the order (check offset < `MasterKey::load_or_create` offset < `V4lCameraManager::spawn` offset). Otherwise, drop the "nothing written" claim from §10.
- **[MINOR] n-2: the local gate loses parity with CI.** The workflow-level `LIBONNXRUNTIME_NO_PKG_CONFIG: "1"` is not mirrored in `save.sh`, which runs the identical `--all-features` clippy/test commands. On a developer host with a distro `libonnxruntime.pc`, the local result depends on that library (mixed link if it is older than 1.27, a dynamic distro link if newer). **Required change**: `export LIBONNXRUNTIME_NO_PKG_CONFIG=1` in `save.sh` before the cargo gates (unless the caller already set it), and mention it in the ADR. Otherwise, state the divergence explicitly in the ADR and in `Docs/INFERENCE_ORT_CRATE.md` as accepted.
- **[MINOR] n-3: the m-10 source scan can be bypassed by grouped imports.** A substring scan for `ort::ep` misses `use ort::{ep::CUDA, session::Session}` and a direct `SomeEp.register(&mut builder)`. It could also false-positive on a type ending in `ort` followed by `::ep…` (for example `Report::epoch`). **Required change (tester)**: make the scan token-aware, for example by rejecting `ep::` path segments in any file outside `runtime.rs` that imports `ort`, plus `impl ExecutionProvider` and `.register(` on an `ort` builder outside `runtime.rs`.
- **[MINOR] n-4: the superseded text in §2, §3 and §6 still contradicts §10.** §2 still shows `CpuProviderProbe` and a re-export list without `execution_provider_listed`; §3's "Both features enabled" paragraph still describes pkg-config-first in CI; §6 still lists the old `--all-targets` REQUIRE command. §10 is declared normative, so this is not blocking. **Required change**: replace those passages in place with pointers to §10 so the developer and traceability agents do not implement the stale variant.

No CRITICAL or MAJOR finding. All three round-1 MAJORs (M-1, M-2) and the stale base (M-3) are resolved, and all round-1 MINORs are addressed. The workflow-level env line breaks no existing invariant.

## 5. Verdict
VALIDATION_VERDICT: APPROVED
