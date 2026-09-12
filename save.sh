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
step "3/3 : cargo test"
info "Exécution des tests..."
if cargo test; then
    success "Tous les tests passent."
else
    error "Des tests ont échoué."
    error "Corrigez les tests avant de sauvegarder."
    exit 1
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
    exit 0
fi

# ---------------------------------------------------------------------------
# Génération du message de commit
# ---------------------------------------------------------------------------
# Si un message personnalisé est fourni en argument, l'utiliser directement
if [[ $# -ge 1 ]]; then
    COMMIT_MSG="$1"
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
                # Extraire le nom du crate
                crate_name=$(echo "$file" | cut -d'/' -f2)
                # Ajouter si pas déjà présent
                if [[ ! " ${CRATES_CHANGED[*]:-} " =~ " ${crate_name} " ]]; then
                    CRATES_CHANGED+=("$crate_name")
                fi
                ;;
            crates/*/tests/* | tests/*)
                TESTS_CHANGED=true
                ;;
            docs/* | *.md)
                DOCS_CHANGED=true
                ;;
            AI/*)
                AI_CHANGED=true
                ;;
            Cargo.toml | Cargo.lock | rust-toolchain.toml | deny.toml)
                CONFIG_CHANGED=true
                ;;
            *.sh | Dockerfile | .dockerignore)
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
        PARTS+=("test: mise à jour des tests")
    fi
    if [[ "$DOCS_CHANGED" == true ]]; then
        PARTS+=("docs: mise à jour de la documentation")
    fi
    if [[ "$AI_CHANGED" == true ]]; then
        PARTS+=("docs(ai): mise à jour des documents de stratégie")
    fi
    if [[ "$SCRIPTS_CHANGED" == true ]]; then
        PARTS+=("chore(infra): mise à jour des scripts/conteneur")
    fi
    if [[ "$CONFIG_CHANGED" == true ]]; then
        PARTS+=("chore(config): mise à jour de la configuration")
    fi

    # Assembler le message final
    if [[ ${#PARTS[@]} -eq 0 ]]; then
        COMMIT_MSG="chore: mise à jour"
    elif [[ ${#PARTS[@]} -eq 1 ]]; then
        COMMIT_MSG="${PARTS[0]}"
    else
        # Plusieurs catégories : utiliser la première comme titre,
        # les autres comme corps
        COMMIT_MSG="${PARTS[0]}"
        for ((i = 1; i < ${#PARTS[@]}; i++)); do
            COMMIT_MSG="${COMMIT_MSG}
- ${PARTS[$i]}"
        done
    fi

    # Ajouter le nombre de fichiers modifiés
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
# Résumé
# ---------------------------------------------------------------------------
echo ""
success "═══════════════════════════════════════════════"
success "  Sauvegarde réussie !"
success "═══════════════════════════════════════════════"
echo ""
info "Dernier commit :"
git log --oneline -1
echo ""
warn "Rappel : ce script ne fait PAS de 'git push'."
warn "Poussez manuellement quand vous êtes prêt : git push"
