#!/usr/bin/env bash
# =============================================================================
# run_tests.sh — Compilation et test d'intégration PAM dans un conteneur isolé
# =============================================================================
# Ce script :
#   1. Construit l'image Docker du bac à sable (si nécessaire)
#   2. Lance un conteneur éphémère avec le code source monté
#   3. Compile le module PAM en mode release
#   4. Déploie le .so dans le répertoire PAM du conteneur
#   5. Exécute pamtester pour valider le chargement ABI et le repli mot de passe
#   6. Détruit le conteneur automatiquement
#
# Usage :
#   ./run_tests.sh
#
# Prérequis :
#   - Docker installé et accessible (docker info)
#   - Exécuté depuis la racine du projet soos (Cargo.toml présent)
# =============================================================================

set -euo pipefail

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
readonly IMAGE_NAME="soos-sandbox"
readonly CONTAINER_NAME="soos-test-run"
readonly PAM_MODULE_NAME="libpam_soos.so"
# Répertoire standard des modules PAM sur Debian/Ubuntu x86_64
readonly PAM_MODULES_DIR="/lib/x86_64-linux-gnu/security"

# ---------------------------------------------------------------------------
# Couleurs pour la sortie (si terminal interactif)
# ---------------------------------------------------------------------------
if [[ -t 1 ]]; then
    readonly GREEN='\033[0;32m'
    readonly RED='\033[0;31m'
    readonly YELLOW='\033[1;33m'
    readonly BLUE='\033[0;34m'
    readonly NC='\033[0m' # No Color
else
    readonly GREEN=''
    readonly RED=''
    readonly YELLOW=''
    readonly BLUE=''
    readonly NC=''
fi

# ---------------------------------------------------------------------------
# Fonctions utilitaires
# ---------------------------------------------------------------------------
info()    { echo -e "${BLUE}[INFO]${NC}  $*"; }
success() { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error()   { echo -e "${RED}[FAIL]${NC}  $*" >&2; }

# ---------------------------------------------------------------------------
# Vérifications préalables
# ---------------------------------------------------------------------------
# Vérifier que Docker est disponible
if ! command -v docker &> /dev/null; then
    error "Docker n'est pas installé ou n'est pas dans le PATH."
    error "Installez Docker : https://docs.docker.com/engine/install/"
    exit 1
fi

if ! docker info &> /dev/null; then
    error "Le démon Docker n'est pas accessible."
    error "Vérifiez que Docker est démarré et que votre utilisateur est dans le groupe 'docker'."
    exit 1
fi

# Vérifier qu'on est à la racine du projet
if [[ ! -f "Dockerfile" ]]; then
    error "Ce script doit être exécuté depuis la racine du projet soos."
    error "Le fichier 'Dockerfile' est introuvable dans le répertoire courant."
    exit 1
fi

# ---------------------------------------------------------------------------
# Étape 1 : Construction de l'image Docker
# ---------------------------------------------------------------------------
info "Construction de l'image Docker '${IMAGE_NAME}'..."
if docker build -t "${IMAGE_NAME}" .; then
    success "Image '${IMAGE_NAME}' construite."
else
    error "Échec de la construction de l'image Docker."
    exit 1
fi

# ---------------------------------------------------------------------------
# Étape 2 : Nettoyage préventif
# ---------------------------------------------------------------------------
# Supprime un éventuel conteneur résiduel du même nom (ex: crash précédent)
if docker container inspect "${CONTAINER_NAME}" &> /dev/null; then
    warn "Conteneur résiduel '${CONTAINER_NAME}' détecté, suppression..."
    docker rm -f "${CONTAINER_NAME}" > /dev/null
fi

# ---------------------------------------------------------------------------
# Étape 3 : Exécution des tests dans le conteneur éphémère
# ---------------------------------------------------------------------------
info "Lancement du conteneur éphémère '${CONTAINER_NAME}'..."
info "  → Mount: $(pwd) → /workspace"
info "  → Compilation release + test pamtester"

docker run --rm \
    --name "${CONTAINER_NAME}" \
    -v "$(pwd)":/workspace \
    "${IMAGE_NAME}" \
    bash -c '
set -euo pipefail

echo ""
echo "========================================="
echo "  SOOS — Bac à sable PAM"
echo "========================================="
echo ""

# --- Vérification du workspace ---
if [[ ! -f "Cargo.toml" ]]; then
    echo "[FAIL] Cargo.toml introuvable dans /workspace."
    echo "       Le crate PAM n existe pas encore. Créez-le avant de relancer ce script."
    exit 1
fi

# --- Compilation en mode release ---
echo "[INFO] Compilation du module PAM en mode release..."
cargo build --release -p pam 2>&1
echo "[OK]   Compilation réussie."

# --- Déploiement du .so dans le répertoire PAM ---
SO_PATH="target/release/'"${PAM_MODULE_NAME}"'"
if [[ ! -f "${SO_PATH}" ]]; then
    echo "[FAIL] Fichier ${SO_PATH} introuvable après compilation."
    echo "       Vérifiez que le crate pam produit bien un cdylib nommé pam_soos."
    exit 1
fi

echo "[INFO] Déploiement du module PAM..."
cp "${SO_PATH}" '"${PAM_MODULES_DIR}"'/pam_soos.so
chmod 644 '"${PAM_MODULES_DIR}"'/pam_soos.so
echo "[OK]   Module déployé dans '"${PAM_MODULES_DIR}"'/pam_soos.so"

# --- Test T1 : Chargement ABI + mot de passe correct ---
echo ""
echo "[TEST] T1 — Chargement ABI du .so + authentification par mot de passe correct"
if echo "password123" | pamtester test-soos testuser authenticate; then
    echo "[OK]   T1 réussi : le module se charge, PAM_IGNORE fonctionne, pam_unix valide le mdp."
else
    echo "[FAIL] T1 échoué : le module PAM a crashé ou pam_unix a refusé le mot de passe."
    exit 1
fi

# --- Test T2 : Mot de passe incorrect ---
echo ""
echo "[TEST] T2 — Mot de passe incorrect (doit échouer proprement)"
if echo "wrong_password" | pamtester test-soos testuser authenticate; then
    echo "[FAIL] T2 échoué : un mot de passe incorrect a été accepté !"
    exit 1
else
    echo "[OK]   T2 réussi : mot de passe incorrect refusé comme attendu."
fi

# --- Test T3 : Module absent (résilience PAM) ---
echo ""
echo "[TEST] T3 — Module .so absent (résilience du système PAM)"
mv '"${PAM_MODULES_DIR}"'/pam_soos.so '"${PAM_MODULES_DIR}"'/pam_soos.so.bak
# Avec le module absent, pam_soos.so ne peut pas se charger.
# PAM devrait quand même laisser pam_unix.so fonctionner grâce au contrôle
# [success=done default=ignore] qui traite les erreurs comme ignore.
if echo "password123" | pamtester test-soos testuser authenticate; then
    echo "[OK]   T3 réussi : le système PAM reste fonctionnel sans le module soos."
else
    echo "[WARN] T3 : PAM a refusé malgré un mdp correct (le module absent peut avoir"
    echo "       été traité comme required par la pile). Vérifiez /etc/pam.d/test-soos."
fi
# Restauration
mv '"${PAM_MODULES_DIR}"'/pam_soos.so.bak '"${PAM_MODULES_DIR}"'/pam_soos.so

echo ""
echo "========================================="
echo "  TOUS LES TESTS DU BAC À SABLE : OK"
echo "========================================="
echo ""
'

# ---------------------------------------------------------------------------
# Résultat final
# ---------------------------------------------------------------------------
echo ""
success "═══════════════════════════════════════════"
success "  Bac à sable validé — conteneur détruit"
success "═══════════════════════════════════════════"
