# Walkthrough — Garde-fous IA, Tests d'invariants & Automatisation Push/PR

> Date : 2026-09-13  
> Branche : `feat/guardrails-pr-automation`  
> Phase : Fondation — Renforcement des garde-fous IA et automatisation complète du cycle Push & PR

---

## Résumé de l'étape

Cette étape met en place les sécurités ultimes pour empêcher tout dérapage lors du développement assisté par IA :
1. **Verrou physique anti-commit sur `main`** : Bloque techniquement toute tentative de commit direct sur `main` via un hook Git pre-commit.
2. **Suite de tests d'invariants de sécurité (`tests/invariants`)** : 5 tests automatisés vérifient en continu le respect absolu de `AI/ARCHITECTURE.md`.
3. **Filtre anti-fuite de secrets** : Détection et blocage de clés privées et tokens d'API avant commit.
4. **Commande unifiée `./save.sh --push-pr`** : Valide, committe, pousse la branche sur GitHub et génère la Pull Request en une seule opération.

---

## 1. Fichiers créés et modifiés

| Fichier | Rôle |
|---|---|
| [`.githooks/pre-commit`](file:///home/hadrien/soos/.githooks/pre-commit) | Hook Git versionné bloquant les commits directs sur `main` et scannant les secrets |
| [`tests/invariants/Cargo.toml`](file:///home/hadrien/soos/tests/invariants/Cargo.toml) | Déclaration de la crate de tests d'invariants architecturaux |
| [`tests/invariants/src/lib.rs`](file:///home/hadrien/soos/tests/invariants/src/lib.rs) | 5 tests vérifiant statiquement les invariants de sécurité critiques |
| [`save.sh`](file:///home/hadrien/soos/save.sh) | Ajout du blocage sur `main`, option `--push-pr`, push origin et création PR |
| [`AGENTS.md`](file:///home/hadrien/soos/AGENTS.md) | Inscription obligatoire de la commande `./save.sh --push-pr` et du test d'invariants |
| [`.agents/skills/dev-workflow/SKILL.md`](file:///home/hadrien/soos/.agents/skills/dev-workflow/SKILL.md) | Intégration de `--push-pr` dans la phase post-implémentation |
| [`Docs/CYCLE_DE_DEVELOPPEMENT.md`](file:///home/hadrien/soos/Docs/CYCLE_DE_DEVELOPPEMENT.md) | Guide à jour avec la commande unifiée Push & PR |

---

## 2. Détail des 5 invariants de sécurité testés automatiquement

Chaque exécution de `cargo test` lance désormais :
- **`test_business_crates_forbid_unsafe_code`** : Garantit que `#![forbid(unsafe_code)]` est présent dans toutes les crates métier (`protocol`, `policy`, `vision`).
- **`test_pam_crate_has_no_unwraps_or_expects_in_production_code`** : Garantit zéro `unwrap()` ou `expect()` dans le code de production de `crates/pam/src/`.
- **`test_pam_crate_has_no_tokio_dependency`** : Garantit l'absence absolue de Tokio dans le module PAM.
- **`test_no_opencv_in_any_cargo_toml`** : Garantit l'absence totale de dépendance OpenCV dans tout le workspace.
- **`test_protocol_request_and_response_have_no_sensitive_fields`** : Garantit qu'aucun champ sensible (password, secret, embedding, frame, image) ne transite dans les requêtes/réponses IPC.

---

## 3. Résultats des vérifications

```bash
cargo fmt --check            # ✅ Code parfaitement formaté
cargo clippy --all-targets   # ✅ 0 warning
cargo test --all-targets     # ✅ 25 tests passants (16 protocol + 4 PAM + 5 invariants)
cargo deny check             # ✅ Licences, failles RustSec, bans et sources validés
.githooks/pre-commit         # ✅ Hook actif via core.hooksPath
```
