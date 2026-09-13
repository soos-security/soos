# Pipeline Qualité, CI/CD et Audit de Sécurité

> Projet : `soos`  
> Statut : Opérationnel  

---

## 1. Vue d'ensemble des Quality Gates

Le projet impose une vérification de conformité à 3 niveaux concentriques :

```
┌─────────────────────────────────────────────────────────────┐
│  Niveau 1 : save.sh (Local — exécuté avant chaque commit)   │
│  - cargo fmt                                                │
│  - cargo clippy --all-targets -- -D warnings                │
│  - cargo test --all-targets                                 │
│  - cargo deny check (licences, vulnérabilités, bans)        │
└──────────────────────────────┬──────────────────────────────┘
                               │ git push / Pull Request
                               ▼
┌─────────────────────────────────────────────────────────────┐
│  Niveau 2 : GitHub Actions CI (.github/workflows/ci.yml)    │
│  - Job 1: Quality (fmt + clippy + test)                     │
│  - Job 2: Security (cargo-deny)                             │
│  - Job 3: PAM Integration (Docker)                          │
└──────────────────────────────┬──────────────────────────────┘
                               │ Validation sandbox PAM
                               ▼
┌─────────────────────────────────────────────────────────────┐
│  Niveau 3 : Tests d'intégration Docker (run_tests.sh)       │
│  - Compilation du .so dans Ubuntu 24.04                     │
│  - pamtester test-soos (T1: succès, T2: refus, T3: résilience│
└─────────────────────────────────────────────────────────────┘
```

---

## 2. Le script local `save.sh`

Le script [`save.sh`](../save.sh) est l'outil principal du contributeur et de l'IA pour valider et committer le travail :
- **Exécution séquentielle** avec arrêt immédiat au premier incident (`set -euo pipefail`).
- **Génération automatique du message de commit conventionnel** basée sur la nature des fichiers modifiés (`feat(...)`, `test:`, `docs:`, `chore:`).
- **Contrôle du push** : Par défaut, `save.sh` effectue uniquement une sauvegarde et un commit locaux sans aucun `git push`. Pour pousser la branche vers GitHub et créer la Pull Request de manière intégrée, utiliser l'option `--push-pr`.

Utilisation :
```bash
# Sauvegarde locale avec message automatique :
./save.sh

# Sauvegarde locale avec message personnalisé :
./save.sh "feat(policy): implémentation du rate limiting par UID"

# Sauvegarde, push et création automatique de Pull Request :
./save.sh --push-pr
```

---

## 3. Audit des dépendances avec `cargo-deny`

Le fichier [`deny.toml`](../deny.toml) verrouille les approvisionnements de code tiers selon les directives d'architecture :

### Règles appliquées :
- **Advisories** : Toute vulnérabilité RustSec connue non résolue entraîne l'échec de la CI.
- **Licences** : Seules les licences OSS permissives sont autorisées pour les dépendances (MIT, Apache-2.0, BSD-2/3, ISC, etc.). Les crates internes du workspace sous licence AGPL-3.0 sont isolées via `publish = false` et `[licenses.private] ignore = true`.
- **Sources** : Seul l'index officiel `crates.io` est autorisé (interdiction des registres obscurs ou dépôts Git non audités).
- **Bans explicites** : Interdiction absolue de crates indésirables (notamment `opencv` conformément à [`AI/ARCHITECTURE.md`](../AI/ARCHITECTURE.md)).

Pour exécuter l'audit manuellement :
```bash
cargo deny check
```

---

## 4. Tests d'intégration PAM sous Docker (`run_tests.sh`)

Pour éviter de compromettre la machine de développement avec des modules PAM expérimentaux, les tests d'intégration sont exécutés dans un conteneur éphémère Ubuntu :
```bash
./run_tests.sh
```
Ce script vérifie :
1. **T1 (ABI + nominal)** : Chargement de `pam_soos.so`, renvoi de `PAM_IGNORE`, succès via `pam_unix`.
2. **T2 (Refus)** : Mauvais mot de passe rejeté normalement.
3. **T3 (Résilience)** : Absence du module `.so` ne bloquant pas l'authentification système.
