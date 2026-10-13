# Architect Spec — GitHub #375: build against the system ONNX Runtime for distribution packages

- **Date**: 2026-10-13
- **Issue**: GitHub-only #375 = PR 1 of the distribution packaging umbrella #361 (planned split item 1); no `AI/BACKLOG.md` id.
- **Branch**: `feat/system-onnxruntime` (not registered in `scripts/sync_issue.py`; the commit carries `Closes #375`).
- **Base commit**: `6f32ef8` (`origin/main`, #373; revision 1 rebased the spec from `e86993a`).
- **Owner decisions** (#361, 2026-10-12, restated for this run): maintainer identity is the `soos-security` organization; nothing is published until the owner says go. This PR publishes nothing: no tag, release, package, channel or vendor tarball (all out of scope, later PRs of #361).

## 0. Research Evidence (gathered before the design)

| Question | Evidence | Result |
|---|---|---|
| `ort` / `ort-sys` features | `ort-2.0.0-rc.13/Cargo.toml`: `default = [std, ndarray, tracing, download-binaries, tls-native, copy-dylibs, api-27]`; `pkg-config = ["ort-sys/pkg-config"]`; `load-dynamic`; `api-17` … `api-28` chained (`api-N` enables `api-(N-1)`). `ort-sys` `default = [std, api-17]`. | The system build needs `default-features = false`. |
| How `ort-sys` links | `ort-sys-2.0.0-rc.13/build/main.rs`: (1) `pkg-config` feature → `pkg_config::attempt()` first; (2) `ORT_LIB_PATH`/`ORT_LIB_LOCATION`; (3) otherwise `download-binaries` (skipped when `ORT_SKIP_DOWNLOAD`, `ORT_OFFLINE` or `CARGO_NET_OFFLINE` is set); without any source it emits `cfg(link_error_generic)` so `cargo check` works and **linking fails** (fail closed at build time). | Without `download-binaries` there is no download code at all in the build graph. |
| pkg-config version gate | `build/pkg_config.rs`: probes `libonnxruntime`; rejects (falls back) when the library minor version `< ORT_API_VERSION`; `src/version.rs`: `ORT_API_VERSION = 17 + Σ api-18..api-28`. | `api-17` accepts any ONNX Runtime ≥ 1.17. |
| Runtime API handshake | `ort-2.0.0-rc.13/src/lib.rs::setup_api`: `GetApi(ORT_API_VERSION)` and `NonNull::new(..).expect("Failed to initialize ORT API")`: a library older than the requested API **panics** at the first ORT call. | The startup check must catch that panic (§2). |
| CPU EP availability | `ort::ep::CPU::is_available()` returns `Ok(true)` **without asking the library**; the default `ExecutionProvider::is_available` calls `GetAvailableProviders` and compares names; the trait is public and not sealed. | Probe with a private `ExecutionProvider` implementation named `CPUExecutionProvider` that is never registered (§2). |
| API level our code needs | Experiment on this branch: `cargo check --offline --locked --workspace --all-targets --no-default-features --features system-onnxruntime` with `ort/api-17` → **Finished, zero errors** (rust 1.98.1). No `api-18+`-gated item is used by the workspace. | Lowest level = **`api-17`** (the floor of `ort` rc.13). |
| What distributions ship | Arch `[extra]` `onnxruntime-cpu 1.29.0-3` (queried in the pinned `archlinux:latest@sha256:b21322c6…` image on 2026-10-13; ships `/usr/lib/libonnxruntime.so.1.29.0` and `/usr/lib/pkgconfig/libonnxruntime.pc`, version `1.29.0`); Fedora 43 `1.20.1`, Fedora 44 `1.22.2`, Debian sid `1.19.2+dfsg` (#361 research). | `api-17` ≤ every target (API 19/20/22/29). |
| MSRV | Highest `rust-version` declared by the 516 locked packages: **1.92** (`egui`/`eframe`/`epaint` 0.35); 1.88 for `ort`/`ort-sys`. Rust 1.92.0 (pinned rustup, Arch container, offline): `cargo check --locked --offline --workspace --all-targets --all-features` → Finished. Rust 1.88.0 with `--ignore-rust-version` fails in `epaint` (`std::iter::chain`, stabilized 1.91). | `rust-version = "1.92"`; proven by the CI job building with exactly 1.92.0 (§6). |
| Default graph unchanged | `cargo tree -e normal,build,dev -f '{p} [{f}]'` over the workspace, before and after the manifest change: `ort`, `ort-sys`, `ureq`, `ndarray`, `tracing` feature lists byte-identical (`diff` empty). | The default dev/CI build is unchanged. |

## 1. Scope & Blast Radius

| Area | Change |
|---|---|
| `Cargo.toml` (root) | `[workspace.package] rust-version = "1.92"`; `ort = { version = "=2.0.0-rc.13", default-features = false, features = ["std", "ndarray", "tracing"] }`; `soos-inference-ort`, `soos-vision`, `soos-enrollment-cli` workspace entries gain `default-features = false` |
| every member `Cargo.toml` (18, `crates/frame` included) | `rust-version.workspace = true` |
| `crates/inference-ort/Cargo.toml` | `[features] default = ["download-binaries"]`, `download-binaries = ["ort/default"]`, `system-onnxruntime = ["ort/pkg-config", "ort/api-17"]` |
| `crates/vision`, `crates/daemon`, `crates/enrollment-cli`, `crates/gui` `Cargo.toml` | the same two features plus `default`, forwarding to their ORT-dependent workspace crates (§3) |
| `Cargo.lock` | `ort-sys` gains its already-locked optional build dependency `pkg-config 0.3.34` (no new package, no new version) |
| `crates/inference-ort/src/runtime.rs` | **new** module: CPU execution provider startup check (§2) |
| `crates/inference-ort/src/error.rs` | new variant `InferenceError::CpuExecutionProviderUnavailable(String)` |
| `crates/inference-ort/src/lib.rs` | `pub mod runtime;` + re-exports |
| `crates/daemon/src/pipeline.rs` | `initialize_pipeline` step 0: `soos_inference_ort::verify_cpu_execution_provider()?` before the camera, keys, stores and models |
| `tests/docker/Dockerfile.system-ort` | **new**: digest-pinned Arch image, `onnxruntime-cpu` from `[extra]`, Rust = MSRV through `scripts/install_rustup.sh` |
| `tests/docker/system_onnxruntime_test.sh` | **new** harness: online fetch phase, then `--network none` build and test phase (§6) |
| `.github/workflows/ci.yml` | **new** job `system-onnxruntime` (after `lint`, PRs and pushes), added to `ci-success.needs` |
| tests | `crates/inference-ort/tests/system_onnxruntime_tests.rs`, `crates/daemon/tests/system_onnxruntime_startup_tests.rs`, `tests/invariants/src/system_onnxruntime_contract.rs` (§7) |
| Docs | `Docs/INFERENCE_ORT_CRATE.md`, `Docs/PACKAGING_AND_PROVISIONING.md` (new §7.5), `Docs/CI_CD_AND_SECURITY.md`, `Docs/DAEMON.md` (startup order), `.claude/skills/dev-workflow/references/project-facts.md` (MSRV + build line) |
| Traceability | ADR "Distribution Builds: System ONNX Runtime", matrix section `SORT*`, walkthrough 201 |

Untouched: `crates/pam` (its dependency graph has no `ort`, before and after: `cargo tree -p soos-pam -e normal,build | grep -c ort` = 0, pinned by SORT-S6), the IPC protocol, the models and manifest, `scripts/install.sh`, `scripts/build_*.sh`, `packaging/**` (they keep the default `download-binaries` build in this PR; switching packages to the system build is a later #361 PR), `scripts/prefetch_onnxruntime.sh` (its `cargo check --locked -p ort` still resolves `ort` with `download-binaries`: verified with `cargo tree -p ort`, the AFC5 contract stays as is).

Consumers of changed public items: `InferenceError` gains a variant. `grep -rn "InferenceError::" crates/*/src` shows no exhaustive `match` outside `soos-inference-ort` (all consumers use `?`, `matches!` or `Display`); the daemon wraps it in `DaemonError::Inference` unchanged.

## 2. Types & Signatures (`crates/inference-ort/src/runtime.rs`, `#![forbid(unsafe_code)]` inherited)

> **Superseded in part by §10 M-2**: the probe is `ListedProviderProbe(&'static str)` (no `is_available` override) behind `#[doc(hidden)] pub fn execution_provider_listed(name: &'static str) -> Result<bool, String>`, which `verify_cpu_execution_provider` calls with `CPU_EXECUTION_PROVIDER`; `execution_provider_listed` is reachable as `soos_inference_ort::runtime::execution_provider_listed`. `CpuProviderProbe` below is the round-0 name.

```rust
/// ONNX Runtime name of the CPU execution provider (the only provider soos uses, ADR 2026-09-12).
pub const CPU_EXECUTION_PROVIDER: &str = "CPUExecutionProvider";

/// Upper bound, in bytes, of the detail kept in `CpuExecutionProviderUnavailable` (an ORT error
/// string or a panic payload); longer text is truncated on a UTF-8 character boundary.
pub const MAX_RUNTIME_ERROR_DETAIL_BYTES: usize = 256;

/// Fail-closed startup check of the linked ONNX Runtime: the library must answer the C API
/// handshake at the compiled API level and list `CPUExecutionProvider` among its available
/// providers. Never registers a provider and never creates a session.
pub fn verify_cpu_execution_provider() -> Result<(), InferenceError>;

/// Pure decision behind `verify_cpu_execution_provider`, injectable for tests. `probe` returns
/// whether the CPU provider is listed (`Ok(true)`), not listed (`Ok(false)`) or an ORT error
/// (`Err(text)`); a panic inside `probe` (the `ort` handshake `expect` on a library older than
/// the compiled API level) is caught and is a failure.
pub fn check_cpu_execution_provider<F>(probe: F) -> Result<(), InferenceError>
where
    F: FnOnce() -> Result<bool, String> + std::panic::UnwindSafe;
```

Semantics of `check_cpu_execution_provider` (every non-`Ok(true)` outcome is an error, never a default):

| `probe` outcome | Result |
|---|---|
| `Ok(true)` | `Ok(())` |
| `Ok(false)` | `Err(CpuExecutionProviderUnavailable("CPUExecutionProvider is not listed by the linked ONNX Runtime"))` |
| `Err(text)` | `Err(CpuExecutionProviderUnavailable(bounded(text)))` |
| panic with `&str` / `String` payload | `Err(CpuExecutionProviderUnavailable(bounded("ONNX Runtime initialization panicked: " + payload)))` |
| panic with another payload type | `Err(CpuExecutionProviderUnavailable("ONNX Runtime initialization panicked"))` |

`bounded(s)`: at most `MAX_RUNTIME_ERROR_DETAIL_BYTES` bytes, cut at the largest char boundary `<=` the bound (empty input stays empty); no allocation beyond the bound plus the fixed prefix.

`verify_cpu_execution_provider()` = `check_cpu_execution_provider(|| CpuProviderProbe.is_available().map_err(|e| e.to_string()))`, where

```rust
/// Never registered: exists only so the default `ExecutionProvider::is_available` asks the
/// library (`GetAvailableProviders`) for `CPUExecutionProvider`. `ort::ep::CPU::is_available`
/// returns `Ok(true)` without asking and cannot be used.
struct CpuProviderProbe;
impl ort::ep::ExecutionProvider for CpuProviderProbe {
    fn name(&self) -> &'static str { CPU_EXECUTION_PROVIDER }
    fn register(&self, _: &mut ort::session::builder::SessionBuilder) -> ort::Result<()> { Ok(()) }
}
```

Error variant (`crates/inference-ort/src/error.rs`):

```rust
/// The linked ONNX Runtime failed the startup check (fail closed: the daemon refuses to start).
#[error("ONNX Runtime CPU execution provider unavailable: {0}")]
CpuExecutionProviderUnavailable(String),
```

Re-exports in `lib.rs`: `pub use runtime::{check_cpu_execution_provider, verify_cpu_execution_provider, CPU_EXECUTION_PROVIDER, MAX_RUNTIME_ERROR_DETAIL_BYTES};`.

Logging: on success the daemon logs one `info!` line ("ONNX Runtime CPU execution provider available"); on failure the existing `error!` of `run()` ("Failed to initialize pipeline components; refusing to start daemon (fail-closed)") carries the error `Display`. The detail is an ORT error string or a panic payload: no frames, embeddings, keys or credentials can reach it. The daemon panic hook still records a caught handshake panic (location only in release builds, GitHub #259/#287); that is expected.

## 3. Features (the only build-configuration surface)

Feature name: **`system-onnxruntime`**; its default counterpart: **`download-binaries`**.

| Crate | `default` | `download-binaries` | `system-onnxruntime` |
|---|---|---|---|
| `soos-inference-ort` | `["download-binaries"]` | `["ort/default"]` (exactly ort's own defaults: `std, ndarray, tracing, download-binaries, tls-native, copy-dylibs, api-27`) | `["ort/pkg-config", "ort/api-17"]` |
| `soos-vision` | `["download-binaries"]` | `["soos-inference-ort/download-binaries"]` | `["soos-inference-ort/system-onnxruntime"]` |
| `soos-daemon` | `["download-binaries"]` | `["soos-inference-ort/download-binaries", "soos-vision/download-binaries"]` | `["soos-inference-ort/system-onnxruntime", "soos-vision/system-onnxruntime"]` |
| `soos-enrollment-cli` | `["download-binaries"]` | same as daemon | same as daemon |
| `soos-gui` | `["download-binaries"]` | daemon set + `"soos-enrollment-cli/download-binaries"` | daemon set + `"soos-enrollment-cli/system-onnxruntime"` |

The workspace `ort` entry carries only the features both builds need (`std`, `ndarray`, `tracing`), with `default-features = false`; every edge to `soos-inference-ort`, `soos-vision` and `soos-enrollment-cli` is `default-features = false` (workspace entries), so no crate can re-enable the download in the system build. The dev-only direct `ort` dev-dependencies of daemon, vision and enrollment-cli inherit the workspace entry.

Resulting `ort-sys` feature sets (pinned by SORT-S3, `cargo tree --offline -e features`):
- default workspace build: `__tls, api-17 … api-27, copy-dylibs, download-binaries, std, tls-native` (unchanged);
- `--no-default-features --features system-onnxruntime` (workspace or any of the five packages): exactly `api-17, pkg-config, std`.

Distribution build command (records the deviation from the literal #361 line, which cannot work because Cargo features are additive and the download is a default feature):

```bash
ORT_SKIP_DOWNLOAD=1 cargo build --locked --offline --release --workspace \
    --no-default-features --features system-onnxruntime
```

`--no-default-features` on the workspace only removes the new `download-binaries` defaults: no other member has a non-empty `default` (`camera-v4l` `default = []`, `pam` has none).

**Both features enabled** (`--all-features`, used by CI `clippy`/`test`/`i686` and the developer gate):

> **Superseded by §10 m-4 / revision 2 n-2**: CI and `save.sh` set `LIBONNXRUNTIME_NO_PKG_CONFIG=1`, so `--all-features` gates resolve the verified download only; the text below describes a host without that variable. not an error (a `compile_error!` would break the mandatory `--all-features` gates). `ort-sys` then tries pkg-config first with the `api-27` gate (system ONNX Runtime ≥ 1.27 with a `libonnxruntime.pc`), otherwise falls back to the SHA-256-verified download. The GitHub Ubuntu runners have no `libonnxruntime.pc`, so CI keeps the downloaded binary; a developer host with a recent distro ONNX Runtime may link it dynamically under `--all-features` only. Documented (ADR, `Docs/INFERENCE_ORT_CRATE.md`); the default build and the system build are each deterministic.

## 4. Constants & Config

| Name | Location | Value | Sentinel semantics |
|---|---|---|---|
| `CPU_EXECUTION_PROVIDER` | `crates/inference-ort/src/runtime.rs` | `"CPUExecutionProvider"` | exact, case-sensitive match only |
| `MAX_RUNTIME_ERROR_DETAIL_BYTES` | same | 256 | 0-length detail allowed; longer truncated |
| `rust-version` | root `Cargo.toml` `[workspace.package]` | `"1.92"` | must be ≤ `rust-toolchain.toml` channel (`1.98.1`) |
| ORT API level of the system build | `crates/inference-ort/Cargo.toml` | `api-17` | ONNX Runtime ≥ 1.17 accepted at build (pkg-config) and run time (`GetApi(17)`) |
| Arch base image | `tests/docker/Dockerfile.system-ort` | `archlinux:latest@sha256:b21322c663be387c0ed9cbc7bbbfe18e41633ad4e7b7c77cfad45f128be20040` | same digest as `tests/docker/Dockerfile.arch` |

No runtime configuration key is added. Nothing reads `ORT_DYLIB_PATH` (no `load-dynamic`).

## 5. Error Taxonomy & Fail-Closed Path

| Condition | Where | Outcome |
|---|---|---|
| `libonnxruntime.so.1` missing at run time | dynamic loader, before `main` | daemon exits non-zero, socket never created → `pam_soos.so` connect fails → `PAM_IGNORE` (password) |
| library older than API 17 (handshake `expect`) | `check_cpu_execution_provider` catches the panic | `InferenceError::CpuExecutionProviderUnavailable` → `DaemonError::Inference` → `run()` returns `Err`, daemon refuses to start → `PAM_IGNORE` |
| CPU provider not listed / `GetAvailableProviders` error | same | same |
| no `libonnxruntime.pc`, or version < 1.17, at build time | `ort-sys` build script | `cfg(link_error_generic)` → link error, no artifact (never a silent download: the download code is not compiled) |
| model checksum / shape / PAD self-test failure | unchanged (`ModelRegistry`, `validate_pad_detector`) | unchanged fail closed |

No path maps any of these to `Allow` or `PAM_SUCCESS`. The PAM module never links or loads ORT (SORT-S6, plus `readelf -d` of `libpam_soos.so` in the CI job).

## 6. CI Job `system-onnxruntime` ("System ONNX Runtime (Arch, offline)")

> **Superseded in part by §10 m-6 / m-7**: the test commands are the explicit list of §10 m-6, plus the MSRV `cargo check --all-features` of m-7; `tests/docker/system_onnxruntime_test.sh` is normative.

- Runs on `pull_request`, `push` to `main` and `workflow_dispatch` (not on `schedule`), `needs: lint`, `timeout-minutes: 90`, `permissions` inherited (`contents: read`), checkout pinned by SHA with `persist-credentials: false`, single step `run: ./tests/docker/system_onnxruntime_test.sh` (no `${{ }}` in `run:`), part of `ci-success.needs`.
- `tests/docker/Dockerfile.system-ort`: `FROM archlinux:latest@sha256:<Dockerfile.arch digest>`; `pacman -Syu --noconfirm --needed base-devel clang openssl pkgconf pam onnxruntime-cpu curl ca-certificates`; Rust installed only through `scripts/install_rustup.sh --default-toolchain "${SOOS_MSRV}"` (verified rustup-init, no `curl | sh`); `ARG SOOS_MSRV` without default.
- Harness phases (host side, `set -euo pipefail`):
  1. **Image** (network): `docker build --build-arg SOOS_MSRV=<x.y.0> -f tests/docker/Dockerfile.system-ort`, MSRV read from `[workspace.package] rust-version` (`1.92` → `1.92.0`).
  2. **Fetch** (network, separate step): `cargo fetch --locked` into the named volume mounted at `$CARGO_HOME/registry`; attested models with `scripts/download_models.sh --target-dir` into a models volume (or, locally, the host `/var/lib/soos/models` read-only when `SOOS_SORT_MODELS=host` and it holds a `manifest.toml`).
  3. **Offline build and tests** (`docker run --network none`, repository mounted read-only, `CARGO_TARGET_DIR` on a volume, `RUSTUP_TOOLCHAIN=<MSRV>` so `rust-toolchain.toml` is not used, `ORT_SKIP_DOWNLOAD=1`, `CARGO_NET_OFFLINE=true`, `ORT_LIB_LOCATION`/`ORT_LIB_PATH` unset), in-container assertions:
     - `/sys/class/net` lists `lo` only; `rustc --version` starts with `rustc <MSRV>`; `onnxruntime-cpu` installed and its version printed;
     - no `$HOME/.cache/ort.pyke.io` before and after;
     - `cargo tree --locked --offline -e features -i ort-sys --workspace --no-default-features --features system-onnxruntime` shows no `download-binaries` and no `api-18`;
     - `cargo build --locked --offline --release --workspace --no-default-features --features system-onnxruntime`;
     - `readelf -d target/release/soos-daemon` NEEDS `libonnxruntime.so.1`, resolved by `ldd` to `/usr/lib/libonnxruntime.so.1` owned by `onnxruntime-cpu` (`pacman -Qo`); `libpam_soos.so` does **not** need `libonnxruntime`;
     - `SOOS_MODELS_DIR=/models SOOS_REQUIRE_REAL_MODELS=1 cargo test --locked --offline --no-default-features --features system-onnxruntime -p soos-inference-ort --all-targets`, then `-p soos-vision --test pad_crop_real_model_tests`, then `-p soos-daemon --test pad_nonface_pipeline_real_model_tests --test system_onnxruntime_startup_tests`;
     - fail closed: `pacman -Rdd --noconfirm onnxruntime-cpu` (local, offline), then `target/release/soos-daemon --version` must exit non-zero with `libonnxruntime` in its error output.
- Rolling-release trade-off: the base image is digest-pinned but `pacman -Syu` installs the current `[extra]` `onnxruntime-cpu` (same policy as `Dockerfile.arch`): the job is meant to catch ABI drift of what Arch actually ships; the harness prints the installed version.

## 7. Test Hooks for the Tester (contract `SORT*`)

Unit/integration (`crates/inference-ort/tests/system_onnxruntime_tests.rs`):
- SORT1 `test_sort_check_accepts_listed_cpu_provider` (`Ok(true)` → `Ok`).
- SORT2 `test_sort_check_rejects_unlisted_cpu_provider` (`Ok(false)` → `CpuExecutionProviderUnavailable`, message names `CPUExecutionProvider`).
- SORT3 `test_sort_check_rejects_probe_error_with_bounded_detail` (`Err` of 10 000 bytes, multibyte chars at the cut → detail ≤ `MAX_RUNTIME_ERROR_DETAIL_BYTES` with no prefix on the `Err` path, valid UTF-8; the panic path bounds prefix plus payload together, §2).
- SORT4 `test_sort_check_catches_handshake_panic` (`&str` payload, `String` payload, non-string payload → error, never a propagated panic).
- SORT5 `test_sort_linked_runtime_lists_cpu_provider` (real `verify_cpu_execution_provider()` → `Ok` against the linked library: the downloaded binary in normal CI, the Arch library in the system job).
- SORT6 `test_sort_constants` (`CPU_EXECUTION_PROVIDER == "CPUExecutionProvider"`, bound = 256).

Daemon (`crates/daemon/tests/system_onnxruntime_startup_tests.rs`):
- SORT7 `test_sort_daemon_pipeline_starts_on_linked_runtime_with_real_models` (mock camera, temp stores, `SOOS_MODELS_DIR`; `SKIPPED` without models unless `SOOS_REQUIRE_REAL_MODELS=1`): `initialize_pipeline` → `Ok`.

Static invariants (`tests/invariants/src/system_onnxruntime_contract.rs`, registered in `lib.rs`):
- SORT-S1 manifests: workspace `ort` line exact; the three `default-features = false` workspace entries; the five crates' feature tables exact (§3); no `load-dynamic`, `alternative-backend`, `preload-dylibs`, `copy-dylibs` (outside `ort/default`), GPU or NPU execution-provider feature of `ort` in any manifest (CPU only).
- SORT-S2 MSRV: `rust-version = "1.92"` under `[workspace.package]`; every workspace member declares `rust-version.workspace = true`; `rust-version` ≤ the `rust-toolchain.toml` channel (numeric comparison).
- SORT-S3 feature graph (`cargo tree --locked --offline -e features -f '{p} [{f}]'`): default workspace graph `ort-sys` features equal the §3 default list; system graph for `--workspace` and for each of the five packages equals `{api-17, pkg-config, std}`.
- SORT-S4 daemon order: `initialize_pipeline` calls `soos_inference_ort::verify_cpu_execution_provider()` before the camera step (`resolve_pipeline_camera`), `ModelRegistry::new` and `MasterKey::load_or_create`.
- SORT-S5 CI job + harness + Dockerfile contents (§6), including `--network none`, `ORT_SKIP_DOWNLOAD=1`, `--locked --offline`, the separate `cargo fetch --locked`, `SOOS_REQUIRE_REAL_MODELS=1`, digest equal to `Dockerfile.arch`, `onnxruntime-cpu`, `install_rustup.sh`, MSRV derived from `Cargo.toml`, `ci-success` dependency, no event interpolation.
- SORT-S6 PAM isolation: `cargo tree --locked --offline -p soos-pam -e normal,build` (default and `--all-features`) contains no `ort` / `ort-sys` / `soos-inference-ort`.
- SORT-S7 docs: ADR title "Distribution Builds: System ONNX Runtime" present with `load-dynamic`, `download-binaries`, `pkg-config`, `api-17`, `rust-version`, "distro-signed"/"SHA-256" trust statement; `Docs/PACKAGING_AND_PROVISIONING.md` gives the exact build command; `Docs/INFERENCE_ORT_CRATE.md` documents `verify_cpu_execution_provider`.

Existing evidence reused (unchanged): `crates/pam/tests/ipc_tests.rs::test_ipc_offline_daemon_returns_ignore` (daemon absent → `PAM_IGNORE`), `pipeline_init_tests::test_pipeline_init_missing_models_fails_closed`, the real-model suites, AFC5 prefetch contract.

## 8. Invariants Touched

- ADR 2026-09-12 "ort CPU execution mode, no OpenCV": kept; the system build uses only the CPU provider and the startup check asserts it; `test_no_opencv_in_any_cargo_toml` unchanged.
- Fail closed (ARCHITECTURE §2): new failure modes all end in "daemon refuses to start" → `PAM_IGNORE`.
- PAM: no change to `crates/pam`, no Tokio, no new dependency (SORT-S6).
- `#![forbid(unsafe_code)]` in `soos-inference-ort`: the probe uses only the safe `ort` trait API.
- Supply chain: no new crate; `pkg-config 0.3.34` already locked; `cargo deny` (`[graph] all-features = true`) unchanged in outcome; the Microsoft prebuilt (SHA-256 pinned by `ort-sys`) stays the default; the system build trusts the distribution-signed package instead (ADR).
- Latency budget: one extra `GetAvailableProviders` call at daemon start only; the auth path is untouched.

## 9. Documentation Drift / ADR

- New ADR (appended last in `AI/DECISIONS.md`): **"[2026-10-13] Distribution Builds: System ONNX Runtime"** — feature names and sets, `api-17` evidence, the `--no-default-features` build line (deviation from the literal #361 line), the `--all-features` behaviour, the startup check, MSRV 1.92 and its proof, the CI job, the trust change (a distro-signed `libonnxruntime` instead of the SHA-256-pinned Microsoft archive), and the rejected alternatives: bundling the Microsoft prebuilt in packages (official repositories forbid bundled binaries), `load-dynamic`/`dlopen` in the root daemon (adds a library-path trust decision, `ORT_DYLIB_PATH`, to a root process), post-install downloads in maintainer scripts (no network in maintainer scripts, not reproducible); also rejected: an `ORT_LIB_LOCATION`-only build without the pkg-config version gate, and a `compile_error!` on both features (breaks `--all-features`).
- `.claude/skills/dev-workflow/references/project-facts.md` §5: MSRV line and the system build command.
- Drift found: #361 writes the build command without `--no-default-features`; recorded in the ADR, not "fixed" in #361.

## 10. Revision 1 — Plan Evaluation Findings (AI/plan_evaluator_report.md, REVISION_REQUIRED)

| Finding | Resolution (normative, supersedes the sections above where they differ) |
|---|---|
| **M-1** `dependency_tooling_contract::test_prerelease_workspace_dependencies_are_exact_pinned` asserts `section.contains("ort = \"=2.0.0-rc.")`, which the inline-table `ort` entry no longer matches | **Contract Migration** (tester-owned, §11). Old: `section.contains("ort = \"=2.0.0-rc.")`. New: the `[workspace.dependencies]` line starting with `ort = ` contains `version = "=2.0.0-rc.`. Same intent (ort stays exact-pinned to its release candidate), same strength; mandated by #375/#361 ("build `ort` with default-features off"), which needs the inline table. |
| **M-2** the guard could silently become a no-op (`ep::CPU::is_available` or a constant probe) | `runtime.rs` gets a private `ListedProviderProbe(&'static str)` implementing `ort::ep::ExecutionProvider` **without** overriding `is_available`, and `#[doc(hidden)] pub fn execution_provider_listed(name: &'static str) -> Result<bool, String>` (`ListedProviderProbe(name).is_available().map_err(to_string)`); `verify_cpu_execution_provider()` = `check_cpu_execution_provider(\|\| execution_provider_listed(CPU_EXECUTION_PROVIDER))`. New linked-library test SORT9 `test_sort_provider_probe_asks_the_linked_library`: `"CPUExecutionProvider"` → `Ok(true)`, `"SoosNoSuchExecutionProvider"` → `Ok(false)`, `"cpuexecutionprovider"` → `Ok(false)`. Static clause in SORT-S4: `runtime.rs` contains `impl ExecutionProvider for ListedProviderProbe`, no `fn is_available`, never `ep::CPU`, and `verify_cpu_execution_provider` calls `execution_provider_listed(CPU_EXECUTION_PROVIDER)`. |
| **M-3** stale base | Rebased on `6f32ef8`: 18 members (`crates/frame`); SORT-S2 reads the members from `[workspace] members` (no count). The non-required `portable-macos` job runs `--all-features` on `soos-inference-ort`/`soos-vision`: both features are on there, no `libonnxruntime.pc` is installed, so it keeps the verified download (and `LIBONNXRUNTIME_NO_PKG_CONFIG=1`, below, makes that explicit). SORT1–SORT9 run there too and pass against the downloaded binary. |
| **m-4** both features: the pkg-config crate prints `-L`/`-l onnxruntime` before `ort-sys` rejects a library older than the api-27 gate, so the fallback would link the downloaded static archive **and** the distro shared library | The workflow-level `env:` of `ci.yml` sets `LIBONNXRUNTIME_NO_PKG_CONFIG: "1"` (honoured by `pkg-config` 0.3.34, `<NAME>_NO_PKG_CONFIG`): every `--all-features` job (clippy, i686, test, macOS) resolves the download only, as before this change. The system harness passes an explicit environment to its offline container and asserts the variable is unset there. §3, the ADR and `Docs/INFERENCE_ORT_CRATE.md` document the mixed-link hazard and the opt-out for developer hosts with an older distro ONNX Runtime. SORT-S5 pins the env line. |
| **m-5** overstated deviation | ADR/§3 wording: the literal #361 line (without `--no-default-features`) can link the Arch library through pkg-config, but it compiles the download code, raises the pkg-config gate to API 27 (excluding Fedora 43/44 and Debian), and silently downloads when `ORT_SKIP_DOWNLOAD` is unset and no `.pc` is found; hence `--no-default-features`. |
| **m-6** §6 command / SORT3 wording | §6 is replaced by the harness: one plain `-p soos-inference-ort --all-targets` run (models present, not required: the retired-ArcFace evaluation skips), then `SOOS_REQUIRE_REAL_MODELS=1` runs of the explicit list `system_onnxruntime_tests`, `pad_real_model_tests`, `embedding_real_model_tests`, `scrfd_real_model_tests`, `scrfd_hot_path_real_model_tests`, `pad_output_contract_tests`, `pad_input_range_tests` (inference-ort), `pad_crop_real_model_tests` (vision), `pad_nonface_pipeline_real_model_tests`, `system_onnxruntime_startup_tests` (daemon); SORT-S5 pins the list. SORT3 wording aligned (§7). |
| **m-7** partial MSRV coverage | The offline phase also runs `cargo check --locked --offline --workspace --all-targets --all-features` under the MSRV toolchain (no linking; pkg-config satisfies the API 27 gate with Arch 1.29). The ADR notes that clippy's `incompatible_msrv` lint now reads `rust-version` in the normal clippy job. |
| **m-8** rolling release in the PR gate | Accepted risk, recorded in the ADR and `Docs/CI_CD_AND_SECURITY.md` with a triage procedure: the harness prints `onnxruntime-cpu` and `rustc` versions; on a failure caused by an `[extra]` change (soname, dependency, mirror) the job is re-run once; a reproducible distro regression is filed as an issue and fixed forward (or the base digest bumped) in a dedicated `fix/` PR; the job is never made `continue-on-error` and never removed from CI Success without an ADR amendment. Rationale: the job exists to detect ABI drift of what Arch ships; an Arch Linux Archive snapshot would hide exactly that and adds a slow third-party mirror. |
| **m-9** `soos-enroll` / `soos-gui` | `soos-enroll` (root) runs `soos_inference_ort::verify_cpu_execution_provider()?` in `build_full_service_with_notes` after the mock-camera early return and before `V4lCameraManager::spawn` / `ModelRegistry::new` (error → `EnrollmentCliError::Inference`, non-zero exit, nothing written); `soos-gui` (`crates/gui/src/main.rs`, real-model branch) builds its own preview sessions, so it runs the same check before its `ModelRegistry::new` (error → the GUI exits with the check's message instead of panicking at the first ORT call); both pinned by SORT-S4. |
| **m-10** CPU only in source | SORT-S1 source scan: no `with_execution_providers` in any `crates/*/src`, and `ort::ep` / `use ort::ep` appears only in `crates/inference-ort/src/runtime.rs`. |

## 11. Contract Migration (tester-owned)

| Test | Old assertion | New assertion | Mandating acceptance line |
|---|---|---|---|
| `tests/invariants/src/dependency_tooling_contract.rs::test_prerelease_workspace_dependencies_are_exact_pinned` | `section.contains("ort = \"=2.0.0-rc.")` | the `ort = ` line of `[workspace.dependencies]` contains `version = "=2.0.0-rc.` | #375 / #361: "build `ort` with `default-features = false` … `pkg-config`" requires the inline-table form; the exact rc pin is kept |

## 12. Revision 2 — Plan Evaluation Round 2 (APPROVED; minor findings applied)

| Finding | Resolution |
|---|---|
| n-1 `soos-enroll` writes the master key and store directory before the check | The check runs at the top of `build_full_service_with_notes` under `if !cli.mock`, before `MasterKey::load_or_create`, `BiometricStore::new`, the camera and the registry; SORT-S4 pins that order. |
| n-2 local gate parity | `save.sh` exports `LIBONNXRUNTIME_NO_PKG_CONFIG=1` before its cargo gates (same environment as CI); documented in `Docs/INFERENCE_ORT_CRATE.md`. |
| n-3 substring scan | SORT-S1 scans identifier-bounded tokens `ep::`, `ExecutionProvider`, `ort::ep` (and `with_execution_providers`) outside `runtime.rs`, with a self-test of the token matcher (`test_sort_s1_token_scan_is_identifier_aware`). |
| n-4 stale text | Pointers to §10 added at the head of §2, the "both features" paragraph of §3 and §6. |
