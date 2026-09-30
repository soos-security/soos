# =============================================================================
# Dockerfile — Isolated PAM Sandbox for soos
# =============================================================================
# This container provides a complete Ubuntu environment with:
#   - Rust toolchain (rustup, 1.98.1 pinned by rust-toolchain.toml)
#   - Compilation dependencies for Linux-PAM modules
#   - python3 for the mock daemon; the PAM host is tests/docker/pam_test_runner.c,
#     compiled in-container by tests/docker/test_suite.sh
#   - Dummy test user (testuser) for authentication testing
#
# Usage:
#   docker build -t soos-sandbox .
#   docker run --rm -v "$(pwd)":/workspace soos-sandbox bash
#
# IMPORTANT: This Dockerfile NEVER copies source code into the image.
#             Code is mounted via bind-mount at runtime to guarantee
#             complete isolation between the image and host system.
# =============================================================================

FROM ubuntu:24.04

# ---------------------------------------------------------------------------
# Environment Variables
# ---------------------------------------------------------------------------
# Prevent interactive prompts during apt-get install
ENV DEBIAN_FRONTEND=noninteractive
# Rust directories: system-level paths accessible to root
ENV RUSTUP_HOME=/usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo
ENV PATH="/usr/local/cargo/bin:${PATH}"

# ---------------------------------------------------------------------------
# System Dependencies
# ---------------------------------------------------------------------------
# build-essential  : gcc, make, etc. — required by cargo to compile native crates
# pkg-config       : library search path resolution (.pc files)
# libpam0g-dev     : PAM headers (pam_appl.h, pam_modules.h) — required for pam bindings
# libclang-dev     : required by bindgen (used by pam-bindings to generate FFI)
# curl             : rustup installer download
# git              : potentially required by Cargo git dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential \
        pkg-config \
        libpam0g-dev \
        libclang-dev \
        python3 \
        curl \
        ca-certificates \
        git \
    && rm -rf /var/lib/apt/lists/*

# ---------------------------------------------------------------------------
# Rust Installation via rustup
# ---------------------------------------------------------------------------
# -y                    : non-interactive mode
# --default-toolchain   : install the pinned release of rust-toolchain.toml (1.98.1)
# --profile minimal     : install rustc, cargo, rust-std
#                         clippy and rustfmt are added explicitly afterwards
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain 1.98.1 --profile minimal \
    && rustup component add clippy rustfmt \
    && echo "Rust $(rustc --version) installed"

# ---------------------------------------------------------------------------
# Dummy User for PAM Testing
# ---------------------------------------------------------------------------
# testuser: non-root user with known password.
# This password is intentionally trivial — used only in an ephemeral
# isolated container sandbox, never on a real host system.
RUN useradd -m -s /bin/bash testuser \
    && echo "testuser:password123" | chpasswd

# ---------------------------------------------------------------------------
# Test PAM Service Configuration
# ---------------------------------------------------------------------------
# Service "test-soos": minimal PAM stack to validate ABI loading
# of pam_soos.so.
#
# Expected behavior:
#   1. pam_soos.so loads, detects no daemon socket -> returns PAM_IGNORE
#   2. Control flag [success=done default=ignore] ignores PAM_IGNORE
#   3. pam_unix.so takes over and verifies password normally
#
# Validates Invariant 5 of ARCHITECTURE.md:
#   "An absent socket degrades to password, never to authorization."
# printf '%s\n' writes one argument per line on every /bin/sh (dash on Ubuntu,
# bash on Fedora/Arch). Never use `echo "...\n..."`: bash's builtin echo writes a
# literal backslash-n and PAM would see a single comment line (GitHub #162).
RUN printf '%s\n' \
        '# Test PAM service for soos' \
        '# pam_soos.so: loaded first, returns PAM_IGNORE if daemon is absent' \
        'auth  [success=done default=ignore]  pam_soos.so timeout_ms=250' \
        '# pam_unix.so: standard password verification fallback' \
        'auth  required                       pam_unix.so' \
        '# Minimal account and session management' \
        'account required pam_unix.so' \
        'session required pam_unix.so' \
    > /etc/pam.d/test-soos

# Configure standard Debian/Ubuntu common-auth integration
RUN printf '%s\n' \
        '# /etc/pam.d/common-auth integration for soos' \
        'auth  [success=done default=ignore]  pam_soos.so timeout_ms=250' \
        'auth  [success=1 default=ignore]    pam_unix.so nullok' \
        'auth  requisite                      pam_deny.so' \
        'auth  required                       pam_permit.so' \
    > /etc/pam.d/common-auth

# ---------------------------------------------------------------------------
# Biometric & Model Storage Directory
# ---------------------------------------------------------------------------
RUN mkdir -p /var/lib/soos/models && chmod 755 /var/lib/soos/models

# ---------------------------------------------------------------------------
# Working Directory
# ---------------------------------------------------------------------------
WORKDIR /workspace

# Container is designed to run with a runtime bind-mount:
#   docker run --rm -v "$(pwd)":/workspace soos-sandbox <command>
CMD ["bash"]
