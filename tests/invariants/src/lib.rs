//! Automated architectural security invariant test suite for the `soos` workspace.
//!
//! These tests validate on every `cargo test` run that all code (written by humans or AI)
//! strictly adheres to the non-negotiable security invariants defined in `AI/ARCHITECTURE.md`
//! and `AGENTS.md`.
//!
//! All tests fail strictly (fail-closed): if a required file is missing or misplaced,
//! the test immediately fails.

#![forbid(unsafe_code)]

#[cfg(all(test, unix))]
mod installer_contract;

#[cfg(test)]
mod pad_contract;

/// Multi-distribution test infrastructure invariants (GitHub #162, #163, #168).
#[cfg(test)]
mod distro_matrix;

/// Repository-wide verification-matrix citation invariants (GitHub #187, TCI-04).
#[cfg(test)]
mod matrix_citations;

/// Physical validation suite versus the `soos-enroll` CLI (GitHub #186).
#[cfg(test)]
mod physical_contract;

/// PAM deadline wording in the normative documents (GitHub #185).
#[cfg(test)]
mod pam_deadline_contract;

/// Storage CLI deadline, JSON, dead-code and privilege-order hygiene (GitHub #231-#237).
#[cfg(test)]
mod cli_hygiene_contract;

/// Dependency hygiene, Docker sandbox toolchain and `save.sh` commit contract
/// (GitHub #243, #244, #245; matrix DDS1–DDS8).
#[cfg(all(test, unix))]
mod dependency_tooling_contract;

/// PAD startup self-test wiring in the daemon (GitHub #214).
#[cfg(test)]
mod pad_startup_contract;

/// Traceability tooling contract for `scripts/sync_issue.py` (GitHub #188).
#[cfg(all(test, unix))]
mod sync_issue_contract;

/// Camera documentation drift invariants (GitHub #197, CAM-15).
#[cfg(test)]
mod camera_docs_contract;

/// Response timestamps are not documented as replay protection (GitHub #219).
#[cfg(test)]
mod pam_response_freshness_contract;

/// PAM feedback seam, `PAM_SILENT` and nightly fuzzing (GitHub #220, #221, #227).
#[cfg(test)]
mod pam_feedback_contract;

/// Strict protocol codec, single-buffer encoding and IPC documentation (GitHub #224–#226).
#[cfg(test)]
mod protocol_codec_contract;

/// Metadata-only template listing in `soos-enroll list` and the GUI (GitHub #235, EFL).
#[cfg(test)]
mod enroll_list_metadata_contract;

/// PAM handle-guard removal and fault-injection ordering (GitHub #220, PHS4, PHS11).
#[cfg(test)]
mod pam_handle_guard_removal_contract;

/// Daemon documentation versus code (GitHub #201, #205, #206).
#[cfg(test)]
mod daemon_docs_contract;

/// Onboarding commands and packaging metadata (GitHub #207, #210).
#[cfg(all(test, unix))]
mod onboarding_packaging_contract;

/// Installer distro templates, model download hardening and first-start guards
/// (GitHub #208, #209, #211).
#[cfg(all(test, unix))]
mod installer_templates_contract;

/// Repository hygiene and documentation-drift contract (GitHub #238, #239, #240).
#[cfg(all(test, unix))]
mod maintainer_hygiene_contract;

/// Shared test-fixture crate contract (GitHub #241, TCI-10).
#[cfg(test)]
mod fixtures_contract;

/// Hardened production-code lexing and PAM panic / async-runtime checks (GitHub #242, TCI-11).
#[cfg(test)]
mod lexing_contract;

/// End-to-end contract of `scripts/candid_review.sh` (GitHub #242, TCI-11).
#[cfg(all(test, unix))]
mod candid_review_contract;

/// Packaging, GDM timeout and PAM camera follow-ups (GitHub #281, rows QFU1–QFU8).
#[cfg(all(test, unix))]
mod quality_followups;

/// Pinned Rust toolchain in rust-toolchain.toml and the sandbox Dockerfiles (rows TCP1–TCP3).
#[cfg(test)]
mod toolchain_pin_contract;

/// Project licence AGPL-3.0-or-later: workspace metadata, LICENSE file, README (rows LIC1–LIC3).
#[cfg(test)]
mod licence_contract;

/// Attested inference path: verified in-memory model bytes, explicit SCRFD score activation,
/// legacy 4-model artefacts removed (GitHub #246, #247, #249).
#[cfg(test)]
mod vision_attestation_contract;

/// Vision threshold single-source and vision documentation invariants (GitHub #215, #250, #251).
#[cfg(test)]
mod vision_threshold_contract;

/// ORT-owned output tensors are wiped in place (GitHub #255).
#[cfg(test)]
mod ort_output_zeroize_contract;

/// PAM module hygiene: panic hook chaining, protocol predicates, config warnings
/// (GitHub #263, #264, #265, rows PHY1–PHY9).
#[cfg(test)]
mod pam_hygiene_contract;

/// Encrypted stores write only AES-GCM AAD-bound payloads (GitHub #266, rows SAD1–SAD6).
#[cfg(test)]
mod storage_aad_contract;

/// Verified, pinned rustup bootstrap for hosts and sandbox images (GitHub #260, rows IRP1–IRP5).
#[cfg(all(test, unix))]
mod rustup_bootstrap_contract;

/// `soos-daemon.service` start conditions evaluated by systemd (GitHub #211, rows IRP6–IRP7).
#[cfg(all(test, unix))]
mod unit_start_guard_contract;

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Locates the Cargo workspace root by traversing up to Cargo.toml
    fn workspace_root() -> PathBuf {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest_dir
            .parent()
            .and_then(|p| p.parent())
            .expect("Unable to locate workspace root directory")
            .to_path_buf()
    }

    /// Invariant 1 — #![forbid(unsafe_code)] mandatory in all business crates
    #[test]
    fn test_business_crates_forbid_unsafe_code() {
        let root = workspace_root();
        let business_crates = [
            "protocol",
            "policy",
            "vision",
            "inference-ort",
            "biometric-store",
            "evidence-store",
            "enrollment-cli",
            "admin-cli",
            "gui",
        ];

        for crate_name in business_crates {
            let lib_path = root
                .join("crates")
                .join(crate_name)
                .join("src")
                .join("lib.rs");
            if lib_path.exists() {
                let content = fs::read_to_string(&lib_path)
                    .unwrap_or_else(|e| panic!("Error reading {}: {}", lib_path.display(), e));
                assert!(
                    content.contains("#![forbid(unsafe_code)]"),
                    "INVARIANT VIOLATION: Business crate '{}' in {} MUST declare '#![forbid(unsafe_code)]'!",
                    crate_name,
                    lib_path.display()
                );
            }
        }
    }

    /// Invariant 2 — Zero unwrap() or expect() in PAM module production code
    /// (Fail-closed: fails if crates/pam/src is missing)
    #[test]
    fn test_pam_crate_has_no_unwraps_or_expects_in_production_code() {
        let root = workspace_root();
        let pam_src = root.join("crates").join("pam").join("src");

        assert!(
            pam_src.exists(),
            "STRUCTURE VIOLATION: Required directory '{}' not found!",
            pam_src.display()
        );

        let mut rs_files = Vec::new();
        collect_rs_files(&pam_src, &mut rs_files);
        assert!(
            !rs_files.is_empty(),
            "No .rs source files discovered in '{}'",
            pam_src.display()
        );

        for file in rs_files {
            let content = fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("Error reading {}: {}", file.display(), e));

            let prod_code = extract_production_code(&content);

            let unwraps: Vec<(usize, &str)> = prod_code
                .lines()
                .enumerate()
                .filter(|(_, line)| {
                    let trimmed = line.trim();
                    !trimmed.starts_with("//")
                        && !trimmed.starts_with("/*")
                        && !trimmed.starts_with('*')
                        && (trimmed.contains(".unwrap()") || trimmed.contains(".expect("))
                })
                .collect();

            assert!(
                unwraps.is_empty(),
                "SECURITY INVARIANT VIOLATION: unwrap() or expect() detected in PAM production code of {}:\n{:#?}",
                file.display(),
                unwraps
            );
        }
    }

    /// Invariant 3 — Absolute prohibition of Tokio runtime in PAM module
    /// (Fail-closed: fails if crates/pam/Cargo.toml is missing)
    #[test]
    fn test_pam_crate_has_no_tokio_dependency() {
        let root = workspace_root();
        let pam_cargo = root.join("crates").join("pam").join("Cargo.toml");

        assert!(
            pam_cargo.exists(),
            "STRUCTURE VIOLATION: Required file '{}' not found!",
            pam_cargo.display()
        );

        let content = fs::read_to_string(&pam_cargo)
            .unwrap_or_else(|e| panic!("Error reading {}: {}", pam_cargo.display(), e));
        assert!(
            !content.contains("tokio"),
            "ARCHITECTURE VIOLATION: The pam_soos crate must NEVER depend on Tokio!"
        );
    }

    /// Invariant 4 — Absolute prohibition of OpenCV across entire workspace
    #[test]
    fn test_no_opencv_in_any_cargo_toml() {
        let root = workspace_root();
        let mut cargo_tomls = Vec::new();
        collect_files_named(&root, "Cargo.toml", &mut cargo_tomls);

        assert!(
            !cargo_tomls.is_empty(),
            "No Cargo.toml files found in project workspace"
        );

        for cargo_file in cargo_tomls {
            let content = fs::read_to_string(&cargo_file)
                .unwrap_or_else(|e| panic!("Error reading {}: {}", cargo_file.display(), e));
            assert!(
                !content.contains("opencv"),
                "ARCHITECTURE VIOLATION: Forbidden reference to 'opencv' in {}!",
                cargo_file.display()
            );
        }
    }

    /// Invariant 5 — Zero password, secret, frame, or embedding fields in Request and Response
    /// (Fail-closed: fails if crates/protocol/src/types.rs is missing)
    #[test]
    fn test_protocol_request_and_response_have_no_sensitive_fields() {
        let root = workspace_root();
        let types_rs = root
            .join("crates")
            .join("protocol")
            .join("src")
            .join("types.rs");

        assert!(
            types_rs.exists(),
            "STRUCTURE VIOLATION: Required file '{}' not found!",
            types_rs.display()
        );

        let content = fs::read_to_string(&types_rs)
            .unwrap_or_else(|e| panic!("Error reading {}: {}", types_rs.display(), e));

        let forbidden_keywords = [
            "password",
            "secret",
            "credential",
            "embedding",
            "frame",
            "image",
        ];

        for struct_keyword in ["struct Request", "struct Response"] {
            let body = extract_struct_body(&content, struct_keyword)
                .unwrap_or_else(|| panic!("Structure '{}' not found in types.rs", struct_keyword));

            for line in body.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("//")
                    || trimmed.starts_with("/*")
                    || trimmed.starts_with('*')
                    || trimmed.starts_with('#')
                {
                    continue;
                }

                if let Some(colon_idx) = trimmed.find(':') {
                    let field_decl = &trimmed[..colon_idx].to_lowercase();
                    for &forbidden in &forbidden_keywords {
                        let words: Vec<&str> = field_decl.split_whitespace().collect();
                        if let Some(field_name) = words.last() {
                            assert!(
                                !field_name.contains(forbidden),
                                "SECURITY INVARIANT VIOLATION: Struct '{}' contains forbidden field '{}' (declaration: '{}')!",
                                struct_keyword,
                                forbidden,
                                trimmed
                            );
                        }
                    }
                }
            }
        }
    }

    /// Invariant 6 — Absolute prohibition of Nokhwa camera crate across entire workspace
    #[test]
    fn test_no_nokhwa_in_any_cargo_toml() {
        let root = workspace_root();
        let mut cargo_tomls = Vec::new();
        collect_files_named(&root, "Cargo.toml", &mut cargo_tomls);

        assert!(
            !cargo_tomls.is_empty(),
            "No Cargo.toml files found in project workspace"
        );

        for cargo_file in cargo_tomls {
            let content = fs::read_to_string(&cargo_file)
                .unwrap_or_else(|e| panic!("Error reading {}: {}", cargo_file.display(), e));
            assert!(
                !content.contains("nokhwa"),
                "ARCHITECTURE VIOLATION: Forbidden reference to 'nokhwa' in {}!",
                cargo_file.display()
            );
        }
    }

    /// Invariant 7 — Mandatory overflow-checks in both release and dev profiles
    #[test]
    fn test_workspace_cargo_toml_enforces_overflow_checks() {
        let root = workspace_root();
        let root_cargo = root.join("Cargo.toml");
        assert!(
            root_cargo.exists(),
            "STRUCTURE VIOLATION: Root Cargo.toml not found!"
        );

        let content = fs::read_to_string(&root_cargo)
            .unwrap_or_else(|e| panic!("Error reading {}: {}", root_cargo.display(), e));

        assert!(
            content.contains("[profile.release]"),
            "SECURITY VIOLATION: Root Cargo.toml must define [profile.release]!"
        );
        assert!(
            content.contains("overflow-checks = true"),
            "SECURITY VIOLATION: Root Cargo.toml must enforce 'overflow-checks = true'!"
        );
        assert!(
            content.contains("[workspace.lints.clippy]"),
            "SECURITY VIOLATION: Root Cargo.toml must define [workspace.lints.clippy]!"
        );
    }

    /// Invariant 8 — Absolute prohibition of stdout/stderr prints in PAM production code
    #[test]
    fn test_pam_crate_has_no_stdout_or_stderr_prints_in_production_code() {
        let root = workspace_root();
        let pam_src = root.join("crates").join("pam").join("src");
        assert!(
            pam_src.exists(),
            "STRUCTURE VIOLATION: Required directory '{}' not found!",
            pam_src.display()
        );

        let mut rs_files = Vec::new();
        collect_rs_files(&pam_src, &mut rs_files);
        assert!(
            !rs_files.is_empty(),
            "No .rs source files discovered in '{}'",
            pam_src.display()
        );

        for file in rs_files {
            let content = fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("Error reading {}: {}", file.display(), e));

            let prod_code = extract_production_code(&content);

            let prints: Vec<(usize, &str)> = prod_code
                .lines()
                .enumerate()
                .filter(|(_, line)| {
                    let trimmed = line.trim();
                    !trimmed.starts_with("//")
                        && !trimmed.starts_with("/*")
                        && !trimmed.starts_with('*')
                        && (trimmed.contains("println!(")
                            || trimmed.contains("eprintln!(")
                            || trimmed.contains("print!(")
                            || trimmed.contains("eprint!(")
                            || trimmed.contains("dbg!("))
                })
                .collect();

            assert!(
                prints.is_empty(),
                "SECURITY INVARIANT VIOLATION: stdout/stderr print or dbg! detected in PAM production code of {}:\n{:#?}",
                file.display(),
                prints
            );
        }
    }

    /// Invariant 9 — Zero network dependencies or socket usage in evidence-store crate
    #[test]
    fn test_evidence_store_has_no_network_dependencies() {
        let root = workspace_root();
        let evidence_cargo = root
            .join("crates")
            .join("evidence-store")
            .join("Cargo.toml");

        assert!(
            evidence_cargo.exists(),
            "STRUCTURE VIOLATION: Required file '{}' not found!",
            evidence_cargo.display()
        );

        let cargo_content = fs::read_to_string(&evidence_cargo)
            .unwrap_or_else(|e| panic!("Error reading {}: {}", evidence_cargo.display(), e));

        let forbidden_crates = [
            "reqwest",
            "hyper",
            "curl",
            "ureq",
            "tungstenite",
            "tokio-tungstenite",
            "surf",
        ];

        for &banned in &forbidden_crates {
            assert!(
                !cargo_content.contains(banned),
                "SECURITY INVARIANT VIOLATION: Forbidden network dependency '{}' found in {}!",
                banned,
                evidence_cargo.display()
            );
        }

        let evidence_src = root.join("crates").join("evidence-store").join("src");
        if evidence_src.exists() {
            let mut rs_files = Vec::new();
            collect_rs_files(&evidence_src, &mut rs_files);
            for file in rs_files {
                let content = fs::read_to_string(&file)
                    .unwrap_or_else(|e| panic!("Error reading {}: {}", file.display(), e));
                let prod = extract_production_code(&content);
                assert!(
                    !prod.contains("std::net") && !prod.contains("tokio::net"),
                    "SECURITY INVARIANT VIOLATION: Network namespace usage found in production code of {}!",
                    file.display()
                );
            }
        }
    }

    // --- Lexical Analysis Helpers ---

    /// Extracts struct body between outer `{` and `}` braces
    fn extract_struct_body<'a>(source: &'a str, struct_keyword: &str) -> Option<&'a str> {
        let pos = source.find(struct_keyword)?;
        let after_keyword = &source[pos..];
        let open_brace = after_keyword.find('{')?;
        let struct_content = &after_keyword[open_brace + 1..];

        let mut depth = 1;
        for (idx, ch) in struct_content.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&struct_content[..idx]);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Filters source code to discard `#[cfg(test)] mod ... { ... }` blocks
    fn extract_production_code(source: &str) -> String {
        let mut result = String::new();
        let lines: Vec<&str> = source.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            let trimmed = lines[i].trim();
            if trimmed.contains("#[cfg(test)]")
                || (trimmed.starts_with("mod tests") && trimmed.contains('{'))
            {
                while i < lines.len() && !lines[i].contains('{') {
                    i += 1;
                }
                if i < lines.len() {
                    let mut depth = 1;
                    while i < lines.len() && depth > 0 {
                        i += 1;
                        if i < lines.len() {
                            for ch in lines[i].chars() {
                                if ch == '{' {
                                    depth += 1;
                                } else if ch == '}' {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                result.push_str(lines[i]);
                result.push('\n');
            }
            i += 1;
        }

        result
    }

    /// Invariant: Full PAM Docker test matrix artifacts must exist and specify supported distributions
    #[test]
    fn test_pam_docker_matrix_files_and_distro_configs_exist() {
        let root = workspace_root();
        let docker_dir = root.join("tests").join("docker");
        assert!(
            docker_dir.exists(),
            "INVARIANT VIOLATION: tests/docker directory must exist for PAM test matrix"
        );

        let required_files = [
            "Dockerfile.ubuntu",
            "Dockerfile.fedora",
            "Dockerfile.arch",
            "run_matrix.sh",
            "test_suite.sh",
            "mock_daemon.py",
            "pam_test_runner.c",
        ];

        for file_name in required_files {
            let path = docker_dir.join(file_name);
            assert!(
                path.exists(),
                "INVARIANT VIOLATION: Required matrix artifact '{}' not found in tests/docker/",
                file_name
            );
            let metadata = fs::metadata(&path).expect("metadata accessible");
            assert!(
                metadata.len() > 0,
                "INVARIANT VIOLATION: Matrix artifact '{}' is empty!",
                file_name
            );
        }

        // Verify multi-distro PAM configuration integration in Dockerfiles
        let ubuntu_df =
            fs::read_to_string(docker_dir.join("Dockerfile.ubuntu")).expect("ubuntu dockerfile");
        assert!(
            ubuntu_df.contains("common-auth") || ubuntu_df.contains("test-soos"),
            "Ubuntu Dockerfile must configure PAM stack (common-auth or test-soos)"
        );

        let fedora_df =
            fs::read_to_string(docker_dir.join("Dockerfile.fedora")).expect("fedora dockerfile");
        assert!(
            fedora_df.contains("system-auth") || fedora_df.contains("test-soos"),
            "Fedora Dockerfile must configure PAM stack (system-auth or test-soos)"
        );

        let arch_df =
            fs::read_to_string(docker_dir.join("Dockerfile.arch")).expect("arch dockerfile");
        assert!(
            arch_df.contains("system-auth") || arch_df.contains("test-soos"),
            "Arch Dockerfile must configure PAM stack (system-auth or test-soos)"
        );
    }

    /// PA11 Invariant: crates/pam must depend on pam-bindings 0.3.0 and implement PamHooks trait.
    #[test]
    fn test_pam_crate_uses_pam_bindings_and_implements_pam_hooks() {
        let root = workspace_root();
        let pam_cargo = root.join("crates").join("pam").join("Cargo.toml");
        assert!(pam_cargo.exists());
        let cargo_content = fs::read_to_string(&pam_cargo).expect("pam Cargo.toml");
        assert!(
            cargo_content.contains("pam_bindings") || cargo_content.contains("pam-bindings"),
            "crates/pam/Cargo.toml must declare pam_bindings dependency"
        );

        let root_cargo = root.join("Cargo.toml");
        let root_content = fs::read_to_string(&root_cargo).expect("root Cargo.toml");
        assert!(
            root_content.contains("pam-bindings") && root_content.contains("0.3.0"),
            "root Cargo.toml must declare pam-bindings version 0.3.0"
        );

        let lib_rs = root.join("crates").join("pam").join("src").join("lib.rs");
        let lib_content = fs::read_to_string(&lib_rs).expect("pam lib.rs");
        assert!(
            lib_content.contains("PamHooks for SoosPam") || lib_content.contains("impl PamHooks"),
            "crates/pam/src/lib.rs must implement PamHooks trait"
        );
    }

    /// PA12 Invariant: crates/pam must include syslog panic logging module and never log secrets.
    #[test]
    fn test_pam_crate_has_syslog_panic_logging_without_secrets() {
        let root = workspace_root();
        let syslog_rs = root
            .join("crates")
            .join("pam")
            .join("src")
            .join("syslog.rs");
        assert!(
            syslog_rs.exists(),
            "crates/pam/src/syslog.rs must exist for panic syslog logging"
        );
        let syslog_content = fs::read_to_string(&syslog_rs).expect("syslog.rs content");
        assert!(
            syslog_content.contains("libc::syslog"),
            "crates/pam/src/syslog.rs must call libc::syslog"
        );
        assert!(
            syslog_content.contains("LOG_AUTHPRIV"),
            "crates/pam/src/syslog.rs must use LOG_AUTHPRIV facility"
        );
    }

    fn collect_rs_files(dir: &Path, files: &mut Vec<PathBuf>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    collect_rs_files(&path, files);
                } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
                    files.push(path);
                }
            }
        }
    }

    fn collect_files_named(dir: &Path, target_name: &str, files: &mut Vec<PathBuf>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if file_name == "target" || file_name == ".git" {
                    continue;
                }
                if path.is_dir() {
                    collect_files_named(&path, target_name, files);
                } else if file_name == target_name {
                    files.push(path);
                }
            }
        }
    }

    /// Invariant: PAM Configuration Ordering Matches Spec (Sub-issue #26.2)
    /// Validates that Debian, Fedora, and Arch PAM configurations place pam_soos.so
    /// before pam_unix, and password-failed event handler after pam_unix.
    #[test]
    fn test_pam_config_ordering_matches_spec() {
        let root = workspace_root();

        // 1. Debian pam-auth-update profiles
        let debian_soos = root.join("packaging/pam/debian/soos");
        let debian_notify = root.join("packaging/pam/debian/soos-notify");
        assert!(debian_soos.exists(), "packaging/pam/debian/soos must exist");
        assert!(
            debian_notify.exists(),
            "packaging/pam/debian/soos-notify must exist"
        );

        let deb_soos_content = fs::read_to_string(&debian_soos).expect("read debian soos");
        let deb_notify_content = fs::read_to_string(&debian_notify).expect("read debian notify");

        assert!(
            deb_soos_content
                .lines()
                .any(is_default_timeout_primary_soos_rule),
            "Debian soos profile must carry the primary rule relying on the default timeout"
        );
        assert!(
            deb_soos_content.contains("Priority: 260") || deb_soos_content.contains("Priority: 26")
        );
        assert!(deb_notify_content.contains("pam_soos.so event=password-failed timeout_ms=20"));
        assert!(
            deb_notify_content.contains("Priority: 128")
                || deb_notify_content.contains("Priority: 12")
        );

        // 2. Fedora authselect profile
        let fedora_system_auth = root.join("packaging/pam/fedora/soos/system-auth");
        assert!(
            fedora_system_auth.exists(),
            "packaging/pam/fedora/soos/system-auth must exist"
        );
        let fedora_content =
            fs::read_to_string(&fedora_system_auth).expect("read fedora system-auth");

        let soos_pos =
            primary_soos_rule_offset(&fedora_content, "packaging/pam/fedora/soos/system-auth");
        let unix_pos = fedora_content
            .find("pam_unix.so")
            .expect("unix auth in fedora");
        let fail_pos = fedora_content
            .find("pam_soos.so event=password-failed")
            .expect("failed auth in fedora");
        assert!(
            soos_pos < unix_pos,
            "soos must appear before pam_unix in Fedora PAM stack"
        );
        assert!(
            unix_pos < fail_pos,
            "password-failed handler must appear after pam_unix in Fedora PAM stack"
        );

        // 3. Arch Linux system-auth and snippet
        let arch_system_auth = root.join("packaging/pam/arch/system-auth");
        let arch_snippet = root.join("packaging/pam/arch/system-auth.snippet");
        assert!(
            arch_system_auth.exists(),
            "packaging/pam/arch/system-auth must exist"
        );
        assert!(
            arch_snippet.exists(),
            "packaging/pam/arch/system-auth.snippet must exist"
        );

        let arch_content = fs::read_to_string(&arch_system_auth).expect("read arch system-auth");
        let arch_soos_pos =
            primary_soos_rule_offset(&arch_content, "packaging/pam/arch/system-auth");
        let arch_unix_pos = arch_content.find("pam_unix.so").expect("unix auth in arch");
        let arch_fail_pos = arch_content
            .find("pam_soos.so event=password-failed")
            .expect("failed auth in arch");
        assert!(
            arch_soos_pos < arch_unix_pos,
            "soos must appear before pam_unix in Arch PAM stack"
        );
        assert!(
            arch_unix_pos < arch_fail_pos,
            "password-failed handler must appear after pam_unix in Arch PAM stack"
        );

        let snippet_content = fs::read_to_string(&arch_snippet).expect("read arch snippet");
        assert!(snippet_content
            .lines()
            .any(|l| l.starts_with("auth") && is_default_timeout_primary_soos_rule(l)));
        assert!(snippet_content.contains(
            "auth  optional                       pam_soos.so event=password-failed timeout_ms=20"
        ));
    }

    /// Invariant: Install Script Creates Required Directories (Sub-issue #26.1)
    #[test]
    fn test_install_script_creates_required_directories() {
        let root = workspace_root();
        let install_sh = root.join("scripts/install.sh");
        assert!(install_sh.exists(), "scripts/install.sh must exist");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&install_sh).expect("metadata of install.sh");
            assert_ne!(
                metadata.permissions().mode() & 0o111,
                0,
                "scripts/install.sh must be executable"
            );
        }

        // Test running install.sh with --destdir into a temporary directory
        let tmp_dir =
            std::env::temp_dir().join(format!("soos_install_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp_dir);
        fs::create_dir_all(&tmp_dir).expect("create tmp_dir");

        // Contract migration (GitHub #164 / ONB-06): install.sh now fails closed when
        // an artifact is missing, so the staging run is given a complete artifact set.
        let artifact_dir = crate::installer_contract::scratch_dir("install_dirs_artifacts");
        crate::installer_contract::stage_fixture_artifacts(&artifact_dir);
        let status = std::process::Command::new("bash")
            .arg(&install_sh)
            .arg("--destdir")
            .arg(&tmp_dir)
            .arg("--artifact-dir")
            .arg(&artifact_dir)
            .arg("--skip-models")
            .arg("--skip-systemd")
            .status()
            .expect("execute install.sh");
        let _ = fs::remove_dir_all(&artifact_dir);

        assert!(
            status.success(),
            "install.sh must exit successfully with --destdir"
        );

        // Verify required directory hierarchy and permissions
        let biometrics_dir = tmp_dir.join("var/lib/soos/biometrics");
        let evidence_dir = tmp_dir.join("var/lib/soos/evidence");
        let models_dir = tmp_dir.join("var/lib/soos/models");
        let libexec_dir = tmp_dir.join("usr/libexec/soos");
        let master_key = tmp_dir.join("var/lib/soos/master.key");

        assert!(
            biometrics_dir.is_dir(),
            "biometrics directory must be created"
        );
        assert!(evidence_dir.is_dir(), "evidence directory must be created");
        assert!(models_dir.is_dir(), "models directory must be created");
        assert!(libexec_dir.is_dir(), "libexec directory must be created");
        // Contract migration (GitHub #144 / ONB-01): a staged tree is package
        // content, so the master key must be generated on the target host by the
        // post-install scriptlet, never inside the staging root.
        assert!(
            !master_key.exists(),
            "master.key must NOT be generated under --destdir (GitHub #144)"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let bio_meta = fs::metadata(&biometrics_dir).expect("biometrics meta");
            assert_eq!(
                bio_meta.permissions().mode() & 0o777,
                0o700,
                "biometrics dir must be mode 0700"
            );

            let ev_meta = fs::metadata(&evidence_dir).expect("evidence meta");
            assert_eq!(
                ev_meta.permissions().mode() & 0o777,
                0o700,
                "evidence dir must be mode 0700"
            );
        }

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    /// Recursively collects every regular file below `dir` whose name ends with `.key`.
    fn collect_key_files(dir: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_type = entry.file_type().expect("file type");
            if file_type.is_dir() {
                collect_key_files(&path, found);
            } else if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".key"))
            {
                found.push(path);
            }
        }
    }

    /// Invariant: `install.sh --destdir` stages a package tree that contains no key
    /// material and ships the key provisioning helper instead (GitHub #144 / ONB-01).
    #[test]
    fn test_install_script_destdir_stages_no_key_material() {
        let root = workspace_root();
        let install_sh = root.join("scripts/install.sh");

        let tmp_dir =
            std::env::temp_dir().join(format!("soos_install_nokey_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp_dir);
        fs::create_dir_all(&tmp_dir).expect("create tmp_dir");

        // Contract migration (GitHub #164 / ONB-06): complete artifact set required.
        let artifact_dir = crate::installer_contract::scratch_dir("install_nokey_artifacts");
        crate::installer_contract::stage_fixture_artifacts(&artifact_dir);
        let status = std::process::Command::new("bash")
            .arg(&install_sh)
            .arg("--destdir")
            .arg(&tmp_dir)
            .arg("--artifact-dir")
            .arg(&artifact_dir)
            .arg("--skip-models")
            .arg("--skip-systemd")
            .status()
            .expect("execute install.sh");
        let _ = fs::remove_dir_all(&artifact_dir);
        assert!(status.success(), "install.sh --destdir must succeed");

        let mut staged_keys = Vec::new();
        collect_key_files(&tmp_dir, &mut staged_keys);
        assert!(
            staged_keys.is_empty(),
            "staged tree must contain no *.key file, found: {staged_keys:?}"
        );
        assert!(
            !tmp_dir.join("var/lib/soos/master.key").exists(),
            "var/lib/soos/master.key must be absent from the staged tree"
        );

        // The helper that generates the key on the target host must be shipped.
        let helper = tmp_dir.join("usr/libexec/soos/provision-master-key");
        assert!(
            helper.is_file(),
            "usr/libexec/soos/provision-master-key must be staged"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = fs::metadata(&helper).expect("helper meta");
            assert_eq!(
                meta.permissions().mode() & 0o777,
                0o755,
                "provision-master-key must be mode 0755"
            );
        }

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    /// Invariant: the shared key provisioning helper generates a 32-byte key with
    /// mode 0600 exactly once and never overwrites an existing key (GitHub #144).
    #[test]
    fn test_provision_master_key_helper_generates_0600_key_once() {
        let root = workspace_root();
        let helper = root.join("scripts/provision_master_key.sh");
        assert!(
            helper.exists(),
            "scripts/provision_master_key.sh must exist"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = fs::metadata(&helper).expect("helper metadata");
            assert_ne!(
                meta.permissions().mode() & 0o111,
                0,
                "scripts/provision_master_key.sh must be executable"
            );
        }
        let helper_content = fs::read_to_string(&helper).expect("read helper");
        assert!(
            helper_content.contains("umask 077"),
            "helper must restrict the creation umask so the key is 0600 from inception"
        );

        let tmp_dir =
            std::env::temp_dir().join(format!("soos_provision_key_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp_dir);
        let state_dir = tmp_dir.join("var/lib/soos");
        let key_path = state_dir.join("master.key");

        // 1. Fresh state directory (created by the helper): key generated.
        let status = std::process::Command::new("sh")
            .arg(&helper)
            .arg("--state-dir")
            .arg(&state_dir)
            .status()
            .expect("execute provision_master_key.sh");
        assert!(status.success(), "helper must succeed on a fresh state dir");
        assert!(key_path.is_file(), "helper must create master.key");
        let first = fs::read(&key_path).expect("read key");
        assert_eq!(first.len(), 32, "master.key must be 32 bytes");
        assert!(
            first.iter().any(|b| *b != 0),
            "master.key must not be all zeros"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = fs::metadata(&key_path).expect("key meta");
            assert_eq!(
                meta.permissions().mode() & 0o777,
                0o600,
                "master.key must be mode 0600"
            );
        }
        assert!(
            !state_dir.join("master.key.tmp").exists()
                && fs::read_dir(&state_dir)
                    .expect("read state dir")
                    .flatten()
                    .all(|e| e.file_name() == "master.key"),
            "helper must leave no temporary file behind"
        );

        // 2. Second run: idempotent, key content unchanged.
        let status = std::process::Command::new("sh")
            .arg(&helper)
            .arg("--state-dir")
            .arg(&state_dir)
            .status()
            .expect("execute provision_master_key.sh twice");
        assert!(status.success(), "helper must succeed when the key exists");
        let second = fs::read(&key_path).expect("read key again");
        assert_eq!(
            first, second,
            "an existing master.key must never be overwritten"
        );

        // 3. Pre-existing key with lax permissions: content kept, mode tightened.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&key_path, fs::Permissions::from_mode(0o644))
                .expect("loosen key mode");
            let status = std::process::Command::new("sh")
                .arg(&helper)
                .arg("--state-dir")
                .arg(&state_dir)
                .status()
                .expect("execute provision_master_key.sh on lax key");
            assert!(status.success(), "helper must succeed on a lax key");
            let meta = fs::metadata(&key_path).expect("key meta");
            assert_eq!(
                meta.permissions().mode() & 0o777,
                0o600,
                "helper must tighten an existing key to mode 0600"
            );
            assert_eq!(
                fs::read(&key_path).expect("read key"),
                first,
                "tightening permissions must not change the key"
            );
        }

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    /// Invariant: the key provisioning helper refuses symlinks and non-regular
    /// files at the key path and fails closed without writing through them.
    #[test]
    fn test_provision_master_key_helper_refuses_symlink_and_non_regular() {
        let root = workspace_root();
        let helper = root.join("scripts/provision_master_key.sh");
        assert!(
            helper.exists(),
            "scripts/provision_master_key.sh must exist"
        );

        let tmp_dir = std::env::temp_dir().join(format!(
            "soos_provision_key_symlink_test_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp_dir);

        // 1. Dangling symlink at master.key: must not be followed.
        let state_dir = tmp_dir.join("symlink/var/lib/soos");
        fs::create_dir_all(&state_dir).expect("create state dir");
        let target = tmp_dir.join("symlink/escaped.bin");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, state_dir.join("master.key")).expect("symlink");
        let status = std::process::Command::new("sh")
            .arg(&helper)
            .arg("--state-dir")
            .arg(&state_dir)
            .status()
            .expect("execute helper on symlink");
        assert!(
            !status.success(),
            "helper must fail when master.key is a symlink"
        );
        assert!(
            !target.exists(),
            "helper must not write through the symlink target"
        );

        // 2. Directory at master.key: must fail closed.
        let dir_state = tmp_dir.join("dir/var/lib/soos");
        fs::create_dir_all(dir_state.join("master.key")).expect("create dir at key path");
        let status = std::process::Command::new("sh")
            .arg(&helper)
            .arg("--state-dir")
            .arg(&dir_state)
            .status()
            .expect("execute helper on directory");
        assert!(
            !status.success(),
            "helper must fail when master.key is not a regular file"
        );

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    /// Invariant: every native package scriptlet generates the master key on the
    /// target host through the shared helper, and no installer or scriptlet keeps an
    /// inline key generator (one source of truth, GitHub #144 / ONB-01).
    #[test]
    fn test_package_scriptlets_provision_key_via_shared_helper() {
        let root = workspace_root();
        let helper_path = "/usr/libexec/soos/provision-master-key";

        let postinst = fs::read_to_string(root.join("packaging/debian/postinst"))
            .expect("read debian postinst");
        let arch_install = fs::read_to_string(root.join("packaging/arch/soos.install"))
            .expect("read arch soos.install");
        let spec = fs::read_to_string(root.join("packaging/rpm/soos.spec")).expect("read rpm spec");
        let pkgbuild =
            fs::read_to_string(root.join("packaging/arch/PKGBUILD")).expect("read PKGBUILD");
        let install_sh =
            fs::read_to_string(root.join("scripts/install.sh")).expect("read install.sh");
        let uninstall_sh =
            fs::read_to_string(root.join("scripts/uninstall.sh")).expect("read uninstall.sh");

        // 1. Each post-install path invokes the shipped helper.
        let configure_branch = postinst
            .split("configure)")
            .nth(1)
            .expect("postinst configure branch");
        assert!(
            configure_branch.contains(helper_path),
            "debian postinst configure branch must call {helper_path}"
        );
        let post_install = arch_install
            .split("post_install()")
            .nth(1)
            .expect("soos.install post_install body");
        assert!(
            post_install.contains(helper_path),
            "arch post_install must call {helper_path}"
        );
        let post_section = spec.split("%post\n").nth(1).expect("spec %post section");
        assert!(
            post_section.contains(helper_path),
            "rpm %post must call {helper_path}"
        );

        // 2. The helper is shipped by every package recipe and removed on uninstall.
        assert!(
            spec.contains(&format!("%{{buildroot}}{helper_path}"))
                || spec.contains(&format!("%{{buildroot}}/{}", &helper_path[1..])),
            "rpm %install must stage {helper_path}"
        );
        let files_section = spec.split("%files\n").nth(1).expect("spec %files section");
        assert!(
            files_section.contains(helper_path),
            "rpm %files must list {helper_path}"
        );
        // User-approved assertion change (2026-09-30, walkthrough 106): the key must never be
        // owned by the RPM package, not even as `%ghost`, because RPM deletes owned `%ghost`
        // files on `rpm -e` (the key then no longer decrypts enrolled templates).
        assert!(
            !files_section
                .lines()
                .map(str::trim)
                .filter(|l| !l.starts_with('#'))
                .any(|l| l.contains("master.key")),
            "rpm %files must not list master.key in any form (host state, never packaged or owned)"
        );
        assert!(
            pkgbuild.contains("provision-master-key"),
            "PKGBUILD package() must install the provision-master-key helper"
        );
        assert!(
            install_sh.contains("provision-master-key"),
            "install.sh must install the provision-master-key helper"
        );
        assert!(
            uninstall_sh.contains("provision-master-key"),
            "uninstall.sh must remove the provision-master-key helper"
        );

        // 3. One source of truth: no inline key generator outside the helper.
        for (name, content) in [
            ("packaging/debian/postinst", postinst.as_str()),
            ("packaging/arch/soos.install", arch_install.as_str()),
            ("packaging/rpm/soos.spec", spec.as_str()),
            ("packaging/arch/PKGBUILD", pkgbuild.as_str()),
            ("scripts/install.sh", install_sh.as_str()),
        ] {
            assert!(
                !content.contains("openssl rand") && !content.contains("/dev/urandom"),
                "{name} must not generate key material inline (use the shared helper)"
            );
        }

        // 4. install.sh only provisions the key for a live install (empty DESTDIR).
        assert!(
            install_sh.contains("provision_master_key.sh"),
            "install.sh must delegate live key generation to scripts/provision_master_key.sh"
        );
    }

    /// Invariant: package builders refuse to package a staged tree that contains key
    /// material, through a shared, fail-closed guard (GitHub #144 / ONB-01).
    #[test]
    fn test_package_builders_refuse_staged_key_material() {
        let root = workspace_root();
        let guard = root.join("scripts/check_no_key_material.sh");
        assert!(
            guard.exists(),
            "scripts/check_no_key_material.sh must exist"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = fs::metadata(&guard).expect("guard metadata");
            assert_ne!(
                meta.permissions().mode() & 0o111,
                0,
                "scripts/check_no_key_material.sh must be executable"
            );
        }

        // 1. Every builder that stages through install.sh --destdir invokes the guard.
        for builder in [
            "scripts/build_deb.sh",
            "scripts/build_arch.sh",
            "packaging/debian/rules",
        ] {
            let content = fs::read_to_string(root.join(builder)).expect("read builder");
            assert!(
                content.contains("check_no_key_material.sh"),
                "{builder} must run scripts/check_no_key_material.sh on the staged tree"
            );
        }

        // 2. Functional: clean tree passes, a staged key fails closed.
        let tmp_dir =
            std::env::temp_dir().join(format!("soos_key_guard_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp_dir);
        let clean = tmp_dir.join("clean/var/lib/soos/biometrics");
        fs::create_dir_all(&clean).expect("create clean tree");
        fs::write(tmp_dir.join("clean/var/lib/soos/README"), b"no key here").expect("write");
        let status = std::process::Command::new("bash")
            .arg(&guard)
            .arg(tmp_dir.join("clean"))
            .status()
            .expect("execute guard on clean tree");
        assert!(
            status.success(),
            "guard must accept a tree without key material"
        );

        let dirty = tmp_dir.join("dirty/var/lib/soos");
        fs::create_dir_all(&dirty).expect("create dirty tree");
        fs::write(dirty.join("master.key"), [0x5Au8; 32]).expect("write fake key");
        let status = std::process::Command::new("bash")
            .arg(&guard)
            .arg(tmp_dir.join("dirty"))
            .status()
            .expect("execute guard on dirty tree");
        assert!(
            !status.success(),
            "guard must reject a tree containing var/lib/soos/master.key"
        );

        let nested = tmp_dir.join("nested/usr/share/soos");
        fs::create_dir_all(&nested).expect("create nested tree");
        fs::write(nested.join("evidence.key"), [0x11u8; 32]).expect("write nested key");
        let status = std::process::Command::new("bash")
            .arg(&guard)
            .arg(tmp_dir.join("nested"))
            .status()
            .expect("execute guard on nested tree");
        assert!(
            !status.success(),
            "guard must reject any *.key file anywhere in the tree"
        );

        // 3. A missing directory argument is an error, never a silent pass.
        let status = std::process::Command::new("bash")
            .arg(&guard)
            .arg(tmp_dir.join("does-not-exist"))
            .status()
            .expect("execute guard on missing dir");
        assert!(
            !status.success(),
            "guard must fail when the staged tree does not exist"
        );

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    /// Invariant: Uninstall Restores PAM Config and Cleans Up (Sub-issue #26.3)
    #[test]
    fn test_uninstall_restores_pam_config() {
        let root = workspace_root();
        let uninstall_sh = root.join("scripts/uninstall.sh");
        assert!(uninstall_sh.exists(), "scripts/uninstall.sh must exist");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&uninstall_sh).expect("metadata of uninstall.sh");
            assert_ne!(
                metadata.permissions().mode() & 0o111,
                0,
                "scripts/uninstall.sh must be executable"
            );
        }

        let tmp_dir =
            std::env::temp_dir().join(format!("soos_uninstall_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp_dir);
        let bio_dir = tmp_dir.join("var/lib/soos/biometrics");
        let bin_file = tmp_dir.join("usr/bin/soos-admin");
        let gui_bin_file = tmp_dir.join("usr/bin/soos-gui");
        let helper_file = tmp_dir.join("usr/libexec/soos/provision-master-key");
        let service_file = tmp_dir.join("etc/systemd/system/soos-daemon.service");
        let backup_pam = tmp_dir.join("etc/pam.d/system-auth.soos-backup");
        let active_pam = tmp_dir.join("etc/pam.d/system-auth");

        fs::create_dir_all(&bio_dir).expect("create bio_dir");
        fs::create_dir_all(tmp_dir.join("usr/bin")).expect("create bin dir");
        fs::create_dir_all(tmp_dir.join("usr/libexec/soos")).expect("create libexec dir");
        fs::create_dir_all(tmp_dir.join("etc/systemd/system")).expect("create systemd dir");
        fs::create_dir_all(tmp_dir.join("etc/pam.d")).expect("create pam.d dir");

        fs::write(&bin_file, b"binary content").expect("write bin");
        fs::write(&gui_bin_file, b"gui binary content").expect("write gui bin");
        fs::write(&helper_file, b"#!/bin/sh\nexit 0\n").expect("write helper");
        fs::write(&service_file, b"unit content").expect("write service");
        fs::write(
            &backup_pam,
            b"# Original PAM config\nauth required pam_unix.so\n",
        )
        .expect("write backup pam");
        fs::write(&active_pam, b"# Modified PAM config\nauth pam_soos.so\n")
            .expect("write active pam");
        fs::write(tmp_dir.join("var/lib/soos/master.key"), vec![0u8; 32]).expect("write key");

        // Run uninstall.sh with --destdir and --keep-data
        let status = std::process::Command::new("bash")
            .arg(&uninstall_sh)
            .arg("--destdir")
            .arg(&tmp_dir)
            .arg("--keep-data")
            .arg("--skip-systemd")
            .status()
            .expect("execute uninstall.sh");

        assert!(status.success(), "uninstall.sh must succeed");
        assert!(!bin_file.exists(), "binary must be removed");
        assert!(!gui_bin_file.exists(), "gui binary must be removed");
        assert!(
            !helper_file.exists(),
            "provision-master-key helper must be removed (GitHub #144)"
        );
        assert!(!service_file.exists(), "service file must be removed");
        assert!(
            bio_dir.exists(),
            "biometric data must be kept when --keep-data is specified"
        );
        assert!(
            tmp_dir.join("var/lib/soos/master.key").exists(),
            "master.key kept with --keep-data"
        );

        // Verify PAM restoration from backup
        let restored_content = fs::read_to_string(&active_pam).expect("read restored pam");
        assert!(
            restored_content.contains("auth required pam_unix.so"),
            "PAM backup must be restored"
        );
        assert!(
            !restored_content.contains("pam_soos.so"),
            "pam_soos must be gone after restoration"
        );

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    /// Invariant: Debian Packaging Specification (Sub-issue #27.1)
    #[test]
    fn test_debian_packaging_specification() {
        let root = workspace_root();
        let control_path = root.join("packaging/debian/control");
        let rules_path = root.join("packaging/debian/rules");
        let postinst_path = root.join("packaging/debian/postinst");
        let prerm_path = root.join("packaging/debian/prerm");

        assert!(control_path.exists(), "packaging/debian/control must exist");
        assert!(rules_path.exists(), "packaging/debian/rules must exist");
        assert!(
            postinst_path.exists(),
            "packaging/debian/postinst must exist"
        );
        assert!(prerm_path.exists(), "packaging/debian/prerm must exist");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let rules_meta = fs::metadata(&rules_path).expect("rules metadata");
            assert_ne!(
                rules_meta.permissions().mode() & 0o111,
                0,
                "packaging/debian/rules must be executable"
            );

            let postinst_meta = fs::metadata(&postinst_path).expect("postinst metadata");
            assert_ne!(
                postinst_meta.permissions().mode() & 0o111,
                0,
                "packaging/debian/postinst must be executable"
            );

            let prerm_meta = fs::metadata(&prerm_path).expect("prerm metadata");
            assert_ne!(
                prerm_meta.permissions().mode() & 0o111,
                0,
                "packaging/debian/prerm must be executable"
            );
        }

        let control_content = fs::read_to_string(&control_path).expect("read control");
        assert!(
            control_content.contains("Package: soos"),
            "control must declare Package: soos"
        );
        assert!(
            control_content.contains("Architecture:"),
            "control must declare Architecture"
        );
        assert!(
            control_content.contains("pam"),
            "control must specify PAM dependency"
        );
        assert!(
            control_content.contains("systemd"),
            "control must specify systemd dependency"
        );

        let postinst_content = fs::read_to_string(&postinst_path).expect("read postinst");
        assert!(
            postinst_content.contains("soos"),
            "postinst must handle soos system group"
        );
        assert!(
            postinst_content.contains("0700"),
            "postinst must enforce mode 0700 on biometrics/evidence"
        );
        assert!(
            postinst_content.contains("0750"),
            "postinst must enforce mode 0750 on runtime dir"
        );
        assert!(
            postinst_content.contains("master.key") && postinst_content.contains("0600"),
            "postinst must generate/enforce master.key mode 0600"
        );
        assert!(
            postinst_content.contains("pam-auth-update")
                || postinst_content.contains("pam-configs"),
            "postinst must configure Debian PAM stack"
        );

        let prerm_content = fs::read_to_string(&prerm_path).expect("read prerm");
        assert!(
            prerm_content.contains("soos-daemon.service"),
            "prerm must stop/disable soos-daemon.service"
        );
    }

    /// Invariant: RPM Packaging Specification (Sub-issue #27.2)
    #[test]
    fn test_rpm_packaging_specification() {
        let root = workspace_root();
        let spec_path = root.join("packaging/rpm/soos.spec");
        assert!(spec_path.exists(), "packaging/rpm/soos.spec must exist");

        let spec_content = fs::read_to_string(&spec_path).expect("read spec");
        assert!(
            spec_content.contains("Name: soos") || spec_content.contains("Name:\tsoos"),
            "RPM spec must declare Name: soos"
        );
        assert!(
            spec_content.contains("Requires:") && spec_content.contains("pam"),
            "RPM spec must require pam"
        );
        assert!(
            spec_content.contains("systemd"),
            "RPM spec must specify systemd requirement or build requirement"
        );
        assert!(
            spec_content.contains("%pre\n") || spec_content.contains("%pre "),
            "RPM spec must include %pre scriptlet"
        );
        assert!(
            spec_content.contains("%post\n") || spec_content.contains("%post "),
            "RPM spec must include %post scriptlet"
        );
        assert!(
            spec_content.contains("%preun\n") || spec_content.contains("%preun "),
            "RPM spec must include %preun scriptlet"
        );
        assert!(
            spec_content.contains("%files\n") || spec_content.contains("%files "),
            "RPM spec must include %files section"
        );

        // Verify group creation in %pre
        assert!(
            spec_content.contains("groupadd") || spec_content.contains("getent group soos"),
            "RPM spec %pre must create soos group"
        );

        // Verify directory and key permissions in %files or %post
        assert!(
            spec_content.contains("0700") && spec_content.contains("biometrics"),
            "RPM spec must specify 0700 mode for biometrics"
        );
        // User-approved assertion change (2026-09-30, walkthrough 106): the 0600 key mode is
        // enforced by the provisioning helper called from %post, not by a %files entry.
        let helper = fs::read_to_string(root.join("scripts/provision_master_key.sh"))
            .expect("read scripts/provision_master_key.sh");
        assert!(
            spec_content.contains("provision-master-key")
                && helper
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.starts_with('#'))
                    .any(|l| l.starts_with("chmod 0600") || l == "umask 077"),
            "RPM %post must provision the master key through the helper that enforces mode 0600"
        );
        assert!(
            spec_content.contains("0750") && spec_content.contains("soos"),
            "RPM spec must specify 0750 mode for runtime socket directory"
        );

        // Verify authselect integration for Fedora
        assert!(
            spec_content.contains("authselect"),
            "RPM spec must integrate with Fedora authselect"
        );
    }

    /// Invariant: Arch Linux PKGBUILD Specification (Sub-issue #27.3)
    #[test]
    fn test_arch_packaging_specification() {
        let root = workspace_root();
        let pkgbuild_path = root.join("packaging/arch/PKGBUILD");
        let install_path = root.join("packaging/arch/soos.install");

        assert!(pkgbuild_path.exists(), "packaging/arch/PKGBUILD must exist");
        assert!(
            install_path.exists(),
            "packaging/arch/soos.install must exist"
        );

        let pkgbuild_content = fs::read_to_string(&pkgbuild_path).expect("read PKGBUILD");
        assert!(
            pkgbuild_content.contains("pkgname=soos"),
            "PKGBUILD must declare pkgname=soos"
        );
        assert!(
            pkgbuild_content.contains("depends=") && pkgbuild_content.contains("pam"),
            "PKGBUILD must depend on pam"
        );
        assert!(
            pkgbuild_content.contains("install=soos.install"),
            "PKGBUILD must specify install=soos.install"
        );
        assert!(
            pkgbuild_content.contains("build()"),
            "PKGBUILD must define build() function"
        );
        assert!(
            pkgbuild_content.contains("package()"),
            "PKGBUILD must define package() function"
        );

        let install_content = fs::read_to_string(&install_path).expect("read soos.install");
        assert!(
            install_content.contains("post_install()"),
            "soos.install must define post_install()"
        );
        assert!(
            install_content.contains("pre_remove()"),
            "soos.install must define pre_remove()"
        );
        assert!(
            install_content.contains("groupadd") || install_content.contains("getent group soos"),
            "soos.install must create soos system group"
        );
        assert!(
            install_content.contains("0700"),
            "soos.install must enforce mode 0700 on biometrics/evidence"
        );
        assert!(
            install_content.contains("master.key") && install_content.contains("0600"),
            "soos.install must provision master.key with mode 0600"
        );
    }

    /// Invariant: Distribution Package Build Scripts Executable & CLI Help
    #[test]
    fn test_package_build_scripts_executable_and_help() {
        let root = workspace_root();
        let deb_script = root.join("scripts/build_deb.sh");
        let rpm_script = root.join("scripts/build_rpm.sh");
        let arch_script = root.join("scripts/build_arch.sh");
        let pkg_script = root.join("scripts/build_packages.sh");

        assert!(deb_script.exists(), "scripts/build_deb.sh must exist");
        assert!(rpm_script.exists(), "scripts/build_rpm.sh must exist");
        assert!(arch_script.exists(), "scripts/build_arch.sh must exist");
        assert!(pkg_script.exists(), "scripts/build_packages.sh must exist");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for script in [&deb_script, &rpm_script, &arch_script, &pkg_script] {
                let meta = fs::metadata(script).expect("script metadata");
                assert_ne!(
                    meta.permissions().mode() & 0o111,
                    0,
                    "script {:?} must be executable",
                    script.file_name()
                );
            }
        }

        // Test running scripts/build_packages.sh --help
        let status = std::process::Command::new("bash")
            .arg(&pkg_script)
            .arg("--help")
            .status()
            .expect("execute build_packages.sh --help");

        assert!(
            status.success(),
            "build_packages.sh --help must exit successfully"
        );
    }

    /// Invariant: Physical Hardware End-to-End Validation Suite (Issue #31 / GitHub #70)
    #[test]
    fn test_physical_hardware_validation_suite_spec() {
        let root = workspace_root();
        let physical_dir = root.join("tests/physical");
        assert!(
            physical_dir.exists(),
            "tests/physical/ directory must exist for physical validation suite"
        );

        let enrollment_script = physical_dir.join("enrollment_test.sh");
        let pam_script = physical_dir.join("pam_integration_test.sh");
        let multi_user_script = physical_dir.join("multi_user_test.sh");
        let screensaver_doc = physical_dir.join("screensaver_test.md");
        let adversarial_script = physical_dir.join("adversarial_test.sh");

        assert!(
            enrollment_script.exists(),
            "tests/physical/enrollment_test.sh must exist (Sub-issue #31.1)"
        );
        assert!(
            pam_script.exists(),
            "tests/physical/pam_integration_test.sh must exist (Sub-issue #31.2)"
        );
        assert!(
            multi_user_script.exists(),
            "tests/physical/multi_user_test.sh must exist (Sub-issue #31.3)"
        );
        assert!(
            screensaver_doc.exists(),
            "tests/physical/screensaver_test.md must exist (Sub-issue #31.4)"
        );
        assert!(
            adversarial_script.exists(),
            "tests/physical/adversarial_test.sh must exist (Sub-issue #31.5)"
        );

        // Verify executable permissions on shell scripts
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for script in [
                &enrollment_script,
                &pam_script,
                &multi_user_script,
                &adversarial_script,
            ] {
                let meta = fs::metadata(script).expect("script metadata");
                assert_ne!(
                    meta.permissions().mode() & 0o111,
                    0,
                    "Script {:?} must have executable permissions",
                    script.file_name()
                );
            }
        }

        // Verify shell script safety headers
        for script in [
            &enrollment_script,
            &pam_script,
            &multi_user_script,
            &adversarial_script,
        ] {
            let content = fs::read_to_string(script).expect("read script");
            assert!(
                content.starts_with("#!/usr/bin/env bash") || content.starts_with("#!/bin/bash"),
                "Script {:?} must start with bash shebang",
                script.file_name()
            );
            assert!(
                content.contains("set -euo pipefail"),
                "Script {:?} must enable strict bash error flags 'set -euo pipefail'",
                script.file_name()
            );
        }

        // Verify screensaver_test.md contains required display managers and operational procedures
        let doc_content = fs::read_to_string(&screensaver_doc).expect("read screensaver_test.md");
        for dm in ["swaylock", "hyprlock", "gdm", "login", "sudo"] {
            assert!(
                doc_content.contains(dm),
                "screensaver_test.md must document operational validation for '{}'",
                dm
            );
        }

        // Verify enrollment_test.sh covers the full lifecycle
        let enroll_content =
            fs::read_to_string(&enrollment_script).expect("read enrollment_test.sh");
        for cmd in ["enroll", "verify", "list", "delete"] {
            assert!(
                enroll_content.contains(cmd),
                "enrollment_test.sh must exercise '{}' subcommand",
                cmd
            );
        }

        // Verify pam_integration_test.sh tests nominal PAM_SUCCESS and password fallback
        let pam_content = fs::read_to_string(&pam_script).expect("read pam_integration_test.sh");
        assert!(
            pam_content.contains("PAM_SUCCESS") || pam_content.contains("pam_soos.so"),
            "pam_integration_test.sh must test PAM module integration"
        );
        assert!(
            pam_content.contains("PAM_IGNORE") || pam_content.contains("password"),
            "pam_integration_test.sh must test fallback to password authentication"
        );

        // Verify multi_user_test.sh covers multi-user and cross-user rejection
        let multi_content =
            fs::read_to_string(&multi_user_script).expect("read multi_user_test.sh");
        assert!(
            multi_content.contains("cross")
                || multi_content.contains("User B")
                || multi_content.contains("mismatch"),
            "multi_user_test.sh must test cross-user isolation and rejection"
        );

        // Verify adversarial_test.sh covers presentation attacks
        let adv_content =
            fs::read_to_string(&adversarial_script).expect("read adversarial_test.sh");
        for attack in ["photo", "screen", "video"] {
            assert!(
                adv_content.to_lowercase().contains(attack),
                "adversarial_test.sh must evaluate presentation attack type '{}'",
                attack
            );
        }

        // Verify all scripts handle --help cleanly with exit code 0
        for script in [
            &enrollment_script,
            &pam_script,
            &multi_user_script,
            &adversarial_script,
        ] {
            let output = std::process::Command::new("bash")
                .arg(script)
                .arg("--help")
                .output()
                .expect("execute script --help");
            assert!(
                output.status.success(),
                "Script {:?} --help must exit with status 0",
                script.file_name()
            );
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                !stdout.trim().is_empty(),
                "Script {:?} --help must print usage information to stdout",
                script.file_name()
            );
        }
    }

    /// Invariant: `adversarial_test.sh --mock` is a simulation and never fabricates PAD metrics
    /// (review finding PAD-06 / GitHub #172). The mock branch must not assign attack or bona fide
    /// counters, must announce itself as a simulation and must exit before the APCER / BPCER
    /// report; no mode may claim a "100%" rejection rate.
    #[test]
    fn test_adversarial_mock_mode_never_fabricates_pad_metrics() {
        let script = workspace_root().join("tests/physical/adversarial_test.sh");
        let content = fs::read_to_string(&script).expect("read adversarial_test.sh");

        assert!(
            content.contains("SIMULATION \u{2013} no security metrics"),
            "adversarial_test.sh --mock must print 'SIMULATION \u{2013} no security metrics'"
        );
        assert!(
            !content.contains("100% of presentation attacks"),
            "adversarial_test.sh must never claim a 100% presentation attack rejection rate"
        );
        assert!(
            content.contains("pad_real_model_tests"),
            "adversarial_test.sh --mock must point at the real-model PAD evidence target"
        );

        let start = content
            .find("if [[ \"${USE_MOCK}\" == \"true\" ]]; then")
            .expect("adversarial_test.sh must have an explicit mock branch");
        let rest = &content[start..];
        let end = rest
            .find("\nelse\n")
            .expect("mock branch must be followed by the physical branch");
        let mock_branch = &rest[..end];
        for counter in [
            "ATTACKS_TESTED=",
            "ATTACKS_REJECTED=",
            "ATTACKS_ACCEPTED=",
            "BONA_FIDE_TESTED=",
            "BONA_FIDE_ACCEPTED=",
            "BONA_FIDE_REJECTED=",
        ] {
            assert!(
                !mock_branch.contains(counter),
                "adversarial_test.sh mock branch must not fabricate counter '{counter}'"
            );
        }
        assert!(
            mock_branch.contains("exit 0"),
            "adversarial_test.sh mock branch must exit before the APCER / BPCER report"
        );
    }

    /// Invariant: Distribution-Specific Deployment Validation Suite (Issue #32 / GitHub #71)
    #[test]
    fn test_distro_validation_suite_spec() {
        let root = workspace_root();
        let distro_dir = root.join("tests/distro");
        assert!(
            distro_dir.exists(),
            "tests/distro/ directory must exist for distribution validation suite"
        );

        let deb_script = distro_dir.join("debian_ubuntu_test.sh");
        let fedora_script = distro_dir.join("fedora_rhel_test.sh");
        let arch_script = distro_dir.join("arch_linux_test.sh");
        let runner_script = distro_dir.join("run_distro_validation.sh");
        let distro_doc = root.join("Docs/DISTRIBUTION_DEPLOYMENT.md");

        assert!(
            deb_script.exists(),
            "tests/distro/debian_ubuntu_test.sh must exist (Sub-issue #32.1)"
        );
        assert!(
            fedora_script.exists(),
            "tests/distro/fedora_rhel_test.sh must exist (Sub-issue #32.2)"
        );
        assert!(
            arch_script.exists(),
            "tests/distro/arch_linux_test.sh must exist (Sub-issue #32.3)"
        );
        assert!(
            runner_script.exists(),
            "tests/distro/run_distro_validation.sh must exist"
        );
        assert!(
            distro_doc.exists(),
            "Docs/DISTRIBUTION_DEPLOYMENT.md must exist"
        );

        // Verify executable permissions on all shell scripts
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for script in [&deb_script, &fedora_script, &arch_script, &runner_script] {
                let meta = fs::metadata(script).expect("script metadata");
                assert_ne!(
                    meta.permissions().mode() & 0o111,
                    0,
                    "Script {:?} must have executable permissions",
                    script.file_name()
                );
            }
        }

        // Verify strict bash error flags on all scripts
        for script in [&deb_script, &fedora_script, &arch_script, &runner_script] {
            let content = fs::read_to_string(script).expect("read script");
            assert!(
                content.starts_with("#!/usr/bin/env bash") || content.starts_with("#!/bin/bash"),
                "Script {:?} must start with bash shebang",
                script.file_name()
            );
            assert!(
                content.contains("set -euo pipefail"),
                "Script {:?} must enable strict bash error flags 'set -euo pipefail'",
                script.file_name()
            );
        }

        // Verify debian_ubuntu_test.sh covers package/install, enrollment, PAM auth, fallback, rollback
        let deb_content = fs::read_to_string(&deb_script).expect("read debian_ubuntu_test.sh");
        for needle in ["install", "enroll", "PAM_SUCCESS", "password", "rollback"] {
            assert!(
                deb_content.to_lowercase().contains(&needle.to_lowercase()),
                "debian_ubuntu_test.sh must cover '{}'",
                needle
            );
        }

        // Verify fedora_rhel_test.sh covers authselect, pam_faillock, sudo, gdm
        let fedora_content = fs::read_to_string(&fedora_script).expect("read fedora_rhel_test.sh");
        for needle in ["authselect", "pam_faillock", "sudo", "gdm"] {
            assert!(
                fedora_content.contains(needle),
                "fedora_rhel_test.sh must cover '{}'",
                needle
            );
        }

        // Verify arch_linux_test.sh covers PKGBUILD/pacman, system-auth, swaylock/hyprlock
        let arch_content = fs::read_to_string(&arch_script).expect("read arch_linux_test.sh");
        for needle in ["system-auth", "swaylock"] {
            assert!(
                arch_content.contains(needle),
                "arch_linux_test.sh must cover '{}'",
                needle
            );
        }
        assert!(
            arch_content.contains("PKGBUILD") || arch_content.contains("pacman"),
            "arch_linux_test.sh must cover PKGBUILD or pacman"
        );

        // Verify Docs/DISTRIBUTION_DEPLOYMENT.md documents all 3 distribution families and procedures
        let doc_content = fs::read_to_string(&distro_doc).expect("read DISTRIBUTION_DEPLOYMENT.md");
        for topic in [
            "Debian",
            "Ubuntu",
            "Fedora",
            "RHEL",
            "Arch",
            "authselect",
            "pam_faillock",
            "swaylock",
            "rollback",
        ] {
            assert!(
                doc_content.contains(topic),
                "Docs/DISTRIBUTION_DEPLOYMENT.md must document '{}'",
                topic
            );
        }

        // Verify all scripts handle --help cleanly with exit code 0 and non-empty output
        for script in [&deb_script, &fedora_script, &arch_script, &runner_script] {
            let output = std::process::Command::new("bash")
                .arg(script)
                .arg("--help")
                .output()
                .expect("execute script --help");
            assert!(
                output.status.success(),
                "Script {:?} --help must exit with status 0",
                script.file_name()
            );
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                !stdout.trim().is_empty(),
                "Script {:?} --help must print usage information to stdout",
                script.file_name()
            );
        }
    }

    /// Extracts one `[section]` of a TOML document as raw text (up to the next `[` header).
    fn toml_section<'a>(content: &'a str, header: &str) -> &'a str {
        let start = content
            .find(header)
            .unwrap_or_else(|| panic!("TOML section '{}' not found", header));
        let body_start = start + header.len();
        let rest = &content[body_start..];
        let end = rest.find("\n[").unwrap_or(rest.len());
        &rest[..end]
    }

    /// Returns true if a line is TOML/shell content rather than a comment or blank line.
    fn is_code_line(line: &str) -> bool {
        let trimmed = line.trim();
        !trimmed.is_empty() && !trimmed.starts_with('#')
    }

    /// PAM-01 / TCI-01 (GitHub #148) — The shipped `pam_soos.so` must be built with
    /// `panic = "unwind"`: under `panic = "abort"` every `catch_unwind` in the module is a
    /// no-op and a panic aborts the host login process (gdm, sudo, login) instead of
    /// degrading to `PAM_IGNORE` (ARCHITECTURE.md invariant 5).
    ///
    /// The artifact is produced by `[profile.release]` in every packaging path, so that profile
    /// is the single source of truth: it must say `panic = "unwind"` explicitly, and no
    /// packaging or Docker build command may select another profile.
    #[test]
    fn test_release_profile_unwinds_so_pam_catch_unwind_is_effective() {
        let root = workspace_root();
        let root_cargo = root.join("Cargo.toml");
        let content = fs::read_to_string(&root_cargo)
            .unwrap_or_else(|e| panic!("Error reading {}: {}", root_cargo.display(), e));

        let release = toml_section(&content, "[profile.release]");
        let panic_lines: Vec<&str> = release
            .lines()
            .filter(|l| is_code_line(l) && l.trim().starts_with("panic"))
            .collect();

        assert_eq!(
            panic_lines.len(),
            1,
            "SECURITY VIOLATION (PAM-01): [profile.release] must declare the panic strategy exactly once, found: {:?}",
            panic_lines
        );
        assert_eq!(
            panic_lines[0].trim(),
            "panic = \"unwind\"",
            "SECURITY VIOLATION (PAM-01): [profile.release] must set panic = \"unwind\" so that catch_unwind in pam_soos.so is effective (found: {})",
            panic_lines[0].trim()
        );

        // No profile in the workspace may reintroduce abort: the PAM cdylib inherits whatever
        // profile the packaging scripts select, and `panic` cannot be overridden per package.
        let manifests = [
            root_cargo.clone(),
            root.join("crates").join("pam").join("Cargo.toml"),
        ];
        for manifest in manifests {
            let text = fs::read_to_string(&manifest)
                .unwrap_or_else(|e| panic!("Error reading {}: {}", manifest.display(), e));
            let abort_lines: Vec<&str> = text
                .lines()
                .filter(|l| is_code_line(l) && l.replace(' ', "").starts_with("panic=\"abort\""))
                .collect();
            assert!(
                abort_lines.is_empty(),
                "SECURITY VIOLATION (PAM-01): {} must not set panic = \"abort\" in any profile: {:?}",
                manifest.display(),
                abort_lines
            );
        }

        // Every production build path must use the plain release profile (no `--profile <x>`),
        // otherwise the artifact could silently come from a profile with different semantics.
        let build_paths = [
            "scripts/build_deb.sh",
            "scripts/build_rpm.sh",
            "scripts/build_arch.sh",
            "scripts/build_packages.sh",
            "packaging/debian/rules",
            "packaging/rpm/soos.spec",
            "packaging/arch/PKGBUILD",
            "tests/docker/test_suite.sh",
            "tests/docker/test_packages.sh",
        ];
        for rel in build_paths {
            let path = root.join(rel);
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("Error reading {}: {}", path.display(), e));
            // Command lines only (usage text such as "Skip cargo build step" is not a build).
            let builds: Vec<&str> = text
                .lines()
                .filter(|l| l.trim_start().starts_with("cargo build"))
                .collect();
            assert!(
                !builds.is_empty(),
                "{} must contain at least one 'cargo build' invocation",
                rel
            );
            for line in builds {
                assert!(
                    line.contains("--release"),
                    "PACKAGING VIOLATION (PAM-01): {} builds without --release: '{}'",
                    rel,
                    line.trim()
                );
                assert!(
                    !line.contains("--profile"),
                    "PACKAGING VIOLATION (PAM-01): {} selects a custom profile; the PAM artifact must come from [profile.release]: '{}'",
                    rel,
                    line.trim()
                );
            }
        }
    }

    /// PAM-01 (GitHub #148) — The test-only fault-injection hook of `soos-pam` must be an
    /// opt-in Cargo feature that is never part of `default`, never referenced by any packaging,
    /// install or CI build command, and whose production call sites are `cfg`-gated.
    #[test]
    fn test_pam_fault_injection_feature_is_opt_in_and_never_packaged() {
        let root = workspace_root();
        let pam_cargo = root.join("crates").join("pam").join("Cargo.toml");
        let content = fs::read_to_string(&pam_cargo)
            .unwrap_or_else(|e| panic!("Error reading {}: {}", pam_cargo.display(), e));

        assert!(
            content.contains("[features]"),
            "crates/pam/Cargo.toml must declare a [features] table for the fault-injection hook"
        );
        let features = toml_section(&content, "[features]");
        let feature_line = features
            .lines()
            .find(|l| is_code_line(l) && l.trim().starts_with("fault-injection"))
            .expect("crates/pam/Cargo.toml must declare the 'fault-injection' feature");
        assert_eq!(
            feature_line.replace(' ', "").trim(),
            "fault-injection=[]",
            "The fault-injection feature must not pull any dependency or other feature"
        );
        let default_line = features
            .lines()
            .find(|l| is_code_line(l) && l.trim().starts_with("default"));
        if let Some(default_line) = default_line {
            assert!(
                !default_line.contains("fault-injection"),
                "SECURITY VIOLATION: fault-injection must never be a default feature: '{}'",
                default_line.trim()
            );
        }

        // Production code: the hook module and its call sites are compiled only under the feature.
        let lib_rs = root.join("crates").join("pam").join("src").join("lib.rs");
        let lib_src = fs::read_to_string(&lib_rs)
            .unwrap_or_else(|e| panic!("Error reading {}: {}", lib_rs.display(), e));
        let prod = extract_production_code(&lib_src);
        assert!(
            prod.contains("#[cfg(feature = \"fault-injection\")]\npub mod fault_injection;"),
            "crates/pam/src/lib.rs must declare `pub mod fault_injection;` guarded by #[cfg(feature = \"fault-injection\")]"
        );
        let hook_calls = prod.matches("fault_injection::").count();
        assert!(
            hook_calls >= 1,
            "crates/pam/src/lib.rs must invoke the fault-injection hook inside the catch_unwind region"
        );
        assert_eq!(
            prod.matches("#[cfg(feature = \"fault-injection\")]").count(),
            hook_calls + 1,
            "Every reference to fault_injection in crates/pam/src/lib.rs must be preceded by #[cfg(feature = \"fault-injection\")]"
        );

        // Packaging, install and CI must never enable the feature.
        let never_enable = [
            "scripts/build_deb.sh",
            "scripts/build_rpm.sh",
            "scripts/build_arch.sh",
            "scripts/build_packages.sh",
            "scripts/install.sh",
            "packaging/debian/rules",
            "packaging/rpm/soos.spec",
            "packaging/arch/PKGBUILD",
            "tests/docker/test_packages.sh",
            ".github/workflows/ci.yml",
            "Dockerfile",
            "tests/docker/Dockerfile.ubuntu",
            "tests/docker/Dockerfile.fedora",
            "tests/docker/Dockerfile.arch",
        ];
        for rel in never_enable {
            let path = root.join(rel);
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("Error reading {}: {}", path.display(), e));
            let offending: Vec<&str> = text
                .lines()
                .filter(|l| is_code_line(l) && l.contains("fault-injection"))
                .collect();
            assert!(
                offending.is_empty(),
                "SECURITY VIOLATION: {} must never enable the fault-injection feature: {:?}",
                rel,
                offending
            );
        }
    }

    /// PAM-01 (GitHub #148) — The Docker matrix must load the *release-built* `pam_soos.so`
    /// (same `[profile.release]` as packaging, plus the opt-in fault-injection feature built
    /// into a separate target directory) and prove that a panic inside the module returns
    /// `PAM_IGNORE` with password fallback instead of aborting the PAM host process.
    #[test]
    fn test_pam_docker_suite_proves_release_panic_returns_pam_ignore() {
        let root = workspace_root();
        let suite = root.join("tests").join("docker").join("test_suite.sh");
        let text = fs::read_to_string(&suite)
            .unwrap_or_else(|e| panic!("Error reading {}: {}", suite.display(), e));

        let build_lines: Vec<&str> = text
            .lines()
            .filter(|l| {
                is_code_line(l) && l.contains("cargo build") && l.contains("fault-injection")
            })
            .collect();
        assert_eq!(
            build_lines.len(),
            1,
            "test_suite.sh must build the fault-injection variant of soos-pam exactly once: {:?}",
            build_lines
        );
        let build = build_lines[0];
        for required in [
            "--release",
            "-p soos-pam",
            "--features fault-injection",
            "--target-dir target/fault-injection",
        ] {
            assert!(
                build.contains(required),
                "test_suite.sh fault-injection build must contain '{}': '{}'",
                required,
                build.trim()
            );
        }

        for required in [
            "T10",
            "fault_inject=panic",
            "fault_inject=overflow",
            "pam_soos_fault.so",
            "134",
        ] {
            assert!(
                text.contains(required),
                "test_suite.sh must contain '{}' for the release panic-safety case (T10)",
                required
            );
        }

        let matrix_doc = root.join("Docs").join("PAM_DOCKER_TEST_MATRIX.md");
        let doc = fs::read_to_string(&matrix_doc)
            .unwrap_or_else(|e| panic!("Error reading {}: {}", matrix_doc.display(), e));
        assert!(
            doc.contains("T10") && doc.contains("fault_inject=panic"),
            "Docs/PAM_DOCKER_TEST_MATRIX.md must document the T10 release panic-safety case"
        );
    }

    /// Invariant (GitHub #146, PAD-01): `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX` is the single
    /// source of truth for the MiniFASNet live class. Production code must construct
    /// `OrtPadDetector` with `OrtPadDetector::new`; the explicit-index constructors
    /// (`new_with_class_index`, `with_live_class_index`) are test-only. A production override
    /// silently inverts anti-spoofing (walkthrough 72 fixed the constant, the daemon and the
    /// enrollment CLI kept a literal `2` = ScreenReplay).
    #[test]
    fn test_no_pad_live_class_index_override_outside_tests() {
        let root = workspace_root();
        let crates_dir = root.join("crates");
        assert!(crates_dir.is_dir(), "crates/ directory must exist");

        let definition_file = crates_dir.join("inference-ort/src/pad.rs");
        let definition_source =
            fs::read_to_string(&definition_file).expect("read crates/inference-ort/src/pad.rs");
        assert!(
            definition_source.contains("pub const DEFAULT_MINIFASNET_LIVE_CLASS_INDEX: usize = 1;"),
            "DEFAULT_MINIFASNET_LIVE_CLASS_INDEX must be defined as 1 in crates/inference-ort/src/pad.rs"
        );

        let forbidden = ["new_with_class_index(", "with_live_class_index("];
        let mut src_files = Vec::new();
        for entry in fs::read_dir(&crates_dir).expect("read crates/") {
            let crate_dir = entry.expect("crate dir entry").path();
            let src_dir = crate_dir.join("src");
            if src_dir.is_dir() {
                collect_rs_files(&src_dir, &mut src_files);
            }
        }
        assert!(
            !src_files.is_empty(),
            "At least one production source file must be scanned"
        );

        let mut violations = Vec::new();
        let mut construction_sites = 0usize;
        for file in &src_files {
            let source = fs::read_to_string(file).expect("read production source file");
            let production = extract_production_code(&source);
            let rel = file
                .strip_prefix(&root)
                .unwrap_or(file)
                .display()
                .to_string();
            for (line_no, line) in production.lines().enumerate() {
                let trimmed = line.trim();
                if trimmed.starts_with("//") {
                    continue;
                }
                if trimmed.contains("OrtPadDetector::new(") {
                    construction_sites += 1;
                }
                for pattern in forbidden {
                    if !trimmed.contains(pattern) {
                        continue;
                    }
                    // The definitions in pad.rs are the only permitted occurrences.
                    let is_definition = *file == definition_file
                        && trimmed.starts_with("pub fn ")
                        && trimmed.contains(pattern);
                    if !is_definition {
                        violations.push(format!("{rel}:{}: {trimmed}", line_no + 1));
                    }
                }
            }
        }

        assert!(
            violations.is_empty(),
            "Production code must not override the MiniFASNet live class index \
             (use OrtPadDetector::new so DEFAULT_MINIFASNET_LIVE_CLASS_INDEX is the single source of truth):\n{}",
            violations.join("\n")
        );
        assert!(
            construction_sites >= 3,
            "Expected the daemon, the enrollment CLI and the GUI to construct OrtPadDetector::new, found {construction_sites} site(s)"
        );
    }

    /// Files an authselect custom profile must ship so that activating it never leaves a
    /// generated system file empty (authselect writes every managed file from the profile).
    const AUTHSELECT_PROFILE_FILES: [&str; 10] = [
        "README",
        "REQUIREMENTS",
        "system-auth",
        "password-auth",
        "nsswitch.conf",
        "fingerprint-auth",
        "smartcard-auth",
        "postlogin",
        "dconf-db",
        "dconf-locks",
    ];

    /// Returns the byte offset of `needle` in `haystack`, failing with `what` when absent.
    fn offset_of(haystack: &str, needle: &str, what: &str) -> usize {
        haystack
            .find(needle)
            .unwrap_or_else(|| panic!("{what}: '{needle}' not found"))
    }

    /// `DEFAULT_TIMEOUT_MS` of the PAM module, read from `crates/pam/src/config.rs`
    /// (GitHub #185: packaged rules rely on the module default instead of a literal).
    fn pam_default_timeout_ms() -> u64 {
        let source = fs::read_to_string(workspace_root().join("crates/pam/src/config.rs"))
            .expect("read crates/pam/src/config.rs");
        source
            .lines()
            .find_map(|l| {
                l.trim()
                    .strip_prefix("pub const DEFAULT_TIMEOUT_MS: u64 = ")
                    .and_then(|v| v.trim_end_matches(';').trim().parse().ok())
            })
            .expect("DEFAULT_TIMEOUT_MS must be defined in crates/pam/src/config.rs")
    }

    /// Whether `line` is the primary soos rule (`[success=done default=ignore] pam_soos.so`,
    /// whitespace-insensitive, not the password-failed event rule) and relies on the module
    /// default deadline: no `timeout_ms=` argument, or `timeout_ms=<DEFAULT_TIMEOUT_MS>`
    /// (GitHub #185, user-approved assertion change 2026-09-30).
    fn is_default_timeout_primary_soos_rule(line: &str) -> bool {
        let normalized = line.split_whitespace().collect::<Vec<_>>().join(" ");
        let Some(module_at) = normalized.find("pam_soos.so") else {
            return false;
        };
        if !normalized[..module_at].contains("[success=done default=ignore]") {
            return false;
        }
        let default_arg = format!("timeout_ms={}", pam_default_timeout_ms());
        let args: Vec<&str> = normalized[module_at + "pam_soos.so".len()..]
            .split_whitespace()
            .collect();
        !args.iter().any(|a| a.starts_with("event="))
            && args
                .iter()
                .filter(|a| a.starts_with("timeout_ms="))
                .all(|a| *a == default_arg)
    }

    /// Byte offset of `pam_soos.so` on the primary soos rule of `content` (see
    /// [`is_default_timeout_primary_soos_rule`]); panics with `what` when absent.
    fn primary_soos_rule_offset(content: &str, what: &str) -> usize {
        let mut offset = 0;
        for line in content.split_inclusive('\n') {
            if is_default_timeout_primary_soos_rule(line) {
                let module_at = line.find("pam_soos.so").expect("rule names pam_soos.so");
                return offset + module_at;
            }
            offset += line.len();
        }
        panic!("{what}: no primary `[success=done default=ignore] pam_soos.so` rule relying on the default timeout");
    }

    /// Collects the feature names referenced by authselect conditionals
    /// (`{include if "x"}`, `{if "x":...}`, `{exclude if "x"}`, `{continue if "x"}`).
    fn referenced_features(template: &str) -> std::collections::BTreeSet<String> {
        let mut features = std::collections::BTreeSet::new();
        for (idx, _) in template.match_indices("{") {
            let rest = &template[idx..];
            let Some(end) = rest.find('}') else { continue };
            let block = &rest[..end];
            let is_conditional = block.starts_with("{include if ")
                || block.starts_with("{exclude if ")
                || block.starts_with("{continue if ")
                || block.starts_with("{if ");
            if !is_conditional {
                continue;
            }
            let mut parts = block.split('"');
            parts.next();
            while let (Some(name), Some(_)) = (parts.next(), parts.next()) {
                features.insert(name.to_string());
            }
        }
        features
    }

    /// Invariant: Fedora authselect profile is complete (Issue ONB-02 / GitHub #145)
    ///
    /// `authselect select custom/soos` regenerates every managed file from the profile
    /// directory. A profile shipping only `system-auth`/`password-auth` empties
    /// `/etc/nsswitch.conf` on activation. The profile must therefore ship the full
    /// `local` layout and use real authselect conditional syntax.
    #[test]
    fn test_fedora_authselect_profile_is_complete() {
        let root = workspace_root();
        let profile_dir = root.join("packaging/pam/fedora/soos");
        assert!(
            profile_dir.is_dir(),
            "packaging/pam/fedora/soos must be a directory"
        );

        for name in AUTHSELECT_PROFILE_FILES {
            let path = profile_dir.join(name);
            assert!(
                path.is_file(),
                "authselect profile file '{name}' is missing from packaging/pam/fedora/soos"
            );
            let content = fs::read_to_string(&path).expect("read profile file");
            assert!(
                !content.contains("{?"),
                "'{name}' uses the unsupported '{{?feature:...}}' syntax; use \
                 '{{include if \"feature\"}}' (authselect-profiles(5))"
            );
        }

        let nsswitch =
            fs::read_to_string(profile_dir.join("nsswitch.conf")).expect("read nsswitch.conf");
        for database in ["passwd:", "shadow:", "group:", "hosts:", "services:"] {
            assert!(
                nsswitch.lines().any(|l| l.starts_with(database)),
                "nsswitch.conf template must define the '{database}' database"
            );
        }
        assert!(
            nsswitch.contains("hosts:") && nsswitch.contains("resolve [!UNAVAIL=return] dns"),
            "nsswitch.conf template must keep the systemd-resolved hosts chain"
        );

        let readme = fs::read_to_string(profile_dir.join("README")).expect("read README");
        assert!(
            readme.contains("AVAILABLE OPTIONAL FEATURES"),
            "README must declare its optional features under 'AVAILABLE OPTIONAL FEATURES'"
        );
        assert!(
            readme.contains("with-faillock::"),
            "README must declare the 'with-faillock' feature (otherwise \
             'authselect select custom/soos with-faillock' is rejected)"
        );

        // Every feature referenced by a template must be declared in the README,
        // otherwise authselect rejects it with "Unknown profile feature".
        let mut referenced = std::collections::BTreeSet::new();
        for name in AUTHSELECT_PROFILE_FILES {
            if name == "README" {
                continue;
            }
            let content = fs::read_to_string(profile_dir.join(name)).expect("read template");
            referenced.extend(referenced_features(&content));
        }
        assert!(
            referenced.contains("with-faillock"),
            "templates must reference the 'with-faillock' feature"
        );
        for feature in &referenced {
            assert!(
                readme.contains(&format!("{feature}::")),
                "feature '{feature}' is referenced by a template but not declared in README"
            );
        }
    }

    /// Invariant: Fedora authselect profile preserves pam_faillock ordering
    /// (Issue ONB-02 / GitHub #145, auditor checklist item 8)
    ///
    /// Generated stacks must contain `pam_faillock.so preauth` before `pam_soos.so`,
    /// `pam_soos.so` before `pam_unix.so`, `pam_faillock.so authfail` after `pam_unix.so`
    /// and the account-phase `pam_faillock.so`, all gated by `{include if "with-faillock"}`.
    #[test]
    fn test_fedora_authselect_profile_preserves_faillock_ordering() {
        let root = workspace_root();
        let profile_dir = root.join("packaging/pam/fedora/soos");
        let include_if = "{include if \"with-faillock\"}";

        for template in ["system-auth", "password-auth"] {
            let content = fs::read_to_string(profile_dir.join(template)).expect("read template");
            let ctx = format!("packaging/pam/fedora/soos/{template}");

            let preauth = offset_of(&content, "pam_faillock.so preauth silent", &ctx);
            let soos = primary_soos_rule_offset(&content, &ctx);
            let unix = offset_of(&content, "pam_unix.so", &ctx);
            let authfail = offset_of(&content, "pam_faillock.so authfail", &ctx);
            let event = offset_of(&content, "pam_soos.so event=password-failed", &ctx);

            assert!(
                preauth < soos,
                "{ctx}: faillock preauth must precede pam_soos"
            );
            assert!(soos < unix, "{ctx}: pam_soos must precede pam_unix");
            assert!(
                unix < authfail,
                "{ctx}: faillock authfail must follow pam_unix"
            );
            assert!(
                unix < event,
                "{ctx}: password-failed event must follow pam_unix"
            );
            assert!(
                content
                    .lines()
                    .any(|l| l.starts_with("auth") && is_default_timeout_primary_soos_rule(l)),
                "{ctx}: pam_soos primary line must be [success=done default=ignore]"
            );
            assert!(
                content.lines().any(|l| l.starts_with("auth")
                    && l.contains("optional")
                    && l.contains("pam_soos.so event=password-failed timeout_ms=20")),
                "{ctx}: password-failed line must be 'auth optional ... timeout_ms=20'"
            );

            // Each pam_faillock line must be conditional on the declared feature.
            let faillock_lines: Vec<&str> = content
                .lines()
                .filter(|l| l.contains("pam_faillock.so"))
                .collect();
            assert_eq!(
                faillock_lines.len(),
                3,
                "{ctx}: expected preauth, authfail and account pam_faillock lines"
            );
            for line in &faillock_lines {
                assert!(
                    line.trim_end().ends_with(include_if),
                    "{ctx}: pam_faillock line must end with {include_if}: '{line}'"
                );
            }
            assert!(
                faillock_lines
                    .iter()
                    .any(|l| l.starts_with("account") && !l.contains("preauth")),
                "{ctx}: account phase must run pam_faillock.so"
            );

            // The password fallback must remain reachable: pam_unix stays `sufficient`,
            // and no `[default=die]` control can short-circuit the stack before pam_deny.
            assert!(
                content.lines().any(|l| l.starts_with("auth")
                    && l.contains("sufficient")
                    && l.contains("pam_unix.so")),
                "{ctx}: pam_unix.so auth line must be 'sufficient'"
            );
            assert!(
                !content.contains("[default=die]"),
                "{ctx}: '[default=die]' would skip the password-failed event"
            );
            assert!(
                content
                    .lines()
                    .any(|l| l.starts_with("auth") && l.contains("pam_deny.so")),
                "{ctx}: auth stack must end with pam_deny.so"
            );
        }
    }

    /// Invariant: activation, validation and rollback of the Fedora profile are
    /// scripted and documented with commands authselect accepts (Issue ONB-02 / GitHub #145)
    #[test]
    fn test_fedora_authselect_activation_and_rollback_are_scripted() {
        let root = workspace_root();
        let activation = "authselect select custom/soos with-faillock --force";

        // 1. Documentation uses the supported activation command and real syntax.
        for doc in [
            "Docs/DISTRIBUTION_DEPLOYMENT.md",
            "Docs/PACKAGING_AND_PROVISIONING.md",
        ] {
            let content = fs::read_to_string(root.join(doc)).expect("read doc");
            assert!(
                content.contains(activation),
                "{doc} must document '{activation}'"
            );
            assert!(
                !content.contains("{?with-faillock"),
                "{doc} must not show the unsupported '{{?with-faillock:...}}' syntax"
            );
            assert!(
                content.contains("authselect.previous"),
                "{doc} must document the recorded previous profile used for rollback"
            );
        }

        // 2. install.sh records the previously selected profile; uninstall.sh restores it
        //    before deleting the custom profile directory.
        let install = fs::read_to_string(root.join("scripts/install.sh")).expect("read install");
        assert!(
            install.contains("authselect current --raw") && install.contains("authselect.previous"),
            "scripts/install.sh must record 'authselect current --raw' into authselect.previous"
        );
        let uninstall =
            fs::read_to_string(root.join("scripts/uninstall.sh")).expect("read uninstall");
        assert!(
            uninstall.contains("authselect.previous") && uninstall.contains("authselect select"),
            "scripts/uninstall.sh must restore the recorded profile with 'authselect select'"
        );
        let restore_pos = offset_of(&uninstall, "authselect select", "uninstall.sh");
        let remove_pos = offset_of(&uninstall, "rm -rf \"${FEDORA_AUTH_DIR}\"", "uninstall.sh");
        assert!(
            restore_pos < remove_pos,
            "scripts/uninstall.sh must restore the previous profile before removing custom/soos"
        );

        // 3. RPM scriptlets implement the same record/restore contract.
        let spec = fs::read_to_string(root.join("packaging/rpm/soos.spec")).expect("read spec");
        // Section headers start a line; comments may mention other scriptlets.
        let post = offset_of(&spec, "\n%post\n", "soos.spec");
        let preun = offset_of(&spec, "\n%preun\n", "soos.spec");
        let postun = offset_of(&spec, "\n%postun\n", "soos.spec");
        assert!(post < preun && preun < postun, "soos.spec scriptlet order");
        let post_section = &spec[post..preun];
        let preun_section = &spec[preun..postun];
        assert!(
            post_section.contains("authselect current --raw")
                && post_section.contains("authselect.previous"),
            "soos.spec %post must record the current authselect profile"
        );
        assert!(
            preun_section.contains("authselect select")
                && preun_section.contains("authselect.previous"),
            "soos.spec %preun must restore the recorded authselect profile"
        );
        assert!(
            spec.contains("%ghost %{_sysconfdir}/soos/authselect.previous"),
            "soos.spec must own authselect.previous as a %ghost file"
        );

        // 4. The distribution test activates the profile for real instead of tolerating
        //    an `authselect check` failure.
        let fedora_test = fs::read_to_string(root.join("tests/distro/fedora_rhel_test.sh"))
            .expect("read fedora_rhel_test.sh");
        assert!(
            !fedora_test.contains("authselect check || true"),
            "fedora_rhel_test.sh must not ignore 'authselect check' failures"
        );
        assert!(
            fedora_test.contains(activation),
            "fedora_rhel_test.sh must activate the profile with '{activation}'"
        );
        assert!(
            fedora_test.contains("/etc/nsswitch.conf"),
            "fedora_rhel_test.sh must verify /etc/nsswitch.conf after activation"
        );

        // 5. A Dockerized Fedora validation exists, is strict and is wired into run_tests.sh.
        let docker_test = root.join("tests/docker/authselect_profile_test.sh");
        assert!(
            docker_test.is_file(),
            "tests/docker/authselect_profile_test.sh must exist"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = fs::metadata(&docker_test).expect("docker test metadata");
            assert_ne!(
                meta.permissions().mode() & 0o111,
                0,
                "tests/docker/authselect_profile_test.sh must be executable"
            );
        }
        let docker_content = fs::read_to_string(&docker_test).expect("read docker test");
        assert!(
            docker_content.starts_with("#!/usr/bin/env bash")
                && docker_content.contains("set -euo pipefail"),
            "authselect_profile_test.sh must be a strict bash script"
        );
        for needle in [
            activation,
            "authselect check",
            "/etc/nsswitch.conf",
            "pam_faillock.so preauth",
            "pam_faillock.so authfail",
            "pamtester",
            "scripts/uninstall.sh",
        ] {
            assert!(
                docker_content.contains(needle),
                "authselect_profile_test.sh must cover '{needle}'"
            );
        }
        let run_tests = fs::read_to_string(root.join("run_tests.sh")).expect("read run_tests.sh");
        assert!(
            run_tests.contains("authselect_profile_test.sh"),
            "run_tests.sh must expose the authselect profile test"
        );
        let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("read ci.yml");
        assert!(
            ci.contains("authselect_profile_test.sh"),
            "ci.yml must run the Fedora authselect profile test"
        );
    }

    /// Returns the body of the first `fn <name>(` or `fn <name><` in `source` (brace matched).
    fn function_body<'a>(source: &'a str, name: &str) -> Option<&'a str> {
        let start = source
            .find(&format!("fn {name}("))
            .or_else(|| source.find(&format!("fn {name}<")))?;
        let open = start + source[start..].find('{')?;
        let mut depth = 0usize;
        for (offset, ch) in source[open..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&source[open..=open + offset]);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Candid review finding 3 (GitHub #152): the daemon's production camera resolution must
    /// go through `resolve_pipeline_camera`, the function the parity tests exercise, so that the
    /// tested path and the shipped path cannot drift apart.
    #[test]
    fn test_daemon_production_camera_resolution_uses_tested_resolver() {
        let root = workspace_root();
        let source = fs::read_to_string(root.join("crates/daemon/src/pipeline.rs"))
            .expect("read crates/daemon/src/pipeline.rs");
        let production = extract_production_code(&source);

        let init = function_body(&production, "initialize_pipeline")
            .expect("initialize_pipeline must exist in crates/daemon/src/pipeline.rs");
        assert!(
            init.contains("resolve_pipeline_camera("),
            "initialize_pipeline must resolve its camera through resolve_pipeline_camera"
        );

        let resolver = function_body(&production, "resolve_pipeline_camera")
            .expect("resolve_pipeline_camera must exist in crates/daemon/src/pipeline.rs");
        assert!(
            resolver.contains("plan_camera_device("),
            "resolve_pipeline_camera must be built on plan_camera_device (single daemon path)"
        );
        let planner = function_body(&production, "plan_camera_device")
            .expect("plan_camera_device must exist in crates/daemon/src/pipeline.rs");
        assert!(
            !planner.contains("read_dir") && !planner.contains("enumerate_capture_devices"),
            "plan_camera_device must not enumerate devices on its own"
        );
    }

    /// Candid review finding 4 (GitHub #152): exactly one bounded `/dev/v4l/by-id/` scanner.
    /// `stable_device_path` must delegate to the resolver's enumerator instead of scanning the
    /// directory with its own, different bound.
    #[test]
    fn test_single_bounded_by_id_scanner() {
        let root = workspace_root();
        let stable = fs::read_to_string(root.join("crates/camera-v4l/src/stable_path.rs"))
            .expect("read crates/camera-v4l/src/stable_path.rs");
        let stable_production = extract_production_code(&stable);
        assert!(
            !stable_production.contains("read_dir"),
            "stable_path.rs must not scan /dev/v4l/by-id itself"
        );
        assert!(
            !stable_production.contains("const MAX_BY_ID_ENTRIES"),
            "stable_path.rs must not define its own by-id bound"
        );
        assert!(
            stable_production.contains("by_id_aliases"),
            "stable_device_path must delegate to CameraEnumerator::by_id_aliases"
        );

        let resolver = fs::read_to_string(root.join("crates/camera-v4l/src/resolver.rs"))
            .expect("read crates/camera-v4l/src/resolver.rs");
        assert_eq!(
            resolver.matches("pub const MAX_BY_ID_ENTRIES").count(),
            1,
            "the single by-id bound lives in resolver.rs"
        );
    }

    /// Parses a `pam-auth-update` profile (`/usr/share/pam-configs/*`) into its
    /// single-valued header fields and the module lines of each multi-line field
    /// (`Auth`, `Auth-Initial`, ...).
    fn parse_pam_config_profile(
        content: &str,
    ) -> (
        std::collections::BTreeMap<String, String>,
        std::collections::BTreeMap<String, Vec<String>>,
    ) {
        let mut fields = std::collections::BTreeMap::new();
        let mut blocks: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        let mut current: Option<String> = None;
        for line in content.lines() {
            if line.starts_with([' ', '\t']) {
                if let Some(name) = &current {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        blocks
                            .entry(name.clone())
                            .or_default()
                            .push(trimmed.to_string());
                    }
                }
                continue;
            }
            let Some((key, value)) = line.split_once(':') else {
                current = None;
                continue;
            };
            let value = value.trim();
            fields.insert(key.trim().to_string(), value.to_string());
            current = Some(key.trim().to_string());
        }
        (fields, blocks)
    }

    /// Returns the bracketed/keyword control of a profile module line
    /// (`[success=done default=ignore]` or `optional`) and the module part.
    fn split_pam_control(line: &str) -> (String, String) {
        if let Some(rest) = line.strip_prefix('[') {
            let end = rest.find(']').expect("unterminated PAM control bracket");
            (
                format!("[{}]", &rest[..end]),
                rest[end + 1..].trim().to_string(),
            )
        } else {
            let mut parts = line.splitn(2, char::is_whitespace);
            let control = parts.next().unwrap_or_default().to_string();
            (control, parts.next().unwrap_or_default().trim().to_string())
        }
    }

    /// Invariant: on Debian/Ubuntu the password-failed notification profile is a
    /// Primary profile placed after pam_unix and before pam_deny, and can never
    /// grant or deny authentication by itself (Issue ONB-03 / GitHub #161).
    ///
    /// pam-auth-update emits every `Additional` profile after
    /// `auth requisite pam_deny.so`, which terminates the stack on a wrong password:
    /// such a hook is only ever reached after a SUCCESSFUL authentication.
    #[test]
    fn test_debian_notify_profile_sits_between_pam_unix_and_pam_deny() {
        const DEBIAN_UNIX_PRIORITY: u32 = 256;
        let root = workspace_root();

        let notify = fs::read_to_string(root.join("packaging/pam/debian/soos-notify"))
            .expect("read soos-notify");
        let (fields, blocks) = parse_pam_config_profile(&notify);
        assert_eq!(
            fields.get("Auth-Type").map(String::as_str),
            Some("Primary"),
            "soos-notify must be a Primary profile: Additional profiles are emitted after \
             'auth requisite pam_deny.so' and are never reached on a wrong password"
        );
        let priority: u32 = fields
            .get("Priority")
            .expect("soos-notify Priority")
            .parse()
            .expect("numeric soos-notify Priority");
        assert!(
            priority > 0 && priority < DEBIAN_UNIX_PRIORITY,
            "soos-notify Priority ({priority}) must sort after pam_unix ({DEBIAN_UNIX_PRIORITY})"
        );
        for block in ["Auth", "Auth-Initial"] {
            let lines = blocks
                .get(block)
                .unwrap_or_else(|| panic!("soos-notify must define {block}"));
            assert_eq!(
                lines.len(),
                1,
                "soos-notify {block} must hold one module line"
            );
            let (control, module) = split_pam_control(&lines[0]);
            assert_eq!(
                control, "[default=ignore]",
                "soos-notify {block}: the event hook must be ignored whatever it returns \
                 (never 'success=...', 'optional' or 'sufficient')"
            );
            assert_eq!(
                module, "pam_soos.so event=password-failed timeout_ms=20",
                "soos-notify {block} module line"
            );
        }

        let soos =
            fs::read_to_string(root.join("packaging/pam/debian/soos")).expect("read soos profile");
        let (soos_fields, soos_blocks) = parse_pam_config_profile(&soos);
        assert_eq!(
            soos_fields.get("Auth-Type").map(String::as_str),
            Some("Primary")
        );
        let soos_priority: u32 = soos_fields
            .get("Priority")
            .expect("soos Priority")
            .parse()
            .expect("numeric soos Priority");
        assert!(
            soos_priority > DEBIAN_UNIX_PRIORITY,
            "the biometric profile must sort before pam_unix"
        );
        for block in ["Auth", "Auth-Initial"] {
            let lines = soos_blocks.get(block).expect("soos Auth block");
            let (control, _) = split_pam_control(&lines[0]);
            assert_eq!(
                control, "[success=done default=ignore]",
                "soos {block}: biometric line must keep success=done"
            );
        }

        // Package scripts enable/remove both profiles non-interactively.
        let postinst =
            fs::read_to_string(root.join("packaging/debian/postinst")).expect("postinst");
        assert!(postinst.contains("pam-auth-update --package --enable soos soos-notify"));
        let prerm = fs::read_to_string(root.join("packaging/debian/prerm")).expect("prerm");
        assert!(prerm.contains("pam-auth-update --package --remove soos soos-notify"));

        // Documentation must not describe the notify profile as an Additional one.
        for doc in [
            "Docs/DISTRIBUTION_DEPLOYMENT.md",
            "Docs/PACKAGING_AND_PROVISIONING.md",
        ] {
            let content = fs::read_to_string(root.join(doc)).expect("read doc");
            assert!(
                !content.contains("Priority `128`"),
                "{doc} still documents the former Additional soos-notify profile (Priority 128)"
            );
            assert!(
                content.contains("pam_deny"),
                "{doc} must explain that the notify line sits before pam_deny"
            );
        }

        // The Docker PAM matrix proves the generated position with the real module.
        let suite = fs::read_to_string(root.join("tests/docker/test_suite.sh")).expect("suite");
        for needle in [
            "T11",
            "pam-auth-update --package --force --enable soos soos-notify",
            "--record",
            "password-failed",
        ] {
            assert!(
                suite.contains(needle),
                "tests/docker/test_suite.sh must cover '{needle}'"
            );
        }
        let mock = fs::read_to_string(root.join("tests/docker/mock_daemon.py")).expect("mock");
        assert!(
            mock.contains("--record"),
            "mock_daemon.py must record received events"
        );
    }

    /// Writes `content` to `path`, creating parent directories.
    fn write_fixture(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture dir");
        fs::write(path, content).expect("write fixture");
    }

    /// Runs `scripts/pam_snapshot.sh <cmd> --destdir <dir>` and returns the exit code.
    fn run_pam_snapshot(root: &Path, cmd: &str, destdir: &Path) -> i32 {
        std::process::Command::new("bash")
            .arg(root.join("scripts/pam_snapshot.sh"))
            .arg(cmd)
            .arg("--destdir")
            .arg(destdir)
            .output()
            .expect("run pam_snapshot.sh")
            .status
            .code()
            .unwrap_or(-1)
    }

    /// Runs `scripts/uninstall.sh --destdir <dir> --keep-data --skip-systemd`.
    fn run_uninstall_destdir(root: &Path, destdir: &Path) -> std::process::Output {
        std::process::Command::new("bash")
            .arg(root.join("scripts/uninstall.sh"))
            .arg("--destdir")
            .arg(destdir)
            .arg("--keep-data")
            .arg("--skip-systemd")
            .output()
            .expect("execute uninstall.sh")
    }

    const PRISTINE_GDM: &str = "#%PAM-1.0\nauth requisite pam_nologin.so\n@include common-auth\n";
    const PRISTINE_ARCH: &str = "#%PAM-1.0\n-auth [success=2 default=ignore] pam_systemd_home.so\nauth [success=1 default=bad] pam_unix.so try_first_pass nullok\nauth [default=die] pam_faillock.so authfail\nauth optional pam_permit.so\n";

    /// Invariant: the pre-install PAM snapshot helper records every PAM file with a
    /// sha256 manifest in a root-only directory, never overwrites the first snapshot
    /// and detects drift (Issue ONB-08 / GitHub #166).
    #[test]
    fn test_pam_snapshot_helper_records_and_verifies_pre_install_state() {
        let root = workspace_root();
        let helper = root.join("scripts/pam_snapshot.sh");
        assert!(helper.is_file(), "scripts/pam_snapshot.sh must exist");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&helper)
                .expect("helper metadata")
                .permissions()
                .mode();
            assert_ne!(
                mode & 0o111,
                0,
                "scripts/pam_snapshot.sh must be executable"
            );
        }

        let tmp = std::env::temp_dir().join(format!("soos_pam_snapshot_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        write_fixture(&tmp.join("etc/pam.d/gdm-password"), PRISTINE_GDM);
        write_fixture(&tmp.join("etc/pam.d/system-auth"), PRISTINE_ARCH);
        write_fixture(&tmp.join("etc/nsswitch.conf"), "passwd: files\n");

        assert_eq!(
            run_pam_snapshot(&root, "verify", &tmp),
            2,
            "verify without snapshot"
        );
        assert_eq!(run_pam_snapshot(&root, "snapshot", &tmp), 0, "snapshot");

        let snap = tmp.join("var/lib/soos/state/pam-backup");
        assert_eq!(
            fs::read_to_string(snap.join("pam.d/gdm-password")).expect("gdm copy"),
            PRISTINE_GDM
        );
        assert_eq!(
            fs::read_to_string(snap.join("pam.d/system-auth")).expect("system-auth copy"),
            PRISTINE_ARCH
        );
        let manifest = fs::read_to_string(snap.join("SHA256SUMS")).expect("manifest");
        for rel in ["pam.d/gdm-password", "pam.d/system-auth", "nsswitch.conf"] {
            assert!(
                manifest.lines().any(|l| l.ends_with(&format!("  {rel}"))),
                "SHA256SUMS must list {rel}"
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for dir in ["var/lib/soos/state", "var/lib/soos/state/pam-backup"] {
                let mode = fs::metadata(tmp.join(dir))
                    .expect("dir")
                    .permissions()
                    .mode();
                assert_eq!(mode & 0o777, 0o700, "{dir} must be 0700");
            }
            for entry in walk_files(&snap) {
                let mode = fs::metadata(&entry).expect("entry").permissions().mode();
                assert_eq!(
                    mode & 0o002,
                    0,
                    "{} must not be world-writable",
                    entry.display()
                );
            }
        }

        assert_eq!(
            run_pam_snapshot(&root, "verify", &tmp),
            0,
            "verify identical state"
        );

        // Drift is detected, and a second snapshot never replaces the pristine one.
        write_fixture(
            &tmp.join("etc/pam.d/gdm-password"),
            &format!("auth  sufficient  pam_soos.so timeout_ms=2500\n{PRISTINE_GDM}"),
        );
        assert_eq!(
            run_pam_snapshot(&root, "verify", &tmp),
            1,
            "verify must detect drift"
        );
        assert_eq!(
            run_pam_snapshot(&root, "snapshot", &tmp),
            0,
            "second snapshot"
        );
        assert_eq!(
            fs::read_to_string(snap.join("pam.d/gdm-password")).expect("gdm copy"),
            PRISTINE_GDM,
            "an existing snapshot must never be overwritten"
        );

        assert_eq!(run_pam_snapshot(&root, "discard", &tmp), 0, "discard");
        assert!(!snap.exists(), "discard removes the snapshot");
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Recursively lists the regular files below `dir`.
    fn walk_files(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for entry in fs::read_dir(&d).expect("read_dir").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.push(path);
                }
            }
        }
        out
    }

    /// Invariant: uninstall removes residual `pam_soos.so` lines (e.g. inserted by
    /// `soos-admin gdm enable` before backups existed, or an Arch snippet added
    /// without touching jump counts), restores the exact pre-install bytes and
    /// discards the snapshot once the PAM state is verified identical
    /// (Issue ONB-08 / GitHub #166).
    #[test]
    fn test_uninstall_restores_pre_install_pam_state_byte_for_byte() {
        let root = workspace_root();
        let tmp = std::env::temp_dir().join(format!("soos_pam_rollback_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let pam_d = tmp.join("etc/pam.d");
        write_fixture(&pam_d.join("gdm-password"), PRISTINE_GDM);
        write_fixture(&pam_d.join("system-auth"), PRISTINE_ARCH);
        write_fixture(&pam_d.join("sudo"), "#%PAM-1.0\nauth include system-auth\n");
        write_fixture(&tmp.join("etc/nsswitch.conf"), "passwd: files\n");
        assert_eq!(run_pam_snapshot(&root, "snapshot", &tmp), 0, "snapshot");

        // Post-install modifications performed by soos.
        let module = tmp.join("usr/lib/security/pam_soos.so");
        write_fixture(&module, "module");
        write_fixture(
            &pam_d.join("gdm-password"),
            "#%PAM-1.0\nauth requisite pam_nologin.so\nauth  sufficient  pam_soos.so timeout_ms=2500\n@include common-auth\n",
        );
        write_fixture(
            &pam_d.join("system-auth"),
            "#%PAM-1.0\n-auth [success=2 default=ignore] pam_systemd_home.so\nauth  [success=done default=ignore]  pam_soos.so timeout_ms=250\nauth [success=1 default=bad] pam_unix.so try_first_pass nullok\nauth  optional  pam_soos.so event=password-failed timeout_ms=20\nauth [default=die] pam_faillock.so authfail\nauth optional pam_permit.so\n",
        );
        write_fixture(&pam_d.join("soos.snippet"), "auth optional pam_soos.so\n");

        let output = run_uninstall_destdir(&root, &tmp);
        assert!(
            output.status.success(),
            "uninstall.sh must succeed: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            fs::read_to_string(pam_d.join("gdm-password")).expect("gdm"),
            PRISTINE_GDM,
            "gdm-password must be restored byte-for-byte"
        );
        assert_eq!(
            fs::read_to_string(pam_d.join("system-auth")).expect("system-auth"),
            PRISTINE_ARCH,
            "system-auth must be restored byte-for-byte"
        );
        assert!(!pam_d.join("soos.snippet").exists(), "snippet removed");
        assert!(
            !module.exists(),
            "pam_soos.so removed once no PAM file references it"
        );
        assert!(
            !tmp.join("var/lib/soos/state/pam-backup").exists(),
            "a verified snapshot is discarded"
        );
        let leftovers: Vec<_> = fs::read_dir(&pam_d)
            .expect("pam.d")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with('.') || n.ends_with(".soos-backup"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "no temporary or backup file left: {leftovers:?}"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for name in ["gdm-password", "system-auth"] {
                let mode = fs::metadata(pam_d.join(name))
                    .expect("m")
                    .permissions()
                    .mode();
                assert_eq!(
                    mode & 0o022,
                    0,
                    "{name} must not become group/world-writable"
                );
            }
        }
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Invariant: when a residual `pam_soos.so` line cannot be removed safely (the
    /// file uses numeric `success=N` jumps and no pre-install copy matches), the
    /// uninstaller leaves the file untouched, KEEPS `pam_soos.so` installed (a
    /// present module degrades to PAM_IGNORE; a missing one under a `required`
    /// control would break login), keeps the snapshot for the operator and exits
    /// non-zero (Issue ONB-08 / GitHub #166).
    #[test]
    fn test_uninstall_keeps_module_when_pam_rollback_is_unsafe() {
        let root = workspace_root();
        let tmp =
            std::env::temp_dir().join(format!("soos_pam_rollback_unsafe_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let pam_d = tmp.join("etc/pam.d");
        write_fixture(&pam_d.join("common-auth"), PRISTINE_ARCH);
        assert_eq!(run_pam_snapshot(&root, "snapshot", &tmp), 0, "snapshot");

        // The operator adjusted a jump count while inserting the soos line: stripping
        // the line would silently change which module the jump lands on.
        let edited = "#%PAM-1.0\n-auth [success=3 default=ignore] pam_systemd_home.so\nauth [success=done default=ignore] pam_soos.so timeout_ms=250\nauth [success=1 default=bad] pam_unix.so try_first_pass nullok\nauth [default=die] pam_faillock.so authfail\nauth optional pam_permit.so\n";
        write_fixture(&pam_d.join("common-auth"), edited);
        let module = tmp.join("usr/lib/security/pam_soos.so");
        write_fixture(&module, "module");

        let output = run_uninstall_destdir(&root, &tmp);
        assert!(
            !output.status.success(),
            "uninstall.sh must report an incomplete PAM rollback"
        );
        assert_eq!(
            fs::read_to_string(pam_d.join("common-auth")).expect("common-auth"),
            edited,
            "an unsafe file must be left untouched"
        );
        assert!(
            module.exists(),
            "pam_soos.so must be kept while PAM files reference it"
        );
        assert!(
            tmp.join("var/lib/soos/state/pam-backup/pam.d/common-auth")
                .is_file(),
            "the snapshot must be kept for manual restoration"
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Invariant: every PAM edit path records a restorable state and the rescue
    /// procedures only reference files that are actually created (Issue ONB-08 /
    /// GitHub #166).
    #[test]
    fn test_pam_rollback_is_wired_and_documented() {
        let root = workspace_root();

        // 1. install.sh snapshots the live PAM state before installing PAM templates.
        let install = fs::read_to_string(root.join("scripts/install.sh")).expect("install.sh");
        let snap_pos = offset_of(&install, "pam_snapshot.sh", "install.sh");
        let templates_pos = offset_of(
            &install,
            "Installing distribution PAM configuration templates",
            "install.sh",
        );
        assert!(
            snap_pos < templates_pos,
            "install.sh must snapshot PAM state before installing PAM templates"
        );

        // 2. uninstall.sh verifies the rollback against the snapshot, handles residual
        //    pam_soos lines and writes PAM files atomically.
        let uninstall = fs::read_to_string(root.join("scripts/uninstall.sh")).expect("uninstall");
        for needle in ["pam_snapshot.sh", "verify", "mktemp", "mv -f"] {
            assert!(
                uninstall.contains(needle),
                "scripts/uninstall.sh must use '{needle}'"
            );
        }
        assert!(
            !uninstall.contains("cp -f \"${backup}\" \"${orig}\""),
            "backups must be restored atomically (temp file + rename), not with cp -f"
        );

        // 3. soos-admin gdm enable creates a .soos-backup and writes atomically.
        let gdm = fs::read_to_string(root.join("crates/admin-cli/src/gdm.rs")).expect("gdm.rs");
        assert!(
            gdm.contains(".soos-backup"),
            "gdm.rs must create a .soos-backup"
        );
        assert!(
            gdm.contains("sync_all"),
            "gdm.rs must fsync before renaming"
        );
        assert!(
            gdm.contains("fs::rename"),
            "gdm.rs must replace the file atomically"
        );

        // 4. Documentation references the real rescue artifacts.
        for doc in [
            "Docs/DISTRIBUTION_DEPLOYMENT.md",
            "Docs/PACKAGING_AND_PROVISIONING.md",
        ] {
            let content = fs::read_to_string(root.join(doc)).expect("doc");
            assert!(
                content.contains("/var/lib/soos/state/pam-backup"),
                "{doc} must document the pre-install PAM snapshot"
            );
        }
        let deploy = fs::read_to_string(root.join("Docs/DISTRIBUTION_DEPLOYMENT.md")).expect("doc");
        assert!(
            !deploy.contains("sudo cp /etc/pam.d/system-auth.soos-backup /etc/pam.d/system-auth")
                || deploy
                    .contains("sudo cp /etc/pam.d/system-auth /etc/pam.d/system-auth.soos-backup"),
            "the Arch rescue path may only restore a backup the documented procedure creates"
        );

        // 5. A Dockerized rollback test exists and is wired into run_tests.sh and CI.
        let docker_test = root.join("tests/docker/pam_rollback_test.sh");
        assert!(
            docker_test.is_file(),
            "tests/docker/pam_rollback_test.sh must exist"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&docker_test).expect("m").permissions().mode();
            assert_ne!(mode & 0o111, 0, "pam_rollback_test.sh must be executable");
        }
        let docker = fs::read_to_string(&docker_test).expect("docker test");
        for needle in [
            "set -euo pipefail",
            "pam-auth-update --package --enable soos soos-notify",
            "authselect select custom/soos with-faillock --force",
            "scripts/pam_snapshot.sh",
            "scripts/uninstall.sh",
            "sha256sum",
            "pamtester",
        ] {
            assert!(
                docker.contains(needle),
                "pam_rollback_test.sh must cover '{needle}'"
            );
        }
        let run_tests = fs::read_to_string(root.join("run_tests.sh")).expect("run_tests.sh");
        assert!(run_tests.contains("pam_rollback_test.sh"));
        let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("ci.yml");
        assert!(
            ci.contains("pam_rollback_test.sh"),
            "ci.yml must run the rollback test"
        );
    }

    /// Runs `scripts/uninstall.sh --destdir <dir> --purge-data --skip-systemd`.
    fn run_uninstall_purge(root: &Path, destdir: &Path) -> std::process::Output {
        std::process::Command::new("bash")
            .arg(root.join("scripts/uninstall.sh"))
            .arg("--destdir")
            .arg(destdir)
            .arg("--purge-data")
            .arg("--skip-systemd")
            .output()
            .expect("execute uninstall.sh --purge-data")
    }

    /// Invariant: `--purge-data` never destroys the pre-install PAM snapshot while the
    /// PAM rollback is incomplete or unverified: the uninstaller tells the operator to
    /// restore from `state/pam-backup`, so that directory must survive the purge. All
    /// other state (templates, key, evidence) is still purged (GitHub #166 review).
    #[test]
    fn test_uninstall_purge_keeps_snapshot_when_pam_rollback_is_incomplete() {
        let root = workspace_root();
        let tmp = std::env::temp_dir().join(format!("soos_purge_unsafe_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let pam_d = tmp.join("etc/pam.d");
        write_fixture(&pam_d.join("common-auth"), PRISTINE_ARCH);
        assert_eq!(run_pam_snapshot(&root, "snapshot", &tmp), 0, "snapshot");

        let edited = "#%PAM-1.0\n-auth [success=3 default=ignore] pam_systemd_home.so\nauth [success=done default=ignore] pam_soos.so timeout_ms=250\nauth [success=1 default=bad] pam_unix.so try_first_pass nullok\nauth [default=die] pam_faillock.so authfail\nauth optional pam_permit.so\n";
        write_fixture(&pam_d.join("common-auth"), edited);
        write_fixture(&tmp.join("usr/lib/security/pam_soos.so"), "module");
        let state = tmp.join("var/lib/soos");
        write_fixture(&state.join("master.key"), "k");
        write_fixture(&state.join("biometrics/1000.cbor.enc"), "t");

        let output = run_uninstall_purge(&root, &tmp);
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !output.status.success(),
            "an incomplete PAM rollback must still fail: {text}"
        );
        assert_eq!(
            fs::read_to_string(state.join("state/pam-backup/pam.d/common-auth"))
                .expect("the snapshot must survive --purge-data"),
            PRISTINE_ARCH,
            "the pre-install copy must be kept byte-for-byte"
        );
        assert!(
            state.join("state/pam-backup/SHA256SUMS").is_file(),
            "the snapshot manifest must be kept"
        );
        assert!(
            !state.join("master.key").exists() && !state.join("biometrics").exists(),
            "--purge-data must still purge the key and the templates"
        );
        assert!(
            text.contains("Keeping") && text.contains("pam-backup"),
            "the uninstaller must say that the snapshot is kept: {text}"
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Invariant: after a verified PAM rollback, `--purge-data` removes the whole state
    /// directory (the snapshot has been discarded as verified) (GitHub #166 review).
    #[test]
    fn test_uninstall_purge_removes_state_after_verified_rollback() {
        let root = workspace_root();
        let tmp = std::env::temp_dir().join(format!("soos_purge_safe_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let pam_d = tmp.join("etc/pam.d");
        write_fixture(&pam_d.join("gdm-password"), PRISTINE_GDM);
        assert_eq!(run_pam_snapshot(&root, "snapshot", &tmp), 0, "snapshot");
        write_fixture(
            &pam_d.join("gdm-password"),
            "#%PAM-1.0\nauth requisite pam_nologin.so\nauth  sufficient  pam_soos.so timeout_ms=2500\n@include common-auth\n",
        );
        let state = tmp.join("var/lib/soos");
        write_fixture(&state.join("master.key"), "k");

        let output = run_uninstall_purge(&root, &tmp);
        assert!(
            output.status.success(),
            "uninstall.sh --purge-data must succeed: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            fs::read_to_string(pam_d.join("gdm-password")).expect("gdm"),
            PRISTINE_GDM
        );
        assert!(
            !state.exists(),
            "a verified rollback purges the whole state"
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Invariant: a snapshot that fails part-way (here: a PAM entry cannot be copied)
    /// leaves no `state/.pam-backup.*` temporary directory and no partial snapshot
    /// behind, so a journaled install rollback can remove the state directory
    /// (GitHub #166 review).
    #[cfg(unix)]
    #[test]
    fn test_pam_snapshot_failure_leaves_no_temporary_directory() {
        use std::os::unix::fs::PermissionsExt;
        let root = workspace_root();
        let tmp = std::env::temp_dir().join(format!("soos_snap_fail_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        write_fixture(&tmp.join("etc/pam.d/system-auth"), PRISTINE_ARCH);
        write_fixture(&tmp.join("etc/pam.d/sudo"), "auth include system-auth\n");

        // `cp` shim: fails for any copy into a snapshot temp directory, else real cp.
        let shim_dir = tmp.join("shim");
        let shim = shim_dir.join("cp");
        write_fixture(
            &shim,
            "#!/bin/sh\nfor a in \"$@\"; do\n  case \"$a\" in\n    */.pam-backup.*/pam.d/*) echo 'cp: simulated failure' >&2; exit 1 ;;\n  esac\ndone\nfor d in /usr/bin /bin; do\n  [ -x \"$d/cp\" ] && exec \"$d/cp\" \"$@\"\ndone\nexit 127\n",
        );
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).expect("chmod shim");
        let path = format!(
            "{}:{}",
            shim_dir.display(),
            std::env::var("PATH").unwrap_or_default()
        );

        let out = std::process::Command::new("bash")
            .arg(root.join("scripts/pam_snapshot.sh"))
            .arg("snapshot")
            .arg("--destdir")
            .arg(&tmp)
            .env("PATH", &path)
            .output()
            .expect("run pam_snapshot.sh");
        assert!(
            !out.status.success(),
            "a failed copy must fail the snapshot"
        );
        let state = tmp.join("var/lib/soos/state");
        let leftovers: Vec<String> = fs::read_dir(&state)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            leftovers.is_empty(),
            "a failed snapshot must leave nothing in {}: {leftovers:?}",
            state.display()
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    /// Invariant: install.sh journals the PAM snapshot as created BEFORE invoking the
    /// helper when none pre-existed, so a helper that fails part-way is still discarded by
    /// the rollback; and the Docker rollback test drives a failing live install through
    /// install.sh itself (GitHub #166 review).
    #[test]
    fn test_install_journals_pam_snapshot_before_invoking_helper() {
        let root = workspace_root();
        let install = fs::read_to_string(root.join("scripts/install.sh")).expect("install.sh");
        let block = &install[offset_of(&install, "# 6a.", "install.sh")..];
        let flag = offset_of(block, "PAM_SNAPSHOT_CREATED=true", "install.sh block 6a");
        let call = offset_of(block, "pam_snapshot.sh\" snapshot", "install.sh block 6a");
        assert!(
            flag < call,
            "install.sh must mark the snapshot as created before running the helper"
        );
        let docker =
            fs::read_to_string(root.join("tests/docker/pam_rollback_test.sh")).expect("docker");
        for needle in ["scripts/install.sh", ".pam-backup.", "rolled back"] {
            assert!(
                docker.contains(needle),
                "pam_rollback_test.sh must exercise a failing live install ('{needle}')"
            );
        }
    }

    /// Invariant (STO-07, GitHub #180): every `gdm_tests::test_*` /
    /// `gdm_stack_order_tests::test_*` reference in the verification matrix names an
    /// existing test function, and `GDM_PAM_LINE` exists in the admin CLI.
    #[test]
    fn test_matrix_gdm_references_resolve_to_real_tests() {
        let root = workspace_root();
        let matrix = fs::read_to_string(root.join("AI/VERIFICATION_MATRIX.md")).expect("matrix");
        let mut checked = 0_usize;
        for suite in ["gdm_tests", "gdm_stack_order_tests"] {
            let source =
                fs::read_to_string(root.join(format!("crates/admin-cli/tests/{suite}.rs")))
                    .expect("gdm test suite");
            let prefix = format!("`{suite}::");
            for (start, _) in matrix.match_indices(&prefix) {
                let rest = &matrix[start + prefix.len()..];
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                assert!(
                    source.contains(&format!("fn {name}(")),
                    "AI/VERIFICATION_MATRIX.md cites {suite}::{name}, which does not exist"
                );
                checked += 1;
            }
        }
        assert!(
            checked >= 10,
            "the GDM rows must cite their tests ({checked} found)"
        );
        let gdm = fs::read_to_string(root.join("crates/admin-cli/src/gdm.rs")).expect("gdm.rs");
        if matrix.contains("`GDM_PAM_LINE`") {
            assert!(gdm.contains("pub const GDM_PAM_LINE"));
        }
    }
}
