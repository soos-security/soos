# Walkthrough — Mise en place du harnais IA + Monorepo Cargo

> Date : 2026-09-13
> Phase : Fondation — Harnais IA professionnel + squelette compilable

## Résumé

Mise en place complète de l'infrastructure de développement par IA pour le projet soos :
- **Harnais IA** : instructions persistantes, matrice de vérification, workflow multi-agents
- **Monorepo Cargo** : workspace avec 2 crates (`protocol`, `pam`), 20 tests passants
- **Pipeline qualité** : `fmt` + `clippy` + `test` + commit automatisé validé

## Fichiers créés / modifiés

| Fichier | Rôle | Statut |
|---|---|---|
| [`AGENTS.md`](file:///home/hadrien/soos/AGENTS.md) | Instructions persistantes chargées automatiquement par l'IA | **NEW** |
| [`.gitignore`](file:///home/hadrien/soos/.gitignore) | Exclusion de `target/`, modèles ONNX, données biométriques | **NEW** |
| [`AI/DECISIONS.md`](file:///home/hadrien/soos/AI/DECISIONS.md) | Corrigé : `pam_ztlh` → `pam_soos`, dates ajoutées, décisions nommage/codec | **MODIFIED** |
| [`AI/VERIFICATION_MATRIX.md`](file:///home/hadrien/soos/AI/VERIFICATION_MATRIX.md) | Matrice composant × critères d'acceptation, checklist actionnable | **NEW** |
| [`Cargo.toml`](file:///home/hadrien/soos/Cargo.toml) | Workspace root, `resolver = "2"`, versions centralisées | **NEW** |
| [`rust-toolchain.toml`](file:///home/hadrien/soos/rust-toolchain.toml) | Pin toolchain stable + clippy + rustfmt | **NEW** |
| [`crates/protocol/Cargo.toml`](file:///home/hadrien/soos/crates/protocol/Cargo.toml) | Crate protocol : types IPC, codec postcard/serde | **NEW** |
| [`crates/protocol/src/lib.rs`](file:///home/hadrien/soos/crates/protocol/src/lib.rs) | Point d'entrée, `#![forbid(unsafe_code)]` | **NEW** |
| [`crates/protocol/src/types.rs`](file:///home/hadrien/soos/crates/protocol/src/types.rs) | Request, Response, Verdict, ReasonClass, Event + validation | **NEW** |
| [`crates/protocol/src/codec.rs`](file:///home/hadrien/soos/crates/protocol/src/codec.rs) | Codec binaire borné + 16 tests unitaires | **NEW** |
| [`crates/pam/Cargo.toml`](file:///home/hadrien/soos/crates/pam/Cargo.toml) | Crate PAM cdylib | **NEW** |
| [`crates/pam/src/lib.rs`](file:///home/hadrien/soos/crates/pam/src/lib.rs) | Module PAM squelette (PAM_IGNORE) + catch_unwind + 4 tests | **NEW** |
| [`.agents/skills/dev-workflow/SKILL.md`](file:///home/hadrien/soos/.agents/skills/dev-workflow/SKILL.md) | Skill Antigravity : workflow TDD 4 phases | **NEW** |

---

## Tests validés

### `cargo test` — 20/20 ✅

**Protocol (16 tests)** :
- Round-trip : Request, Response, Event
- Rejet : service trop long, taille déclarée trop grande, version non supportée
- Validation : request_id 256 bits, verdict should_ignore, response is_allow
- Robustesse : buffer vide, buffer tronqué, buffer court

**PAM (4 tests)** :
- `authenticate_returns_pam_ignore` — invariant 5 de ARCHITECTURE.md
- `setcred_returns_pam_ignore`
- `pam_ignore_has_correct_value` — vérifie la constante Linux-PAM
- `panic_safety_returns_pam_ignore` — garantie catch_unwind

### Pipeline qualité ✅
- `cargo fmt --check` : OK
- `cargo clippy -- -D warnings` : zéro warning (y compris pedantic pour protocol)
- `./save.sh` : commit `a5bad02` réussi

---

## Architecture du harnais IA

```
AGENTS.md ──────────────────────► Chargé automatiquement par Antigravity
                                  à chaque nouvelle conversation
                                  (règles TDD, interdictions, nommage)

AI/ARCHITECTURE.md ─────────────► Document maître (déjà existant)
AI/DECISIONS.md ────────────────► Anti-hallucination (corrigé)
AI/MOCK_STRATEGY.md ────────────► Simulation sans matériel (déjà existant)
AI/VERIFICATION_MATRIX.md ──────► Critères d'acceptation par composant (NEW)

.agents/skills/dev-workflow/ ───► Skill activable automatiquement :
                                  Architecte → Testeur → Auditeur → Développeur
```

---

## Prochaine étape

Le monorepo est fonctionnel et le harnais IA est en place. Les prochaines tâches naturelles sont :

1. **Tester `./run_tests.sh` avec Docker** — valider le module PAM dans le conteneur
2. **Implémenter le client IPC dans `crates/pam/`** — connexion socket, timeout, requête/réponse
3. **Créer `crates/policy/`** — logique de décision, rate limit, sans I/O
