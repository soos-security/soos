# Documentation Technique — Projet soos

Bienvenue dans la documentation technique du projet **soos** (module PAM de vérification biométrique faciale locale pour Linux).

---

## Sommaire de la documentation

| Document | Description | Public cible |
|---|---|---|
| [**Protocole IPC**](file:///home/hadrien/soos/Docs/PROTOCOLE_IPC.md) | Spécification du protocole binaire v1 entre `pam_soos.so` et `soos-daemon` (types, bornes, codec, invariants). | Développeurs PAM, Développeurs démon |
| [**CI/CD et Sécurité**](file:///home/hadrien/soos/Docs/CI_CD_ET_SECURITE.md) | Fonctionnement du pipeline de qualité, audit des dépendances `cargo-deny`, et tests Docker. | Tous développeurs, DevOps |
| [**Cycle de Développement & Branches**](file:///home/hadrien/soos/Docs/CYCLE_DE_DEVELOPPEMENT.md) | Guide du workflow Git, politique de branches, Pull Requests et cycle TDD multi-agents. | Contributeurs, Orchestration IA |

---

## Documents d'architecture de référence

Pour les spécifications de haut niveau et les choix de conception figés :
- [`AI/ARCHITECTURE.md`](file:///home/hadrien/soos/AI/ARCHITECTURE.md) : Modèle de menace, budget de latence, séparation des privilèges et invariants fondamentaux.
- [`AI/DECISIONS.md`](file:///home/hadrien/soos/AI/DECISIONS.md) : Registre des décisions architecturales actées (anti-dérive).
- [`AI/VERIFICATION_MATRIX.md`](file:///home/hadrien/soos/AI/VERIFICATION_MATRIX.md) : Matrice des critères d'acceptation par composant.
- [`AI/walkthroughs/`](file:///home/hadrien/soos/AI/walkthroughs/) : Historique chronologique et traçabilité de chaque étape de développement.
