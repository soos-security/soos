# Walkthrough — Boucle Autonome PR, Revue Copilot & Auto-Merge

> Date : 2026-09-13  
> Branche : `feat/auto-pr-copilot-loop`  
> Phase : Fondation — Automatisation intégrale du cycle de vie des Pull Requests

---

## Résumé de l'étape

Cette étape répond à la demande d'autonomie complète pour le développement assisté par IA :
À chaque fin de tâche ou étape, l'IA ne s'arrête plus à la création d'une Pull Request. Elle exécute désormais un cycle complet et autonome qui :
1. Pousse la branche vers GitHub et crée la Pull Request.
2. Déclenche et surveille l'analyse automatique de GitHub Copilot.
3. Surveille les 3 jobs GitHub Actions (`Quality`, `Security`, `PAM Integration Docker`).
4. Récupère et résout immédiatement les éventuels retours et suggestions de Copilot.
5. Dès que tous les feux sont au vert, **fusionne automatiquement la PR vers `main`** (`--squash --delete-branch`) et synchronise l'environnement local sur `main`.

L'humain n'a plus besoin d'intervenir manuellement sur GitHub entre les étapes : les puissants garde-fous (5 tests d'invariants de sécurité, cargo-deny, sandbox Docker, hook pre-commit) garantissent la sûreté du système en continu.

---

## 1. Fichiers créés et modifiés

| Fichier | Rôle |
|---|---|
| [`scripts/pr_loop.sh`](file:///home/hadrien/soos/scripts/pr_loop.sh) | Script d'orchestration autonome (push, création PR, review Copilot, watch CI, auto-merge et sync main) |
| [`save.sh`](file:///home/hadrien/soos/save.sh) | Support natif de l'option `--auto-merge` qui active la boucle complète |
| [`AGENTS.md`](file:///home/hadrien/soos/AGENTS.md) | Directive permanente imposant le cycle autonome jusqu'au merge sur `main` |
| [`.agents/skills/dev-workflow/SKILL.md`](file:///home/hadrien/soos/.agents/skills/dev-workflow/SKILL.md) | Mise à jour de la checklist et de la phase post-implémentation |
| [`Docs/CYCLE_DE_DEVELOPPEMENT.md`](file:///home/hadrien/soos/Docs/CYCLE_DE_DEVELOPPEMENT.md) | Documentation technique du cycle autonome à destination des contributeurs |

---

## 2. Commandes disponibles

```bash
# Lancement de la boucle autonome complète :
./save.sh --auto-merge

# Ou via le script dédié avec message :
./scripts/pr_loop.sh "feat(nom): message explicite"
```
