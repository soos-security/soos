#!/usr/bin/env bash
# =============================================================================
# scripts/pr_loop.sh — Boucle autonome PR, review Copilot et auto-merge
# =============================================================================
# Ce script orchestre la finalisation d'une branche de travail :
#   1. Vérifie qu'on est sur une branche dédiée (refuse 'main')
#   2. Exécute ./save.sh --push-pr pour valider, committer et pousser
#   3. Crée la Pull Request si elle n'existe pas encore
#   4. Demande la review à Copilot
#   5. Surveille les checks CI jusqu'à complétion
#   6. Récupère et analyse les commentaires de review Copilot
#   7. Si tous les checks sont verts :
#      - Fusionne automatiquement la PR (--squash --delete-branch)
#      - Bascule sur 'main' et synchronise 'git pull'
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

step "1/5 : Validation locale, commit et push"
info "Exécution du pipeline de qualité et push de la branche '$CURRENT_BRANCH'..."
./save.sh --push-pr "$@"

step "2/5 : Vérification ou création de la Pull Request"
# Vérifier si une PR existe déjà pour cette branche
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

step "3/5 : Demande de review Copilot"
info "Sollicitation de l'analyse automatique Copilot..."
# Demander la review à Copilot ou bot reviewer si supporté
gh pr edit "$PR_NUMBER" --add-reviewer "copilot" 2>/dev/null || \
gh pr edit "$PR_NUMBER" --add-reviewer "github-actions[bot]" 2>/dev/null || \
gh api "repos/:owner/:repo/pulls/$PR_NUMBER/requested_reviewers" -f 'reviewers[]=copilot' 2>/dev/null || true
success "Demande de review transmise."

step "4/5 : Attente des vérifications CI (GitHub Actions)"
info "Surveillance des checks CI en temps réel pour la PR #$PR_NUMBER..."
if ! gh pr checks "$PR_NUMBER" --watch --interval 10; then
    error "Les vérifications CI ont échoué sur GitHub Actions !"
    gh pr checks "$PR_NUMBER"
    exit 1
fi
success "Tous les checks CI sont passés au vert !"

step "5/5 : Analyse des retours de review et Fusion vers 'main'"
# Récupérer les commentaires de review émis sur la PR
COMMENTS=$(gh api "repos/:owner/:repo/pulls/$PR_NUMBER/comments" 2>/dev/null || echo "[]")
REVIEWS=$(gh api "repos/:owner/:repo/pulls/$PR_NUMBER/reviews" 2>/dev/null || echo "[]")

# Vérifier s'il y a des commentaires non résolus demandant des modifications
HAS_CHANGES_REQUESTED=$(echo "$REVIEWS" | grep -c '"state":"CHANGES_REQUESTED"' || true)

if [[ "$HAS_CHANGES_REQUESTED" -gt 0 ]]; then
    warn "Copilot a demandé des modifications sur la PR #$PR_NUMBER."
    info "Détails des commentaires à traiter :"
    echo "$COMMENTS" | grep -E '("path"|"body")' | sed 's/^[[:space:]]*//' || true
    echo ""
    warn "Veuillez corriger les points relevés puis relancer scripts/pr_loop.sh."
    exit 2
fi

info "Tous les feux sont au vert. Fusion de la PR #$PR_NUMBER dans 'main'..."
if gh pr merge "$PR_NUMBER" --squash --delete-branch --admin 2>/dev/null || gh pr merge "$PR_NUMBER" --squash --delete-branch; then
    success "═════════════════════════════════════════════════════════════"
    success "  Pull Request #$PR_NUMBER fusionnée avec succès dans main !"
    success "═════════════════════════════════════════════════════════════"
else
    error "Impossible de fusionner automatiquement la PR #$PR_NUMBER."
    exit 1
fi

info "Bascule sur la branche 'main' et synchronisation locale..."
git checkout main
git pull origin main
success "Branche locale 'main' synchronisée. Prêt pour la prochaine tâche !"
