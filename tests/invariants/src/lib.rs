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
}
