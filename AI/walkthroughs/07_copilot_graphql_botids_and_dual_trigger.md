# Walkthrough 07 — Sollicitation Copilot via GraphQL botIds et Double Déclencheur

## 1. Contexte & Problématique Identifiée
Lors des tests précédents, l'appel REST standard `POST /repos/:owner/:repo/pulls/:number/requested_reviewers` avec `-f 'reviewers[]=copilot'` échouait silencieusement (renvoyant une liste vide `[]`) car **Copilot** n'est pas un utilisateur humain ou une équipe classique dans l'API REST de GitHub, mais une GitHub App / Bot (`copilot-pull-request-reviewer`, ID: `175728472`, node_id: `BOT_kgDOCnlnWA`). En conséquence, aucun indicateur de demande de revue n'apparaissait dans l'interface web de GitHub.

## 2. Découverte & Solution Technique
L'inspection du schéma GraphQL GitHub de `RequestReviewsInput` a révélé le champ dédié :
```graphql
input RequestReviewsInput {
  pullRequestId: ID!
  userIds: [ID!]
  botIds: [ID!]       # <-- Spécifique aux bots GitHub Apps tels que Copilot !
  teamIds: [ID!]
  union: Boolean
}
```

Nous avons implémenté une stratégie à double déclencheur infaillible :
1. **Assignation Formelle Reviewers (GraphQL)** :
   ```bash
   PR_NODE_ID=$(gh api "repos/:owner/:repo/pulls/$PR_NUMBER" --jq '.node_id')
   gh api graphql -f query='mutation { requestReviews(input: { pullRequestId: "'"$PR_NODE_ID"'", botIds: ["BOT_kgDOCnlnWA"] }) { pullRequest { id } } }'
   ```
   -> Déclenche l'événement `review_requested` et affiche immédiatement **Copilot** avec le voyant jaune sous *Reviewers* dans la colonne latérale droite de la PR sur GitHub.

2. **Mention Explicite (@copilot review)** :
   ```bash
   gh pr comment "$PR_NUMBER" --body "@copilot review"
   ```
   -> Déclenche immédiatement l'événement `copilot_work_started` par le `copilot-swe-agent` qui publie son analyse technique directement en commentaire de la PR.

## 3. Détection Robuste & Écoute Active
Dans `scripts/pr_loop.sh` :
- **Détection des checks CI** : Utilisation du code retour direct de `gh pr checks "$PR_NUMBER"` (code 0 quand tous les checks sont `pass`), garantissant un fonctionnement parfait en environnement non interactif / sans TTY.
- **Détection Copilot** : Surveillance croisée des reviews formelles (`/pulls/:number/reviews`), des commentaires (`/issues/:number/comments`) et du workflow GitHub Actions `Running Copilot Code Review`.

## 4. Vérifications & Résultats
- PR #3 : https://github.com/Mysticaly622/soos/pull/3
- Workflow GitHub Actions : `Running Copilot Code Review` déclenché et actif.
- Réponse immédiate de Copilot obtenue : *"Revue effectuée sur les 2 derniers commits de cette branche. Je n’ai pas identifié de correction supplémentaire à appliquer pour l’instant."*
- Auto-merge vers `main` programmé dès achèvement de la boucle sans friction humaine.
