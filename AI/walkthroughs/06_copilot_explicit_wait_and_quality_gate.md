# Walkthrough — Attente Active de Copilot & Verrou Qualité de Fusion

> Date : 2026-09-13  
> Branche : `feat/wait-copilot-review`  
> Phase : Fondation — Intégration de l'attente active de GitHub Copilot

---

## Résumé de l'étape

Sur la PR #2, le pipeline CI s'était achevé en 58 secondes, provoquant l'auto-merge avant même que Copilot n'ait eu le temps d'initialiser son workflow de revue (qui requiert entre 2 et 6 minutes sur GitHub).

Cette étape verrouille l'exigence d'excellence :
- **Attente active de Copilot** : [`scripts/pr_loop.sh`](file:///home/hadrien/soos/scripts/pr_loop.sh) surveille désormais l'exécution du workflow `Running Copilot Code Review` et l'arrivée de la review Copilot dans l'API GitHub (`/pulls/$PR_NUMBER/reviews`).
- **Verrou anti-fusion si commentaires présents** : Dès que Copilot publie sa review, le script inspecte `/pulls/$PR_NUMBER/comments`. Si des remarques existent, la fusion est **bloquée**, les détails sont affichés, et l'IA reçoit pour consigne de corriger et re-pousser.
- **Fusion vers `main` conditionnelle** : La PR n'est fusionnée que si les 3 jobs CI sont au vert **ET** que la revue Copilot est achevée sans commentaire bloquant.

---

## Fichiers modifiés

- [`scripts/pr_loop.sh`](file:///home/hadrien/soos/scripts/pr_loop.sh) : boucle d'attente active de Copilot et inspection des commentaires avant merge.
- [`Docs/CYCLE_DE_DEVELOPPEMENT.md`](file:///home/hadrien/soos/Docs/CYCLE_DE_DEVELOPPEMENT.md) : documentation du cycle à 8 phases.
