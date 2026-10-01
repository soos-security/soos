//! The rustup bootstrap never edits the operator's shell profiles (GitHub #287; matrix rows
//! SMI7–SMI9).
//!
//! `scripts/install_rustup.sh` passes `--no-modify-path` to `rustup-init`, exports the cargo
//! `bin` directory for its own subsequent steps (the post-install `rustc --version` check) and
//! prints the `export PATH=...` line for the operator instead. The manual digest bump procedure
//! for the pinned `rustup-init` SHA-256 digests is documented in the script and in
//! `Docs/CI_CD_AND_SECURITY.md`.
//!
//! The hermetic run stubs `curl`, `sha256sum` and `uname` and uses a scratch HOME, CARGO_HOME
//! and RUSTUP_HOME under the workspace `target/` directory: the real home directory is never
//! touched and nothing is downloaded.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contract tests utilize direct assertions, panics, and indexing"
)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SCRIPT: &str = "scripts/install_rustup.sh";

/// Shell start-up files that `rustup-init` edits unless `--no-modify-path` is given.
const PROFILE_FILES: [&str; 6] = [
    ".profile",
    ".bashrc",
    ".bash_profile",
    ".zshenv",
    ".zshrc",
    ".config/fish/conf.d/rustup.fish",
];

const PROFILE_SENTINEL: &str = "# operator profile, must stay byte-for-byte identical\n";

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read_repo(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn instructions(text: &str) -> String {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

fn shell_assignment(text: &str, name: &str) -> String {
    let prefix = format!("{name}=\"");
    text.lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix(&prefix)
                .and_then(|rest| rest.split('"').next())
                .map(str::to_string)
        })
        .unwrap_or_else(|| panic!("{name} missing"))
}

fn combined(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn write_exec(path: &Path, body: &str) {
    fs::write(path, body).expect("write stub");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod stub");
}

/// Scratch directory under the workspace `target/` (never the real home directory).
fn scratch(tag: &str) -> PathBuf {
    let dir = workspace_root()
        .join("target")
        .join(format!("soos_rustup_path_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

struct Run {
    work: PathBuf,
    home: PathBuf,
    cargo_bin: PathBuf,
    init_log: PathBuf,
    out: Output,
}

/// Runs the installer with stub tools whose `sha256sum` reports the pinned digest, so that the
/// stub `rustup-init` "download" is executed. The stub records its arguments and creates a
/// `rustc` that prints a version in `${CARGO_HOME:-$HOME/.cargo}/bin`.
fn run_success(tag: &str, set_cargo_home: bool) -> Run {
    let script = read_repo(SCRIPT);
    let digest = shell_assignment(&script, "RUSTUP_INIT_SHA256_X86_64");
    let toolchain = shell_assignment(&script, "PINNED_TOOLCHAIN");

    let work = scratch(tag);
    let bin = work.join("bin");
    let tmp = work.join("tmp");
    let home = work.join("home");
    for dir in [&bin, &tmp, &home, &home.join(".config/fish/conf.d")] {
        fs::create_dir_all(dir).expect("fixture dir");
    }
    for name in PROFILE_FILES {
        fs::write(home.join(name), PROFILE_SENTINEL).expect("profile fixture");
    }
    let init_log = work.join("rustup-init.args");

    let fake_init = format!(
        "#!/bin/sh\n\
         echo \"$@\" > '{log}'\n\
         bin_dir=\"${{CARGO_HOME:-$HOME/.cargo}}/bin\"\n\
         mkdir -p \"$bin_dir\"\n\
         printf '#!/bin/sh\\necho \"rustc {toolchain} (stub)\"\\n' > \"$bin_dir/rustc\"\n\
         chmod 0755 \"$bin_dir/rustc\"\n",
        log = init_log.display(),
    );
    let fake_init_path = work.join("fake-rustup-init");
    write_exec(&fake_init_path, &fake_init);

    let curl = format!(
        "#!/bin/sh\n\
         out=''\n\
         while [ $# -gt 0 ]; do\n\
           if [ \"$1\" = '-o' ]; then out=\"$2\"; shift; fi\n\
           shift\n\
         done\n\
         [ -n \"$out\" ] || exit 22\n\
         cp '{src}' \"$out\"\n",
        src = fake_init_path.display()
    );
    write_exec(&bin.join("curl"), &curl);
    write_exec(
        &bin.join("sha256sum"),
        &format!("#!/bin/sh\necho \"{digest}  $1\"\n"),
    );
    write_exec(&bin.join("uname"), "#!/bin/sh\necho x86_64\n");

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut cmd = Command::new("bash");
    cmd.arg(workspace_root().join(SCRIPT))
        .env("PATH", path)
        .env("TMPDIR", &tmp)
        .env("HOME", &home)
        .env("RUSTUP_HOME", work.join("rustup"));
    let cargo_home = if set_cargo_home {
        let cargo_home = work.join("cargo");
        cmd.env("CARGO_HOME", &cargo_home);
        cargo_home
    } else {
        cmd.env_remove("CARGO_HOME");
        home.join(".cargo")
    };
    let out = cmd.output().expect("execute install_rustup.sh");
    Run {
        work,
        home,
        cargo_bin: cargo_home.join("bin"),
        init_log,
        out,
    }
}

fn assert_profiles_untouched(run: &Run) {
    for name in PROFILE_FILES {
        assert_eq!(
            fs::read_to_string(run.home.join(name)).unwrap(),
            PROFILE_SENTINEL,
            "{name} must never be edited by the bootstrap"
        );
    }
}

/// SMI7: rustup-init is always invoked with `--no-modify-path` (static check of the script).
#[test]
fn test_rustup_installer_passes_no_modify_path() {
    let code = instructions(&read_repo(SCRIPT));
    let invocation = code
        .split("\"${workdir}/rustup-init\" -y")
        .nth(1)
        .expect("the installer runs the verified rustup-init with -y");
    let invocation: String = invocation
        .lines()
        .take_while(|l| !l.contains("|| die"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        invocation.contains("--no-modify-path"),
        "rustup-init must not edit shell profiles: {invocation}"
    );
}

/// SMI8: a verified install passes `--no-modify-path`, leaves every shell profile
/// byte-for-byte unchanged, exports the cargo bin directory for its own post-install check
/// and prints the PATH line for the operator, with and without an explicit CARGO_HOME.
#[test]
fn test_rustup_installer_leaves_profiles_untouched_and_prints_path() {
    for (tag, set_cargo_home) in [("cargo_home", true), ("default_home", false)] {
        let run = run_success(tag, set_cargo_home);
        let output = combined(&run.out);
        assert!(
            run.out.status.success(),
            "the stubbed install must succeed ({tag}); output:\n{output}"
        );
        let args = fs::read_to_string(&run.init_log).expect("rustup-init executed");
        for flag in ["-y", "--profile minimal", "--no-modify-path"] {
            assert!(args.contains(flag), "rustup-init args lack {flag}: {args}");
        }
        assert!(args.contains("--default-toolchain"), "{args}");
        assert_profiles_untouched(&run);
        assert!(
            output.contains("(stub)"),
            "the installer must run the installed rustc through the exported PATH; output:\n{output}"
        );
        let export_line = format!("export PATH=\"{}:$PATH\"", run.cargo_bin.display());
        assert!(
            output.contains(&export_line),
            "the installer must print `{export_line}`; output:\n{output}"
        );
        let _ = fs::remove_dir_all(&run.work);
    }
}

/// SMI9: the manual bump procedure of the pinned rustup-init digests is documented in the
/// script header and in `Docs/CI_CD_AND_SECURITY.md`, together with the PATH behaviour.
#[test]
fn test_rustup_digest_bump_procedure_is_documented() {
    let script = read_repo(SCRIPT);
    assert!(
        script.contains("Digest bump procedure"),
        "the script header must carry the digest bump procedure"
    );
    let docs = read_repo("Docs/CI_CD_AND_SECURITY.md");
    for needle in [
        "Digest bump procedure",
        "rustup-init.sha256",
        "--no-modify-path",
        "RUSTUP_INIT_SHA256_X86_64",
        "RUSTUP_INIT_SHA256_AARCH64",
    ] {
        assert!(
            docs.contains(needle),
            "Docs/CI_CD_AND_SECURITY.md must document {needle}"
        );
    }
}
