# Walkthrough 271 — CI Unblock: Full Arch Upgrade in the Package Matrix and `p256 0.14`

- **Date**: 2026-11-20
- **Issue**: GitHub #505 (RUSTSEC-2026-0387), refs #477 (security review tracking); no backlog
  id — **Branch**: `fix/ci-unblock`
- **Matrix criteria**: SDP30 (new), RMC39 (amended)
- **ADR**: "[2026-11-20] `p256 0.14` Migration for RUSTSEC-2026-0387" (amends item (8) of the
  2026-10-06 passkey ADR)

## 1. Context & Objectives

Two required checks were red on `main`, which blocked every pull request:

1. `setup.sh package matrix (Docker)`: on the digest-pinned `archlinux` image the `companion`
   group install failed with "failed to prepare transaction (could not satisfy dependencies)",
   although every package name (`jq`, `bind`, `curl`, `ca-certificates`) resolved.
2. `Security (cargo-deny)`: RUSTSEC-2026-0387 (non-constant-time operations in
   `crypto-bigint`), reached through `p256 0.13.2` in `soos-remote`.

## 2. Arch Package Matrix

### Root cause

Reproduced in the pinned image. The harness ran `pacman -Sy` (refresh only) and then
`pacman -S --needed jq bind curl ca-certificates`. Arch has moved to OpenSSL 4.1
(`libcrypto.so=4`); installing the current `bind`/`curl` pulls `openssl 4.1.0`, which breaks
`libcrypto.so=3-64` / `libssl.so=3-64` still required by the image's own `pacman`,
`libarchive`, `cryptsetup`, `libsasl` and `libngtcp2`. This is an Arch partial upgrade: the
package list of `scripts/check_build_deps.sh` is correct, and a user on an up-to-date Arch
system is not affected (`setup.sh` already prints `pacman --sync --refresh --sysupgrade` as its
refresh hint). The defect is in the harness only.

### Change

- `tests/docker/setup_packages_test.sh`: the Arch refresh is `pacman -Syu --noconfirm`, like
  `tests/docker/Dockerfile.arch` and `tests/docker/Dockerfile.system-ort`. The image digest
  stays pinned (invariant SDP22); only the package state is brought current.
- New invariant SDP30 (`test_sdp30_arch_matrix_never_partially_upgrades`): the script contains
  the full upgrade, and no `pacman` line refreshes without upgrading, whatever the spelling
  (`-Sy`, `-yS`, `--sync --refresh` without `--sysupgrade`; the classifier is checked on these
  spellings first).
- Docs: `Docs/CI_CD_AND_SECURITY.md` (job table) and `Docs/PACKAGING_AND_PROVISIONING.md`
  (preflight dependency matrix, range SDP1–SDP30).

## 3. `p256 0.14` (RUSTSEC-2026-0387)

The only patched release is `crypto-bigint >= 0.7.6`, which needs `p256 0.14.0`
(`elliptic-curve 0.14`, `ecdsa 0.17`, `signature 3`, `sha2 0.11`, `rand_core 0.10`; MSRV 1.87,
within the workspace 1.92). The owner approved the migration and the related test edits on
2026-11-20. No advisory exception was added: `[advisories] ignore = []` stays empty.

- **Workspace**: `p256 = { version = "0.14.0", default-features = false, features = ["ecdsa"] }`;
  the lock now has `crypto-bigint 0.7.11`, and `der 0.7.10` is gone.
- **Code** (`crates/remote/src/webpush.rs` only): the SEC1 encoder is
  `ToSec1Point::to_sec1_point` (the renamed `to_encoded_point`). `hmac_sha256` used to wrap
  the HMAC output in `Zeroizing`, which needed the `generic-array/zeroize` feature that
  `elliptic-curve 0.13` turned on; nothing enables it now, so the buffer is copied out and
  then wiped with an explicit slice `zeroize()`. WebAuthn verification (`webauthn.rs`,
  `credentials.rs`) and the RFC 8291 ECDH compile unchanged.
- **`deny.toml`**: the stale `der@0.7.10` skip is removed. `hmac 0.12` (pinned by RMC-S33) and
  the workspace `sha2 0.10` stay, so the RustCrypto `digest 0.10` and `0.11` generations
  coexist: new skips for `block-buffer@0.12.1`, `cpufeatures@0.3.1`, `crypto-common@0.2.2`,
  `digest@0.11.3`, `hmac@0.13.0`, `rand_core@0.10.1` and `sha2@0.11.1`, with reasons taken
  from `cargo tree -i <crate>@<version> --depth 1`; `rand_core` (0.6.4, 0.9.5, 0.10.1) joins the
  list of crates locked in three versions.

### Owner-approved test edits (2026-11-20)

- `tests/invariants/src/remote_passkey_contract.rs` Test 46 (RMC-S13) and
  `tests/invariants/src/remote_push_contract.rs` Test 43 (RMC-S33): the pinned `p256` line reads
  `0.14.0` instead of `0.13.2`.
- `tests/invariants/src/remote_passkey_contract.rs` Test 47 (RMC-S14): inverted and renamed
  `test_rmc_s14_no_stale_der_skip_remains` (no `der@0.7.10` skip may remain); the old form
  (exactly one such skip) contradicted `test_deny_skip_entries_are_locked_versions` once
  `der 0.7` left the lock.
- `crates/remote/tests/common/passkey.rs` and `crates/remote/tests/common/push.rs`: call
  `to_sec1_point` instead of `to_encoded_point` (setup only, no assertion changed).

## 4. Verification

- SDP30 fails against the old `pacman -Sy` line and passes with `pacman -Syu`.
- The Arch container of the matrix passes end to end in the pinned image: every package name
  resolves, the `companion` and `android` groups install, and `jq`, `dig`, `curl`, `java`,
  `unzip` resolve.
- Every `soos-remote` test passes unchanged, including the RFC 8291 Appendix A vector, the
  WebAuthn known-answer and high-S tests, and the VAPID signature checks.
- `cargo deny --locked check`: `advisories ok, bans ok, licenses ok, sources ok`.
- `cargo fmt`, `cargo clippy -D warnings`, the workspace tests and the candid review gate
  (through `./save.sh`).
