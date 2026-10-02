//! Build script of `soos-gui` (GitHub #318): derives the `soos-enroll` path of the privileged
//! actions from the build-time `SOOS_BINDIR` (exported by `scripts/install.sh --build` as
//! `<prefix>/bin`). Unset or `/usr/bin`: the default constant `/usr/bin/soos-enroll` is used.
//! Any other value must pass [`bindir::validate_bindir`]; an invalid value fails the build.

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stdout,
    reason = "cargo build scripts communicate with cargo through stdout directives"
)]

#[path = "build_support/bindir.rs"]
#[allow(
    dead_code,
    reason = "enroll_program_for is used by the GUI contract tests only"
)]
mod bindir;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=build_support/bindir.rs");
    println!("cargo::rerun-if-env-changed=SOOS_BINDIR");
    println!("cargo::rustc-check-cfg=cfg(soos_custom_bindir)");

    let Some(value) = std::env::var_os("SOOS_BINDIR") else {
        return;
    };
    let Some(dir) = value.to_str() else {
        println!("cargo::error=SOOS_BINDIR is not valid UTF-8");
        return;
    };
    if let Err(reason) = bindir::validate_bindir(dir) {
        println!("cargo::error={reason} (got {dir:?})");
        return;
    }
    if dir != bindir::DEFAULT_BINDIR {
        println!("cargo::rustc-cfg=soos_custom_bindir");
    }
}
