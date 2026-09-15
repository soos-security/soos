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
    let mut pam_so_found = false;
    for dir in &candidate_dirs {
        let p = Path::new(dir).join("libpam.so");
        if p.exists() {
            pam_so_found = true;
            break;
        }
    }

    if pam_so_found {
        return;
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
            break;
        }
    }
}
