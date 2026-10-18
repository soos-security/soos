# Releasing soos

Status: **no release has been published.** This page describes the release dry run that exists
today and the owner-only steps that a real release will add later (epic #361; ADR 2026-10-18
"Release Dry Run Publishes Nothing").

## 1. Release Dry Run (available now, publishes nothing)

`scripts/release_dry_run.sh` builds the release source files locally from the committed `HEAD`:

| File | Content |
|---|---|
| `soos-<version>.tar.gz` | `git archive` of `HEAD` under `soos-<version>/`, every mtime = the commit time, `gzip -n -9` |
| `soos-<version>-vendor.tar.zst` | `soos-<version>/vendor/` (`cargo vendor --locked`) and `soos-<version>/.cargo/config.toml` (vendored sources), sorted names, owner `0/0`, mtime = the commit time, `zstd -12` |
| `SHA256SUMS` | SHA-256 of both tarballs |

`<version>` is the `version` of `[workspace.package]` in `HEAD:Cargo.toml`; `SOURCE_DATE_EPOCH`
is `git log -1 --format=%ct HEAD`. Two runs on the same commit give byte-identical files.

```bash
scripts/release_dry_run.sh --dry-run          # print the plan, write nothing
scripts/release_dry_run.sh                    # fetch, vendor, build into target/release-dry-run/
scripts/release_dry_run.sh --offline          # same, without the fetch step (crates already cached)
scripts/release_dry_run.sh --out-dir DIR      # another output directory
scripts/release_dry_run.sh --check-vendor VENDOR_DIR CARGO_LOCK   # only the vendor check
```

- Only committed content is packed; uncommitted changes print a warning and are left out.
- The only network access is `cargo fetch --locked`; `cargo vendor` runs `--offline`.
- After vendoring, every `.cargo-checksum.json` package checksum is compared with `Cargo.lock`
  (same set, crates.io only); any difference fails the run.
- Exit codes: `0` success, `1` failure, `2` usage error.

Building from the two tarballs (what a distribution build farm does, without network):

```bash
tar -xzf soos-<version>.tar.gz
tar --zstd -xf soos-<version>-vendor.tar.zst
cd soos-<version>
ORT_SKIP_DOWNLOAD=1 cargo build --locked --offline --release --workspace --no-default-features --features system-onnxruntime
```

The build needs the distribution ONNX Runtime with its pkg-config file and Rust ≥ the declared
`rust-version` ([PACKAGING_AND_PROVISIONING.md](PACKAGING_AND_PROVISIONING.md) §7.5). Set
`RUSTUP_TOOLCHAIN` (or use the distribution `rustc`) when the pinned `rust-toolchain.toml`
toolchain is not installed.

CI runs `tests/docker/release_dry_run_test.sh` inside the `system-onnxruntime` job (advisory on
pull requests, required on `main`): two runs must be byte-identical, then the tarballs alone build
the release workspace in the clean Arch image under `--network none`, with no `.git` and no cargo
registry. Nothing is uploaded: the repository is public, so the dry run never creates a GitHub
Actions artifact, a tag or a release ([CI_CD_AND_SECURITY.md](CI_CD_AND_SECURITY.md)).

## 2. Owner-only Checklist (not automated, not done)

These steps need the owner's key, account or decision. No script or CI job in this repository
performs them, and the dry run must never be extended to do so without a new ADR.

| # | Step | Owner-only because | Status |
|---|---|---|---|
| 1 | Create the OpenPGP release key (offline primary key, signing subkey), publish its fingerprint (for example in `README.md`) | The key is the root of trust of every release | Owner-only, pending |
| 2 | Choose the first version (`0.1.0` or `1.0.0`, #361 Q6) and set `[workspace.package] version` | Product decision | Owner-only, pending |
| 3 | Run `scripts/release_dry_run.sh` on the release commit and check `SHA256SUMS` against a second machine | Verifies reproducibility outside CI | Owner-only, pending |
| 4 | Create the signed tag `v<version>` (`git tag -s`) with the release key and push it | Needs the key and write access | Owner-only, pending |
| 5 | Sign `SHA256SUMS` (detached OpenPGP signature `SHA256SUMS.asc`) | Needs the key | Owner-only, pending |
| 6 | Create the GitHub Release for the tag and upload both tarballs, `SHA256SUMS` and `SHA256SUMS.asc` | Publishing; the repository is public | Owner-only, pending |
| 7 | Point `packaging/arch/PKGBUILD` `source=()` (empty today) and the `Source0` of `packaging/rpm/soos.spec` at the published tarball URLs, with their checksums | Needs a published URL | Owner-only, pending |
| 8 | Submit to channels: AUR, Fedora COPR, Ubuntu PPA, openSUSE OBS (then official repositories) | Accounts and maintainer identity (`soos-security`, maintainer email) | Owner-only, pending |
| 9 | Decide the soos-models package (#361 Q3), the package split (Q5) and the app id / desktop file / AppStream metadata (Q2) | Product decisions | Owner-only, pending |

Never: a release from an unsigned tag, tarballs uploaded as CI artifacts, a key or passphrase in
the repository or in CI secrets without a new ADR, or a release whose `SHA256SUMS` differs from a
local dry run of the same commit.
