//! Static contracts of the opt-in polkit Wi-Fi lock (GitHub #365; ADR "[2026-10-13] Opt-In
//! polkit Wi-Fi Lock and `soos-admin network-lock`"; spec `AI/architect_spec_network_lock.md`;
//! matrix NLK1, NLK2, NLK3, NLK9, NLK10):
//!
//! - the rules file lists exactly the documented action ids and only ever returns
//!   `AUTH_ADMIN` for them, falling through for every other action;
//! - the rfkill udev rule only removes the `uaccess` tag and sorts between 70 and 73;
//! - both files are single-sourced from `packaging/` into `soos-admin`;
//! - the lock is opt-in (the installer never installs it) and `scripts/uninstall.sh` removes it;
//! - the Docker `pkcheck` test is wired into CI;
//! - documentation, ADR and walkthrough.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

const RULES: &str = "packaging/polkit/49-soos-network-lock.rules";
const RFKILL_RULE: &str = "packaging/udev/72-soos-network-lock-rfkill.rules";
const MODULE: &str = "crates/admin-cli/src/network_lock.rs";
const DOC: &str = "Docs/NETWORK_LOCK.md";
const DOCKER_TEST: &str = "tests/docker/network_lock_pkcheck_test.sh";
const CI: &str = ".github/workflows/ci.yml";
const ADR_TITLE: &str = "[2026-10-13] Opt-In polkit Wi-Fi Lock and `soos-admin network-lock`";
const WALKTHROUGH: &str = "AI/walkthroughs/196_network_lock.md";
const ACTION_PREFIX: &str = "org.freedesktop.NetworkManager.";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// The rules file without its `//` comment lines.
fn rule_code() -> String {
    read(RULES)
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every `"org.freedesktop.NetworkManager.*"` string literal of `text`, in order.
fn quoted_action_ids(text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let needle = format!("\"{ACTION_PREFIX}");
    let mut rest = text;
    while let Some(at) = rest.find(&needle) {
        let after = &rest[at + 1..];
        let end = after.find('"').expect("closing quote");
        ids.push(after[..end].to_string());
        rest = &after[end..];
    }
    ids
}

/// Action ids of the first column of the "Locked actions" table of `Docs/NETWORK_LOCK.md`.
fn documented_action_ids() -> Vec<String> {
    let doc = read(DOC);
    let start = doc
        .find("## 2. Locked actions")
        .expect("Docs/NETWORK_LOCK.md has a `## 2. Locked actions` section");
    let section = &doc[start..];
    let end = section[3..].find("\n## ").map_or(section.len(), |e| e + 3);
    section[..end]
        .lines()
        .filter_map(|l| l.strip_prefix("| `"))
        .filter_map(|l| l.split('`').next())
        .filter(|id| id.starts_with(ACTION_PREFIX))
        .map(str::to_string)
        .collect()
}

/// Index of the first line of `text` containing `needle`: among comment lines only when
/// `comment` is true (section markers), else among non-comment lines only.
fn code_line_index(text: &str, needle: &str, comment: bool) -> usize {
    text.lines()
        .position(|l| l.contains(needle) && l.trim_start().starts_with('#') == comment)
        .unwrap_or_else(|| {
            panic!(
                "no {} line containing `{needle}`",
                if comment { "comment" } else { "code" }
            )
        })
}

/// NLK1: the rules file, the `NETWORK_LOCK_ACTION_IDS` constant and the documentation table
/// name exactly the same five actions, each once.
#[test]
fn test_nlk_rules_file_lists_exactly_the_documented_action_ids() {
    let in_rule = quoted_action_ids(&rule_code());
    let module = read(MODULE);
    let start = module
        .find("pub const NETWORK_LOCK_ACTION_IDS")
        .expect("NETWORK_LOCK_ACTION_IDS constant");
    let block_end = module[start..].find("];").expect("end of constant") + start;
    let in_code = quoted_action_ids(&module[start..block_end]);
    let in_doc = documented_action_ids();

    assert_eq!(in_rule.len(), 5, "five locked actions: {in_rule:?}");
    let unique: BTreeSet<&String> = in_rule.iter().collect();
    assert_eq!(unique.len(), in_rule.len(), "no duplicate action id");
    assert_eq!(
        in_rule, in_code,
        "rule file and NETWORK_LOCK_ACTION_IDS differ"
    );
    assert_eq!(
        in_rule, in_doc,
        "rule file and Docs/NETWORK_LOCK.md table differ"
    );
    for scoped in [
        "org.freedesktop.NetworkManager.network-control",
        "org.freedesktop.NetworkManager.enable-disable-wifi",
    ] {
        assert!(
            in_rule.iter().any(|id| id == scoped),
            "{scoped} must be locked"
        );
    }
    for out_of_scope in [
        "org.freedesktop.NetworkManager.settings.modify.hostname",
        "org.freedesktop.NetworkManager.settings.modify.global-dns",
        "org.freedesktop.NetworkManager.enable-disable-wwan",
        // Default already `no` for every subject: AUTH_ADMIN would weaken it (round 1 F-1).
        "org.freedesktop.NetworkManager.sleep-wake",
    ] {
        assert!(
            !in_rule.iter().any(|id| id == out_of_scope),
            "{out_of_scope} must not be listed (out of scope or default already stronger)"
        );
    }
    assert!(
        !rule_code().contains("org.freedesktop.network1")
            && !rule_code().contains("org.freedesktop.resolve1"),
        "networkd and resolved actions are outside the Wi-Fi scope"
    );
}

/// NLK1: the rule only ever returns AUTH_ADMIN (never YES, NO, a cached or self result).
#[test]
fn test_nlk_rules_file_only_returns_auth_admin() {
    let code = rule_code();
    for forbidden in [
        "polkit.Result.YES",
        "polkit.Result.NO",
        "AUTH_ADMIN_KEEP",
        "AUTH_SELF",
        "NOT_HANDLED",
        "addAdminRule",
        "spawn",
    ] {
        assert!(
            !code.contains(forbidden),
            "{RULES} must not contain `{forbidden}`"
        );
    }
    assert_eq!(code.matches("polkit.addRule(").count(), 1, "one rule");
    assert_eq!(
        code.matches("return").count(),
        1,
        "exactly one return statement"
    );
    assert_eq!(
        code.matches("return polkit.Result.AUTH_ADMIN;").count(),
        1,
        "the single return is AUTH_ADMIN"
    );
    assert!(
        !code.contains("subject."),
        "the rule applies to every subject (it never reads `subject`)"
    );
}

/// NLK1: every other action falls through: the only return sits inside the membership guard
/// and there is no `else` branch.
#[test]
fn test_nlk_rules_file_falls_through_for_other_actions() {
    let code = rule_code();
    let guard = code
        .find("if (soosNetworkLockActions.indexOf(action.id) >= 0) {")
        .expect("membership guard on action.id");
    let ret = code.find("return").expect("return");
    assert!(guard < ret, "the return is inside the guard");
    let after_guard = &code[guard..];
    let close = after_guard.find('}').expect("end of guard");
    assert!(
        after_guard[..close].contains("return polkit.Result.AUTH_ADMIN;"),
        "AUTH_ADMIN only inside the guard"
    );
    assert!(
        !code.contains("else"),
        "no else branch: other actions fall through"
    );
    assert!(!code.contains("switch"), "no other dispatch");
}

/// NLK9: the rfkill rule is one line removing the uaccess tag, ordered after
/// `70-uaccess.rules` and before `73-seat-late.rules`.
#[test]
fn test_nlk_rfkill_rule_only_removes_the_uaccess_tag() {
    let text = read(RFKILL_RULE);
    let rules: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    assert_eq!(
        rules,
        vec![r#"SUBSYSTEM=="misc", KERNEL=="rfkill", TAG-="uaccess""#],
        "the rfkill option only removes the uaccess tag"
    );
    let name = RFKILL_RULE.rsplit('/').next().expect("file name");
    let prefix: u32 = name[..2].parse().expect("numeric prefix");
    assert!(
        prefix > 70 && prefix < 73,
        "{name} must sort after 70-uaccess.rules and before 73-seat-late.rules"
    );
}

/// NLK2: `soos-admin` embeds both files from `packaging/` (single source, byte-identical
/// comparison in `status`); no copy of the rule text lives in the crate.
#[test]
fn test_nlk_rule_contents_single_sourced_from_packaging() {
    let module = read(MODULE);
    assert!(
        module.contains("include_str!(\"../../../packaging/polkit/49-soos-network-lock.rules\")")
    );
    assert!(module
        .contains("include_str!(\"../../../packaging/udev/72-soos-network-lock-rfkill.rules\")"));
    assert!(
        !module.contains("polkit.addRule"),
        "the rule text is not duplicated in the crate"
    );
    assert!(module.contains("Sha256"), "status compares SHA-256 digests");
    assert!(
        module.contains("O_NOFOLLOW"),
        "rule files are opened with O_NOFOLLOW"
    );
}

/// NLK2, NLK3: opt-in (the installer does not install the lock) and removed by uninstall,
/// before the PAM rollback step that can end the script early.
#[test]
fn test_nlk_lock_is_opt_in_and_removed_on_uninstall() {
    let install = read("scripts/install.sh");
    assert!(
        !install.contains("network-lock install") && !install.contains("49-soos-network-lock"),
        "scripts/install.sh must not install the opt-in lock"
    );
    for pkg in [
        "packaging/debian/rules",
        "packaging/debian/postinst",
        "packaging/arch/PKGBUILD",
        "packaging/arch/soos.install",
        "packaging/rpm/soos.spec",
    ] {
        let text = read(pkg);
        assert!(
            !text.contains("49-soos-network-lock") && !text.contains("network-lock install"),
            "{pkg} must not activate the opt-in lock"
        );
    }
    let uninstall = read("scripts/uninstall.sh");
    for path in [
        "polkit-1/rules.d/49-soos-network-lock.rules",
        "udev/rules.d/72-soos-network-lock-rfkill.rules",
    ] {
        assert!(
            uninstall.contains(&format!("${{DESTDIR}}${{SYSCONFDIR}}/{path}"))
                || uninstall.contains(path),
            "scripts/uninstall.sh must remove {path}"
        );
        // Only executable lines count (plan evaluation round 2, N-2): a comment naming the
        // path before the PAM step must not satisfy the ordering.
        let pam_step = code_line_index(&uninstall, "# 2. Roll Back PAM", true);
        let listed = code_line_index(&uninstall, path, false);
        assert!(
            listed < pam_step,
            "{path} is removed before the PAM rollback (which can exit 1)"
        );
    }
    let pam_step = code_line_index(&uninstall, "# 2. Roll Back PAM", true);
    let removal = code_line_index(&uninstall, "rm -f -- \"${lock_file}\"", false);
    assert!(
        removal < pam_step,
        "the network lock files are deleted before the PAM rollback"
    );
    assert!(
        read(DOC).contains("before removing the package"),
        "Docs/NETWORK_LOCK.md tells to uninstall the lock before removing the package"
    );
}

/// NLK10: the Docker `pkcheck` test runs real polkitd in the pinned image and is part of
/// `CI Success`.
#[test]
fn test_nlk_docker_pkcheck_job_wired_into_ci() {
    let script = read(DOCKER_TEST);
    let default_image = script
        .lines()
        .find(|l| l.contains("SOOS_DEBIAN_IMAGE:-"))
        .expect("overridable default image");
    assert!(
        default_image.contains(
            "ubuntu:24.04@sha256:008173c23f95b170204355c12626cb5a965d779a7e1283b09e9cffbb1bf33ca3"
        ),
        "pinned image digest (same as tests/docker/pam_rollback_test.sh): {default_image}"
    );
    // Sentinel differential (plan evaluation round 1, F-2): a later rule answers YES for
    // every NetworkManager action, so every result the soos rule does not produce is rc 0.
    for needle in [
        "90-soos-test-sentinel.rules",
        "polkit.Result.YES",
        "pkaction",
    ] {
        assert!(
            script.contains(needle),
            "{DOCKER_TEST} must contain `{needle}`"
        );
    }
    assert!(
        !script.contains("${{ github.event"),
        "no untrusted event data in the script"
    );
    for needle in [
        "pkcheck --action-id",
        "polkitd",
        "packaging/polkit/49-soos-network-lock.rules",
        "retains_authorization_after_challenge",
        "settings.modify.hostname",
    ] {
        assert!(
            script.contains(needle),
            "{DOCKER_TEST} must contain `{needle}`"
        );
    }
    assert!(
        !script.contains(":/workspace\"") && !script.contains(":/workspace "),
        "the workspace is mounted read-only"
    );
    let ci = read(CI);
    assert!(
        ci.contains("  network-lock-polkit:"),
        "CI job network-lock-polkit"
    );
    assert!(ci.contains(&format!("run: ./{DOCKER_TEST}")));
    let needs = ci
        .lines()
        .find(|l| l.trim_start().starts_with("needs: [lint, clippy"))
        .expect("ci-success needs");
    assert!(needs.contains("network-lock-polkit"), "{needs}");
}

/// NLK11: documentation, ADR and walkthrough.
#[test]
fn test_nlk_docs_adr_and_walkthrough() {
    let doc = read(DOC);
    for needle in [
        "AUTH_ADMIN",
        "own password",
        "sudo",
        "separate admin account",
        "face",
        "polkit-agent-helper",
        "rfkill",
        "iwd",
        "hardware",
        "root",
        "soos-admin network-lock status",
        "soos-admin network-lock install",
        "soos-admin network-lock uninstall",
        "--rfkill",
        "unsupported_backend",
        "insecure_permissions",
        "rfkill_uaccess",
        "rfkill_reboot_pending",
        "iwd_dbus_bypass",
        "netdev",
        "docker",
        "auth-polkit",
        "20-gdm.rules",
    ] {
        assert!(doc.contains(needle), "{DOC} must mention `{needle}`");
    }
    assert!(read("AI/DECISIONS.md").contains(ADR_TITLE), "ADR present");
    assert!(
        root().join(WALKTHROUGH).is_file(),
        "walkthrough 196 present"
    );
    assert!(
        read("Docs/PACKAGING_AND_PROVISIONING.md").contains("NETWORK_LOCK.md"),
        "packaging docs point to the network lock page"
    );
}
