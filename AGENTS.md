# Règles Projet — soos (Biométrie PAM Linux)

## Identité du projet
- **Nom** : soos (anciennement "Zero-Trust Linux Hello" / "ZTLH")
- **Objectif** : Module PAM de vérification faciale locale pour Linux
- **Langage** : Rust, monorepo Cargo workspace

## Documents de référence OBLIGATOIRES
Avant toute implémentation, l'agent DOIT lire ces fichiers :
1. `AI/ARCHITECTURE.md` — Architecture maître, modèle de menace, invariants, budget de latence
2. `AI/DECISIONS.md` — Registre des décisions actées (anti-hallucination)
3. `AI/MOCK_STRATEGY.md` — Stratégie de simulation sans matériel
4. `AI/VERIFICATION_MATRIX.md` — Critères d'acceptation par composant

## Workflow Multi-Agents (TDD Strict)
Pour chaque fonctionnalité, suivre ces 4 étapes dans l'ordre :

### Étape 1 — Agent Architecte
- Lister les structs, enums, traits nécessaires
- Vérifier la cohérence avec `AI/ARCHITECTURE.md`
- Identifier les invariants de sécurité impactés

### Étape 2 — Agent Testeur
- Écrire les tests unitaires AVANT le code métier (TDD)
- Les tests DOIVENT échouer initialement (red phase)
- Tout composant PAM doit avoir un test garantissant `PAM_IGNORE` en cas d'erreur

### Étape 3 — Agent Auditeur
- Traquer les paniques (`unwrap`, `expect`) dans le chemin PAM
- Vérifier l'absence de fuites mémoire potentielles
- Valider le respect des invariants de `ARCHITECTURE.md`
- Vérifier que `#![forbid(unsafe_code)]` est actif dans les crates métier

### Étape 4 — Agent Développeur
- Fournir l'implémentation qui passe les tests
- Respecter toutes les contraintes identifiées par les étapes précédentes

## Décisions Architecturales Actées (NE PAS MODIFIER)
- **IPC** : Unix Domain Socket uniquement (`SOCK_SEQPACKET` / `SOCK_STREAM`)
- **Module PAM** : JAMAIS de runtime async (Tokio). Uniquement `std::os::unix::net::UnixStream`, timeout 200-250ms
- **Paniques PAM** : `catch_unwind` obligatoire, retour `PAM_IGNORE` systématique
- **Caméra** : Le démon root est l'unique détenteur de `/dev/video*`. Crate `v4l`, pas `nokhwa` en production
- **IA** : `ort` (ONNX Runtime) CPU. Interdiction absolue d'OpenCV
- **Nommage** : Le module s'appelle `pam_soos.so`, le démon `soos-daemon`, le projet `soos`

## Interdictions strictes
- ❌ Ne JAMAIS utiliser OpenCV
- ❌ Ne JAMAIS démarrer Tokio dans le `.so` PAM
- ❌ Ne JAMAIS utiliser `unwrap()` ou `expect()` dans le chemin critique PAM
- ❌ Ne JAMAIS stocker de mot de passe ou le transmettre via le socket
- ❌ Ne JAMAIS rendre le socket world-writable (`0666`)
- ❌ Ne JAMAIS logger des frames, embeddings ou données biométriques
- ❌ Ne JAMAIS transformer une erreur en `PAM_SUCCESS`

## Structure du monorepo
```
soos/
├── Cargo.toml              # workspace resolver="2"
├── crates/
│   ├── protocol/           # types bornés, codec v1, fuzz
│   ├── policy/             # décision, rate limit, aucune I/O
│   ├── pam/                # cdylib pam_soos.so
│   ├── daemon/             # binaire root, Tokio
│   ├── camera-v4l/         # V4L2, mock-camera feature
│   ├── vision/             # prétraitement, alignement
│   ├── inference-ort/      # ONNX Runtime isolé
│   ├── biometric-store/    # embeddings chiffrés
│   ├── evidence-store/     # images intrusion opt-in
│   ├── enrollment-cli/     # commande d'enrôlement
│   └── admin-cli/          # diagnostic
├── models/                 # manifest.toml + SHA-256
├── tests/                  # intégration, fuzz, fixtures
└── AI/                     # docs stratégie IA
```

## Stratégie de Branches et Pull Requests (OBLIGATOIRE)
- **Jamais de commit direct sur `main`** : Toute modification ou étape importante se fait sur une branche dédiée.
- **Nommage des branches** :
  - `feat/<nom>` : nouvelle fonctionnalité (ex: `feat/ipc-client`, `feat/policy-rate-limit`)
  - `fix/<nom>` : correction de bug ou vulnérabilité (ex: `fix/pam-timeout`)
  - `test/<nom>` : ajout de tests ou fixtures (ex: `test/docker-pam-matrix`)
  - `chore/<nom>` : CI, outillage, dépendances (ex: `chore/ci-deny-rules`)
- **Workflow de branche** :
  1. Avant de coder : créer et basculer sur la branche (`git checkout -b <type>/<nom>`). Un hook pre-commit bloque physiquement tout commit direct sur `main`.
  2. Suivre le workflow TDD (Architecte → Testeur → Auditeur → Développeur).
  3. Valider les invariants de sécurité automatiquement via `cargo test` (crate `tests/invariants`).
  4. Mettre à jour la documentation dans `Docs/` et le compte-rendu dans `AI/walkthroughs/`.
  5. Exécuter `./save.sh --push-pr` : valide le pipeline 4/4 (`fmt`, `clippy`, `test`, `deny`), committe, pousse la branche sur GitHub et génère la Pull Request vers `main`.

## Conventions de code Rust
- `#![forbid(unsafe_code)]` dans les crates métier (protocol, policy, vision)
- `unsafe` uniquement dans les crates d'adaptation (pam, camera-v4l), isolé et commenté
- `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test` obligatoires avant commit
- Utiliser `./save.sh` pour les commits (pipeline qualité automatisé)
- Utiliser `./run_tests.sh` pour les tests d'intégration PAM (conteneur Docker)

## Documentation et Walkthroughs (OBLIGATOIRE)
- **Walkthroughs systématiques dans `AI/walkthroughs/`** :
  - À chaque modification, tâche ou étape importante, créer un fichier de compte-rendu dans `AI/walkthroughs/` numéroté séquentiellement : `AI/walkthroughs/NN_<nom_etape>.md` (ex: `01_...`, `02_...`, `03_...`).
  - Le walkthrough résume : le contexte, les fichiers livrés/modifiés, les choix techniques, les résultats des tests et vérifications, et la prochaine étape.
- **Documentation technique systématique dans `Docs/`** :
  - Pour chaque composant créé, protocole défini, workflow ou configuration notable, créer ou mettre à jour un document technique dans `Docs/` (ex: `Docs/PROTOCOL.md`, `Docs/CI_CD_SECURITY.md`, `Docs/DEVELOPMENT.md`).
  - Cette documentation s'adresse aux développeurs humains et administrateurs du projet et doit rester synchronisée avec l'état réel du code.

## Gestion des erreurs de compilation
Si l'utilisateur fournit une sortie `cargo check`, analyser silencieusement et fournir le code corrigé sans explications verbeuses.
