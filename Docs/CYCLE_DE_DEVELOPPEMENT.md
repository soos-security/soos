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
│ 3. Exécution de ./save.sh --push-pr :                       │
│    - fmt, clippy, tests unitaires + invariants, deny        │
│    - hook pre-commit (anti-commit main, secret scanner)     │
│    - git push origin <branche>                              │
│    - création automatique de la Pull Request                │
└─────────────────────────────────────────────────────────────┘
```

---

## 4. Préparation et création de la Pull Request

L'IA et le développeur utilisent la commande unifiée :

```bash
# Sauvegarde locale, push et génération de Pull Request en une seule commande :
./save.sh --push-pr

# Ou avec un message de commit explicite :
./save.sh "feat(policy): implémentation du rate limiting" --push-pr
```

Cette commande :
1. Valide le pipeline 4/4 local (`cargo fmt`, `cargo clippy`, `cargo test`, `cargo deny check`).
2. Vérifie qu'aucun invariant de sécurité n'est violé via la crate `tests/invariants`.
3. Empêche le commit de secrets (hook pre-commit).
4. Pousse automatiquement la branche courante sur GitHub (`git push -u origin <branche>`).
5. Déclenche la création de la Pull Request via `gh pr create` (ou fournit le lien web direct en un clic si `gh` n'a pas encore été configuré avec `gh auth login`).
6. Déclenche le pipeline CI GitHub Actions complet sur la PR.
