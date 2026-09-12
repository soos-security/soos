# Registre des Décisions Architecturales (ADR)

Ce fichier consigne les choix techniques actés pour éviter que l'IA ne change de stratégie (hallucination) d'une session à l'autre. Toutes ces décisions découlent de `ARCHITECTURE.md`.

## Décisions Actives

* **[2026-09-12] IPC (Inter-Process Communication) :** Utilisation exclusive de Sockets Unix locaux (`SOCK_SEQPACKET` ou `SOCK_STREAM`) avec vérification stricte de `SO_PEERCRED`. Pas de requêtes réseau.
* **[2026-09-12] Module PAM (pam_soos.so) :** Ne doit **jamais** démarrer un runtime asynchrone (Tokio). Utilisation exclusive de la librairie standard bloquante (`std::os::unix::net::UnixStream`) avec un timeout strict de 200-250ms.
* **[2026-09-12] Gestion des Paniques :** Le module PAM intercepte toutes les paniques via `catch_unwind`. En cas d'erreur critique, il retourne systématiquement `PAM_IGNORE` pour retomber sur le mot de passe classique.
* **[2026-09-12] Caméra et Matériel :** Le démon root est l'unique possesseur du périphérique `/dev/video*`. Utilisation de la crate `v4l` (et non `nokhwa`) pour exploiter les buffers MMAP.
* **[2026-09-12] Intelligence Artificielle :** Utilisation de `ort` (ONNX Runtime) en mode CPU. Interdiction absolue d'importer OpenCV.
* **[2026-09-13] Nommage :** Le projet s'appelle `soos`. Le module PAM est `pam_soos.so`, le démon est `soos-daemon`. Les anciennes références à "ZTLH" ou "pam_ztlh" sont obsolètes.
* **[2026-09-13] Codec :** Utiliser `postcard` + `serde` pour la sérialisation du protocole IPC. Pas de JSON. Taille maximale 4096 octets.