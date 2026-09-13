# Walkthrough — CI/CD, Audit cargo-deny & Stratégie Git (Branches & PR)

> Date : 2026-09-13  
> Phase : Fondation — Sécurisation CI/CD, audit de dépendances et workflow de branches

---

## Résumé de l'étape

Cette étape a permis de sécuriser l'intégration continue (GitHub Actions), de corriger les erreurs bloquantes sur l'étape de sécurité (`cargo-deny`), et d'instaurer une discipline Git stricte interdisant tout travail direct sur `main`.

---

## 1. Fichiers livrés et modifiés

| Fichier | Rôle |
|---|---|
| [`.github/workflows/ci.yml`](file:///home/hadrien/soos/.github/workflows/ci.yml) | Pipeline GitHub Actions à 3 jobs (`quality`, `security`, `pam-integration`) |
| [`deny.toml`](file:///home/hadrien/soos/deny.toml) | Configuration cargo-deny 0.20+ (licences permissives, sources crates.io, exclusion OpenCV) |
| [`Cargo.toml`](file:///home/hadrien/soos/Cargo.toml) | Élimination de `atomic-polyfill` via `postcard` no-default-features, ajout `publish = false` |
| [`save.sh`](file:///home/hadrien/soos/save.sh) | Étape 4/4 ajoutée : audit local `cargo deny check` avant tout commit |
| [`AGENTS.md`](file:///home/hadrien/soos/AGENTS.md) | Règles obligatoires de branches, PR, documentation `Docs/` et walkthroughs |
| [`.agents/skills/dev-workflow/SKILL.md`](file:///home/hadrien/soos/.agents/skills/dev-workflow/SKILL.md) | Formalisation de la Phase 0 (Branche) et de la post-implémentation (Docs + PR) |

---

## 2. Résolution des blocages CI/CD (`cargo-deny`)

L'étape 2 de la CI échouait pour 3 raisons résolues :
1. **Drapeau invalide en CI** : suppression de `arguments: --all-features` dans l'action, l'option étant déclarée dans `deny.toml`.
2. **Dépendance non maintenue (`atomic-polyfill`)** : suppression de `heapless` en configurant `postcard = { version = "1", default-features = false, features = ["alloc"] }`.
3. **Licence AGPL des crates internes** : ajout de `publish = false` sur les packages workspace et `[licenses.private] ignore = true` dans `deny.toml`.

---

## 3. Stratégie de Branches et Pull Requests

L'IA et les contributeurs suivent désormais ce cycle pour chaque étape :
1. `git checkout -b <type>/<nom>` (`feat/`, `fix/`, `test/`, `chore/`)
2. Workflow TDD (Architecte → Testeur → Auditeur → Développeur)
3. Rédaction de la documentation dans `Docs/` et du walkthrough dans `AI/walkthroughs/`
4. Validation par `./save.sh` (qui exécute les 4 étapes : fmt, clippy, tests, cargo-deny)
5. Création de la Pull Request vers `main` via `gh pr create`

---

## 4. Résultats des vérifications

- `cargo fmt --check` : **OK**
- `cargo clippy --all-targets -- -D warnings` : **0 warning**
- `cargo test --all-targets` : **20 tests passants**
- `cargo deny check` : **advisories ok, bans ok, licenses ok, sources ok**
- `./save.sh` : **Validé (4/4 étapes réussies)**
