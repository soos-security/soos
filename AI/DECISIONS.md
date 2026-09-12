# Registre des Décisions Architecturales (ADR)

Ce fichier consigne les choix techniques actés pour éviter que l'IA ne change de stratégie (hallucination) d'une session à l'autre. Toutes ces décisions découlent de `ARCHITECTURE.md`.

## Décisions Actives
*   **IPC (Inter-Process Communication) :** Utilisation exclusive de Sockets Unix locaux (`SOCK_SEQPACKET` ou `SOCK_STREAM`) avec vérification stricte de `SO_PEERCRED`. Pas de requêtes réseau[cite: 2].
*   **Module PAM (pam_ztlh.so) :** Ne doit **jamais** démarrer un runtime asynchrone (Tokio). Utilisation exclusive de la librairie standard bloquante (`std::os::unix::net::UnixStream`) avec un timeout strict de 200ms[cite: 2].
*   **Gestion des Paniques :** Le module PAM intercepte toutes les paniques via `catch_unwind`. En cas d'erreur critique, il retourne systématiquement `PAM_IGNORE` pour retomber sur le mot de passe classique[cite: 2].
*   **Caméra et Matériel :** Le démon root est l'unique possesseur du périphérique `/dev/video0`. Utilisation de la crate `v4l` (et non `nokhwa`) pour exploiter les buffers MMAP[cite: 2].
*   **Intelligence Artificielle :** Utilisation de `ort` (ONNX Runtime) en mode CPU. Interdiction absolue d'importer OpenCV[cite: 2].