---
name: dev-workflow
description: >
  Workflow de développement multi-agents pour le projet soos.
  Active ce skill quand l'utilisateur demande d'implémenter une fonctionnalité,
  corriger un bug, ou ajouter un composant. Ce skill impose un processus TDD
  strict en 4 phases (Architecte → Testeur → Auditeur → Développeur).
---

# Workflow de Développement Multi-Agents — soos

## Quand utiliser ce workflow

Ce workflow s'applique à **toute modification du code Rust** :
- Nouvelle fonctionnalité
- Nouveau composant/crate
- Correction de bug
- Refactoring significatif

Il ne s'applique PAS aux modifications de documentation pure, scripts shell, ou configuration.

## Prérequis

Avant de commencer, lire obligatoirement :
1. `AI/ARCHITECTURE.md` — architecture maître
2. `AI/DECISIONS.md` — décisions actées
3. `AI/VERIFICATION_MATRIX.md` — critères d'acceptation du composant concerné

## Phase 0 — Branche Git Dédiée

**Objectif** : Isoler tout travail dans une branche thématique, JAMAIS sur `main`.

1. Vérifier la branche courante (`git status`)
2. Créer et basculer sur une nouvelle branche :
   - `git checkout -b feat/<nom>` (ex: `feat/ipc-client`)
   - `git checkout -b fix/<nom>` (ex: `fix/pam-timeout`)
   - `git checkout -b test/<nom>` (ex: `test/docker-pam`)
   - `git checkout -b chore/<nom>` (ex: `chore/ci-rules`)

## Phase 1 — Agent Architecte

**Objectif** : Concevoir avant de coder.

1. Identifier le composant/crate concerné dans la structure du monorepo
2. Lister les structs, enums et traits nécessaires
3. Vérifier la cohérence avec `AI/ARCHITECTURE.md` :
   - Les invariants de sécurité sont-ils respectés ?
   - La séparation des responsabilités est-elle maintenue ?
   - Les dépendances unidirectionnelles sont-elles préservées ?
4. Documenter les choix dans le plan d'implémentation

**Checklist Architecte** :
- [ ] Composant identifié dans le monorepo
- [ ] Types (structs/enums) listés avec leurs champs
- [ ] Traits et interfaces définis
- [ ] Invariants de sécurité vérifiés
- [ ] Pas de nouvelle dépendance non justifiée

## Phase 2 — Agent Testeur

**Objectif** : Écrire les tests AVANT le code (TDD Red Phase).

1. Écrire les tests unitaires pour chaque type et fonction
2. Les tests DOIVENT échouer initialement (compilation error ou assertion failure)
3. Couvrir les cas nominaux ET les cas d'erreur
4. Pour les composants PAM : toujours un test vérifiant `PAM_IGNORE` en cas d'erreur
5. Pour les composants protocol : tests de round-trip et de rejet

**Checklist Testeur** :
- [ ] Tests unitaires écrits pour chaque fonction publique
- [ ] Cas d'erreur couverts (timeout, buffer invalide, UID falsifié, etc.)
- [ ] Test `PAM_IGNORE` si composant PAM
- [ ] Tests conformes à `AI/VERIFICATION_MATRIX.md`

## Phase 3 — Agent Auditeur

**Objectif** : Revue de sécurité AVANT l'implémentation.

1. Vérifier l'absence de `unwrap()` et `expect()` dans le chemin critique
2. Traquer les fuites mémoire potentielles (allocations non bornées)
3. Vérifier `#![forbid(unsafe_code)]` dans les crates métier
4. Vérifier que `unsafe` est minimal, isolé et commenté dans les crates d'adaptation
5. Contrôler le respect des invariants de `ARCHITECTURE.md`
6. Vérifier qu'aucune donnée sensible n'apparaît dans les logs

**Checklist Auditeur** :
- [ ] Zéro `unwrap()` / `expect()` dans le chemin PAM
- [ ] Allocations bornées vérifiées
- [ ] `forbid(unsafe_code)` dans les crates métier
- [ ] Invariants ARCHITECTURE.md respectés
- [ ] Pas de données sensibles dans les logs
- [ ] `catch_unwind` sur toute frontière FFI

## Phase 4 — Agent Développeur

**Objectif** : Implémenter le code qui passe les tests.

1. Écrire l'implémentation en respectant les contraintes de l'Auditeur
2. Vérifier que tous les tests passent (TDD Green Phase)
3. Lancer la vérification qualité :
   - `cargo fmt --check`
   - `cargo clippy -- -D warnings`
   - `cargo test`
4. Mettre à jour `AI/VERIFICATION_MATRIX.md` (cocher les critères validés)

**Checklist Développeur** :
- [ ] Implémentation conforme aux spécifications de l'Architecte
- [ ] Tous les tests du Testeur passent
- [ ] `cargo fmt` — code formaté
- [ ] `cargo clippy -- -D warnings` — zéro warning
- [ ] `cargo test` — tous les tests passent
- [ ] `AI/VERIFICATION_MATRIX.md` mis à jour

## Post-implémentation & Pull Request

Après avoir complété les 4 phases de développement :
1. Valider et committer avec `./save.sh`
2. Valider l'audit de sécurité local avec `cargo deny check`
3. Si le composant implique le module PAM, valider `./run_tests.sh` pour les tests Docker
4. **Créer la Pull Request vers `main`** :
   - Pousser la branche : `git push -u origin <branche>`
   - Créer la PR via GitHub CLI : `gh pr create --title "<titre>" --body "<description>"` ou fournir le lien web de création de PR
   - La PR déclenche automatiquement la CI GitHub Actions (qualité + cargo-deny + PAM Docker)
5. Mettre à jour le walkthrough dans `AI/walkthroughs/`
