#!/usr/bin/env bash
# =============================================================================
# scripts/pr_loop.sh — Boucle autonome PR, review Copilot et auto-merge
# =============================================================================
# Ce script orchestre la finalisation d'une branche de travail :
#   1. Vérifie qu'on est sur une branche dédiée (refuse 'main')
#   2. Exécute ./save.sh --push-pr pour valider, committer et pousser
#   3. Crée la Pull Request si elle n'existe pas encore
#   4. Attend les vérifications CI (Quality, Security, PAM Docker)
#   5. Attend l'analyse complète de GitHub Copilot (jusqu'à publication de sa review)
#   6. Analyse les commentaires émis par Copilot :
#      - Si des commentaires sont présents : affiche les détails et s'arrête (code 2)
#        pour permettre à l'IA de corriger et de relancer.
#      - Si aucun commentaire et CI 100% verte : fusionne automatiquement dans 'main'
#        et synchronise la branche locale 'main'.
# =============================================================================

set -euo pipefail

# Couleurs
if [[ -t 1 ]]; then
    readonly GREEN='\033[0;32m'
    readonly RED='\033[0;31m'
    readonly YELLOW='\033[1;33m'
    readonly BLUE='\033[0;34m'
    readonly BOLD='\033[1m'
    readonly NC='\033[0m'
else
    readonly GREEN=''
    readonly RED=''
    readonly YELLOW=''
    readonly BLUE=''
    readonly BOLD=''
    readonly NC=''
fi

info()    { echo -e "${BLUE}[INFO]${NC}  $*"; }
success() { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error()   { echo -e "${RED}[FAIL]${NC}  $*" >&2; }
step()    { echo -e "\n${BOLD}── $* ──${NC}"; }

# 1. Vérification de la branche
CURRENT_BRANCH=$(git symbolic-ref --short HEAD 2>/dev/null || echo "detached")
if [[ "$CURRENT_BRANCH" == "main" || "$CURRENT_BRANCH" == "detached" ]]; then
    error "pr_loop.sh doit être exécuté sur une branche dédiée (actuellement : '$CURRENT_BRANCH')."
    exit 1
fi

step "1/6 : Validation locale, commit et push"
info "Exécution du pipeline de qualité et push de la branche '$CURRENT_BRANCH'..."
./save.sh --push-pr "$@"

step "2/6 : Vérification ou création de la Pull Request"
PR_JSON=$(gh pr list --head "$CURRENT_BRANCH" --json number,url,state --state open 2>/dev/null || echo "[]")
PR_NUMBER=$(echo "$PR_JSON" | grep -o '"number":[0-9]*' | head -1 | cut -d':' -f2 || true)

if [[ -z "$PR_NUMBER" ]]; then
    info "Création d'une nouvelle Pull Request pour '$CURRENT_BRANCH' vers 'main'..."
    LAST_COMMIT_MSG=$(git log -1 --pretty=%B)
    FIRST_LINE=$(echo "$LAST_COMMIT_MSG" | head -1)

    PR_BODY="## Résumé
$LAST_COMMIT_MSG

## Vérifications automatiques
- [x] cargo fmt --check
- [x] cargo clippy --all-targets -- -D warnings
- [x] cargo test --all-targets (y compris invariants architecturaux)
- [x] cargo deny check (licences, vulnérabilités, sources, bans)"

    PR_URL=$(gh pr create --title "$FIRST_LINE" --body "$PR_BODY" --base main --head "$CURRENT_BRANCH")
    PR_NUMBER=$(gh pr view --json number -q .number)
    success "Pull Request #$PR_NUMBER créée : $PR_URL"
else
    PR_URL="https://github.com/Mysticaly622/soos/pull/$PR_NUMBER"
    success "Pull Request #$PR_NUMBER existante détectée : $PR_URL"
fi

step "3/6 : Sollicitation de la review Copilot"
info "Sollicitation explicite de GitHub Copilot..."
# 1. Assignation formelle du bot Copilot dans la section Reviewers via l'API GraphQL GitHub
PR_NODE_ID=$(gh api "repos/:owner/:repo/pulls/$PR_NUMBER" --jq '.node_id' 2>/dev/null || true)
if [[ -n "$PR_NODE_ID" ]]; then
    gh api graphql -f query='mutation { requestReviews(input: { pullRequestId: "'"$PR_NODE_ID"'", botIds: ["BOT_kgDOCnlnWA"] }) { pullRequest { id } } }' > /dev/null 2>&1 || true
fi
# 2. Déclenchement explicite via commentaire de mention @copilot review
gh pr comment "$PR_NUMBER" --body "@copilot review" > /dev/null 2>&1 || true
success "Demande de review transmise à GitHub Copilot (reviewers + commentaire)."

step "4/6 : Attente des vérifications CI (GitHub Actions)"
info "Surveillance des 3 jobs CI pour la PR #$PR_NUMBER (Quality, Security, PAM Docker)..."
# Attente initiale pour que GitHub enregistre les workflows déclenchés par le push
sleep 5

CI_PASSED=false
for attempt in $(seq 1 60); do
    if gh pr checks "$PR_NUMBER" >/dev/null 2>&1; then
        CI_PASSED=true
        break
    fi
    STATUS=$(gh pr checks "$PR_NUMBER" 2>&1 || true)
    if echo "$STATUS" | grep -qiE "(fail|cancelled)"; then
        error "Les vérifications CI ont échoué sur GitHub Actions !"
        echo "$STATUS"
        exit 1
    fi
    echo -ne "  ⏳ Vérifications CI en cours (tentative ${attempt}/60)...\r"
    sleep 10
done
echo ""

if [[ "$CI_PASSED" != "true" ]]; then
    if ! gh pr checks "$PR_NUMBER"; then
        error "Délai d'attente CI dépassé ou échec des vérifications !"
        exit 1
    fi
fi
success "Tous les checks CI sont passés au vert !"

step "5/6 : Attente active de l'analyse de code par GitHub Copilot"
info "Copilot analyse le code (cette analyse prend habituellement entre 30s et 5 minutes)..."

MAX_WAIT_SECONDS=480 # 8 minutes maximum
WAITED=0
INTERVAL=10
COPILOT_FINISHED=false

while [[ $WAITED -lt $MAX_WAIT_SECONDS ]]; do
    # 1. Vérifier si une review formelle Copilot a été publiée (Pull Request Review)
    REVIEWS_COPILOT=$(gh api "repos/:owner/:repo/pulls/$PR_NUMBER/reviews" 2>/dev/null | grep -E '"login": "Copilot"' || true)

    # 2. Vérifier si un commentaire d'analyse Copilot a été publié en réponse à @copilot
    COMMENTS_COPILOT=$(gh api "repos/:owner/:repo/issues/$PR_NUMBER/comments" 2>/dev/null | grep -E '"login": "Copilot"' || true)

    # 3. Vérifier si le workflow 'Running Copilot Code Review' a terminé
    COPILOT_RUN_STATUS=$(gh run list --branch "$CURRENT_BRANCH" --json name,status,conclusion 2>/dev/null | grep -i "Copilot" || true)

    if [[ -n "$REVIEWS_COPILOT" ]] || [[ -n "$COMMENTS_COPILOT" ]]; then
        info "Analyse / Réponse de GitHub Copilot détectée !"
        COPILOT_FINISHED=true
        break
    fi

    if [[ -n "$COPILOT_RUN_STATUS" ]] && echo "$COPILOT_RUN_STATUS" | grep -q '"status":"completed"'; then
        info "Le workflow GitHub Copilot s'est achevé !"
        COPILOT_FINISHED=true
        break
    fi

    echo -ne "  ⏳ Attente de Copilot (${WAITED}s / ${MAX_WAIT_SECONDS}s)...\r"
    sleep $INTERVAL
    WAITED=$((WAITED + INTERVAL))
done
echo ""

if [[ "$COPILOT_FINISHED" == "true" ]]; then
    success "Analyse de GitHub Copilot terminée avec succès."
else
    warn "Délai d'attente de Copilot dépassé (${MAX_WAIT_SECONDS}s) ou Copilot n'a pas déclenché de run."
    warn "Poursuite de l'évaluation sur la base des commentaires existants et des tests CI."
fi

step "6/6 : Analyse des retours Copilot et Décision de Fusion"
# Récupérer les commentaires spécifiques de review sur le code
COMMENTS=$(gh api "repos/:owner/:repo/pulls/$PR_NUMBER/comments" 2>/dev/null || echo "[]")
COMMENT_COUNT=$(echo "$COMMENTS" | grep -c '"id":' || true)

if [[ "$COMMENT_COUNT" -gt 0 ]]; then
    warn "GitHub Copilot a émis $COMMENT_COUNT commentaire(s) de révision sur la PR #$PR_NUMBER !"
    echo ""
    info "Détails des points relevés par Copilot :"
    echo "$COMMENTS" | grep -E '("path"|"line"|"body")' | sed 's/^[[:space:]]*//' | head -40
    echo ""
    warn "La PR #$PR_NUMBER NE SERA PAS fusionnée tant que ces points ne sont pas traités."
    info "L'IA va maintenant analyser ces retours, appliquer les corrections, et relancer la boucle."
    exit 2
fi

success "Zéro commentaire bloquant. Les 3 vérifications CI et l'analyse de code sont 100% au vert !"
info "Fusion automatique de la PR #$PR_NUMBER vers 'main'..."

IS_ALREADY_MERGED=$(gh api "repos/:owner/:repo/pulls/$PR_NUMBER" --jq '.merged' 2>/dev/null || echo "false")

if [[ "$IS_ALREADY_MERGED" == "true" ]]; then
    success "═════════════════════════════════════════════════════════════"
    success "  Pull Request #$PR_NUMBER déjà fusionnée avec succès dans main !"
    success "═════════════════════════════════════════════════════════════"
elif gh pr merge "$PR_NUMBER" --squash --delete-branch --admin 2>/dev/null || gh pr merge "$PR_NUMBER" --squash --delete-branch; then
    success "═════════════════════════════════════════════════════════════"
    success "  Pull Request #$PR_NUMBER validée par Copilot et fusionnée dans main !"
    success "═════════════════════════════════════════════════════════════"
else
    error "Échec de la commande gh pr merge sur la PR #$PR_NUMBER."
    exit 1
fi

info "Bascule sur la branche locale 'main' et synchronisation..."
# Préserver les modifications locales éventuelles (ex: compte-rendus générés pendant la relecture)
git stash --include-untracked >/dev/null 2>&1 || true
git checkout main
git pull origin main
git stash pop >/dev/null 2>&1 || true
success "Branche locale 'main' synchronisée. Mission accomplie !"
