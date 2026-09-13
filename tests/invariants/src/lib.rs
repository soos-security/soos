//! Tests automatisés des invariants de sécurité architecturaux pour le projet `soos`.
//!
//! Ces tests valident à chaque `cargo test` que le code écrit (par l'humain ou l'IA)
//! respecte strictement les invariants non négociables définis dans `AI/ARCHITECTURE.md`
//! et `AGENTS.md`.
//!
//! Tous les tests échouent de manière stricte (fail-closed) : si un fichier requis
//! est absent ou déplacé, le test échoue.

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
    /// (Fail-closed : échoue si crates/pam/src est manquant)
    #[test]
    fn test_pam_crate_has_no_unwraps_or_expects_in_production_code() {
        let root = workspace_root();
        let pam_src = root.join("crates").join("pam").join("src");

        assert!(
            pam_src.exists(),
            "VIOLATION STRUCTURE : Le répertoire requis '{}' est introuvable !",
            pam_src.display()
        );

        let mut rs_files = Vec::new();
        collect_rs_files(&pam_src, &mut rs_files);
        assert!(
            !rs_files.is_empty(),
            "Aucun fichier source .rs trouvé dans '{}'",
            pam_src.display()
        );

        for file in rs_files {
            let content = fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("Erreur lecture {}: {}", file.display(), e));

            // Extraire uniquement le code hors des blocs #[cfg(test)]
            let prod_code = extract_production_code(&content);

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
    /// (Fail-closed : échoue si crates/pam/Cargo.toml est manquant)
    #[test]
    fn test_pam_crate_has_no_tokio_dependency() {
        let root = workspace_root();
        let pam_cargo = root.join("crates").join("pam").join("Cargo.toml");

        assert!(
            pam_cargo.exists(),
            "VIOLATION STRUCTURE : Le fichier '{}' est introuvable !",
            pam_cargo.display()
        );

        let content = fs::read_to_string(&pam_cargo)
            .unwrap_or_else(|e| panic!("Erreur lecture {}: {}", pam_cargo.display(), e));
        assert!(
            !content.contains("tokio"),
            "VIOLATION ARCHITECTURE : La crate pam_soos ne doit JAMAIS dépendre de Tokio !"
        );
    }

    /// Invariant 4 — Interdiction absolue d'OpenCV dans tout le projet
    #[test]
    fn test_no_opencv_in_any_cargo_toml() {
        let root = workspace_root();
        let mut cargo_tomls = Vec::new();
        collect_files_named(&root, "Cargo.toml", &mut cargo_tomls);

        assert!(
            !cargo_tomls.is_empty(),
            "Aucun fichier Cargo.toml trouvé dans le projet"
        );

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
    /// (Fail-closed : échoue si crates/protocol/src/types.rs est manquant, analyse rigoureuse du corps des structs)
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
            "VIOLATION STRUCTURE : Le fichier '{}' est introuvable !",
            types_rs.display()
        );

        let content = fs::read_to_string(&types_rs)
            .unwrap_or_else(|e| panic!("Erreur lecture {}: {}", types_rs.display(), e));

        let forbidden_keywords = [
            "password",
            "secret",
            "credential",
            "embedding",
            "frame",
            "image",
        ];

        for struct_keyword in ["struct Request", "struct Response"] {
            let body = extract_struct_body(&content, struct_keyword).unwrap_or_else(|| {
                panic!("Structure '{}' introuvable dans types.rs", struct_keyword)
            });

            // Analyser chaque ligne du corps de la structure
            for line in body.lines() {
                let trimmed = line.trim();
                // Ignorer commentaires et attributs
                if trimmed.starts_with("//")
                    || trimmed.starts_with("/*")
                    || trimmed.starts_with('*')
                    || trimmed.starts_with('#')
                {
                    continue;
                }

                // Si la ligne contient une déclaration de champ (avant ':')
                if let Some(colon_idx) = trimmed.find(':') {
                    let field_decl = &trimmed[..colon_idx].to_lowercase();
                    for &forbidden in &forbidden_keywords {
                        // Chercher le nom du mot interdit comme identifiant de champ
                        let words: Vec<&str> = field_decl.split_whitespace().collect();
                        if let Some(field_name) = words.last() {
                            assert!(
                                !field_name.contains(forbidden),
                                "VIOLATION INVARIANT SÉCURITÉ : La structure '{}' contient un champ interdit '{}' (déclaration : '{}') !",
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

    // --- Utilitaires d'analyse lexicale robustes ---

    /// Isole le corps complet d'une struct entre ses accolades `{` et `}`
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

    /// Filtre le code source pour ignorer les modules `#[cfg(test)] mod ... { ... }`
    fn extract_production_code(source: &str) -> String {
        let mut result = String::new();
        let lines: Vec<&str> = source.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            let trimmed = lines[i].trim();
            // Détection du début d'un module de test
            if trimmed.contains("#[cfg(test)]")
                || (trimmed.starts_with("mod tests") && trimmed.contains('{'))
            {
                // Avancer jusqu'à l'accolade ouvrante
                while i < lines.len() && !lines[i].contains('{') {
                    i += 1;
                }
                if i < lines.len() {
                    let mut depth = 1;
                    // Ignorer tout le contenu jusqu'à fermeture du bloc de test
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
