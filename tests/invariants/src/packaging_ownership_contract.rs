//! Packaging ownership, Docker workspace isolation and Arch PAM stack contract
//! (GitHub #301, #308, #309, #316; matrix rows POA1–POA12, walkthrough 166).
//!
//! - #301 (ONB-NEW-1): every entry of the Arch package archive is owned by `root:root` (uid and
//!   gid 0, names `root`) on both compression paths, whoever builds it; the `.deb` builder keeps
//!   `--root-owner-group`.
//! - #308 (TCI-NEW-1): every read-write `/workspace` bind mount of a Docker harness or CI job also
//!   overlays `/workspace/target` with a named Docker volume, and `install.sh --build` never
//!   compiles as root inside a checkout owned by another user.
//! - #309 (ONB-NEW-2): `packaging/pam/arch/system-auth` is the stock pambase stack with the exact
//!   soos edit (jumps adjusted, the event line only after a failed `pam_unix`, faillock kept), and
//!   the snippet and `Docs/DISTRIBUTION_DEPLOYMENT.md` §5.2 show that same edit.
//! - #316: packages stage the unit under `/usr/lib/systemd/system` and never ship `/run`
//!   (ONB-NEW-3), the Debian pam-configs are `Default: no` and the `.deb` enables them explicitly
//!   (ONB-NEW-4), and the Docker base images are pinned by digest (TCI-NEW-3).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract tests utilize direct assertions, panics, and indexing"
)]

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let path = workspace_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

pub(crate) fn combined(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn have(tool: &str) -> bool {
    Command::new("bash")
        .arg("-c")
        .arg(format!("command -v {tool} >/dev/null 2>&1"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Minimal ustar / pax reader (no external crate): ownership of every entry.
// ---------------------------------------------------------------------------

/// Ownership metadata of one archive entry.
#[derive(Debug)]
struct TarEntry {
    name: String,
    uid: u64,
    gid: u64,
    uname: String,
    gname: String,
}

fn octal_field(field: &[u8]) -> u64 {
    let text: String = field
        .iter()
        .take_while(|b| **b != 0)
        .map(|b| char::from(*b))
        .collect();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return 0;
    }
    u64::from_str_radix(trimmed, 8).unwrap_or_else(|e| panic!("bad octal field {trimmed:?}: {e}"))
}

fn str_field(field: &[u8]) -> String {
    field
        .iter()
        .take_while(|b| **b != 0)
        .map(|b| char::from(*b))
        .collect()
}

/// Parses `len key=value\n` pax records into (key, value) pairs.
fn pax_records(data: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(data);
    let mut out = Vec::new();
    for record in text.split('\n') {
        let Some((_, kv)) = record.split_once(' ') else {
            continue;
        };
        if let Some((k, v)) = kv.split_once('=') {
            out.push((k.to_string(), v.to_string()));
        }
    }
    out
}

/// Every regular entry of an uncompressed tar stream, with pax overrides applied.
fn tar_entries(tar: &[u8]) -> Vec<TarEntry> {
    let mut entries = Vec::new();
    let mut offset = 0usize;
    let mut pending_pax: Vec<(String, String)> = Vec::new();
    while offset + 512 <= tar.len() {
        let header = &tar[offset..offset + 512];
        if header.iter().all(|b| *b == 0) {
            break;
        }
        let size = usize::try_from(octal_field(&header[124..136])).expect("entry size");
        let typeflag = header[156];
        let data_start = offset + 512;
        let data_end = data_start + size;
        assert!(data_end <= tar.len(), "truncated tar entry at {offset}");
        match typeflag {
            b'x' => pending_pax = pax_records(&tar[data_start..data_end]),
            b'g' | b'L' | b'K' => {}
            _ => {
                let prefix = str_field(&header[345..500]);
                let base = str_field(&header[0..100]);
                let mut entry = TarEntry {
                    name: if prefix.is_empty() {
                        base
                    } else {
                        format!("{prefix}/{base}")
                    },
                    uid: octal_field(&header[108..116]),
                    gid: octal_field(&header[116..124]),
                    uname: str_field(&header[265..297]),
                    gname: str_field(&header[297..329]),
                };
                for (k, v) in pending_pax.drain(..) {
                    match k.as_str() {
                        "uid" => entry.uid = v.parse().expect("pax uid"),
                        "gid" => entry.gid = v.parse().expect("pax gid"),
                        "uname" => entry.uname = v,
                        "gname" => entry.gname = v,
                        "path" => entry.name = v,
                        _ => {}
                    }
                }
                entries.push(entry);
            }
        }
        offset = data_start + size.div_ceil(512) * 512;
    }
    entries
}

pub(crate) fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soos_poa_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Stub release artifacts in `<target>/release` (what `install.sh` reads by default).
fn stage_target_dir(target: &Path) {
    crate::installer_contract::stage_fixture_artifacts(&target.join("release"));
}

/// Builds the Arch package from stub artifacts and returns the uncompressed archive bytes.
fn build_arch_archive(tag: &str, tar_bin: &str, compress: &str) -> Vec<u8> {
    let target = scratch(&format!("{tag}_target"));
    stage_target_dir(&target);
    let out_dir = scratch(&format!("{tag}_out"));
    let out = Command::new("bash")
        .arg(workspace_root().join("scripts/build_arch.sh"))
        .args([
            "--skip-build",
            "--tar",
            tar_bin,
            "--compress",
            compress,
            "-o",
        ])
        .arg(&out_dir)
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .expect("run build_arch.sh");
    assert!(
        out.status.success(),
        "build_arch.sh --tar {tar_bin} --compress {compress} failed:\n{}",
        combined(&out)
    );
    let pkg = fs::read_dir(&out_dir)
        .expect("list output dir")
        .flatten()
        .map(|e| e.path())
        .find(|p| p.to_string_lossy().contains(".pkg.tar."))
        .unwrap_or_else(|| panic!("no package written by build_arch.sh:\n{}", combined(&out)));
    let decompressor = if compress == "zstd" { "zstd" } else { "gzip" };
    let raw = Command::new(decompressor)
        .arg("-dc")
        .arg(&pkg)
        .output()
        .expect("decompress package");
    assert!(raw.status.success(), "{decompressor} -dc failed");
    let _ = fs::remove_dir_all(&target);
    let _ = fs::remove_dir_all(&out_dir);
    raw.stdout
}

/// POA1 — #301: a package built by a non-root user ships only `root:root` entries, on the
/// zstd and on the gzip path, with GNU tar and with bsdtar (whichever the host provides).
#[test]
fn test_arch_package_archive_entries_are_root_owned_on_every_archive_path() {
    let mut tars: Vec<&str> = ["bsdtar", "tar"].into_iter().filter(|t| have(t)).collect();
    tars.dedup();
    assert!(!tars.is_empty(), "neither bsdtar nor tar is available");
    let mut compressions = vec!["gzip"];
    if have("zstd") {
        compressions.push("zstd");
    }
    let mut checked = 0;
    for tar_bin in &tars {
        for compress in &compressions {
            let archive =
                build_arch_archive(&format!("arch_{tar_bin}_{compress}"), tar_bin, compress);
            let entries = tar_entries(&archive);
            for required in [
                ".PKGINFO",
                "usr/libexec/soos/soos-daemon",
                "usr/lib/security/pam_soos.so",
                "usr/libexec/soos/provision-master-key",
            ] {
                assert!(
                    entries
                        .iter()
                        .any(|e| e.name.trim_start_matches("./") == required),
                    "{tar_bin}/{compress}: archive lacks {required}"
                );
            }
            let foreign: Vec<String> = entries
                .iter()
                .filter(|e| {
                    e.uid != 0
                        || e.gid != 0
                        || !(e.uname.is_empty() || e.uname == "root")
                        || !(e.gname.is_empty() || e.gname == "root")
                })
                .map(|e| format!("{} {}:{} ({}/{})", e.name, e.uid, e.gid, e.uname, e.gname))
                .collect();
            assert!(
                foreign.is_empty(),
                "{tar_bin}/{compress}: archive entries not owned by root:root \
                 (pacman extracts these owners):\n{}",
                foreign.join("\n")
            );
            checked += 1;
        }
    }
    assert!(checked >= 1);
}

/// POA2 — #301: every archive command of `build_arch.sh` forces owner 0 for both tar
/// implementations, and the `.deb` builder keeps `--root-owner-group`.
#[test]
fn test_package_builders_force_root_ownership_statically() {
    let arch = read("scripts/build_arch.sh");
    for flag in [
        "--owner=root:0",
        "--group=root:0",
        "--uid 0",
        "--gid 0",
        "--uname root",
        "--gname root",
    ] {
        assert!(
            arch.contains(flag),
            "build_arch.sh must pass {flag} to its tar implementation"
        );
    }
    let archive_lines: Vec<&str> = arch
        .lines()
        .filter(|l| l.contains("${TAR_BIN}") && l.contains(" -c"))
        .collect();
    assert!(
        archive_lines.len() >= 2,
        "build_arch.sh must have a zstd and a gzip archive command"
    );
    for line in archive_lines {
        assert!(
            line.contains("\"${TAR_OWNER_FLAGS[@]}\""),
            "every build_arch.sh archive command must carry the owner flags: {line}"
        );
    }
    assert!(
        read("scripts/build_deb.sh").contains("dpkg-deb --build --root-owner-group"),
        "build_deb.sh must build with --root-owner-group"
    );
}

// ---------------------------------------------------------------------------
// #308 — Docker workspace mounts
// ---------------------------------------------------------------------------

/// Logical lines of a shell script or workflow: backslash continuations are joined.
fn logical_lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        let trimmed = line.trim_end();
        if let Some(body) = trimmed.strip_suffix('\\') {
            current.push_str(body);
            current.push(' ');
        } else {
            current.push_str(trimmed);
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Kinds of `/workspace` mounts found on a logical line.
fn workspace_mounts(line: &str) -> (bool, Vec<String>) {
    let mut read_write = false;
    let mut target_sources = Vec::new();
    let mut rest = line;
    while let Some(at) = rest.find(":/workspace") {
        let after = &rest[at + ":/workspace".len()..];
        let before = &rest[..at];
        if let Some(tail) = after.strip_prefix("/target") {
            if tail.is_empty() || tail.starts_with([' ', '"', '\'', ':']) {
                let source = before
                    .rsplit([' ', '\t'])
                    .next()
                    .unwrap_or("")
                    .trim_matches(['"', '\''])
                    .to_string();
                target_sources.push(source);
            }
        } else if after.is_empty()
            || after.starts_with([' ', '"', '\''])
            || after.starts_with(":rw")
        {
            read_write = true;
        }
        rest = after;
    }
    (read_write, target_sources)
}

/// POA3 — #308: every read-write `/workspace` mount also mounts a named Docker volume on
/// `/workspace/target`, so no container ever writes root-owned build output into the host
/// checkout.
#[test]
fn test_read_write_workspace_mounts_overlay_target_with_a_volume() {
    let root = workspace_root();
    let mut files: Vec<PathBuf> = vec![root.join("run_tests.sh"), root.join("save.sh")];
    for dir in [
        "tests/docker",
        "tests/distro",
        "scripts",
        ".github/workflows",
    ] {
        for entry in fs::read_dir(root.join(dir)).expect("read dir").flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "sh" || ext == "yml" || ext == "yaml" {
                files.push(path);
            }
        }
    }
    let mut checked = 0;
    let mut violations = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).expect("read file");
        for line in logical_lines(&text) {
            let (read_write, target_sources) = workspace_mounts(&line);
            if !read_write {
                continue;
            }
            checked += 1;
            let volume = target_sources
                .iter()
                .any(|s| !s.is_empty() && !s.starts_with('/') && !s.starts_with('$'));
            if !volume {
                violations.push(format!(
                    "{}: {}",
                    file.strip_prefix(&root).unwrap_or(&file).display(),
                    line.trim()
                ));
            }
        }
    }
    assert!(
        checked >= 5,
        "expected the read-write workspace mounts of run_tests.sh, run_matrix.sh, \
         run_distro_validation.sh and ci.yml (found {checked})"
    );
    assert!(
        violations.is_empty(),
        "read-write /workspace mounts without a named volume on /workspace/target \
         (root-owned files would land in the host target/):\n{}",
        violations.join("\n")
    );
}

/// Shim directory: `id -u` reports root, `runuser`/`cargo` record their arguments, `getent`
/// resolves uid 4242 to `pkexecuser`.
pub(crate) fn root_shims(dir: &Path, log: &Path) {
    let write = |name: &str, body: String| {
        let path = dir.join(name);
        fs::write(&path, body).expect("write shim");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod shim");
    };
    let log = log.display();
    write(
        "id",
        "#!/bin/bash\nif [[ \"$1\" == \"-u\" && $# -eq 1 ]]; then echo 0; exit 0; fi\nexec /usr/bin/id \"$@\"\n"
            .to_string(),
    );
    write(
        "runuser",
        format!("#!/bin/bash\necho \"runuser $*\" >> \"{log}\"\nexit 0\n"),
    );
    write(
        "cargo",
        format!("#!/bin/bash\necho \"cargo $*\" >> \"{log}\"\nexit 0\n"),
    );
    write(
        "getent",
        "#!/bin/bash\nif [[ \"$1\" == \"passwd\" && \"$2\" == \"4242\" ]]; then echo 'pkexecuser:x:4242:4242::/home/pkexecuser:/bin/bash'; exit 0; fi\nexec /usr/bin/getent \"$@\"\n"
            .to_string(),
    );
}

fn run_install_build_as_fake_root(tag: &str, env: &[(&str, &str)]) -> (Output, String) {
    let shims = scratch(&format!("{tag}_shims"));
    let log = shims.join("calls.log");
    root_shims(&shims, &log);
    let stage = scratch(&format!("{tag}_stage"));
    let target = scratch(&format!("{tag}_target"));
    let path = format!(
        "{}:{}",
        shims.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut cmd = Command::new("bash");
    cmd.arg(workspace_root().join("scripts/install.sh"))
        .args([
            "--build",
            "--skip-models",
            "--skip-systemd",
            "--distro",
            "none",
        ])
        .arg("--destdir")
        .arg(&stage)
        .env("PATH", path)
        .env("CARGO_TARGET_DIR", &target)
        .env_remove("SUDO_USER")
        .env_remove("DOAS_USER")
        .env_remove("PKEXEC_UID");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run install.sh --build");
    let calls = fs::read_to_string(&log).unwrap_or_default();
    let _ = fs::remove_dir_all(&shims);
    let _ = fs::remove_dir_all(&stage);
    let _ = fs::remove_dir_all(&target);
    (out, calls)
}

/// POA4 — #308: `install.sh --build` run as root builds as the invoking user for sudo, doas and
/// pkexec, and refuses to compile as root in a checkout owned by another user.
#[test]
fn test_install_build_as_root_drops_to_invoking_user_or_refuses() {
    for (label, var, value, user) in [
        ("sudo", "SUDO_USER", "sudouser", "sudouser"),
        ("doas", "DOAS_USER", "doasuser", "doasuser"),
        ("pkexec", "PKEXEC_UID", "4242", "pkexecuser"),
    ] {
        let (out, calls) =
            run_install_build_as_fake_root(&format!("build_{label}"), &[(var, value)]);
        assert!(
            calls.contains(&format!("runuser -u {user} --")),
            "{label}: install.sh --build as root must build as '{user}' through runuser \
             (calls: {calls:?}):\n{}",
            combined(&out)
        );
        assert!(
            !calls.lines().any(|l| l.starts_with("cargo ")),
            "{label}: cargo must never run as root (calls: {calls:?})"
        );
    }

    let checkout_owner = fs::metadata(workspace_root())
        .expect("checkout metadata")
        .uid();
    if checkout_owner == 0 {
        // A root-owned checkout may legitimately be built by root (CI containers).
        return;
    }
    let (out, calls) = run_install_build_as_fake_root("build_su", &[]);
    assert!(
        !out.status.success(),
        "install.sh --build as root without an invoking user must fail"
    );
    assert!(
        combined(&out).contains("Refusing to build as root"),
        "the refusal must be explicit:\n{}",
        combined(&out)
    );
    assert!(
        calls.is_empty(),
        "nothing may be built when the root build is refused (calls: {calls:?})"
    );
}

// ---------------------------------------------------------------------------
// #309 — Arch system-auth
// ---------------------------------------------------------------------------

/// One `auth` rule: (control, module, arguments).
#[derive(Debug, Clone)]
pub(crate) struct AuthRule {
    pub(crate) line: String,
    pub(crate) control: String,
    pub(crate) module: String,
    pub(crate) args: Vec<String>,
}

pub(crate) fn auth_rules(text: &str) -> Vec<AuthRule> {
    let mut rules = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (ty, rest) = trimmed.split_once(char::is_whitespace).expect("rule type");
        if ty.trim_start_matches('-') != "auth" {
            continue;
        }
        let rest = rest.trim_start();
        let (control, rest) = if rest.starts_with('[') {
            let end = rest.find(']').expect("closing bracket");
            (rest[..=end].to_string(), rest[end + 1..].trim_start())
        } else {
            let (c, r) = rest.split_once(char::is_whitespace).expect("control");
            (c.to_string(), r.trim_start())
        };
        let mut words = rest.split_whitespace();
        let module = words.next().expect("module").to_string();
        rules.push(AuthRule {
            line: trimmed.to_string(),
            control,
            module,
            args: words.map(str::to_string).collect(),
        });
    }
    rules
}

pub(crate) fn success_jump(control: &str) -> Option<usize> {
    control
        .trim_matches(['[', ']'])
        .split_whitespace()
        .find_map(|kv| kv.strip_prefix("success="))
        .and_then(|v| v.parse().ok())
}

const ARCH_PRIMARY_LINE: &str = "auth  [success=4 default=ignore]  pam_soos.so";
const ARCH_EVENT_LINE: &str =
    "auth  optional                       pam_soos.so event=password-failed timeout_ms=20";

/// POA5 — #309: the shipped Arch stack is the stock pambase `system-auth` with the exact soos
/// edit; every `success=N` jump (since GitHub #318 also the primary soos rule, owner-approved
/// change from `success=done` to `success=4`) lands where stock pambase lands (`pam_permit.so`), the event
/// line is reachable only after a failed `pam_unix`, faillock `authfail`/`authsucc` are kept,
/// and the snippet and the deployment guide show the same lines.
#[test]
fn test_arch_system_auth_is_stock_pambase_with_exact_soos_edit() {
    let full = read("packaging/pam/arch/system-auth");
    let rules = auth_rules(&full);
    let expected: [(&str, &str, Option<&str>); 9] = [
        ("pam_faillock.so", "required", Some("preauth")),
        ("pam_soos.so", "[success=4 default=ignore]", None),
        ("pam_systemd_home.so", "[success=3 default=ignore]", None),
        (
            "pam_unix.so",
            "[success=2 default=bad]",
            Some("try_first_pass"),
        ),
        ("pam_soos.so", "optional", Some("event=password-failed")),
        ("pam_faillock.so", "[default=die]", Some("authfail")),
        ("pam_permit.so", "optional", None),
        ("pam_env.so", "required", None),
        ("pam_faillock.so", "required", Some("authsucc")),
    ];
    assert_eq!(
        rules.len(),
        expected.len(),
        "packaging/pam/arch/system-auth auth rules must be stock pambase plus the two soos \
         lines: {:#?}",
        rules.iter().map(|r| &r.line).collect::<Vec<_>>()
    );
    for (i, (rule, (module, control, first_arg))) in rules.iter().zip(expected).enumerate() {
        assert_eq!(rule.module, module, "auth rule {i}: {}", rule.line);
        assert_eq!(rule.control, control, "auth rule {i}: {}", rule.line);
        if let Some(arg) = first_arg {
            assert_eq!(
                rule.args.first().map(String::as_str),
                Some(arg),
                "auth rule {i}: {}",
                rule.line
            );
        }
    }
    assert!(
        full.lines()
            .any(|l| l.trim_start().starts_with("-auth") && l.contains("pam_systemd_home.so")),
        "pam_systemd_home.so keeps its stock '-auth' (silent if missing) prefix"
    );
    let permit = rules
        .iter()
        .position(|r| r.module == "pam_permit.so")
        .expect("pam_permit.so");
    for (i, rule) in rules.iter().enumerate() {
        if let Some(n) = success_jump(&rule.control) {
            assert_eq!(
                i + 1 + n,
                permit,
                "{} jumps to auth rule {} instead of pam_permit.so ({permit}): \
                 a success would reach pam_faillock authfail or the event line",
                rule.line,
                i + 1 + n
            );
        }
    }
    for line in [ARCH_PRIMARY_LINE, ARCH_EVENT_LINE] {
        assert!(
            full.lines().any(|l| l == line),
            "packaging/pam/arch/system-auth must carry the exact line {line:?}"
        );
    }
    for section in ["account", "password", "session"] {
        assert!(
            !full.lines().any(|l| {
                let t = l.trim_start().trim_start_matches('-');
                t.starts_with(section) && t.contains("pam_soos.so")
            }),
            "pam_soos.so must not appear in the {section} section"
        );
    }

    let snippet = read("packaging/pam/arch/system-auth.snippet");
    let snippet_rules: Vec<&str> = snippet
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .collect();
    assert_eq!(
        snippet_rules,
        vec![ARCH_PRIMARY_LINE, ARCH_EVENT_LINE],
        "the snippet carries exactly the two soos lines of the edited stack"
    );
    for needle in ["success=3", "success=2", "pam_faillock.so authfail"] {
        assert!(
            snippet.contains(needle),
            "the snippet must document the jump adjustment ({needle})"
        );
    }

    let doc = read("Docs/DISTRIBUTION_DEPLOYMENT.md");
    let section = doc
        .split("### 5.2 ")
        .nth(1)
        .and_then(|s| s.split("### 5.3 ").next())
        .expect("Docs/DISTRIBUTION_DEPLOYMENT.md §5.2");
    assert!(
        section.contains(full.trim_end()),
        "Docs/DISTRIBUTION_DEPLOYMENT.md §5.2 must show packaging/pam/arch/system-auth verbatim"
    );
    assert!(
        !section.contains("auth  required                       pam_unix.so"),
        "§5.2 must not show the old 'required pam_unix' edit"
    );
}

/// POA6 — #309: the Arch harness applies the documented edit to the REAL stock
/// `/etc/pam.d/system-auth` (verified against the pambase mtree digest) and asserts face Allow,
/// password fallback without event and faillock tally 0, and one event on a wrong password.
#[test]
fn test_arch_harness_exercises_the_edited_stock_system_auth() {
    let dockerfile = read("tests/docker/Dockerfile.arch");
    let save_at = dockerfile
        .find("cp -p system-auth /usr/local/share/soos-test/system-auth.stock")
        .expect("Dockerfile.arch must keep the stock pambase system-auth before overwriting it");
    let overwrite_at = dockerfile
        .find("> /etc/pam.d/system-auth")
        .expect("Dockerfile.arch synthetic system-auth");
    assert!(
        save_at < overwrite_at,
        "the stock copy is taken before the overwrite"
    );

    let harness = read("tests/distro/arch_linux_test.sh");
    for needle in [
        "/var/lib/pacman/local/pambase-",
        "sha256digest",
        "apply_soos_arch_edit",
        "packaging/pam/arch/system-auth",
        "faillock --user",
        "--record",
        "event kind=password-failed",
        "auth include system-auth",
    ] {
        assert!(
            harness.contains(needle),
            "tests/distro/arch_linux_test.sh must contain {needle:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// #316 — unit directory, /run staging, Debian Default, pinned images, PKGBUILD parity
// ---------------------------------------------------------------------------

/// POA7 — ONB-NEW-3: `--unitdir` stages the unit where packages own it, a staging tree never
/// contains `/run`, and every package builder passes `--unitdir /usr/lib/systemd/system`.
#[test]
fn test_packages_stage_unit_in_unitdir_and_never_ship_run_dir() {
    for (label, extra) in [
        ("unitdir", vec!["--unitdir", "/usr/lib/systemd/system"]),
        ("default", vec![]),
    ] {
        let target = scratch(&format!("unit_{label}_target"));
        stage_target_dir(&target);
        let stage = scratch(&format!("unit_{label}_stage"));
        let out = Command::new("bash")
            .arg(workspace_root().join("scripts/install.sh"))
            .arg("--destdir")
            .arg(&stage)
            .args(["--skip-models", "--skip-systemd", "--distro", "none"])
            .args(&extra)
            .env("CARGO_TARGET_DIR", &target)
            .output()
            .expect("run install.sh");
        assert!(out.status.success(), "{label}: {}", combined(&out));
        assert!(
            !stage.join("run").exists(),
            "{label}: a staging tree must not contain /run (tmpfs, created by the scriptlets)"
        );
        let packaged = stage.join("usr/lib/systemd/system/soos-daemon.service");
        let admin = stage.join("etc/systemd/system/soos-daemon.service");
        if label == "unitdir" {
            assert!(
                packaged.is_file(),
                "--unitdir must place the unit in usr/lib/systemd/system"
            );
            assert!(
                !admin.exists(),
                "--unitdir must not also write etc/systemd/system"
            );
            assert!(
                !stage.join("etc/systemd").exists(),
                "no etc/systemd tree in a package"
            );
        } else {
            assert!(
                admin.is_file(),
                "the default unit directory stays SYSCONFDIR/systemd/system"
            );
        }
        let _ = fs::remove_dir_all(&target);
        let _ = fs::remove_dir_all(&stage);
    }
    for builder in [
        "scripts/build_deb.sh",
        "scripts/build_arch.sh",
        "packaging/debian/rules",
    ] {
        let text = read(builder);
        assert!(
            text.contains("--unitdir /usr/lib/systemd/system"),
            "{builder} must stage the unit with --unitdir /usr/lib/systemd/system"
        );
    }
}

/// POA8 — ONB-NEW-4: the Debian pam-configs are `Default: no` (a later
/// `pam-auth-update --package` never enables facial authentication implicitly) and the `.deb`
/// postinst enables both profiles explicitly.
#[test]
fn test_debian_pam_configs_are_opt_in_and_deb_enables_explicitly() {
    for profile in [
        "packaging/pam/debian/soos",
        "packaging/pam/debian/soos-notify",
    ] {
        let text = read(profile);
        assert!(
            text.lines().any(|l| l.trim() == "Default: no"),
            "{profile} must declare 'Default: no'"
        );
        assert!(
            !text.contains("Default: yes"),
            "{profile} must not be enabled implicitly"
        );
    }
    let postinst = read("packaging/debian/postinst");
    assert!(
        postinst.contains("pam-auth-update --package --enable soos soos-notify"),
        "the .deb postinst must enable both profiles explicitly"
    );
    let rollback = read("tests/docker/pam_rollback_test.sh");
    assert!(
        rollback.contains("D6") && rollback.contains("--distro debian"),
        "the Debian Docker case must run install.sh --distro debian then pam-auth-update --package"
    );
}

/// POA9 — TCI-NEW-3: every sandbox Dockerfile and harness default image is pinned by digest.
/// `tests/docker/Dockerfile.systemd` was exempt until GitHub #318 (owner approval 2026-10-02):
/// it is now pinned too (same digest as `Dockerfile.ubuntu`, row AFC3).
#[test]
fn test_docker_base_images_are_pinned_by_digest() {
    let is_pinned = |reference: &str| {
        reference.split_once("@sha256:").is_some_and(|(_, d)| {
            d.len() >= 64 && d.chars().take(64).all(|c| c.is_ascii_hexdigit())
        })
    };
    for file in [
        "Dockerfile",
        "tests/docker/Dockerfile.ubuntu",
        "tests/docker/Dockerfile.fedora",
        "tests/docker/Dockerfile.arch",
        "tests/docker/Dockerfile.systemd",
    ] {
        let text = read(file);
        let from: Vec<&str> = text.lines().filter(|l| l.starts_with("FROM ")).collect();
        assert!(!from.is_empty(), "{file} has no FROM line");
        for line in from {
            assert!(
                is_pinned(line),
                "{file}: base image not pinned by digest: {line}"
            );
        }
    }
    for (file, var) in [
        ("tests/docker/pam_rollback_test.sh", "SOOS_DEBIAN_IMAGE:-"),
        ("tests/docker/pam_rollback_test.sh", "SOOS_FEDORA_IMAGE:-"),
        (
            "tests/docker/authselect_profile_test.sh",
            "SOOS_FEDORA_IMAGE:-",
        ),
    ] {
        let text = read(file);
        let line = text
            .lines()
            .find(|l| l.contains(var))
            .unwrap_or_else(|| panic!("{file} defines {var}"));
        assert!(
            is_pinned(line),
            "{file}: default image not pinned by digest: {line}"
        );
    }
}

/// POA10 — #301 / ONB-NEW-3: the Docker package harness builds every native package as a
/// NON-root user, asserts `root:root` archive entries and uid 0 installed files, checks the unit
/// location and the absence of `/run`, and builds the PKGBUILD with makepkg for a parity check.
#[test]
fn test_package_harness_builds_as_non_root_and_checks_ownership() {
    let harness = read("tests/docker/test_packages.sh");
    for needle in [
        "BUILD_USER=\"soosbuild\"",
        "runuser -u \"${BUILD_USER}\" -- bash scripts/build_deb.sh",
        "runuser -u \"${BUILD_USER}\" -- bash scripts/build_arch.sh",
        "verify_archive_root_owned",
        "verify_installed_files_root_owned",
        "verify_unit_and_run_layout",
        "makepkg",
        "verify_pkgbuild_parity",
    ] {
        assert!(
            harness.contains(needle),
            "tests/docker/test_packages.sh must contain {needle:?}"
        );
    }
}
