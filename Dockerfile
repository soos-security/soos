# =============================================================================
# Dockerfile — Bac à sable PAM isolé pour le projet soos
# =============================================================================
# Ce conteneur fournit un environnement Ubuntu complet avec :
#   - La toolchain Rust (rustup, stable)
#   - Les dépendances de compilation pour les modules PAM
#   - L'outil pamtester pour simuler des appels PAM
#   - Un utilisateur factice (testuser) pour les tests d'authentification
#
# Usage :
#   docker build -t soos-sandbox .
#   docker run --rm -v "$(pwd)":/workspace soos-sandbox bash
#
# IMPORTANT : Ce Dockerfile ne copie JAMAIS le code source.
#             Le code est monté en bind-mount au runtime pour garantir
#             l'isolation entre l'image et le système hôte.
# =============================================================================

FROM ubuntu:24.04

# ---------------------------------------------------------------------------
# Variables d'environnement
# ---------------------------------------------------------------------------
# Empêche les prompts interactifs pendant apt-get install
ENV DEBIAN_FRONTEND=noninteractive
# Répertoires Rust : installés au niveau système pour être accessibles à root
ENV RUSTUP_HOME=/usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo
ENV PATH="/usr/local/cargo/bin:${PATH}"

# ---------------------------------------------------------------------------
# Dépendances système
# ---------------------------------------------------------------------------
# build-essential  : gcc, make, etc. — requis par cargo pour compiler les crates natives
# pkg-config       : résolution des chemins de bibliothèques (.pc files)
# libpam0g-dev     : headers PAM (pam_appl.h, pam_modules.h) — requis pour pam-bindings
# libclang-dev     : requis par bindgen (utilisé par pam-bindings pour générer les FFI)
# pamtester        : outil CLI pour tester les modules PAM sans session réelle
# curl             : téléchargement de rustup
# git              : potentiellement requis par certaines dépendances Cargo (git deps)
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential \
        pkg-config \
        libpam0g-dev \
        libclang-dev \
        pamtester \
        curl \
        ca-certificates \
        git \
    && rm -rf /var/lib/apt/lists/*

# ---------------------------------------------------------------------------
# Installation de Rust via rustup
# ---------------------------------------------------------------------------
# -y                    : mode non-interactif
# --default-toolchain   : installe stable directement
# --profile minimal     : n'installe que rustc, cargo, rust-std (pas de docs/clippy/rustfmt)
#                         clippy et rustfmt sont ajoutés explicitement ensuite
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain stable --profile minimal \
    && rustup component add clippy rustfmt \
    && echo "Rust $(rustc --version) installé"

# ---------------------------------------------------------------------------
# Utilisateur factice pour les tests PAM
# ---------------------------------------------------------------------------
# testuser : utilisateur non-root avec un mot de passe connu.
# Ce mot de passe est volontairement trivial — il n'est utilisé que dans un
# conteneur éphémère isolé, jamais sur un système réel.
RUN useradd -m -s /bin/bash testuser \
    && echo "testuser:password123" | chpasswd

# ---------------------------------------------------------------------------
# Configuration PAM de test
# ---------------------------------------------------------------------------
# Service "test-soos" : pile PAM minimaliste pour valider le chargement ABI
# du module pam_soos.so.
#
# Comportement attendu en Phase 1 (fondation) :
#   1. pam_soos.so se charge, ne trouve pas de socket démon → retourne PAM_IGNORE
#   2. Le contrôle [success=done default=ignore] fait que PAM_IGNORE est ignoré
#   3. pam_unix.so prend le relais et vérifie le mot de passe normalement
#
# Cela valide l'invariant 5 de ARCHITECTURE.md :
#   "Un socket absent se dégrade en mot de passe, jamais en autorisation."
RUN echo "# Service PAM de test pour soos\n\
# pam_soos.so : chargé en premier, retourne PAM_IGNORE si pas de démon\n\
auth  [success=done default=ignore]  pam_soos.so timeout_ms=250\n\
# pam_unix.so : vérification classique du mot de passe\n\
auth  required                       pam_unix.so\n\
\n\
# Compte et session minimaux\n\
account required pam_unix.so\n\
session required pam_unix.so" > /etc/pam.d/test-soos

# ---------------------------------------------------------------------------
# Répertoire de travail
# ---------------------------------------------------------------------------
WORKDIR /workspace

# Le conteneur est prévu pour être lancé avec un bind-mount :
#   docker run --rm -v "$(pwd)":/workspace soos-sandbox <commande>
CMD ["bash"]
