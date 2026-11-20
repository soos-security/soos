//! Contract tests for security review 2026-11-10 batch 10, finding `PSTK-01` (GitHub #574):
//! a `[success=done]` face rule ends the whole stack, so it must never skip an enforcing auth
//! rule (second factor or gate) that the password path still runs after it.
//!
//! - `soos-admin gdm enable` refuses to insert its managed block when such a rule follows the
//!   anchor (fix-design caveat G302-03: bracketed controls, `-auth` lines and later `include`
//!   or `substack` lines are classified, not only the `required`/`requisite` keywords).
//! - `soos-admin pam check` reports, read-only, every service whose primary soos rule ends the
//!   stack before such a rule (the Debian `@include common-auth` layout).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite uses direct assertions and unwrap"
)]

use soos_admin_cli::args::GdmAction;
use soos_admin_cli::gdm::{configure_gdm, pam_backup_path, GDM_PAM_LINE};
use soos_admin_cli::pam_check::{check_pam_services, PamCheckReport};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};

// ----------------------------------------------------------------------------
// Fixtures
// ----------------------------------------------------------------------------

/// Stock Ubuntu 24.04 `common-auth` (no soos profile).
const UBUNTU_COMMON_AUTH: &str = "\
auth\t[success=1 default=ignore]\tpam_unix.so nullok
auth\trequisite\t\t\tpam_deny.so
auth\trequired\t\t\tpam_permit.so
auth\toptional\t\t\tpam_cap.so
";

/// Ubuntu 24.04 `common-auth` generated with the soos and soos-notify pam-configs
/// (`Docs/DISTRIBUTION_DEPLOYMENT.md` section 3.2).
const DEBIAN_COMMON_AUTH_SOOS: &str = "\
auth\t[success=done default=ignore]\tpam_soos.so
auth\t[success=2 default=ignore]\tpam_unix.so nullok try_first_pass
auth\t[default=ignore]\tpam_soos.so event=password-failed timeout_ms=20
auth\trequisite\t\t\tpam_deny.so
auth\trequired\t\t\tpam_permit.so
auth\toptional\t\t\tpam_cap.so
";

/// Ubuntu `gdm-password` head (gates, then the shared include).
const UBUNTU_GDM_HEAD: &str = "\
#%PAM-1.0
auth    requisite       pam_nologin.so
auth\trequired\tpam_succeed_if.so user != root quiet_success
@include common-auth
";

/// Ubuntu `gdm-password` tail after the include (stock).
const UBUNTU_GDM_TAIL: &str = "\
auth    optional        pam_gnome_keyring.so
@include common-account
session required        pam_limits.so
@include common-session
@include common-password
";

/// Fedora 40 `password-auth` (`authselect select local with-faillock`, no soos rule).
const FEDORA_PASSWORD_AUTH: &str = "\
auth        required      pam_env.so
auth        required      pam_faillock.so preauth silent
auth        sufficient    pam_unix.so nullok
auth        required      pam_faillock.so authfail
auth        required      pam_deny.so
account     required      pam_unix.so
";

/// Fedora 40 `postlogin` (no auth rule).
const FEDORA_POSTLOGIN: &str = "\
session     optional      pam_umask.so silent
";

struct Fixture {
    _dir: TempDir,
    pam_file: PathBuf,
    disable_file: PathBuf,
}

fn fixture(gdm_password: &str, stacks: &[(&str, &str)]) -> Fixture {
    let dir = tempdir().unwrap();
    let pam_dir = dir.path().join("pam.d");
    fs::create_dir(&pam_dir).unwrap();
    let pam_file = pam_dir.join("gdm-password");
    fs::write(&pam_file, gdm_password).unwrap();
    for (name, content) in stacks {
        fs::write(pam_dir.join(name), content).unwrap();
    }
    let disable_file = dir.path().join("soos").join("gdm.disable");
    Fixture {
        _dir: dir,
        pam_file,
        disable_file,
    }
}

/// `gdm enable` must refuse: the error names `module`, the file is unchanged, no backup.
fn assert_enable_refused(f: &Fixture, original: &str, module: &str) {
    let err = configure_gdm(&GdmAction::Enable, &f.pam_file, &f.disable_file)
        .expect_err("enable must refuse a layout where the face rule skips an enforcing rule");
    let message = err.to_string();
    assert!(
        message.contains(module),
        "the refusal must name the skipped module {module:?}: {message}"
    );
    assert!(
        message.contains("Docs/DISTRIBUTION_DEPLOYMENT.md"),
        "the refusal must point to the documentation: {message}"
    );
    assert_eq!(fs::read_to_string(&f.pam_file).unwrap(), original);
    assert!(
        !pam_backup_path(&f.pam_file).exists(),
        "no backup on refusal"
    );
}

/// `gdm enable` must insert the managed block.
fn assert_enable_inserts(f: &Fixture) {
    configure_gdm(&GdmAction::Enable, &f.pam_file, &f.disable_file)
        .expect("enable must accept this layout");
    assert!(fs::read_to_string(&f.pam_file)
        .unwrap()
        .contains(GDM_PAM_LINE));
}

fn ubuntu_gdm_with_tail(extra: &str) -> String {
    format!("{UBUNTU_GDM_HEAD}{extra}{UBUNTU_GDM_TAIL}")
}

// ----------------------------------------------------------------------------
// gdm enable: refusal of a post-anchor enforcing rule (PSTK-01, PSK1-PSK4)
// ----------------------------------------------------------------------------

/// PSK1 — the issue's regression: `@include common-auth` followed by `auth required pam_u2f.so`.
#[test]
fn test_pstk01_gdm_enable_refuses_required_factor_after_include() {
    let gdm = ubuntu_gdm_with_tail("auth required pam_u2f.so cue\n");
    let f = fixture(&gdm, &[("common-auth", UBUNTU_COMMON_AUTH)]);
    assert_enable_refused(&f, &gdm, "pam_u2f.so");
}

/// PSK1 — `requisite` and a case-insensitive keyword are enforcing too.
#[test]
fn test_pstk01_gdm_enable_refuses_requisite_factor_after_include() {
    let gdm = ubuntu_gdm_with_tail("auth Requisite pam_google_authenticator.so\n");
    let f = fixture(&gdm, &[("common-auth", UBUNTU_COMMON_AUTH)]);
    assert_enable_refused(&f, &gdm, "pam_google_authenticator.so");
}

/// PSK2 — caveat G302-03: a bracketed control that can fail the stack is enforcing.
#[test]
fn test_pstk01_gdm_enable_refuses_bracketed_enforcing_control() {
    for control in [
        "[success=ok default=bad]",
        "[success=ok default=die]",
        "[success=ok new_authtok_reqd=ok default=ignore auth_err=bad]",
        "[success=ok]",
        "[success=ignore default=ok]",
        "[success=ok default=reset]",
    ] {
        let gdm = ubuntu_gdm_with_tail(&format!("auth {control} pam_oath.so usersfile=x\n"));
        let f = fixture(&gdm, &[("common-auth", UBUNTU_COMMON_AUTH)]);
        assert_enable_refused(&f, &gdm, "pam_oath.so");
    }
}

/// PSK2 — caveat G302-03: an `-auth` line with an enforcing control is enforcing.
#[test]
fn test_pstk01_gdm_enable_refuses_dash_auth_enforcing_rule() {
    let gdm = ubuntu_gdm_with_tail("-auth required pam_u2f.so\n");
    let f = fixture(&gdm, &[("common-auth", UBUNTU_COMMON_AUTH)]);
    assert_enable_refused(&f, &gdm, "pam_u2f.so");
}

/// PSK3 — caveat G302-03: a later `include`, `substack` or `@include` is followed.
#[test]
fn test_pstk01_gdm_enable_refuses_factor_in_later_delegation() {
    let second_factor = "auth required pam_u2f.so\n";
    for line in [
        "auth include second-factor\n",
        "auth substack second-factor\n",
        "@include second-factor\n",
        "AUTH Include second-factor\n",
    ] {
        let gdm = ubuntu_gdm_with_tail(line);
        let f = fixture(
            &gdm,
            &[
                ("common-auth", UBUNTU_COMMON_AUTH),
                ("second-factor", second_factor),
            ],
        );
        assert_enable_refused(&f, &gdm, "pam_u2f.so");
    }
}

/// PSK3 — a later delegation whose target is missing cannot be classified: refused (fail closed).
#[test]
fn test_pstk01_gdm_enable_refuses_unresolvable_later_delegation() {
    let gdm = ubuntu_gdm_with_tail("auth include missing-stack\n");
    let f = fixture(&gdm, &[("common-auth", UBUNTU_COMMON_AUTH)]);
    assert_enable_refused(&f, &gdm, "missing-stack");
}

/// PSK3 — the Fedora `substack password-auth` layout: the password path returns from the
/// substack and runs the tail, so a tail factor is refused.
#[test]
fn test_pstk01_gdm_enable_refuses_factor_after_fedora_substack() {
    let gdm = "\
auth     [success=done ignore=ignore default=bad] pam_selinux_permit.so
auth        substack      password-auth
auth        optional      pam_gnome_keyring.so
auth        include       postlogin
auth        required      pam_u2f.so
account     include       password-auth
";
    let f = fixture(
        gdm,
        &[
            ("password-auth", FEDORA_PASSWORD_AUTH),
            ("postlogin", FEDORA_POSTLOGIN),
        ],
    );
    assert_enable_refused(&f, gdm, "pam_u2f.so");
}

/// PSK3 — an inline credential that jumps onto the success block keeps running the tail:
/// a factor after `pam_permit.so` is refused.
#[test]
fn test_pstk01_gdm_enable_refuses_factor_after_inline_jump() {
    let gdm = "\
auth requisite pam_nologin.so
auth [success=1 default=ignore] pam_unix.so nullok
auth requisite pam_deny.so
auth required pam_permit.so
auth required pam_u2f.so
";
    let f = fixture(gdm, &[]);
    assert_enable_refused(&f, gdm, "pam_u2f.so");
}

/// PSK4 — stock layouts stay accepted: the Ubuntu tail is `optional` only and the Debian
/// non-auth stacks may be absent from the PAM directory.
#[test]
fn test_pstk01_gdm_enable_accepts_stock_ubuntu_tail() {
    let gdm = ubuntu_gdm_with_tail("");
    let f = fixture(&gdm, &[("common-auth", UBUNTU_COMMON_AUTH)]);
    assert_enable_inserts(&f);
}

/// PSK4 — non-enforcing tail controls are accepted: `optional`, `sufficient`, a bracket whose
/// failure actions are `ignore`, and a numeric jump.
#[test]
fn test_pstk01_gdm_enable_accepts_non_enforcing_tail_controls() {
    let gdm = ubuntu_gdm_with_tail(
        "auth optional pam_u2f.so\n\
         auth [success=ok default=ignore] pam_mount.so\n\
         -auth [success=1 default=ignore] pam_systemd_home.so\n\
         auth optional pam_cap.so\n\
         auth sufficient pam_permit.so\n",
    );
    let f = fixture(&gdm, &[("common-auth", UBUNTU_COMMON_AUTH)]);
    assert_enable_inserts(&f);
}

/// PSK4 — when the password path itself ends the stack (inline `sufficient pam_unix.so`, as in
/// Fedora `system-auth`), the face rule skips nothing the password runs: accepted.
#[test]
fn test_pstk01_gdm_enable_accepts_tail_unreachable_after_password() {
    let gdm = "\
auth requisite pam_nologin.so
auth sufficient pam_unix.so nullok
auth required pam_deny.so
";
    let f = fixture(gdm, &[]);
    assert_enable_inserts(&f);
}

/// PSK4 — a present Debian non-auth stack is analysed: an auth rule hidden in it counts.
#[test]
fn test_pstk01_gdm_enable_analyses_present_debian_phase_stack() {
    let gdm = ubuntu_gdm_with_tail("");
    let f = fixture(
        &gdm,
        &[
            ("common-auth", UBUNTU_COMMON_AUTH),
            (
                "common-account",
                "account required pam_unix.so\nauth required pam_u2f.so\n",
            ),
        ],
    );
    assert_enable_refused(&f, &gdm, "pam_u2f.so");
}

/// PSK3 — an anchor stack that cannot be analysed past its credential module (a missing
/// include after `pam_unix.so`) is unknown: every enforcing tail rule is refused (fail closed),
/// an `optional`-only tail is still accepted.
#[test]
fn test_pstk01_gdm_enable_unanalysable_anchor_stack_fails_closed() {
    let common = format!("{UBUNTU_COMMON_AUTH}auth include vendor-extra\n");
    let gdm = ubuntu_gdm_with_tail("auth required pam_u2f.so\n");
    let f = fixture(&gdm, &[("common-auth", &common)]);
    assert_enable_refused(&f, &gdm, "pam_u2f.so");

    let gdm = ubuntu_gdm_with_tail("");
    let f = fixture(&gdm, &[("common-auth", &common)]);
    assert_enable_inserts(&f);
}

// ----------------------------------------------------------------------------
// pam check (PSTK-01, PSK5-PSK8)
// ----------------------------------------------------------------------------

fn pam_dir(stacks: &[(&str, &str)]) -> (TempDir, PathBuf) {
    let dir = tempdir().unwrap();
    let pam_dir = dir.path().join("pam.d");
    fs::create_dir(&pam_dir).unwrap();
    for (name, content) in stacks {
        fs::write(pam_dir.join(name), content).unwrap();
    }
    (dir, pam_dir)
}

fn check(pam_dir: &Path, services: &[&str]) -> PamCheckReport {
    let services: Vec<String> = services.iter().map(ToString::to_string).collect();
    check_pam_services(pam_dir, &services).expect("pam check runs")
}

/// PSK5 — the issue's Debian scenario: `sudo` = `@include common-auth` + `pam_u2f.so`.
#[test]
fn test_pstk01_pam_check_reports_factor_after_debian_include() {
    let (_d, dir) = pam_dir(&[
        ("common-auth", DEBIAN_COMMON_AUTH_SOOS),
        (
            "sudo",
            "#%PAM-1.0\n@include common-auth\nauth required pam_u2f.so cue\n@include common-account\n",
        ),
    ]);
    let report = check(&dir, &["sudo"]);
    assert_eq!(report.findings.len(), 1, "{report:?}");
    let finding = &report.findings[0];
    assert_eq!(finding.service, "sudo");
    assert_eq!(finding.soos_stack, "common-auth");
    assert_eq!(finding.soos_line, 1);
    assert_eq!(finding.skipped.len(), 1, "{finding:?}");
    assert_eq!(finding.skipped[0].stack, "sudo");
    assert_eq!(finding.skipped[0].line, 3);
    assert!(finding.skipped[0].rule.contains("pam_u2f.so"));
    assert_eq!(report.exit_code(), 1);
    let table = report.format_table();
    assert!(
        table.contains("sudo") && table.contains("pam_u2f.so"),
        "{table}"
    );
    let json: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
    assert_eq!(json["findings"][0]["service"], "sudo");
}

/// PSK6 — stock stacks report nothing: the Debian generated `common-auth` (password jumps onto
/// `pam_permit.so`), a gate before the include, an `optional` rule after it.
#[test]
fn test_pstk01_pam_check_accepts_stock_debian_service() {
    let (_d, dir) = pam_dir(&[
        ("common-auth", DEBIAN_COMMON_AUTH_SOOS),
        (
            "login",
            "auth optional pam_faildelay.so delay=3000000\nauth requisite pam_nologin.so\n\
             @include common-auth\nauth optional pam_group.so\n",
        ),
    ]);
    let report = check(&dir, &[]);
    assert!(report.findings.is_empty(), "{report:?}");
    assert!(report.unanalysed.is_empty(), "{report:?}");
    assert_eq!(report.services_checked, 2);
    assert_eq!(report.exit_code(), 0);
}

/// PSK6 — the packaged Fedora and Arch stacks report nothing: Fedora's password module is
/// `sufficient` (it ends the include as `done` does), Arch's face rule jumps.
#[test]
fn test_pstk01_pam_check_accepts_packaged_fedora_and_arch_stacks() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fedora = strip_authselect_conditionals(
        &fs::read_to_string(root.join("packaging/pam/fedora/soos/system-auth")).unwrap(),
    );
    let arch = fs::read_to_string(root.join("packaging/pam/arch/system-auth")).unwrap();
    let (_d, dir) = pam_dir(&[
        ("system-auth", &fedora),
        (
            "sudo",
            "auth include system-auth\nauth required pam_u2f.so\naccount include system-auth\n",
        ),
    ]);
    let report = check(&dir, &[]);
    assert!(
        report.findings.is_empty() && report.unanalysed.is_empty(),
        "{report:?}"
    );

    let (_d2, dir2) = pam_dir(&[
        ("system-auth", &arch),
        (
            "system-login",
            "auth required pam_shells.so\nauth requisite pam_nologin.so\nauth include system-auth\n",
        ),
        (
            "sudo",
            "auth include system-login\nauth required pam_u2f.so\n",
        ),
    ]);
    let report = check(&dir2, &["sudo"]);
    assert!(
        report.findings.is_empty() && report.unanalysed.is_empty(),
        "{report:?}"
    );
}

/// Removes authselect `{if ...}` / `{include if ...}` template fragments (keeps the rules).
fn strip_authselect_conditionals(text: &str) -> String {
    text.lines()
        .map(|line| match line.find('{') {
            Some(i) => line[..i].trim_end(),
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// PSK7 — a substack confines `done`: the caller's tail runs on both paths, nothing reported.
#[test]
fn test_pstk01_pam_check_substack_confines_done() {
    let (_d, dir) = pam_dir(&[
        (
            "password-auth",
            "auth [success=done default=ignore] pam_soos.so\nauth sufficient pam_unix.so\nauth required pam_deny.so\n",
        ),
        (
            "gdm-password",
            "auth substack password-auth\nauth required pam_u2f.so\n",
        ),
    ]);
    let report = check(&dir, &["gdm-password"]);
    assert!(
        report.findings.is_empty() && report.unanalysed.is_empty(),
        "{report:?}"
    );
}

/// PSK7 — a conditional rule after the face rule is explored on both branches: the factor is
/// skipped by the face match for the users the condition does not match.
#[test]
fn test_pstk01_pam_check_explores_conditional_branches() {
    let (_d, dir) = pam_dir(&[
        ("common-auth", DEBIAN_COMMON_AUTH_SOOS),
        (
            "sudo",
            "@include common-auth\nauth [success=1 default=ignore] pam_succeed_if.so user ingroup nofactor\nauth required pam_u2f.so\n",
        ),
    ]);
    let report = check(&dir, &["sudo"]);
    assert_eq!(report.findings.len(), 1, "{report:?}");
    assert!(report.findings[0].skipped[0].rule.contains("pam_u2f.so"));
}

/// PSK7 — the GDM managed block (`sufficient` or `done` written inline) skips a factor after
/// the anchor; a hand-written `sufficient` soos rule is reported too.
#[test]
fn test_pstk01_pam_check_reports_inline_done_and_sufficient_rules() {
    for rule in [
        "auth  [success=done default=ignore]  pam_soos.so timeout_ms=2500",
        "auth sufficient pam_soos.so",
    ] {
        let (_d, dir) = pam_dir(&[
            ("common-auth", UBUNTU_COMMON_AUTH),
            (
                "gdm-password",
                &format!("{rule}\n@include common-auth\nauth required pam_u2f.so\n"),
            ),
        ]);
        let report = check(&dir, &["gdm-password"]);
        assert_eq!(report.findings.len(), 1, "{rule}: {report:?}");
    }
}

/// PSK8 — fail closed: a missing include, a continuation line or an unknown control makes the
/// service unanalysed (exit 2), never "clean".
#[test]
fn test_pstk01_pam_check_reports_unanalysable_services() {
    let (_d, dir) = pam_dir(&[
        (
            "missing",
            "auth [success=done default=ignore] pam_soos.so\n@include nowhere\n",
        ),
        (
            "continued",
            "auth [success=done default=ignore] pam_soos.so \\\n  debug\n",
        ),
        (
            "weird",
            "auth [success=done default=ignore] pam_soos.so\nauth [success=frobnicate default=ignore] pam_x.so\nauth required pam_u2f.so\n",
        ),
    ]);
    let report = check(&dir, &[]);
    let names: Vec<&str> = report
        .unanalysed
        .iter()
        .map(|u| u.service.as_str())
        .collect();
    assert!(names.contains(&"missing"), "{report:?}");
    assert!(names.contains(&"continued"), "{report:?}");
    assert!(names.contains(&"weird"), "{report:?}");
    assert!(report.findings.is_empty(), "{report:?}");
    assert_eq!(report.exit_code(), 2);
}

/// PSK8 — services without a soos rule that ends the stack are never reported, whatever their
/// layout; a service name outside `[A-Za-z0-9._-]` or a backup file is not examined.
#[test]
fn test_pstk01_pam_check_ignores_services_without_a_done_soos_rule() {
    let (_d, dir) = pam_dir(&[
        ("arch-like", "auth [success=1 default=ignore] pam_soos.so\nauth required pam_unix.so\nauth required pam_u2f.so\n"),
        ("plain", "auth required pam_unix.so\nauth required pam_u2f.so\n"),
        ("gdm-password.soos-backup", "auth [success=done default=ignore] pam_soos.so\nauth required pam_u2f.so\n"),
    ]);
    let report = check(&dir, &[]);
    assert!(
        report.findings.is_empty() && report.unanalysed.is_empty(),
        "{report:?}"
    );
    assert_eq!(report.services_checked, 2);
    assert!(check_pam_services(&dir, &["../etc".to_string()]).is_err());
}

/// PSK8 — end to end through the binary: exit code 1 on a finding, JSON on request.
#[test]
fn test_pstk01_soos_admin_pam_check_cli() {
    let (_d, dir) = pam_dir(&[
        ("common-auth", DEBIAN_COMMON_AUTH_SOOS),
        ("sudo", "@include common-auth\nauth required pam_u2f.so\n"),
    ]);
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_soos-admin"))
        .args(["pam", "check", "--pam-dir"])
        .arg(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("pam_u2f.so"));

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_soos-admin"))
        .args(["--format", "json", "pam", "check", "--pam-dir"])
        .arg(&dir)
        .arg("common-auth")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["services_checked"], 1);
    assert!(json["findings"].as_array().unwrap().is_empty());
}

// ----------------------------------------------------------------------------
// Candid review round 1 (PSK13-PSK15): rules after the credential module of the anchor's own
// delegated stack, and the gates `gdm enable` copied in front of its managed rule.
// ----------------------------------------------------------------------------

/// Arch Linux `pambase` `system-auth` (auth phase).
const ARCH_SYSTEM_AUTH: &str = "\
auth       required                    pam_faillock.so      preauth
-auth      [success=2 default=ignore]  pam_systemd_home.so
auth       [success=1 default=bad]     pam_unix.so          try_first_pass nullok
auth       [default=die]               pam_faillock.so      authfail
auth       optional                    pam_permit.so
auth       required                    pam_env.so
auth       required                    pam_faillock.so      authsucc
account    required                    pam_unix.so
";

/// Arch Linux `pambase` `system-login` (auth phase).
const ARCH_SYSTEM_LOGIN: &str = "\
auth       required   pam_shells.so
auth       requisite  pam_nologin.so
auth       include    system-auth
account    include    system-auth
";

/// Arch Linux `pambase` `system-local-login`.
const ARCH_SYSTEM_LOCAL_LOGIN: &str = "\
auth      include   system-login
account   include   system-login
";

/// Arch Linux `gdm` `gdm-password`.
const ARCH_GDM_PASSWORD: &str = "\
auth       include                     system-local-login
auth       optional                    pam_gnome_keyring.so
account    include                     system-local-login
";

/// Fedora 40 `gdm-password` (auth phase).
const FEDORA_GDM_PASSWORD: &str = "\
auth     [success=done ignore=ignore default=bad] pam_selinux_permit.so
auth        substack      password-auth
auth        optional      pam_gnome_keyring.so
auth        include       postlogin
account     include       password-auth
";

/// `pam check` of `service` in the directory of `f` after a successful `gdm enable`.
fn check_after_enable(f: &Fixture, service: &str) -> PamCheckReport {
    assert_enable_inserts(f);
    check(f.pam_file.parent().unwrap(), &[service])
}

/// PSK13 — a second factor after the credential module of the anchor's own shared stack
/// (stock Ubuntu `common-auth` + a trailing `required pam_u2f.so`) runs on the password path
/// and would be skipped by the managed `[success=done]` rule: `gdm enable` refuses.
#[test]
fn test_pstk01_gdm_enable_refuses_factor_in_anchor_stack_after_credential() {
    let common = format!("{UBUNTU_COMMON_AUTH}auth required pam_u2f.so\n");
    let gdm = "\
auth requisite pam_nologin.so
@include common-auth
auth optional pam_gnome_keyring.so
";
    let f = fixture(gdm, &[("common-auth", &common)]);
    assert_enable_refused(&f, gdm, "pam_u2f.so");

    let common = format!("{UBUNTU_COMMON_AUTH}auth [success=ok default=bad] pam_u2f.so\n");
    let f = fixture(gdm, &[("common-auth", &common)]);
    assert_enable_refused(&f, gdm, "pam_u2f.so");
}

/// PSK13 — the same rule inside a Fedora `substack` after a non-ending credential module is
/// refused too; the stock `sufficient pam_unix.so` substack stays accepted.
#[test]
fn test_pstk01_gdm_enable_refuses_factor_in_anchor_substack_after_credential() {
    let password_auth = "\
auth        required      pam_faillock.so preauth silent
auth        [success=1 default=bad] pam_unix.so nullok
auth        required      pam_deny.so
auth        required      pam_u2f.so
";
    let f = fixture(
        FEDORA_GDM_PASSWORD,
        &[
            ("password-auth", password_auth),
            ("postlogin", FEDORA_POSTLOGIN),
        ],
    );
    assert_enable_refused(&f, FEDORA_GDM_PASSWORD, "pam_u2f.so");
}

/// PSK14 — `pam_faillock.so authsucc` after the credential module is covered only by an
/// enforcing `pam_faillock.so preauth` gate that runs before the managed rule; without it the
/// tally check of `authsucc` is the only lockout check and `gdm enable` refuses.
#[test]
fn test_pstk01_gdm_enable_authsucc_needs_a_preauth_gate() {
    let without_preauth = ARCH_SYSTEM_AUTH.replacen(
        "auth       required                    pam_faillock.so      preauth\n",
        "",
        1,
    );
    let f = fixture(
        ARCH_GDM_PASSWORD,
        &[
            ("system-auth", &without_preauth),
            ("system-login", ARCH_SYSTEM_LOGIN),
            ("system-local-login", ARCH_SYSTEM_LOCAL_LOGIN),
        ],
    );
    assert_enable_refused(&f, ARCH_GDM_PASSWORD, "pam_faillock.so");
}

/// PSK14/PSK15 — stock Arch: `gdm enable` accepts the stack (the `preauth` gate is copied in
/// front of the managed rule and covers `authsucc`), and `pam check` on the result recognises
/// the copied gates: nothing reported, exit code 0.
#[test]
fn test_pstk01_pam_check_clean_after_gdm_enable_on_arch() {
    let f = fixture(
        ARCH_GDM_PASSWORD,
        &[
            ("system-auth", ARCH_SYSTEM_AUTH),
            ("system-login", ARCH_SYSTEM_LOGIN),
            ("system-local-login", ARCH_SYSTEM_LOCAL_LOGIN),
        ],
    );
    let report = check_after_enable(&f, "gdm-password");
    assert!(
        report.findings.is_empty() && report.unanalysed.is_empty(),
        "{report:?}"
    );
    assert_eq!(report.exit_code(), 0);
}

/// PSK15 — stock Fedora (`substack password-auth` with faillock): `pam check` after
/// `gdm enable` recognises the copied `preauth` gate: nothing reported, exit code 0.
#[test]
fn test_pstk01_pam_check_clean_after_gdm_enable_on_fedora() {
    let f = fixture(
        FEDORA_GDM_PASSWORD,
        &[
            ("password-auth", FEDORA_PASSWORD_AUTH),
            ("postlogin", FEDORA_POSTLOGIN),
        ],
    );
    let report = check_after_enable(&f, "gdm-password");
    assert!(
        report.findings.is_empty() && report.unanalysed.is_empty(),
        "{report:?}"
    );
    assert_eq!(report.exit_code(), 0);
}

/// PSK15 — only an identical copy covers a skipped rule: a gate that runs before the face rule
/// with different arguments does not hide the skipped original.
#[test]
fn test_pstk01_pam_check_reports_gate_copy_with_other_arguments() {
    let (_d, dir) = pam_dir(&[
        (
            "system-auth",
            "auth required pam_succeed_if.so uid >= 1000\n\
             auth sufficient pam_unix.so nullok\nauth required pam_deny.so\n",
        ),
        (
            "login",
            "auth required pam_succeed_if.so uid >= 500\n\
             auth [success=done default=ignore] pam_soos.so\nauth include system-auth\n",
        ),
    ]);
    let report = check(&dir, &["login"]);
    assert_eq!(report.findings.len(), 1, "{report:?}");
    assert!(report.findings[0].skipped[0]
        .rule
        .contains("pam_succeed_if.so"));

    let (_d2, dir2) = pam_dir(&[
        (
            "system-auth",
            "auth required pam_succeed_if.so uid >= 1000\n\
             auth sufficient pam_unix.so nullok\nauth required pam_deny.so\n",
        ),
        (
            "login",
            "auth required pam_succeed_if.so uid >= 1000\n\
             auth [success=done default=ignore] pam_soos.so\nauth include system-auth\n",
        ),
    ]);
    let report = check(&dir2, &["login"]);
    assert!(
        report.findings.is_empty() && report.unanalysed.is_empty(),
        "{report:?}"
    );
}

/// PSK13 — the `pam_deny.so` exemption is confined to the failure fallback of the anchor's
/// own stack: an explicit `pam_deny.so` in the edited file after the anchor is still refused.
#[test]
fn test_pstk01_gdm_enable_refuses_deny_in_edited_tail() {
    let gdm = ubuntu_gdm_with_tail("auth required pam_deny.so\n");
    let f = fixture(&gdm, &[("common-auth", UBUNTU_COMMON_AUTH)]);
    assert_enable_refused(&f, &gdm, "pam_deny.so");
}
