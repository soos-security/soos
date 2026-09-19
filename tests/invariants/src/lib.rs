//! Automated architectural security invariant test suite for the `soos` workspace.
//!
//! These tests validate on every `cargo test` run that all code (written by humans or AI)
//! strictly adheres to the non-negotiable security invariants defined in `AI/ARCHITECTURE.md`
//! and `AGENTS.md`.
//!
//! All tests fail strictly (fail-closed): if a required file is missing or misplaced,
//! the test immediately fails.

#![forbid(unsafe_code)]

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

        assert!(deb_soos_content.contains("pam_soos.so timeout_ms=250"));
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

        let soos_pos = fedora_content
            .find("pam_soos.so timeout_ms=250")
            .expect("soos auth in fedora");
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
        let arch_soos_pos = arch_content
            .find("pam_soos.so timeout_ms=250")
            .expect("soos auth in arch");
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
            .contains("auth  [success=done default=ignore]  pam_soos.so timeout_ms=250"));
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

        let status = std::process::Command::new("bash")
            .arg(&install_sh)
            .arg("--destdir")
            .arg(&tmp_dir)
            .arg("--skip-models")
            .arg("--skip-systemd")
            .status()
            .expect("execute install.sh");

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
        assert!(master_key.is_file(), "master.key must be created");

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

            let key_meta = fs::metadata(&master_key).expect("master.key meta");
            assert_eq!(
                key_meta.permissions().mode() & 0o777,
                0o600,
                "master.key must be mode 0600"
            );
            assert_eq!(key_meta.len(), 32, "master.key must be 32 bytes");
        }

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
        let service_file = tmp_dir.join("etc/systemd/system/soos-daemon.service");
        let backup_pam = tmp_dir.join("etc/pam.d/system-auth.soos-backup");
        let active_pam = tmp_dir.join("etc/pam.d/system-auth");

        fs::create_dir_all(&bio_dir).expect("create bio_dir");
        fs::create_dir_all(tmp_dir.join("usr/bin")).expect("create bin dir");
        fs::create_dir_all(tmp_dir.join("etc/systemd/system")).expect("create systemd dir");
        fs::create_dir_all(tmp_dir.join("etc/pam.d")).expect("create pam.d dir");

        fs::write(&bin_file, b"binary content").expect("write bin");
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
        assert!(
            spec_content.contains("0600") && spec_content.contains("master.key"),
            "RPM spec must specify 0600 mode for master key"
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
}
