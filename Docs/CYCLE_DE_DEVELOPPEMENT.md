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

## 3. Workflow de développement TDD en 6 étapes (Étape 0 à Étape 5)

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
│ 3. Exécution de ./save.sh --push-pr :                       │
│    - fmt, clippy, tests unitaires + invariants, deny        │
│    - hook pre-commit (anti-commit main, secret scanner)     │
│    - git push origin <branche>                              │
│    - création automatique de la Pull Request                │
└─────────────────────────────────────────────────────────────┘
```

---

## 4. Préparation, Revue Copilot et Auto-Merge

Pour les développements autonomes par IA ou les contributeurs souhaitant une automatisation complète :

```bash
# Boucle complète autonome : validation, push, PR, review Copilot, et auto-merge vers main :
./save.sh --auto-merge

# Ou via le script dédié :
./scripts/pr_loop.sh "feat(composant): description explicite"
```

### Déroulement de la boucle autonome :
1. **Quality Gates** : Exécution de `cargo fmt`, `cargo clippy -D warnings`, `cargo test` (y compris invariants architecturaux) et `cargo deny check`.
2. **Contrôle Pre-Commit** : Vérification de la branche (refus de `main`) et filtre anti-fuite de secrets.
3. **Push & Pull Request** : Poussée de la branche vers GitHub et création automatique de la Pull Request si elle n'existe pas encore.
4. **Sollicitation Copilot (Double Déclencheur)** :
   - Assignation formelle du bot Copilot dans la section Reviewers via l'API GraphQL (`requestReviews(botIds: ["BOT_kgDOCnlnWA"])`).
   - Déclenchement immédiat de l'agent de revue via un commentaire ciblé `@copilot review`.
5. **Surveillance CI** : Surveillance en temps réel de l'avancement des 3 jobs GitHub Actions (`Quality`, `Security`, `PAM Integration Docker`).
6. **Attente active de l'analyse Copilot** : Le script attend que le workflow d'analyse Copilot ou le bot SWE se termine et publie sa review (entre 30 secondes et 6 minutes en moyenne).
7. **Traitement strict des retours** : Si Copilot émet des commentaires ou demande des changements, la PR n'est **PAS** fusionnée. Le script renvoie les détails précis des fichiers et lignes ciblés pour que l'IA applique les corrections et re-soumette la branche.
8. **Fusion automatique (Auto-Merge)** : Dès que les 3 jobs CI sont passés au vert et que Copilot n'a plus de remarques non résolues, la Pull Request est fusionnée automatiquement dans `main` (`gh pr merge --squash --delete-branch`), et l'environnement local est synchronisé sur `main` (`git checkout main && git pull origin main`).
