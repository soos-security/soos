# macOS Port — Bootstrap Specification for `soos-macos`

- **Status**: preparation only. No Mac is available yet; this repository stays Linux-only.
- **Decision**: ADR "[2026-10-13] macOS Port in a Separate Repository" (`AI/DECISIONS.md`), GitHub #360 (epic) and
  #367 (this first PR). Owner decisions of 2026-10-12 and 2026-10-13.
- **Spec**: `AI/architect_spec_macos_port_foundation.md`. Matrix rows `MAC1`–`MAC9` (`AI/VERIFICATION_MATRIX.md`).

## 1. Positioning and Security Tier

macOS soos is **RGB-only**: no Mac has an IR face camera (Apple Silicon Macs have no Face ID), and AVFoundation does not
reliably expose the IR stream of external UVC "Windows Hello" cameras. Liveness rests on the RGB PAD model, which is
weaker against photo and replay attacks than the Linux IR path. macOS soos is therefore a **convenience factor with
password fallback, never a password replacement**. Every login-window use stays opt-in.

Fail-closed on every platform: an error, panic or timeout never grants access. The OpenPAM module returns `PAM_IGNORE`;
an authorization mechanism denies or defers to the normal password prompt. Passwords, frames and embeddings never cross
the IPC channel and never reach a log (the macOS unified log, `os_log`, is persistent and readable by administrators).

## 2. Unlock Surfaces on macOS (research, with sources)

| Surface | Mechanism | soos plan |
|---|---|---|
| `sudo` (and possibly `su`, `screensharing`) | **OpenPAM** (`/etc/pam.d/*`). Since macOS 14 (Sonoma), `/etc/pam.d/sudo_local` is the Apple-supported place that survives updates; Apple's own `pam_tid.so` (Touch ID for sudo) is configured there | **Phase 1** target: `pam_soos.so` built for OpenPAM (different ABI from Linux-PAM), `PAM_IGNORE` on every failure |
| Login window and lock screen | Not PAM: the **Authorization Services** database (`system.login.console`, `system.login.screensaver`) and **SecurityAgent** authorization plugins in `/Library/Security/SecurityAgentPlugins` (used by XCreds, Jamf Connect). Lock-screen behaviour is unreliable on Sonoma and later; on macOS 26.1 SecurityAgent takes focus even without plugin UI. The plugin API is C/Objective-C, runs inside SecurityAgent and can lock the machine out | **Phase 3**, experimental, opt-in, with a documented recovery procedure (safe mode, `authorizationdb` restore) before anyone enables it |
| FileVault pre-boot unlock | Cannot be extended by third parties: the first login after boot is always the password | out of scope |
| Touch ID / **LocalAuthentication** (`LAContext`) | Only Apple-enrolled biometrics; no public API registers a third-party face template as a system biometric | only to gate soos's own actions (enrollment confirmation) |

Sources: [XCreds admin guide](https://twocanoes.com/knowledge-base/xcreds-admin-guide),
[XCreds security notes](https://twocanoes.com/knowledge-base/xcreds-security/),
[Apple Developer Forums, Security Interface](https://developer.apple.com/forums/tags/securityinterface).

## 3. Camera, TCC and the Capture-Agent Model

Camera access goes through AVFoundation (in Rust, `objc2-av-foundation`) and is gated by **TCC** (`kTCCServiceCamera`),
which also applies to root. The grant is attributed to the responsible signed app bundle, which must carry
`NSCameraUsageDescription` and, under the hardened runtime, the `com.apple.security.device.camera` entitlement. A bare
root `LaunchDaemon` binary gets no frames and no prompt
([Apple Developer Forums thread 654870](https://developer.apple.com/forums/thread/654870),
[macOS TCC overview](https://github.com/yo-yo-yo-jbo/macos_tcc),
[TCC and the platform sandbox policy](https://bdash.net.nz/posts/tcc-and-the-platform-sandbox-policy/)).

The Linux rule "the root daemon exclusively owns `/dev/video*`" therefore does not carry over: the port needs a signed
**capture agent** (app bundle or `LaunchAgent` in the user's GUI session) that holds the TCC grant, and a privileged
**verifier** behind an authenticated local channel. `SO_PEERCRED` does not exist in the same form; XPC with `SecCode`
code-signing requirement checks (or `LOCAL_PEERCRED` / `getpeereid` on a UDS) provides the kernel-backed peer identity.
These are decisions for ADRs in `soos-macos`.

## 4. Inference on macOS

`ort-sys` 2.0.0-rc.13 (the exact pin of this workspace) ships a prebuilt ONNX Runtime **only for
`aarch64-apple-darwin`**, and only as its `coreml` build: the CoreML execution provider is compiled in but
`soos-inference-ort` never registers it, so execution stays on the CPU provider exactly as on Linux (manifest, SHA-256
attestation and shape checks reused unchanged). There is no `x86_64-apple-darwin` prebuilt: Intel Macs would need a
source build of ONNX Runtime. Using CoreML is a separate decision for `soos-macos`, gated on producing the same
embeddings within tolerance as the CPU baseline.

## 5. Portable Crates

| Crate | Status | Evidence |
|---|---|---|
| `soos-frame` | portable (new pure crate: `Frame`, `PixelFormat`, `SensorType`) | `#![forbid(unsafe_code)]`, only dependency `zeroize` |
| `soos-protocol` | portable | serde, postcard, zeroize |
| `soos-policy` | portable | zero I/O |
| `soos-push-protocol` | portable | pure wire contract |
| `soos-inference-ort` | portable on Apple Silicon | `ort` CPU (§4) |
| `soos-vision` | portable | depends on `soos-frame`, no longer on `soos-camera-v4l` |
| `soos-biometric-store` | portable by inspection, compiled on macOS by CI; tests not yet run on macOS | only POSIX calls through `nix`/`libc`: `geteuid`, `flock` (`nix::fcntl::Flock`), `fstatat`, `unlinkat`, `O_NOFOLLOW`, `O_DIRECTORY`, `O_NONBLOCK`, `S_IFMT`/`S_IFREG`, `std::os::unix` metadata and permissions. Unverified on macOS: BSD group inheritance of new files, the `/var` → `/private/var` temp-dir symlink, `flock` on directories |
| `soos-pam`, `soos-daemon`, `soos-camera-v4l`, `soos-remote`, `soos-push-sender`, `soos-gui`, `soos-admin-cli`, `soos-enrollment-cli` | Linux-only by design or application crates | PAM ABI, logind/zbus, V4L2, systemd, desktop stacks |
| `soos-evidence-store` | not classified (not needed for phase 1) | same system-call profile as the biometric store |

`soos-test-fixtures` (dev-only) depends on `soos-frame` only, so the portable crates' tests build on macOS.

### 5.1 Dependency guard

`tests/invariants/src/portable_crates_contract.rs::test_portable_crates_have_no_linux_only_dependencies` runs
`cargo tree --locked -p <crate> -e normal,build --target all --all-features` for every portable crate and for
`soos-test-fixtures`, removes the queried root package, and fails if any of these packages is reached:

| Group | Forbidden packages |
|---|---|
| V4L2 | `v4l`, `v4l2-sys-mit`, `v4l-sys` |
| Linux-PAM ABI | `pam-bindings`, `pam-sys`, `pam` |
| D-Bus / logind | `zbus`, `zbus_macros`, `zbus_names`, `zvariant` |
| udev / systemd / inotify | `udev`, `libudev-sys`, `libsystemd`, `sd-notify`, `inotify`, `inotify-sys` |
| Async runtime | `tokio`, `tokio-macros` |
| Linux desktop session | `wayland-client`, `wayland-sys`, `x11rb`, `x11-dl` |
| Linux-only and application workspace crates | `soos-camera-v4l`, `soos-pam`, `soos-daemon`, `soos-remote`, `soos-push-sender`, `soos-gui`, `soos-admin-cli`, `soos-enrollment-cli`, `soos-test-fixtures` |

Not forbidden: `nix` and `libc` (portable POSIX bindings; compiling on macOS proves the used items exist),
`linux-raw-sys` (a `cfg(target_os = "linux")` backend of `rustix`), and per-target platform backends such as
`windows-sys`, `core-foundation` or `security-framework`. Tokio is forbidden as a policy: shared crates stay
runtime-free so a Tokio-free OpenPAM module can embed them. The guard is proven able to fail by a hermetic broken
manifest (a stub package named `v4l`) and by one witness per forbidden group in the Linux-only crates.

### 5.2 macOS CI job

Job `portable-macos`, named **"Portable crates (macOS, non-required)"**, on `macos-latest` (arm64; it must stay on an
arm64 image, §4): clippy `-D warnings` over the seven portable crates (`--all-targets --all-features`, which also
performs the full `cargo check`), then `cargo test` over the same set without `soos-biometric-store`. Same SHA-pinned
actions as the Linux jobs, no secrets, read-only token. It is not a `CI Success` dependency; `scripts/pr_loop.sh`
treats it as an advisory check through `scripts/pr_checks_gate.sh` (the other one is the system
ONNX Runtime job, GitHub #375, ADR 2026-10-13 "Distribution Builds: System ONNX Runtime").

**Remediation rule** if the job is red: fix portable production code. If `soos-biometric-store` does not compile on
macOS, classify it Linux-only, list the blocking calls here and remove it from the portable set in the same change,
rather than adding untested `cfg` code. Never edit an existing test.

## 6. Shared-Crate Tag Scheme

The shared crates are not published (`publish = false`). `soos-macos` uses git dependencies pinned to an immutable
annotated tag of `soos-security/soos`, never a branch:

```toml
[workspace.dependencies]
soos-vision = { git = "https://github.com/soos-security/soos", tag = "shared-v0.1.0" }
soos-protocol = { git = "https://github.com/soos-security/soos", tag = "shared-v0.1.0" }
```

- Tag format `shared-v<MAJOR>.<MINOR>.<PATCH>`, created on `main` by the owner when `soos-macos` needs a new shared
  version; a breaking change to a portable crate's public API bumps `MAJOR` (or `MINOR` while `0.x`).
- `soos-macos` commits its `Cargo.lock`, which records the resolved commit.
- crates.io publication is deferred and needs its own ADR.

## 7. Suggested Repository

- Name `soos-security/soos-macos`, **private** at first, licence **AGPL-3.0-or-later** (it links AGPL code; any
  distribution carries the AGPL obligations), maintainer identity `soos-security`.
- Same Conventional Commits hooks, the same 4-phase TDD workflow and `AI/` + `Docs/` structure, English only; copy
  `deny.toml` (permissive licence allow-list) and run `cargo-deny`.

```
soos-macos/
  Cargo.toml                 # workspace; git deps on soos shared crates @ shared-v tag
  crates/
    capture-avf/             # AVFoundation capture (objc2), signed agent side
    verifier/                # privileged verifier: vision + policy + store
    xpc-transport/           # XPC with code-signing requirement checks (SO_PEERCRED analogue)
    pam-openpam/             # pam_soos.so for sudo_local (phase 1 target)
    authz-plugin/            # SecurityAgent mechanism (phase 3, experimental, opt-in)
    gui/                     # menu-bar / enrollment app (shares the soos visual identity)
  packaging/                 # .pkg, LaunchDaemon/LaunchAgent plists, entitlements
  .github/workflows/         # arm64 macOS runners; sign + notarize on tags only (when distribution is decided)
```

## 8. Phased Scope

1. **Phase 1**: enrollment and `sudo` through `sudo_local`, opt-in only, always with password fallback.
2. **Phase 2**: menu-bar app (`gui`); soos-remote status client only if the owner decides Mac support (deferred).
3. **Phase 3**: experimental login/lock-screen authorization plugin (`authz-plugin`), explicit opt-in, documented
   recovery procedure first.

## 9. Signing and Distribution

- **Now**: ad-hoc signing (`codesign -s -`) for the owner's own Mac; no Apple Developer Program, no notarization.
- **Only if distribution is decided** (owner action): Apple Developer Program ($99/yr), Developer ID signing, hardened
  runtime, notarization with `notarytool`. Signing secrets (Developer ID certificate, notarytool API key) live only in
  GitHub encrypted secrets of `soos-macos`; the notarize job runs only on tags. An authorization plugin or a PAM module
  must be signed, or SecurityAgent and `sudo` (library validation) refuse to load it.

## 10. Owner checklist

- [ ] Create the private repository `soos-security/soos-macos` (AGPL-3.0-or-later).
- [ ] Enable branch protection and install the Conventional Commits hooks.
- [ ] Create the first shared tag (`shared-v0.1.0`) on `main` once this PR is merged.
- [ ] Enrol in the Apple Developer Program (only when distribution is wanted).
- [ ] Add signing secrets to `soos-macos` (only then).
- [ ] Add the README link "macOS: experimental, separate repository" once the repository exists.

## 11. Deferred Decisions

- crates.io publication of the shared crates (own ADR).
- soos-remote Mac support (logind lock/unlock has no macOS equivalent).
- The reference Mac model and macOS version for physical validation rows.
- The CoreML execution provider (an ADR of `soos-macos`).
- Running the `soos-biometric-store` tests on macOS (row `MAC8`).
