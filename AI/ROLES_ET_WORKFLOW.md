# Règles de Développement et Workflow IA (ZTLH)

Ce document définit les règles strictes que l'agent IA doit respecter lors de la génération de code pour le projet Zero-Trust Linux Hello.

## 1. Méthodologie Multi-Agents (Comité d'Experts)
Pour chaque fonctionnalité demandée, l'IA doit structurer sa réponse en 4 étapes consécutives :
1. **Agent Architecte :** Liste les structures (structs/enums), les traits et les invariants de sécurité.
2. **Agent Testeur :** Rédige les tests unitaires exhaustifs (qui doivent échouer initialement) avant toute implémentation.
3. **Agent Auditeur (Candide) :** Traque les fuites mémoires potentiels, les paniques (`unwrap`, `expect`) et valide le respect du document `ARCHITECTURE.md`.
4. **Agent Développeur :** Fournit l'implémentation finale Rust qui passe les tests de l'Agent Testeur.

## 2. Développement Piloté par les Tests (TDD Strict)
- Ne jamais produire de code métier sans avoir d'abord produit les tests unitaires.
- Tout composant PAM doit avoir un test garantissant qu'il retourne `PAM_IGNORE` en cas d'erreur interne.

## 3. Automatisation des Tâches (Scripts à générer par l'IA)
L'utilisateur s'appuie sur des scripts pour pallier son manque de familiarité avec les outils. Lors de la première session, l'IA devra générer ces scripts :
- **`save.sh` :** Un script bash qui exécute `cargo fmt`, `cargo clippy -D warnings`, et `cargo test`. Si tout passe, le script fait un `git add .`, génère un message de commit basé sur le diff, et fait un `git commit`.
- **`run_tests.sh` & `Dockerfile` :** L'environnement de test. Les modules PAM ne doivent jamais être testés directement sur la machine hôte. Le script doit monter un conteneur Ubuntu éphémère, y compiler le `.so`, exécuter les tests d'intégration avec `pamtester`, puis détruire le conteneur.

## 4. Gestion des Erreurs de Compilation
Si l'utilisateur fournit une sortie brute de `cargo check`, l'IA ne doit pas donner d'explications verbeuses. Elle doit analyser silencieusement l'erreur du *borrow checker* et fournir directement le fichier corrigé complet.