# Matrice de Vérification — Critères d'acceptation par composant

Ce document traduit les critères bloquants du §11 de `ARCHITECTURE.md` en checklist actionnable pour chaque composant. Un composant ne peut être considéré comme **terminé** que lorsque TOUS ses critères sont validés.

---

## Invariants globaux (bloquants pour toute release)

- [ ] Aucun chemin ne transforme une erreur en `PAM_SUCCESS`
- [ ] Aucune caméra n'est ouverte par le module PAM
- [ ] Daemon indisponible = mot de passe fonctionnel
- [ ] Le `.so` ne fait jamais panic (protégé par `catch_unwind`)
- [ ] Chaque modèle ONNX est attesté par manifest + SHA-256
- [ ] Tous fichiers biométriques et preuves sont hors du $HOME et non lisibles par des comptes non-root
- [ ] Chaque intégration distribution est testée dans une VM avec procédure de rollback

---

## Composant : `protocol`

| # | Critère | Test | Statut |
|---|---|---|---|
| P1 | Les types `Request`, `Response`, `Verdict` sont sérialisables/désérialisables | Test round-trip | ☐ |
| P2 | Le codec respecte la taille maximale de 4096 octets | Test de rejet sur message trop grand | ☐ |
| P3 | Le `request_id` est de 256 bits (32 octets) | Test de sérialisation | ☐ |
| P4 | Le protocole est versionné (champ `version`) | Test v1 | ☐ |
| P5 | Fuzz du décodeur : aucun panic sur entrée arbitraire | `cargo fuzz` ou proptest | ☐ |
| P6 | `#![forbid(unsafe_code)]` actif | Vérification compilation | ☐ |

## Composant : `policy`

| # | Critère | Test | Statut |
|---|---|---|---|
| PO1 | Décision `Allow` uniquement si score ≥ seuil ET PAD positif ET UID valide | Tests unitaires paramétriques | ☐ |
| PO2 | Rate limit par UID fonctionne | Test avec rafale de requêtes | ☐ |
| PO3 | Aucune I/O dans le crate | Vérification des dépendances Cargo | ☐ |
| PO4 | `#![forbid(unsafe_code)]` actif | Vérification compilation | ☐ |

## Composant : `pam` (cdylib)

| # | Critère | Test | Statut |
|---|---|---|---|
| PA1 | Retourne `PAM_IGNORE` quand le démon est indisponible | Test pamtester sans démon | ☐ |
| PA2 | Retourne `PAM_IGNORE` sur timeout (>250ms) | Test avec démon lent simulé | ☐ |
| PA3 | `catch_unwind` protège toutes les entrées FFI | Revue de code | ☐ |
| PA4 | Ne démarre JAMAIS Tokio | Grep dans le code source | ☐ |
| PA5 | Aucun `unwrap()` ou `expect()` dans le chemin critique | `cargo clippy` + revue | ☐ |
| PA6 | Ne lit et ne transmet aucun mot de passe | Revue de code | ☐ |
| PA7 | ABI C correcte (chargeable par PAM) | Test pamtester T1 dans Docker | ☐ |
| PA8 | Module absent = système PAM toujours fonctionnel | Test pamtester T3 dans Docker | ☐ |

## Composant : `daemon`

| # | Critère | Test | Statut |
|---|---|---|---|
| D1 | Socket créé dans `/run/soos/` avec mode `0660` | Test d'intégration | ☐ |
| D2 | `SO_PEERCRED` vérifié sur chaque connexion | Test avec UID falsifié | ☐ |
| D3 | Démarre avec `RestrictAddressFamilies=AF_UNIX` | Test systemd | ☐ |
| D4 | Health check expose `socket_ready`, `camera_ready`, `models_verified` | Test d'intégration | ☐ |
| D5 | Aucune information sensible dans les logs | Revue des points de log | ☐ |

## Composant : `camera-v4l`

| # | Critère | Test | Statut |
|---|---|---|---|
| C1 | Feature `mock-camera` fournit un `MockCameraManager` fonctionnel | Test unitaire | ☐ |
| C2 | Frame fraîche disponible en <5ms via `ArcSwap` | Benchmark | ☐ |
| C3 | Gère `ENODEV`, `EIO`, `EBUSY` sans panic | Tests d'erreur simulés | ☐ |
| C4 | Sélection par `/dev/v4l/by-id/` pas par index | Test de configuration | ☐ |
| C5 | Jette 15-30 frames après ouverture (stabilisation exposition) | Test fonctionnel | ☐ |

## Composant : `vision`

| # | Critère | Test | Statut |
|---|---|---|---|
| V1 | Golden tests : prétraitement identique à celui de l'entraînement | Tests avec fixtures | ☐ |
| V2 | Embedding L2-normalisé (norme ≈ 1.0) | Test mathématique | ☐ |
| V3 | Similarité cosinus correcte | Test avec vecteurs connus | ☐ |
| V4 | Refuse si 0 ou >1 visage détecté | Tests unitaires | ☐ |
| V5 | Pipeline complet < 150ms p95 sur machine de référence | Benchmark | ☐ |
| V6 | `#![forbid(unsafe_code)]` actif | Vérification compilation | ☐ |

## Composant : `biometric-store`

| # | Critère | Test | Statut |
|---|---|---|---|
| B1 | Embeddings chiffrés au repos | Test écriture/lecture | ☐ |
| B2 | Fichiers sous `/var/lib/soos/biometrics/<uid>`, mode `0600`, propriétaire `root:root` | Test de permissions | ☐ |
| B3 | `model_id` et version stockés avec chaque profil | Test de migration | ☐ |
| B4 | Suppression et ré-enrôlement fonctionnels | Tests CRUD | ☐ |

## Composant : `evidence-store`

| # | Critère | Test | Statut |
|---|---|---|---|
| E1 | Activable uniquement en opt-in | Test de configuration | ☐ |
| E2 | Rotation après 7 jours par défaut | Test de rétention | ☐ |
| E3 | Maximum par UID/jour respecté | Test de limite | ☐ |
| E4 | Fichiers chiffrés, mode `0600`, `root:root` | Test de permissions | ☐ |
| E5 | JAMAIS envoyé par réseau en phase 1 | Audit des dépendances | ☐ |
