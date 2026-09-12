//! Tests automatisés des invariants de sécurité architecturaux pour le projet `soos`.
//!
//! Ces tests valident à chaque `cargo test` que le code écrit (par l'humain ou l'IA)
//! respecte strictement les invariants non négociables définis dans `AI/ARCHITECTURE.md`
//! et `AGENTS.md`.

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Retrouve la racine du workspace Cargo en remontant jusqu'à Cargo.toml
    fn workspace_root() -> PathBuf {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest_dir
            .parent()
            .and_then(|p| p.parent())
            .expect("Impossible de retrouver la racine du workspace")
            .to_path_buf()
    }

    /// Invariant 1 — #![forbid(unsafe_code)] obligatoire dans toutes les crates métier
    #[test]
    fn test_business_crates_forbid_unsafe_code() {
        let root = workspace_root();
        let business_crates = ["protocol", "policy", "vision"];

        for crate_name in business_crates {
            let lib_path = root
                .join("crates")
                .join(crate_name)
                .join("src")
                .join("lib.rs");
            if lib_path.exists() {
                let content = fs::read_to_string(&lib_path)
                    .unwrap_or_else(|e| panic!("Erreur lecture {}: {}", lib_path.display(), e));
                assert!(
                    content.contains("#![forbid(unsafe_code)]"),
                    "VIOLATION INVARIANT : La crate métier '{}' dans {} DOIT contenir '#![forbid(unsafe_code)]' !",
                    crate_name,
                    lib_path.display()
                );
            }
        }
    }

    /// Invariant 2 — Zéro unwrap() ou expect() dans le code de production du module PAM
    #[test]
    fn test_pam_crate_has_no_unwraps_or_expects_in_production_code() {
        let root = workspace_root();
        let pam_src = root.join("crates").join("pam").join("src");

        if !pam_src.exists() {
            return;
        }

        let mut rs_files = Vec::new();
        collect_rs_files(&pam_src, &mut rs_files);

        for file in rs_files {
            let content = fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("Erreur lecture {}: {}", file.display(), e));

            // Extraire uniquement le code hors des blocs #[cfg(test)]
            let prod_code = strip_test_modules(&content);

            let unwraps: Vec<(usize, &str)> = prod_code
                .lines()
                .enumerate()
                .filter(|(_, line)| {
                    let trimmed = line.trim();
                    // Ignorer les commentaires
                    !trimmed.starts_with("//")
                        && !trimmed.starts_with("/*")
                        && !trimmed.starts_with('*')
                        && (trimmed.contains(".unwrap()") || trimmed.contains(".expect("))
                })
                .collect();

            assert!(
                unwraps.is_empty(),
                "VIOLATION INVARIANT SÉCURITÉ PAM : unwrap() ou expect() détecté dans le code de production de {} :\n{:#?}",
                file.display(),
                unwraps
            );
        }
    }

    /// Invariant 3 — Interdiction absolue du runtime Tokio dans le module PAM
    #[test]
    fn test_pam_crate_has_no_tokio_dependency() {
        let root = workspace_root();
        let pam_cargo = root.join("crates").join("pam").join("Cargo.toml");

        if pam_cargo.exists() {
            let content = fs::read_to_string(&pam_cargo)
                .unwrap_or_else(|e| panic!("Erreur lecture {}: {}", pam_cargo.display(), e));
            assert!(
                !content.contains("tokio"),
                "VIOLATION ARCHITECTURE : La crate pam_soos ne doit JAMAIS dépendre de Tokio !"
            );
        }
    }

    /// Invariant 4 — Interdiction absolue d'OpenCV dans tout le projet
    #[test]
    fn test_no_opencv_in_any_cargo_toml() {
        let root = workspace_root();
        let mut cargo_tomls = Vec::new();
        collect_files_named(&root, "Cargo.toml", &mut cargo_tomls);

        for cargo_file in cargo_tomls {
            let content = fs::read_to_string(&cargo_file)
                .unwrap_or_else(|e| panic!("Erreur lecture {}: {}", cargo_file.display(), e));
            assert!(
                !content.contains("opencv"),
                "VIOLATION ARCHITECTURE : Référence interdite à 'opencv' dans {} !",
                cargo_file.display()
            );
        }
    }

    /// Invariant 5 — Aucun mot de passe, secret, image ou embedding dans Request et Response
    #[test]
    fn test_protocol_request_and_response_have_no_sensitive_fields() {
        let root = workspace_root();
        let types_rs = root
            .join("crates")
            .join("protocol")
            .join("src")
            .join("types.rs");

        if types_rs.exists() {
            let content = fs::read_to_string(&types_rs)
                .unwrap_or_else(|e| panic!("Erreur lecture {}: {}", types_rs.display(), e));

            // Analyse des définitions des structs Request et Response
            for struct_name in ["struct Request", "struct Response"] {
                if let Some(pos) = content.find(struct_name) {
                    let struct_def = &content[pos..pos + 500.min(content.len() - pos)];
                    let end_pos = struct_def.find('}').unwrap_or(struct_def.len());
                    let fields_str = &struct_def[..end_pos].to_lowercase();

                    for forbidden in [
                        "password",
                        "secret",
                        "credential",
                        "embedding",
                        "frame",
                        "image",
                    ] {
                        assert!(
                            !fields_str.contains(&format!("pub {}:", forbidden)),
                            "VIOLATION INVARIANT SÉCURITÉ : La structure '{}' ne doit JAMAIS contenir de champ sensible '{}' !",
                            struct_name,
                            forbidden
                        );
                    }
                }
            }
        }
    }

    // Utilitaires de parcours
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
                // Éviter de fouiller dans target/ ou .git/
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

    /// Découpe le fichier pour ignorer le contenu après `mod tests`
    fn strip_test_modules(content: &str) -> String {
        if let Some(idx) = content.find("mod tests") {
            content[..idx].to_string()
        } else {
            content.to_string()
        }
    }
}
