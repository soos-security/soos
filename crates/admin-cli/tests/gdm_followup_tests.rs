//! Contract tests for the `soos-admin gdm enable` follow-ups of GitHub #278
//! (candid review round 3 of the P1 storage/vision batch):
//!
//! * a delegated gate is never dropped as a duplicate when a `[...=N]` jump of the
//!   edited file can skip the earlier copy (matrix SFU1);
//! * an include target that is a FIFO (or any non-regular file) is refused at once
//!   instead of blocking `gdm enable` forever (matrix SFU2), while a symlinked
//!   regular include target (Fedora authselect layout) is still followed (SFU3).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite uses direct assertions and unwrap"
)]

use soos_admin_cli::args::GdmAction;
use soos_admin_cli::gdm::{configure_gdm, pam_backup_path, GDM_BLOCK_BEGIN, GDM_BLOCK_END};
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;
use tempfile::tempdir;

/// Upper bound for a `gdm enable` run on a tiny fixture; a blocked open never returns.
const ENABLE_DEADLINE: Duration = Duration::from_secs(10);

struct Fixture {
    _dir: tempfile::TempDir,
    pam_dir: PathBuf,
    pam_file: PathBuf,
    disable_file: PathBuf,
}

fn fixture(gdm_password: &str, shared: &[(&str, &str)]) -> Fixture {
    let dir = tempdir().unwrap();
    let pam_dir = dir.path().join("pam.d");
    fs::create_dir(&pam_dir).unwrap();
    for (name, content) in shared {
        fs::write(pam_dir.join(name), content).unwrap();
    }
    let pam_file = pam_dir.join("gdm-password");
    fs::write(&pam_file, gdm_password).unwrap();
    let disable_file = dir.path().join("soos").join("gdm.disable");
    Fixture {
        _dir: dir,
        pam_dir,
        pam_file,
        disable_file,
    }
}

/// Text between the managed block markers.
fn managed_block(content: &str) -> &str {
    let begin = content.find(GDM_BLOCK_BEGIN).expect("managed block begin");
    let end = content.find(GDM_BLOCK_END).expect("managed block end");
    &content[begin..end]
}

/// SFU1: `pam_succeed_if ... ingroup vip` jumps over the in-file `requisite
/// pam_nologin.so`; the delegated copy must stay in the managed block, otherwise a
/// VIP face success skips nologin entirely.
#[test]
fn test_gdm_enable_keeps_a_duplicate_gate_when_a_jump_can_skip_the_earlier_copy() {
    let original = "#%PAM-1.0\n\
auth [success=1 default=ignore] pam_succeed_if.so user ingroup vip\n\
auth requisite pam_nologin.so\n\
auth required pam_env.so\n\
@include common-auth\n";
    let f = fixture(
        original,
        &[(
            "common-auth",
            "auth requisite pam_nologin.so\nauth [success=1 default=ignore] pam_unix.so nullok\nauth requisite pam_deny.so\nauth required pam_permit.so\n",
        )],
    );
    configure_gdm(&GdmAction::Enable, &f.pam_file, &f.disable_file).expect("enable must succeed");
    let content = fs::read_to_string(&f.pam_file).unwrap();
    let block = managed_block(&content);
    assert!(
        block.contains("pam_nologin.so"),
        "a jump can skip the earlier nologin, so the delegated gate must be copied:\n{content}"
    );
    let block_at = content.find(GDM_BLOCK_BEGIN).unwrap();
    let soos_at = content.find("pam_soos.so").unwrap();
    let nologin_in_block = block_at + block.find("pam_nologin.so").unwrap();
    assert!(
        nologin_in_block < soos_at,
        "the copied gate runs before pam_soos.so:\n{content}"
    );
}

/// SFU2: a FIFO include target used to block `open(2)` forever (no writer). It must
/// be refused promptly, with nothing written.
#[test]
fn test_gdm_enable_refuses_a_fifo_include_target_without_blocking() {
    let original = "#%PAM-1.0\nauth requisite pam_nologin.so\n@include common-auth\n";
    let f = fixture(original, &[]);
    let fifo = f.pam_dir.join("common-auth");
    nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();

    let (tx, rx) = mpsc::channel();
    let pam_file = f.pam_file.clone();
    let disable_file = f.disable_file.clone();
    std::thread::spawn(move || {
        let result = configure_gdm(&GdmAction::Enable, &pam_file, &disable_file);
        let _ = tx.send(result.map_err(|e| e.to_string()));
    });
    let outcome = rx.recv_timeout(ENABLE_DEADLINE);
    if outcome.is_err() {
        // Release the blocked reader so the worker thread can finish.
        let _ = fs::OpenOptions::new().write(true).open(&fifo);
        panic!("gdm enable blocked on a FIFO include target for {ENABLE_DEADLINE:?}");
    }
    let err = outcome
        .unwrap()
        .expect_err("a FIFO include target must be refused");
    assert!(
        err.contains("common-auth") && err.contains("not a regular file"),
        "the error names the target and why it is refused: {err}"
    );
    assert_eq!(fs::read_to_string(&f.pam_file).unwrap(), original);
    assert!(
        !pam_backup_path(&f.pam_file).exists(),
        "no backup on refusal"
    );
}

/// SFU3: authselect ships `system-auth` as a symlink to `/etc/authselect/...`; the
/// descriptor check must still follow a symlink to a regular file.
#[test]
fn test_gdm_enable_still_follows_a_symlinked_regular_include_target() {
    let original = "#%PAM-1.0\nauth requisite pam_nologin.so\nauth substack system-auth\n";
    let f = fixture(original, &[]);
    let real = f.pam_dir.parent().unwrap().join("authselect-system-auth");
    fs::write(
        &real,
        "auth required pam_faillock.so preauth silent\nauth sufficient pam_unix.so nullok\nauth required pam_deny.so\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(&real, f.pam_dir.join("system-auth")).unwrap();
    configure_gdm(&GdmAction::Enable, &f.pam_file, &f.disable_file).expect("enable must succeed");
    let content = fs::read_to_string(&f.pam_file).unwrap();
    assert!(
        managed_block(&content).contains("pam_faillock.so preauth silent"),
        "the gate of the symlinked stack is copied:\n{content}"
    );
}
