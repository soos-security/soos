use std::env;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = match env::var("OUT_DIR") {
        Ok(val) => PathBuf::from(val),
        Err(_) => return,
    };

    let candidate_dirs = [
        "/lib/x86_64-linux-gnu",
        "/usr/lib/x86_64-linux-gnu",
        "/lib/aarch64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/lib64",
        "/usr/lib64",
        "/usr/lib",
        "/lib",
    ];

    // First check if libpam.so already exists in standard search paths
    for dir in &candidate_dirs {
        let p = Path::new(dir).join("libpam.so");
        if p.exists() {
            // Re-run if the development symlink disappears (GitHub #264).
            println!("cargo:rerun-if-changed={}", p.display());
            return;
        }
    }

    // If libpam.so is missing (e.g. libpam0g-dev not installed), check for libpam.so.0
    for dir in &candidate_dirs {
        let soname = Path::new(dir).join("libpam.so.0");
        if soname.exists() {
            let target = out_dir.join("libpam.so");
            if !target.exists() {
                let _ = symlink(&soname, &target);
            }
            println!("cargo:rustc-link-search=native={}", out_dir.display());
            // Re-run when the runtime library changes or when a development `libpam.so`
            // appears in this directory (e.g. libpam0g-dev installed later), so the
            // OUT_DIR fallback symlink stops being used (GitHub #264). Cargo tracks the
            // modification times under a directory path.
            println!("cargo:rerun-if-changed={}", soname.display());
            println!("cargo:rerun-if-changed={dir}");
            break;
        }
    }
}
