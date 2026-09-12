# Cycle de Développement, Branches et Pull Requests

> Projet : `soos`  
> Public : Développeurs humains & Assistants IA  

---

## 1. Règle d'or : Jamais de commit direct sur `main`

La branche `main` est protégée. Tout développement (nouvelle fonctionnalité, correctif, test, outillage) doit obligatoirement être réalisé dans une branche dédiée avant d'être fusionné par Pull Request.

---

## 2. Convention de nommage des branches

| Préfixe | Usage | Exemple |
|---|---|---|
| `feat/<nom>` | Nouvelle fonctionnalité ou composant | `feat/ipc-client-pam`, `feat/rate-limit-policy` |
| `fix/<nom>` | Correction de bug ou vulnérabilité | `fix/pam-timeout-fallback`, `fix/zeroize-leak` |
| `test/<nom>` | Ajout de tests, benchmarks ou fixtures | `test/fuzz-codec-v1`, `test/docker-pamtester` |
| `chore/<nom>` | Outillage, CI/CD, dépendances | `chore/cargo-deny-rules`, `chore/ci-caching` |

---

## 3. Workflow de développement TDD en 5 étapes

Pour toute tâche de code, l'agent IA et le développeur suivent rigoureusement ce cycle :

```
┌─────────────────────────────────────────────────────────────┐
│ Étape 0 : Création de branche                              │
│ git checkout -b feat/<nom>                                  │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ Étape 1 : Architecte (Spécification & Invariants)           │
│ - Choix des types, structs, enums, traits                   │
│ - Vérification des invariants ARCHITECTURE.md               │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ Étape 2 : Testeur (TDD Red Phase)                           │
│ - Écriture des tests unitaires et property tests            │
│ - Les tests DOIVENT échouer (red phase)                     │
│ - Test PAM_IGNORE obligatoire pour tout code PAM            │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ Étape 3 : Auditeur (Revue de sécurité statique)             │
│ - Absence de unwrap/expect dans le chemin PAM               │
│ - Respect de #![forbid(unsafe_code)]                        │
│ - Absence de fuites mémoire ou log de données sensibles     │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ Étape 4 : Développeur (TDD Green Phase)                     │
│ - Écriture du code minimal qui fait passer les tests        │
│ - cargo fmt, cargo clippy, cargo test                       │
│ - Mise à jour de AI/VERIFICATION_MATRIX.md                  │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ Étape 5 : Post-implémentation, Docs & Pull Request          │
│ 1. Documentation technique dans Docs/                       │
│ 2. Walkthrough dans AI/walkthroughs/NN_<etape>.md           │
│ 3. Exécution de ./save.sh (fmt + clippy + test + deny)      │
│ 4. Création de la Pull Request via gh pr create             │
└─────────────────────────────────────────────────────────────┘
```

---

## 4. Préparation et création de la Pull Request

Une fois le code validé localement avec `./save.sh` :

```bash
# 1. Pousser la branche de travail vers le dépôt distant
git push -u origin feat/<nom>

# 2. Créer la Pull Request
gh pr create --title "feat(composant): description concise" --body "## Résumé
...
## Tests validés
- [x] cargo fmt
- [x] cargo clippy
- [x] cargo test
- [x] cargo deny check
"
```

La Pull Request déclenche automatiquement la suite complète de vérifications CI GitHub Actions.
