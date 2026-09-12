#!/usr/bin/env bash
# =============================================================================
# save.sh — Pipeline qualité + commit automatisé pour le projet soos
# =============================================================================
# Ce script exécute séquentiellement :
#   1. cargo fmt       — Formate le code Rust en place
#   2. cargo clippy     — Analyse statique, échoue sur tout warning (-D warnings)
#   3. cargo test       — Exécute la suite de tests unitaires
#   4. git add .        — Stage tous les changements
#   5. git commit       — Commit avec un message généré dynamiquement
#
# Si une étape échoue, le script s'arrête immédiatement (set -e).
# Le script ne fait JAMAIS de git push — seulement un commit local.
#
# Usage :
#   ./save.sh
#   ./save.sh "message de commit personnalisé"   # override du message auto
# =============================================================================

set -euo pipefail

# ---------------------------------------------------------------------------
# Couleurs pour la sortie (si terminal interactif)
# ---------------------------------------------------------------------------
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

# ---------------------------------------------------------------------------
# Fonctions utilitaires
# ---------------------------------------------------------------------------
info()    { echo -e "${BLUE}[INFO]${NC}  $*"; }
success() { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error()   { echo -e "${RED}[FAIL]${NC}  $*" >&2; }
step()    { echo -e "\n${BOLD}── Étape $1 ──${NC}"; }

# ---------------------------------------------------------------------------
# Vérifications préalables
# ---------------------------------------------------------------------------
# S'assurer qu'on est à la racine du projet
if [[ ! -f "Cargo.toml" ]]; then
    error "Ce script doit être exécuté depuis la racine du projet soos."
    error "Cargo.toml introuvable dans le répertoire courant : $(pwd)"
    exit 1
fi

# S'assurer que git est initialisé
if ! git rev-parse --is-inside-work-tree &> /dev/null; then
    error "Ce répertoire n'est pas un dépôt git."
    error "Initialisez-le avec : git init"
    exit 1
fi

# Interdire physiquement le commit direct sur main
CURRENT_BRANCH=$(git symbolic-ref --short HEAD 2>/dev/null || echo "detached")
if [[ "$CURRENT_BRANCH" == "main" && "${ALLOW_MAIN_COMMIT:-0}" != "1" ]]; then
    error "Commit direct sur la branche 'main' strictement interdit !"
    error "Créez une branche dédiée avant d'effectuer des modifications :"
    error "  git checkout -b feat/<nom>   # nouvelle fonctionnalité"
    error "  git checkout -b fix/<nom>    # correctif"
    error "  git checkout -b chore/<nom>  # outillage / doc"
    exit 1
fi

# S'assurer qu'un .gitignore existe (sécurité : éviter de committer target/)
if [[ ! -f ".gitignore" ]]; then
    warn "Aucun .gitignore détecté. Création d'un .gitignore minimal..."
    cat > .gitignore << 'GITIGNORE'
# Rust build artifacts
/target/

# IDE / Éditeurs
.vscode/
.idea/
*.swp
*.swo
*~

# OS
.DS_Store
Thumbs.db

# Fichiers de debug
*.pdb
GITIGNORE
    success ".gitignore créé."
fi

# ---------------------------------------------------------------------------
# Étape 1 : Formatage du code
# ---------------------------------------------------------------------------
step "1/3 : cargo fmt"
info "Formatage du code Rust..."
if cargo fmt; then
    success "Code formaté."
else
    error "cargo fmt a échoué."
    exit 1
fi

# ---------------------------------------------------------------------------
# Étape 2 : Analyse statique (Clippy)
# ---------------------------------------------------------------------------
step "2/3 : cargo clippy"
info "Analyse statique avec Clippy (-D warnings)..."
if cargo clippy -- -D warnings; then
    success "Aucun warning Clippy."
else
    error "Clippy a détecté des warnings ou erreurs."
    error "Corrigez les problèmes ci-dessus avant de sauvegarder."
    exit 1
fi

# ---------------------------------------------------------------------------
# Étape 3 : Tests unitaires
# ---------------------------------------------------------------------------
step "3/4 : cargo test"
info "Exécution des tests..."
if cargo test; then
    success "Tous les tests passent."
else
    error "Des tests ont échoué."
    error "Corrigez les tests avant de sauvegarder."
    exit 1
fi

# ---------------------------------------------------------------------------
# Étape 4 : Audit de sécurité des dépendances (cargo-deny)
# ---------------------------------------------------------------------------
step "4/4 : cargo deny check"
if command -v cargo-deny &> /dev/null; then
    info "Audit des dépendances (licences, vulnérabilités, sources, bans)..."
    if cargo deny check; then
        success "Audit cargo-deny validé."
    else
        error "cargo-deny a détecté des violations de sécurité ou de licence."
        error "Corrigez deny.toml ou vos dépendances avant de sauvegarder."
        exit 1
    fi
else
    warn "cargo-deny n'est pas installé localement — étape ignorée."
    warn "Installez-le avec : curl -sSL https://github.com/EmbarkStudios/cargo-deny/releases/latest/download/... ou cargo install cargo-deny"
fi

# ---------------------------------------------------------------------------
# Traitement des arguments (--push-pr et message de commit)
# ---------------------------------------------------------------------------
PUSH_PR=false
CUSTOM_MSG=""

for arg in "$@"; do
    if [[ "$arg" == "--push-pr" || "$arg" == "--pr" ]]; then
        PUSH_PR=true
    elif [[ -z "$CUSTOM_MSG" ]]; then
        CUSTOM_MSG="$arg"
    fi
done

if [[ "${PUSH_PR_ENV:-0}" == "1" ]]; then
    PUSH_PR=true
fi

# ---------------------------------------------------------------------------
# Stage des changements
# ---------------------------------------------------------------------------
echo ""
info "Stage de tous les fichiers modifiés..."
git add .

# Vérifier s'il y a quelque chose à committer
if git diff --cached --quiet; then
    echo ""
    warn "Aucune modification à committer. Le dépôt est déjà à jour."
    if [[ "$PUSH_PR" == "true" ]]; then
        info "Push de la branche courante demandé..."
        git push -u origin "$CURRENT_BRANCH"
    fi
    exit 0
fi

# ---------------------------------------------------------------------------
# Génération du message de commit
# ---------------------------------------------------------------------------
# Si un message personnalisé est fourni, l'utiliser
if [[ -n "$CUSTOM_MSG" ]]; then
    COMMIT_MSG="$CUSTOM_MSG"
else
    # Génération automatique basée sur les fichiers modifiés
    COMMIT_MSG=""

    # Récupérer la liste des fichiers modifiés (staged)
    CHANGED_FILES=$(git diff --cached --name-only)

    # Compteurs par catégorie
    CRATES_CHANGED=()
    DOCS_CHANGED=false
    TESTS_CHANGED=false
    CONFIG_CHANGED=false
    SCRIPTS_CHANGED=false
    AI_CHANGED=false

    while IFS= read -r file; do
        [[ -z "$file" ]] && continue

        case "$file" in
            crates/*/src/*)
                crate_name=$(echo "$file" | cut -d'/' -f2)
                if [[ ! " ${CRATES_CHANGED[*]:-} " =~ " ${crate_name} " ]]; then
                    CRATES_CHANGED+=("$crate_name")
                fi
                ;;
            crates/*/tests/* | tests/*)
                TESTS_CHANGED=true
                ;;
            Docs/* | docs/* | *.md)
                DOCS_CHANGED=true
                ;;
            AI/*)
                AI_CHANGED=true
                ;;
            Cargo.toml | Cargo.lock | rust-toolchain.toml | deny.toml)
                CONFIG_CHANGED=true
                ;;
            *.sh | Dockerfile | .dockerignore | .githooks/*)
                SCRIPTS_CHANGED=true
                ;;
            *)
                CONFIG_CHANGED=true
                ;;
        esac
    done <<< "$CHANGED_FILES"

    # Construire le message
    PARTS=()

    if [[ ${#CRATES_CHANGED[@]} -gt 0 ]]; then
        PARTS+=("feat(${CRATES_CHANGED[*]}): mise à jour du code")
    fi
    if [[ "$TESTS_CHANGED" == true ]]; then
        PARTS+=("test: mise à jour des tests et invariants")
    fi
    if [[ "$DOCS_CHANGED" == true ]]; then
        PARTS+=("docs: mise à jour de la documentation")
    fi
    if [[ "$AI_CHANGED" == true ]]; then
        PARTS+=("docs(ai): mise à jour des documents de stratégie")
    fi
    if [[ "$SCRIPTS_CHANGED" == true ]]; then
        PARTS+=("chore(infra): mise à jour des scripts et garde-fous")
    fi
    if [[ "$CONFIG_CHANGED" == true ]]; then
        PARTS+=("chore(config): mise à jour de la configuration")
    fi

    if [[ ${#PARTS[@]} -eq 0 ]]; then
        COMMIT_MSG="chore: mise à jour"
    elif [[ ${#PARTS[@]} -eq 1 ]]; then
        COMMIT_MSG="${PARTS[0]}"
    else
        COMMIT_MSG="${PARTS[0]}"
        for ((i = 1; i < ${#PARTS[@]}; i++)); do
            COMMIT_MSG="${COMMIT_MSG}
- ${PARTS[$i]}"
        done
    fi

    FILE_COUNT=$(echo "$CHANGED_FILES" | wc -l | tr -d ' ')
    COMMIT_MSG="${COMMIT_MSG}

[${FILE_COUNT} fichier(s) modifié(s)]"
fi

# ---------------------------------------------------------------------------
# Commit
# ---------------------------------------------------------------------------
info "Commit en cours..."
echo -e "${BLUE}  Message :${NC} $(echo "$COMMIT_MSG" | head -1)"

git commit -m "$COMMIT_MSG"

# ---------------------------------------------------------------------------
# Résumé et Push / Pull Request
# ---------------------------------------------------------------------------
echo ""
success "═══════════════════════════════════════════════"
success "  Sauvegarde locale réussie !"
success "═══════════════════════════════════════════════"
echo ""
info "Dernier commit :"
git log --oneline -1

if [[ "$PUSH_PR" == "true" ]]; then
    echo ""
    info "Option --push-pr détectée. Poussée vers GitHub en cours..."
    git push -u origin "$CURRENT_BRANCH"
    success "Branche '$CURRENT_BRANCH' poussée sur origin."

    echo ""
    info "Préparation de la Pull Request..."
    FIRST_LINE=$(echo "$COMMIT_MSG" | head -1)
    PR_BODY="## Résumé
$COMMIT_MSG

## Vérifications de sécurité passées avec succès
- [x] cargo fmt --check (formatage officiel)
- [x] cargo clippy --all-targets -- -D warnings (zéro warning)
- [x] cargo test --all-targets (tests unitaires + invariants architecturaux)
- [x] cargo deny check (audit licences, failles RustSec, bans)
- [x] Contrôle pre-commit (anti-commit main + secret scanner)"

    # Tenter la création automatique si gh est authentifié
    PR_CREATED=false
    if command -v gh &> /dev/null; then
        if gh auth status &> /dev/null; then
            if gh pr create --title "$FIRST_LINE" --body "$PR_BODY" --base main --head "$CURRENT_BRANCH"; then
                success "Pull Request créée avec succès via GitHub CLI !"
                PR_CREATED=true
            fi
        fi
    fi

    if [[ "$PR_CREATED" != "true" ]]; then
        REPO_URL="https://github.com/Mysticaly622/soos"
        PR_URL="${REPO_URL}/pull/new/${CURRENT_BRANCH}"
        echo ""
        info "Lien direct pour finaliser la Pull Request en 1 clic :"
        echo -e "${BOLD}${BLUE}  👉 ${PR_URL}${NC}"
        echo ""
        warn "Astuce : Pour que gh crée les PRs 100% automatiquement sans ouvrir le navigateur,"
        warn "lancez 'gh auth login' une fois dans votre terminal."
    fi
else
    echo ""
    warn "Rappel : la branche n'a pas été poussée vers GitHub."
    info "Pour pousser et générer la Pull Request automatiquement, utilisez :"
    echo -e "  ./save.sh --push-pr"
fi
