# Walkthrough — Infrastructure de développement (Dockerfile, run_tests.sh, save.sh)

> Date : 2026-09-12  
> Phase : Fondation — Mise en place de l'environnement de développement

## Fichiers livrés

| Fichier | Rôle | Lignes |
|---|---|---:|
| [`Dockerfile`](file:///home/hadrien/soos/Dockerfile) | Image Ubuntu 24.04 avec Rust + PAM + pamtester + utilisateur factice | ~90 |
| [`run_tests.sh`](file:///home/hadrien/soos/run_tests.sh) | Orchestration conteneur éphémère, compilation release, 3 tests pamtester | ~160 |
| [`save.sh`](file:///home/hadrien/soos/save.sh) | Pipeline `cargo fmt` → `clippy -D warnings` → `test` → `git commit` auto | ~200 |

---

## Ce que fait chaque fichier

### Dockerfile

- Base **Ubuntu 24.04** avec `build-essential`, `libpam0g-dev`, `libclang-dev`, `pamtester`
- Installe Rust via `rustup` (toolchain stable + clippy + rustfmt)
- Crée l'utilisateur `testuser` avec mot de passe `password123`
- Configure un service PAM de test `/etc/pam.d/test-soos` avec la pile :
  ```pam
  auth [success=done default=ignore] pam_soos.so timeout_ms=250
  auth required                      pam_unix.so
  ```
- Le code source n'est **jamais copié** dans l'image — il est monté en bind au runtime

### run_tests.sh

Exécute 3 tests dans le conteneur éphémère :

| Test | Description | Invariant validé |
|---|---|---|
| **T1** | Charge le `.so` + mot de passe correct → succès via `pam_unix` | Le module retourne `PAM_IGNORE`, l'ABI C est correcte |
| **T2** | Mot de passe incorrect → refus | Le module ne bloque pas le flux d'erreur normal |
| **T3** | Module `.so` absent → le système reste fonctionnel | Résilience de la pile PAM |

Le conteneur est détruit automatiquement (`--rm`) après chaque exécution.

### save.sh

Pipeline séquentiel avec arrêt au premier échec :
1. `cargo fmt` — formate en place
2. `cargo clippy -- -D warnings` — zéro warning toléré
3. `cargo test` — suite complète
4. `git add .` — stage tout
5. Génération d'un message de commit catégorisé (feat/test/docs/chore) basé sur les fichiers modifiés
6. `git commit` — commit local uniquement, **jamais de push**

Supporte un message personnalisé : `./save.sh "fix(pam): correction du timeout"`

---

## Prochaine étape

Créer le monorepo Cargo (workspace `Cargo.toml` + crate `pam` avec un `pam_sm_authenticate` minimal retournant `PAM_IGNORE`) pour rendre le bac à sable fonctionnel de bout en bout.
