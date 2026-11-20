//! Security review 2026-11-10 batch 10 (GitHub #487): `success=done` and the PAM stack.
//!
//! - `PSTK-02` (GitHub #575): the `custom/soos` authselect profile is local-accounts-only;
//!   `scripts/install.sh` prints its activation command only for the `local` and `minimal`
//!   profiles (features carried over, no `--force` on an authselect-managed host) and warns
//!   otherwise; the RPM `%post`, the profile README and the deployment guide say the same.
//! - `PSTK-01` (GitHub #574): the deployment guide documents that a `success=done` face rule
//!   skips every auth rule after the include and names `soos-admin pam check`.
//! - `PSTKC-01` (GitHub #581): the guide documents the flag `gdm disable` writes per service.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract tests utilize direct assertions, panics, and indexing"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Body of a top-level shell function `name() {` up to its closing `}` at column 0.
fn shell_function(text: &str, name: &str) -> String {
    let header = format!("\n{name}() {{\n");
    let start = text
        .find(&header)
        .unwrap_or_else(|| panic!("scripts/install.sh must define {name}()"))
        + 1;
    let rest = &text[start..];
    let end = rest.find("\n}\n").map_or(rest.len(), |i| i + 3);
    rest[..end].to_string()
}

/// Runs `authselect_activation_hint "<profile>"` with `info`/`warn` stubs; returns
/// (INFO lines, WARN lines).
fn hint(profile: &str) -> (String, String) {
    let function = shell_function(&read("scripts/install.sh"), "authselect_activation_hint");
    let script = format!(
        "set -euo pipefail\ninfo() {{ echo \"INFO $*\"; }}\nwarn() {{ echo \"WARN $*\"; }}\n\
         {function}\nauthselect_activation_hint \"$1\"\n"
    );
    let out = Command::new("bash")
        .arg("-c")
        .arg(script)
        .arg("hint")
        .arg(profile)
        .output()
        .expect("run bash");
    assert!(
        out.status.success(),
        "{profile}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let info = stdout
        .lines()
        .filter(|l| l.starts_with("INFO "))
        .collect::<Vec<_>>()
        .join("\n");
    let warn = stdout
        .lines()
        .filter(|l| l.starts_with("WARN "))
        .collect::<Vec<_>>()
        .join("\n");
    (info, warn)
}

/// PSK9 — `local`/`minimal`: the activation command carries the recorded features, names
/// `with-faillock` once and never forces an authselect-managed host.
#[test]
fn test_pstk02_install_hint_for_local_profiles_carries_features() {
    let (info, warn) = hint("local with-silent-lastlog with-faillock");
    assert!(warn.is_empty(), "{warn}");
    assert!(
        info.contains(
            "authselect select custom/soos with-faillock with-silent-lastlog && authselect check"
        ),
        "{info}"
    );
    assert!(!info.contains("--force"), "{info}");
    assert_eq!(info.matches("with-faillock").count(), 1, "{info}");

    let (info, warn) = hint("minimal");
    assert!(warn.is_empty(), "{warn}");
    assert!(
        info.contains("authselect select custom/soos with-faillock && authselect check"),
        "{info}"
    );
}

/// PSK9 — `sssd`, `winbind`, `nis` (or any other profile): no activation command, a
/// local-accounts-only warning instead.
#[test]
fn test_pstk02_install_hint_warns_for_directory_profiles() {
    for profile in [
        "sssd with-mkhomedir with-smartcard",
        "winbind",
        "nis with-faillock",
        "vendor/corp",
    ] {
        let (info, warn) = hint(profile);
        assert!(
            !info.contains("authselect select custom/soos") && !warn.contains("authselect select"),
            "{profile}: no activation command may be printed: {info} {warn}"
        );
        assert!(warn.contains("local accounts only"), "{profile}: {warn}");
        assert!(
            warn.contains(profile.split_whitespace().next().unwrap()),
            "{profile}: the warning names the current profile: {warn}"
        );
    }
}

/// PSK9 — no recorded profile (host not managed by authselect): `--force` only with an
/// explicit local-accounts-only condition; an active `custom/soos` needs no command.
#[test]
fn test_pstk02_install_hint_without_profile_and_when_already_selected() {
    let (info, warn) = hint("");
    assert!(
        warn.contains("local accounts only") && warn.contains("--force"),
        "{warn}"
    );
    assert!(info.is_empty(), "{info}");

    let (info, warn) = hint("custom/soos with-faillock");
    assert!(warn.is_empty(), "{warn}");
    assert!(!info.contains("authselect select"), "{info}");
}

/// PSK9 — install.sh calls the function with the recorded profile, and no unconditional
/// activation hint remains.
#[test]
fn test_pstk02_install_uses_the_profile_gated_hint() {
    let install = read("scripts/install.sh");
    assert!(install.contains("authselect_activation_hint \"${AUTHSELECT_CURRENT}\""));
    assert!(
        !install
            .contains("info \"Activate with: authselect select custom/soos with-faillock --force"),
        "the unconditional --force hint must be gone"
    );
}

/// PSK10 — the RPM, the profile README and the deployment guide state the restriction.
#[test]
fn test_pstk02_local_only_restriction_is_documented() {
    let spec = read("packaging/rpm/soos.spec");
    assert!(
        spec.contains("local accounts only") && spec.contains("sssd|winbind|nis"),
        "soos.spec %post must warn on directory profiles"
    );
    let readme = read("packaging/pam/fedora/soos/README");
    assert!(readme.contains("local accounts only"), "profile README");
    assert!(readme.contains("sssd") && readme.contains("winbind"));
    let guide = read("Docs/DISTRIBUTION_DEPLOYMENT.md");
    for section in ["### 4.1 ", "### 4.3 "] {
        let start = guide.find(section).expect(section);
        let rest = &guide[start + section.len()..];
        let body = &rest[..rest.find("\n### ").unwrap_or(rest.len())];
        assert!(
            body.contains("local accounts only"),
            "Docs section {section} must state the local-accounts-only restriction"
        );
    }
}

/// PSK11 — the guide documents the post-include skip, `gdm enable`'s refusal and `pam check`;
/// and the per-service flag of `gdm disable` (PSTKC-01).
#[test]
fn test_pstk01_and_pstkc01_behaviour_is_documented() {
    let guide = read("Docs/DISTRIBUTION_DEPLOYMENT.md");
    assert!(guide.contains("soos-admin pam check"));
    assert!(guide.contains("after the include"));
    assert!(guide.contains("<service>.disable"));
    let decisions = read("AI/DECISIONS.md");
    assert!(decisions.contains("GitHub #487"));
}

/// The `authselect` block of the RPM `%post` scriptlet as the installed package runs it: RPM
/// macro expansion applied (`%%` becomes `%`, `%{_sysconfdir}` becomes `sysconfdir`).
fn rpm_post_authselect_block(sysconfdir: &str) -> String {
    let spec = read("packaging/rpm/soos.spec");
    let post = &spec[spec
        .find("\n%post\n")
        .expect("soos.spec has a %post scriptlet")..];
    let start = post
        .find("\nif command -v authselect >/dev/null 2>&1; then\n")
        .expect("%post runs an authselect block")
        + 1;
    let rest = &post[start..];
    let end = rest
        .find("\nfi\n")
        .expect("the authselect block ends with fi")
        + 4;
    let block = rest[..end]
        .replace("%%", "\u{0}")
        .replace("%{_sysconfdir}", sysconfdir);
    assert!(
        !block.contains('%'),
        "unexpected RPM macro in the %post authselect block: {block}"
    );
    block.replace('\u{0}', "%")
}

/// PSK10b (candid review round 1) — the macro-expanded `%post` warns on every directory
/// profile whatever its features (`sssd with-mkhomedir with-sudo` included), names the
/// recorded profile, stays silent on `local`/`minimal`, and records the profile.
#[test]
fn test_pstk02_rpm_post_warns_on_directory_profiles_after_macro_expansion() {
    let tmp = std::env::temp_dir().join(format!("soos-psk10b-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    let sysconfdir = tmp.to_str().unwrap().to_string();
    let block = rpm_post_authselect_block(&sysconfdir);
    let run = |current: &str| -> String {
        let script = format!("authselect() {{ printf '%s\\n' \"$SOOS_CURRENT\"; }}\n{block}");
        let out = Command::new("sh")
            .arg("-c")
            .arg(script)
            .env("SOOS_CURRENT", current)
            .output()
            .expect("run sh");
        assert!(out.status.success(), "{current}: {out:?}");
        String::from_utf8_lossy(&out.stderr).to_string()
    };
    for current in [
        "sssd",
        "sssd with-mkhomedir",
        "sssd with-mkhomedir with-sudo",
        "winbind with-mkhomedir with-krb5",
        "nis with-faillock with-mkhomedir",
    ] {
        let stderr = run(current);
        assert!(
            stderr.contains("WARNING") && stderr.contains("local accounts only"),
            "{current}: the directory-host warning must fire: {stderr:?}"
        );
        assert!(
            stderr.contains(current),
            "{current}: names the profile: {stderr:?}"
        );
        assert_eq!(
            fs::read_to_string(tmp.join("soos/authselect.previous")).unwrap(),
            format!("{current}\n")
        );
    }
    for current in [
        "local",
        "local with-faillock with-silent-lastlog",
        "minimal",
        "sssdx with-mkhomedir",
        "custom/soos with-faillock",
        "",
    ] {
        let stderr = run(current);
        assert!(
            !stderr.contains("WARNING"),
            "{current}: no directory-host warning: {stderr:?}"
        );
    }
    let _ = fs::remove_dir_all(&tmp);
}
