# soos — Document d’Architecture et de Contexte Maître

## 1. Décision d’architecture

### Objectif de la phase 1

soos (soos) ajoute une vérification faciale locale aux piles PAM de Linux, sans modifier l’interface du gestionnaire de session, de l’écran de verrouillage ou de `sudo`. Une reconnaissance réussie autorise l’authentification ; toute indisponibilité, incertitude ou anomalie **laisse PAM poursuivre le mot de passe existant**. Après l’échec de ce mot de passe, un événement anti-intrusion peut demander au démon de conserver une preuve photographique, sous une politique de confidentialité explicite.

Le système n’est pas un facteur biométrique à haute assurance tant qu’il ne comporte pas de détection robuste de présentation (PAD/liveness) et une évaluation de faux-accepts. Une photo ou un écran est une attaque de présentation documentée par NIST ; les recommandations NIST pour l’authentification faciale imposent un PAD et une mesure du taux de faux-match.[^nist-63b] La phase 1 doit donc être présentée comme **confort + détection locale**, avec mot de passe/repli obligatoire, et non comme substitut universel à une clé matérielle ou un secret.

### Choix retenus

| Domaine | Choix de référence | Pourquoi | À ne pas faire |
|---|---|---|---|
| Frontière PAM/démon | Unix domain socket (UDS) local, `SOCK_SEQPACKET` si disponible, sinon `SOCK_STREAM` à trames bornées | Pas de réseau, latence faible, identité noyau du pair | HTTP, TCP loopback, socket world-writable sans contrôle du pair |
| Exécution PAM | Code synchrone très court, `std::os::unix::net::UnixStream` et délais stricts | PAM est appelé dans le chemin critique et ne doit pas créer un runtime asynchrone durable | Inférence, accès caméra ou téléchargement dans le `.so` |
| Démon | Rust/Tokio, processus root durci, détenteur exclusif de V4L2 et des sessions ONNX | Caméra et modèles restent chauds ; arbitrage unique | Ouvrir `/dev/video0` depuis PAM à chaque tentative |
| Capture Linux | `v4l` 0.14, MMAP + thread dédié ; `nokhwa` seulement comme adaptateur/prototype | Contrôle V4L2 et buffers prévisible | Dépendre d’OpenCV ou garder plusieurs lecteurs de caméra |
| IA | `ort` 2.0.0-rc.13, CPU ONNX Runtime emballé/contrôlé ; UltraFace Slim 320 + MobileFaceNet ONNX validé | Performant et pratique en Rust, sans OpenCV dans l’application | Dire que c’est « pure Rust » : ONNX Runtime reste une dépendance native |
| Secret biométrique | Embeddings chiffrés au repos, images intrus séparées et opt-in | Réduit les données persistantes | Stocker des frames brutes ou des mots de passe |

Les versions ci-dessus sont celles vérifiées le 12 septembre 2026 : `pam-bindings` 0.3.0, `tokio` 1.53.1, `v4l` 0.14.0, `nokhwa` 0.10.11, `ort` 2.0.0-rc.13 et `zeroize` 1.9.0. Les versions doivent être figées dans `Cargo.lock`, soumises à audit et réévaluées avant publication.[^pam-bindings][^tokio][^v4l][^nokhwa][^ort][^zeroize]

## 2. Modèle de menace et invariants

### Actifs

- le droit d’authentifier un UID local ;
- le modèle biométrique (embedding) et la clé qui le chiffre ;
- les frames de caméra et les preuves d’intrusion ;
- l’intégrité du socket, du démon, des modèles ONNX et de la configuration PAM ;
- la disponibilité de l’écran de verrouillage et de `sudo`.

### Adversaires considérés

Un utilisateur local non autorisé, un processus sous son UID, un client UDS malveillant, un modèle ONNX remplacé, une webcam débranchée ou accaparée, et une attaque de présentation (photo, vidéo, écran, masque) sont dans le périmètre. Un attaquant root, un noyau compromis, une caméra matériellement remplacée ou une extraction mémoire physique ne sont pas résolus par cette phase.

### Invariants impératifs

1. Le module PAM ne retourne `PAM_SUCCESS` **que** après une réponse `Allow` fraîche, liée au même UID par les identifiants noyau du socket ; toute autre situation retourne `PAM_IGNORE`.
2. Aucun mot de passe ne transite vers soos, ne doit être lu par son module, ni ne doit apparaître dans un journal.
3. Le démon ne fait jamais confiance au nom d’utilisateur, PID, service PAM ou UID déclaré dans le message : il les recoupe avec `SO_PEERCRED`, `/etc/passwd` et, lorsque nécessaire, la session active `logind`.
4. Une décision `Allow` est mono-usage, liée à une requête aléatoire de 256 bits, à l’UID, au service et à une échéance courte ; elle n’est jamais mise en cache côté PAM.
5. Un timeout, un panic, un socket absent, une caméra indisponible, une ambiguïté de visage, un modèle invalide ou une erreur interne se dégrade en mot de passe, jamais en autorisation.

## 3. Architecture globale

```text
Application PAM (gdm, swaylock, hyprlock, sudo, login)
  │ pam_soos.so — chemin synchrone, budget 250 ms, aucune caméra
  │ connect + requête AuthAttempt (UID implicite par SO_PEERCRED)
  ▼
/run/soos/daemon.sock ── UDS local ── soos-daemon (root, Tokio)
                                      │
                         CameraManager (seul lecteur V4L2)
                                      │ dernière frame fraîche (RAM)
                                      ▼
                         VisionEngine : détecter → aligner → PAD → embedding → comparer
                                      │
                   Allow / Deny / Unavailable, signé contextuellement dans la réponse
                                      │
                         EvidenceStore (seulement après PasswordFailed, opt-in)
```

Le démon démarre avant toute authentification, charge et vérifie les modèles, ouvre la caméra et stabilise l’exposition. PAM ne contient que le client IPC, l’interprétation de la réponse et le code ABI C. Cette séparation garantit qu’une mise à jour d’IA, un redémarrage de caméra ou une fuite mémoire du traitement vidéo ne puissent pas bloquer les appels PAM plus longtemps que le budget fixé.

### États d’une requête

| État démon | Réponse PAM | Effet |
|---|---|---|
| visage unique, PAD acceptable, score ≥ seuil, contexte valide | `Allow` | `PAM_SUCCESS` ; le contrôle PAM court-circuite l’authentification mot de passe |
| pas de visage, plusieurs visages, score faible, PAD négatif | `Deny` | `PAM_IGNORE` ; PAM demande normalement le mot de passe |
| caméra/modèle/socket non prêt, délai dépassé, erreur | `Unavailable` | `PAM_IGNORE` ; même repli silencieux |
| requête malformée, UID incohérent, abuse/rate-limit | `ProtocolError` | `PAM_IGNORE`, log de sécurité côté démon |

`Deny` et `Unavailable` sont volontairement indiscernables pour l’application PAM et ne déclenchent ni prompt ni message révélateur. Le démon peut les distinguer pour l’observabilité, avec des journaux sans image ni embedding.

## 4. IPC PAM ↔ démon

### Socket, chemin et permissions

Créer le répertoire via systemd, pas par `mkdir` opportuniste dans le démon :

```ini
# /etc/systemd/system/soos-daemon.service.d/runtime.conf
[Service]
RuntimeDirectory=soos
RuntimeDirectoryMode=0750
UMask=0077
```

Le démon vérifie que `/run/soos` appartient à `root:soos`, n’est pas inscriptible par un tiers et n’est pas un lien symbolique ; il supprime uniquement son propre nœud socket après une vérification `lstat`, puis crée `/run/soos/daemon.sock` avec mode `0660`, propriétaire `root:soos`. Les comptes utilisant la biométrie sont ajoutés explicitement au groupe système `soos`. Ce choix donne aux processus de verrouillage exécutés sous l’UID de session l’accès nécessaire, tout en refusant les autres utilisateurs.

Les permissions de chemin ne suffisent pas : pour chaque connexion AF_UNIX le démon appelle `getsockopt(..., SO_PEERCRED)` et obtient les identifiants présents lors de `connect(2)`.[^unix7] En Rust, `nix::sys::socket::getsockopt` avec `sockopt::PeerCredentials` est l’adaptateur conseillé.[^nix] Il exige :

- `peer.uid == uid` de l’identité demandée (ou une règle documentée pour le processus PAM root) ;
- un UID local autorisé et, pour un déverrouillage graphique, propriétaire de la session locale active ;
- une taille de message, une version et un type de requête valides avant tout décodage coûteux ;
- au plus une requête active par UID, avec rate limit et limite globale de connexions.

Ne pas rendre le socket `0666`. Même avec `SO_PEERCRED`, cela autorise le déni de service et élargit inutilement la surface. Ne jamais utiliser une adresse UDS abstraite : elle n’a pas les permissions de système de fichiers. Ne pas faire confiance au `PAM_USER` fourni par le client ; `pam_exec` documente d’ailleurs que des éléments PAM exportés vers un enfant doivent être traités avec prudence car l’environnement peut être contrôlé par l’utilisateur.[^pam-exec]

### Protocole

Le protocole est binaire, versionné et borné. Utiliser `postcard` + `serde` ou une implémentation manuelle ; éviter JSON et toute allocation non bornée. Préférer `SOCK_SEQPACKET` sous Linux : les frontières de messages évitent les erreurs de framing. Prévoir un fallback `SOCK_STREAM` avec préfixe `u32` big-endian, taille maximale de 4096 octets, lecture exacte, et rejet avant désérialisation.

```text
Request v1:  version | kind=AUTH | request_id[32] | uid_hint:u32 |
             service_len:u8 | service[≤64] | deadline_monotonic_ns:u64
Response v1: version | request_id[32] | verdict:u8 | reason_class:u8 |
             issued_monotonic_ns:u64 | expires_monotonic_ns:u64
Event v1:    version | kind=PASSWORD_FAILED | request_id[32] |
             peer-derived UID only | service[≤64] | timestamp
```

Le `uid_hint` n’est qu’une assertion de cohérence ; l’UID faisant autorité est celui de `SO_PEERCRED`. Les `request_id` proviennent de `getrandom`, ne sont pas réutilisés, et les réponses ne sont valables que durant 1 à 2 secondes. Il n’est pas utile de signer cryptographiquement un message sur un UDS après authentification par noyau, mais un identifiant aléatoire empêche une confusion/rejeu logique entre demandes concurrentes.

### Tokio : où l’utiliser

Le démon utilise `tokio::net::UnixListener`/`UnixStream`, disponibles avec la fonctionnalité `net` ; `UnixListener::accept()` accepte les connexions et `UnixStream::connect()` exige un runtime I/O.[^tokio][^tokio-stream] Après `accept`, convertir temporairement le descripteur en `std::os::unix::net::UnixStream` ou employer un accès FD compatible pour lire `SO_PEERCRED`, puis le remettre dans Tokio. Les tâches de capture et d’inférence CPU bloquantes passent par une unique worker thread dédiée ou `spawn_blocking` avec sémaphore de capacité 1 ; ne pas encombrer les workers Tokio.

Dans le `.so`, **ne pas démarrer Tokio**. Employer le socket synchrone de `std`, `connect_timeout`/`set_read_timeout`/`set_write_timeout` et un budget total de 200–250 ms. Les API Tokio peuvent panic hors d’un runtime I/O, ce qui est inacceptable dans PAM.[^tokio-stream] Le client ferme immédiatement après une requête-réponse : pas de connexion persistante, pas de tâche détachée, pas de retry caché.

## 5. Module PAM Rust et pile universelle

### Crate et ABI

Employer `pam-bindings` 0.3.0 (nom de crate `pam`) pour `PamHandle`, `PamHooks` et le macro `pam_hooks!`, qui génère les points d’entrée C attendus par Linux-PAM.[^pam-bindings] Le crate exporte aussi des conteneurs de secrets à effacement automatique, mais soos ne doit pas traiter d’authentificateur mot de passe.

`crates/pam/Cargo.toml` produit `crate-type = ["cdylib"]`. Exporter `pam_sm_authenticate`, `pam_sm_setcred` et les hooks requis via `pam_hooks!`; garder le code FFI minuscule. Toute entrée FFI est enveloppée par `catch_unwind(AssertUnwindSafe(...))` **dans une couche externe** : une panique ne doit jamais franchir une frontière `extern "C"`. Sur panic, journaliser au mieux via syslog et retourner `PAM_IGNORE`; aucun `unwrap`, `expect`, allocation massive ou log de données sensibles dans ce chemin.

Pseudo-contrat :

```rust
// sm_authenticate, mode par défaut
match ipc_auth(uid, service, deadline) {
    Ok(Allow { request_id, .. }) => {
        pamh.set_data("soos.request-id", request_id)?; // seulement si nécessaire
        PAM_SUCCESS
    }
    Ok(Deny | Unavailable) | Err(_) => PAM_IGNORE,
}

// sm_authenticate, argument event=password-failed
// best effort, délai 20 ms ; ne consulte jamais un mot de passe
send_password_failed(peer_credentials, service);
PAM_IGNORE
```

Le `request_id` de la tentative faciale est optionnel dans l’événement : le démon peut associer un échec récent au même UID/service sous une courte fenêtre ; l’événement doit néanmoins fonctionner si la tentative faciale n’a pas eu lieu. Le notifier est best-effort : son échec ne modifie pas le résultat PAM.

### Contrôles PAM — règle fondamentale

PAM n’a pas de mécanisme permettant à un module placé avant `pam_unix` de connaître un échec de mot de passe à venir. Il faut donc invoquer le même `.so` une seconde fois **après** le module qui vérifie le mot de passe, sur le chemin d’échec. La syntaxe de contrôle étendue définit `success=done` comme succès qui termine la pile, et `default=ignore` comme résultat sans influence ; `done` ne passe pas outre un échec non ignoré antérieur.[^pam-conf]

#### Sous-pile soos proposée

Le paquet installe `/etc/pam.d/soos-auth` ; le mainteneur de distribution ou l’administrateur l’insère au bon endroit dans la pile existante. Ne pas écraser `system-auth` ou `common-auth` : ces fichiers sont souvent générés par `authselect`, `pam-auth-update` ou un outil équivalent.

```pam
# /etc/pam.d/soos-auth — exemple conceptuel à adapter à la pile appelante
# Doit être placé APRÈS tout preauth/lockout obligatoire, AVANT pam_unix.
auth  [success=done default=ignore]  pam_soos.so timeout_ms=250

# Vérification classique. Sur succès: terminaison. Sur échec: marque la pile
# comme échouée mais continue afin d'envoyer l'événement et de laisser les
# modules de lockout existants agir.
auth  [success=done default=bad]     pam_unix.so try_first_pass

# N'est atteint que si pam_unix n'a pas retourné success. Aucun secret lu.
auth  optional                       pam_soos.so event=password-failed timeout_ms=20

# Conserver ensuite les lignes propres à la distribution, notamment
# pam_faillock.so authfail lorsqu'il est configuré.
```

Cette sous-pile est un *patron*, pas une ligne à copier sans inspection. Si des modules comme `pam_sss`, `pam_krb5`, `pam_fprintd`, OTP ou smartcard sont des chemins d’authentification légitimes, les placer dans une sous-pile « authentification classique » et notifier l’échec seulement après l’échec final de **tous** ces facteurs. Sinon, un mot de passe incorrect suivi d’un OTP correct pourrait à tort devenir une alerte intrusion. De même, placer le visage après `pam_faillock preauth` garantit qu’un compte verrouillé ne soit pas déverrouillé par le visage.

#### Adaptation par famille de distribution

| Famille | Fichier généralement inclus | Intégration maintenable |
|---|---|---|
| Debian/Ubuntu | `/etc/pam.d/common-auth` (souvent administré par `pam-auth-update`) | créer un profil `pam-auth-update` ou ajuster l’intégration générée ; préserver `pam_unix`, `pam_faillock` et `pam_sss` existants |
| RHEL/Fedora | `/etc/pam.d/system-auth` et `password-auth` | créer/activer un profil `authselect` personnalisé ; ne pas éditer à la main un fichier marqué comme géré par authselect |
| Arch | `/etc/pam.d/system-auth`, explicitement inclus par les services | ajouter une sous-pile au bon point et conserver les fichiers `.pacnew` lors des mises à jour |
| openSUSE | pile gérée par YaST/pam-config | utiliser le mécanisme de configuration local/outil de la distribution, puis valider la pile résultante |

Les services ont des piles différentes : GDM, greetd, `login`, `sudo`, `polkit-1`, `swaylock` et Hyprlock doivent être recensés et testés. « Universel » signifie que le module respecte PAM ; cela ne signifie pas qu’une unique modification de fichier atteint tous les gestionnaires. `sudo` mérite une décision produit distincte : une caméra peut améliorer le confort, mais les politiques d’entreprise peuvent exiger mot de passe ou MFA.

Avant activation, conserver une console root ouverte, installer une règle de récupération permettant le mot de passe, exécuter les tests dans une VM, puis tester écran verrouillé, TTY, SSH, `sudo`, utilisateur non enrôlé, caméra absente et démon arrêté. Ne jamais déployer une pile PAM non testée à distance sans accès console.

## 6. Webcam chaude et faible latence

### Recommandation

Le démon est le seul détenteur du périphérique, sélectionné par identifiant stable `/dev/v4l/by-id/...`, pas par l’index fragile `/dev/video0`. `v4l` 0.14 est le choix de production : il fournit des bindings V4L2 sûrs, `MmapStream` et des buffers MMAP ; les pilotes de streaming supportent largement MMAP.[^v4l] Configurer une résolution négociée 640×480 ou 640×360, YUYV/NV12 si l’IA accepte la conversion, 10–15 FPS, 3–4 buffers MMAP.

`nokhwa` 0.10.11 est utile pour un prototype multi-plateforme ou une couche de découverte. Sur Linux son backend natif passe par V4L2 et exige une fonctionnalité `input-*`; sa documentation recommande `input-native`, et ses backends V4L peuvent échouer si la caméra est déjà occupée.[^nokhwa][^nokhwa-v4l] Pour un démon Linux root qui doit contrôler format, buffers et erreurs, garder `v4l` comme implémentation de référence ; offrir `nokhwa` derrière une feature `camera-nokhwa` seulement.

### Pipeline caméra

```text
thread CameraManager (bloquant) : dequeue MMAP → timestamp CLOCK_MONOTONIC
 → conversion/réduction → ArcSwap<LatestFrame> → requeue immédiat
                                              │
                                requête PAM lit un snapshot en RAM
                                (âge max 100–150 ms), jamais la caméra
```

- Ne conserver qu’une frame réduite nécessaire à l’IA (par exemple RGB 640×480) et son horodatage monotone, protégée par `ArcSwap` ou un double-buffer ; pas de file non bornée.
- Pour éviter la copie du buffer driver qui doit être requeue, copier une seule fois vers le buffer de dernière frame ; réutiliser les allocations (`Vec::with_capacity` préalloué). Une frame caméra n’est pas un secret persistant, mais l’effacer au remplacement est une mesure de confidentialité raisonnable si le coût est acceptable.
- La reconnaissance consomme la frame la plus récente seulement si elle a moins de 150 ms ; sinon répondre `Unavailable` ou attendre une seule nouvelle frame jusqu’au budget global.
- Maintenir le streaming garde l’auto-exposition stable. Après ouverture/reconnexion, jeter 15–30 frames ou attendre une variance d’exposition stable avant de marquer la caméra « ready ». Ne pas promettre la disparition de tout flash : certains capteurs et firmwares l’imposent.
- Baisser à 5 FPS après une fenêtre d’inactivité (p. ex. 60 s), mais ne fermer la caméra que si l’utilisateur l’a demandé. Une lecture à 10 FPS de 640×480 est généralement peu coûteuse ; mesurer sur le matériel cible, plutôt que supposer « zéro CPU ».
- Gérer `ENODEV`, `EIO`, `EBUSY` par état `Unavailable`, backoff borné et réouverture ; ne jamais bloquer le listener PAM. Offrir un mode « caméra partagée » documenté comme non garanti : nombre de webcams V4L2 refusent un deuxième ouvreur.

La réussite de la latence dépend davantage de la frame déjà disponible et des sessions ONNX préchauffées que d’un micro-choix de crate. Construire un banc de mesures séparant : âge frame, détection, alignement, PAD, embedding, comparaison et IPC (p50/p95/p99).

## 7. Vision locale, modèles et budget de 150 ms

### Réalité de « pure Rust »

Le code applicatif peut être Rust et sans OpenCV : décodage/conversion avec `image` ou SIMD Rust, tensores avec `ndarray`/slices, inférence par `ort`. Toutefois `ort` est un binding Rust pour ONNX Runtime ; la version vérifiée est une release candidate qui cible ONNX Runtime 1.28.[^ort] Le paquet doit donc fournir une bibliothèque ONNX Runtime exacte, vérifiée par hash, et tester ABI/architecture. Ce n’est pas une solution sans C/C++ au sens binaire.

Alternative : `tract` offre de l’inférence Rust native, mais doit être validé modèle par modèle pour les opérateurs, performance et précision. Ne pas le choisir seulement pour l’étiquette « pure Rust » ; l’objectif PAM favorise aujourd’hui la maturité/performance d’ORT. Isoler l’interface `InferenceBackend` permet une migration future.

### Modèles proposés

| Étape | Référence phase 1 | Entrée/sortie attendue | Décision |
|---|---|---|---|
| Détection | Ultra-Light-Fast-Generic-Face-Detector, Slim 320 ONNX sans post-traitement (~1,04 Mo) | image 320² → boîtes/scores ; NMS dans Rust | modèle minuscule, détection rapide ; vérifier licence et hash du fichier de distribution[^ultraface] |
| Landmarks/alignement | modèle 5 points ONNX, versionné et validé | crop → 5 repères | alignement affine 112×112, indispensable à la comparaison stable |
| Vérification | MobileFaceNet ArcFace-compatible, ONNX FP32/int8 validé | 112×112 RGB → embedding L2-normalisé 128D/512D selon modèle | MobileFaceNet a moins d’un million de paramètres et le papier rapporte 4 Mo/18 ms sur mobile, mais pas sur tout PC[^mobilefacenet] |
| PAD | challenge actif léger (clignement/tourner la tête via landmarks) en phase 1 ; modèle PAD dédié + validation en phase 1.5 | séquence courte | ne pas inférer la vivacité depuis l’embedding |

Ne mélanger jamais des embeddings issus de modèles, normalisations, tailles d’entrée ou conventions RGB/BGR différents. Chaque profil d’enrôlement stocke `model_id`, version, dimension, prétraitement et date ; une mise à niveau force un ré-enrôlement ou une migration explicitement évaluée. Les poids provenant d’un dépôt tiers sont une dépendance de chaîne d’approvisionnement : licence redistribuable, URL d’origine, SHA-256, signature si disponible et carte de modèle sont obligatoires dans le manifest.

### Pipeline

1. Choisir la dernière frame suffisamment fraîche.
2. Redimensionner/cadrer pour la détection ; seuil de confiance et NMS déterministes.
3. Exiger exactement un visage, taille minimale, netteté/exposition et orientation acceptables. Plusieurs visages = refus.
4. Produire landmarks, transformée affine vers 112×112 et normalisation exactement comme l’entraînement.
5. Exécuter le PAD ; échec ou incertitude = repli mot de passe.
6. Produire l’embedding, le normaliser L2, calculer la similarité cosinus avec les embeddings autorisés de l’UID.
7. Autoriser seulement si score ≥ seuil fixe calibré, PAD positif, contexte/UID valides et limites de tentative satisfaites. Effacer les buffers temporaires.

Pour des vecteurs L2-normalisés, `cosine = dot(a,b)`. Le seuil n’est pas une constante universelle : le calibrer sur des tests d’imposteurs et de vrais utilisateurs pour chaque couple modèle/prétraitement/caméra, puis le fixer. NIST recommande une évaluation d’au moins FMR 1/10 000 et FNMR <5 % lorsque l’authentification biométrique est utilisée à ce niveau ; ne pas annoncer cette conformité sans jeu d’essai et protocole équivalents.[^nist-63b]

### Budget cible (mesure p95, machine de référence à définir)

| Segment | Budget p95 |
|---|---:|
| IPC, contrôle et snapshot | 5 ms |
| conversion + détection | 35 ms |
| landmarks + alignement + contrôles qualité | 20 ms |
| PAD/challenge court | 35 ms |
| embedding + cosinus | 30 ms |
| marge ordonnanceur | 25 ms |
| **Total décision** | **≤ 150 ms** |

Un challenge actif peut dépasser 150 ms parce qu’il requiert plusieurs frames ; l’UI ne doit jamais l’attendre indéfiniment. Deux politiques défendables : (a) phase 1, repli mot de passe si le challenge ne tient pas le budget ; (b) autoriser le lockscreen à attendre jusqu’à 1 s avec une configuration explicitement opt-in. La première est recommandée pour préserver PAM et l’expérience. NIST souligne qu’aucun algorithme testé ne détectait tous les types d’attaques de présentation.[^nist-blog] Le PAD doit être testé contre photos imprimées, écrans, vidéos, masques et éclairages réels ; il ne doit pas être simulé par un simple seuil de qualité.

### Sessions ORT

Créer l’environnement ORT une fois au démarrage, construire une `Session` par modèle, faire 5–10 inférences de warm-up et conserver sessions/tensors préalloués. Limiter explicitement les threads intra/inter-op afin qu’un démon root ne monopolise pas le poste ; choisir CPU en phase 1. Les EP GPU (CUDA, OpenVINO, etc.) sont optionnels et changent la matrice de déploiement. `ort` prend en charge des execution providers, mais l’accélération n’est pas un prérequis de compatibilité.[^ort]

## 8. Monorepo Cargo proposé

```text
zero-trust-linux-hello/
├── Cargo.toml                    # workspace resolver="2", versions/patches centralisés
├── Cargo.lock                    # commité et audité
├── rust-toolchain.toml
├── deny.toml                     # cargo-deny licences/advisories/sources
├── crates/
│   ├── protocol/                 # types bornés, codec v1, tests de fuzz
│   ├── policy/                   # décision, rate limit, états, aucune I/O
│   ├── pam/                      # cdylib pam_soos.so : IPC synchrone seulement
│   ├── daemon/                   # binaire root, Tokio, supervision des composants
│   ├── camera-v4l/               # V4L2, buffers, dernière frame, simulation caméra
│   ├── vision/                   # prétraitement, traits, alignement, comparateur
│   ├── inference-ort/            # implémentation ORT isolée derrière trait
│   ├── biometric-store/          # embeddings chiffrés, migration de schéma
│   ├── evidence-store/           # images intrusion, rétention/chiffrement/permissions
│   ├── enrollment-cli/           # commande root/local admin séparée
│   └── admin-cli/                # état, diagnostic sans données biométriques
├── models/
│   ├── manifest.toml             # id, licence, source, SHA-256, I/O, preprocessing
│   └── README.md                 # procédure de validation, aucun poids non vérifié
├── packaging/
│   ├── systemd/                  # service, tmpfiles, hardening
│   ├── pam/                      # soos-auth + snippets par famille distro
│   ├── udev/                     # groupe/droit périphérique si requis
│   └── deb/ rpm/ arch/
├── tests/
│   ├── integration-pam/          # conteneur/VM, pamtester, pas machine hôte
│   ├── protocol-fuzz/
│   ├── camera-fixtures/
│   └── vision-golden/
├── docs/
│   ├── threat-model.md
│   ├── distro-matrix.md
│   └── privacy-retention.md
└── xtask/                         # build/release reproductible, vérification manifest
```

`protocol` ne connaît ni PAM ni Tokio ; `policy` est déterministe et testable ; `vision` ne connaît ni socket ni fichier. Le démon dépend des adaptateurs ; le `.so` dépend seulement de `protocol`, d’un client IPC minuscule et de `pam-bindings`. Cette dépendance unidirectionnelle est ce qui permet une API réseau de phase 2 sans contaminer PAM : la future crate `transport-api` pourra traduire HTTPS/mTLS vers les mêmes commandes de domaine, mais elle ne sera pas liée au processus PAM.

Activer par défaut `#![forbid(unsafe_code)]` dans les crates métier. Les adaptateurs FFI/V4L2, qui ont parfois besoin d’`unsafe` indirect, doivent être isolés, minimaux, commentés par invariant de sûreté et revus séparément. Exécuter `cargo fmt --check`, Clippy avec `-D warnings`, `cargo audit`, `cargo deny check`, tests, fuzz du décodeur de protocole et génération SBOM en CI.

## 9. Persistance, vie privée et anti-intrusion

### Embeddings et enrôlement

L’enrôlement est une commande administrative explicite après authentification forte. Capturer plusieurs images live, exiger liveness, vérifier qualité/diversité, produire des embeddings puis supprimer les frames brutes. Stocker seulement 3–5 embeddings normalisés, chiffrés par UID dans `/var/lib/soos/biometrics/<uid>.cbor.enc`, permissions `0700` sur répertoire et `0600`, propriétaire `root:root`. La clé de chiffrement doit venir d’un mécanisme OS (TPM2 idéalement, sinon secret root protégé) ; documenter clairement la dégradation sans TPM.

Un embedding est une donnée biométrique sensible, pas un hash irréversible. Ne pas le journaliser, l’exporter, ni l’utiliser à des fins de surveillance. Prévoir suppression, ré-enrôlement, rétention minimale et consentement. Sur machines professionnelles ou pays applicables, demander un examen juridique et DPO avant collecte : ce document est une architecture, pas un avis juridique.

### Preuves après échec final

L’événement `PasswordFailed` n’est émis qu’après l’échec final de la pile classique. Le démon fait alors une capture courte et désynchronisée de la réponse PAM : garder une frame de ±200 ms, éventuellement 2–3 frames, les chiffrer puis les écrire de façon atomique dans `/var/lib/soos/evidence/YYYY-MM-DD/<uuid>.webp.enc`. Répertoire `0700`, fichiers `0600`, `root:root`, clé distincte de celle des embeddings, durée de rétention courte (par défaut 7 jours) et rotation contrôlée.

Ne jamais envoyer les photos par réseau en phase 1, ni les enregistrer pour chaque erreur de caméra ou rejet facial. Imposer un maximum par UID/jour et par système pour éviter une amplification de données et de disque. Les métadonnées (UID, service, timestamp monotone/réel, raison) sont minimales ; ne pas inclure un mot de passe, le score facial complet, une image en clair ni des variables PAM. L’activation de cette fonction doit être opt-in, visible et conforme aux règles locales.

## 10. Durcissement du démon et opérations

### Service systemd

Partir d’un service root, puis réduire ses capacités selon le test matériel. Les directives suivantes sont un point de départ, à valider car la caméra, l’ONNX Runtime et les pilotes peuvent nécessiter des assouplissements :

```ini
[Service]
User=root
Group=root
ExecStart=/usr/libexec/soos/soos-daemon
Restart=on-failure
RestartSec=2
UMask=0077
NoNewPrivileges=yes
PrivateTmp=yes
ProtectHome=yes
ProtectSystem=strict
ReadWritePaths=/var/lib/soos /run/soos
DevicePolicy=closed
DeviceAllow=/dev/video* rw
RestrictAddressFamilies=AF_UNIX
LockPersonality=yes
MemoryDenyWriteExecute=yes
RestrictSUIDSGID=yes
SystemCallArchitectures=native
```

Tester `systemd-analyze security` et les fonctionnalités de capture/récupération après chaque restriction. `RestrictAddressFamilies=AF_UNIX` exprime l’absence volontaire de réseau phase 1. Ne pas activer aveuglément `PrivateDevices=yes` : il peut masquer la webcam ; tester une liste `DeviceAllow` précise. Une fois le socket créé, le démon peut abandonner les privilèges inutiles, mais la plupart des webcams exigent un accès approprié : privilégier les permissions `udev`/groupe à un daemon root intégral si le modèle d’exécution le permet.

### Rust et mémoire

- `zeroize` 1.9.0 fournit `Zeroizing` et des effacements qui ne sont pas éliminés par optimisation.[^zeroize] Employer-le pour clés, jetons, buffers de mot de passe si un composant les reçoit accidentellement (idéalement aucun), et embeddings temporaires sensibles.
- Ses garanties ne retirent pas les copies antérieures, registres, swap, core dumps ou attaques microarchitecturales ; réserver les buffers à la bonne capacité, désactiver les core dumps du service et envisager `mlock` seulement après analyse des limites/DoS. La documentation le précise explicitement.[^zeroize]
- Ne dériver ni `Debug`, ni `Serialize`, ni `Clone` pour types qui portent clés, images, embeddings ou jetons. Redacter erreurs et tracing.
- Compiler en release avec débordements vérifiés lorsque possible, LTO évalué, symboles de debug séparés, FORTIFY/linker hardening de la distribution. Signer les paquets, produire SBOM/provenance et vérifier les hashes modèles au démarrage.
- Éviter `panic=abort` pour le `.so` si cela peut terminer le processus appelant ; le garde anti-panic reste nécessaire. Tester échecs d’allocation, daemon absent et descripteurs épuisés.

### Disponibilité et observabilité

Le health check distingue `socket_ready`, `camera_ready`, `models_verified`, `frame_age_ms` et `last_inference_ms`. Il ne divulgue pas de nom d’utilisateur, score, frame ou embedding. Exposer l’état uniquement à root via CLI ou socket d’administration séparé `0600`; pas d’API réseau en phase 1. Appliquer une alarme locale quand le démon est durablement indisponible, mais ne pas générer un bruit par tentative PAM.

## 11. Plan de mise en œuvre et critères d’acceptation

1. **Fondation** — monorepo, protocole v1, démon faux, module PAM qui retourne toujours `PAM_IGNORE`; tests ABI sur distributions cibles.
2. **IPC durci** — socket systemd, `SO_PEERCRED`, codec borné, timeouts, fuzz et tests UID/services falsifiés.
3. **Caméra** — V4L2 chaude, frame fraîche atomique, unplug/replug, budget CPU mesuré.
4. **Vision** — modèles hashés, golden tests prétraitement/embedding, seuil calibré, aucun `Allow` sans visage unique/qualité.
5. **PAM** — VM de chaque famille, piles réelles GDM/TTY/sudo, face succès, face échec, caméra absente, mot de passe succès/échec, `pam_faillock` et méthodes SSSD/OTP selon configuration.
6. **Evidence et enrôlement** — opt-in, permissions, chiffrement, rétention/suppression et tests disque plein.
7. **PAD et publication** — tests de présentation, limites documentées, revue de sécurité externe avant qualifier le système de facteur d’authentification.

Critères bloquants avant une release alpha : aucun chemin ne transforme une erreur en succès ; aucune caméra n’est ouverte par PAM ; daemon indisponible = mot de passe fonctionnel ; le `.so` ne fait pas panic ; chaque modèle est attesté par manifest/hash ; tous fichiers biométriques et preuves sont hors du répertoire utilisateur et non lisibles par des comptes non root ; chaque intégration distribution est testée dans une VM avec procédure de rollback.

## 12. Pièges fréquents à éviter

- Modifier directement `/etc/pam.d/system-auth` sur une distribution qui l’annonce géré par `authselect`, puis perdre l’intégration à la mise à jour.
- Placer le visage avant le pré-contrôle `pam_faillock`, ce qui contourne un verrouillage de compte.
- Utiliser `sufficient` sans examiner les échecs antérieurs : le contrôle exact de PAM et l’ordre de pile déterminent la sécurité.[^pam-conf]
- Définir « mot de passe faux » comme seulement `pam_unix` faux dans une pile contenant SSSD, OTP ou smartcard.
- Passer des images ou embeddings dans l’IPC PAM, dans les logs ou dans un socket accessible à tous.
- Ouvrir `/dev/video0` en supposant que son index reste stable ou qu’il peut être partagé.
- Mesurer seulement une moyenne sur un PC de développement ; exiger p95/p99 sur matériel cible après warm-up et sous charge.
- Déployer un poids ONNX téléchargé à la volée, sans licence, hash, prétraitement exact ni évaluation de biais/faux-match.
- Promettre une authentification sécurisée contre une photo sans PAD. Les évaluations NIST montrent que le PAD logiciel a des compromis et ne couvre pas toutes les présentations.[^nist-pad]

## Références

[^pam-bindings]: `pam-bindings`, [documentation de la crate `pam` 0.3.0](https://docs.rs/pam-bindings/latest/pam/), consultée le 12 septembre 2026.
[^pam-conf]: Linux-PAM, [pam.conf(5)](https://man7.org/linux/man-pages/man5/pam.conf.5.html), syntaxe de contrôle et sémantique de `done`/`ignore`, consultée le 12 septembre 2026.
[^pam-exec]: Linux-PAM, [pam_exec(8)](https://www.man7.org/linux/man-pages/man8/pam_exec.8.html), environnement et avertissement de contrôle utilisateur, consultée le 12 septembre 2026.
[^unix7]: Linux man-pages, [unix(7)](https://man7.org/linux/man-pages/man7/unix.7.html), `SO_PEERCRED`, consultée le 12 septembre 2026.
[^nix]: `nix`, [API socket et `PeerCredentials`](https://docs.rs/nix/latest/nix/sys/socket/), version 0.31.3, consultée le 12 septembre 2026.
[^tokio]: Tokio, [UnixListener](https://docs.rs/tokio/latest/tokio/net/struct.UnixListener.html), version 1.53.1, consultée le 12 septembre 2026.
[^tokio-stream]: Tokio, [UnixStream](https://docs.rs/tokio/latest/tokio/net/struct.UnixStream.html), version 1.53.1, consultée le 12 septembre 2026.
[^v4l]: `v4l`, [documentation 0.14.0](https://docs.rs/v4l/latest/v4l/), MMAP et capture V4L2, consultée le 12 septembre 2026.
[^nokhwa]: `nokhwa`, [README/documentation 0.10.11](https://docs.rs/crate/nokhwa/latest/source/README.md), fonctionnalités `input-native`, consultée le 12 septembre 2026.
[^nokhwa-v4l]: `nokhwa`, [V4LCaptureDevice](https://docs.rs/nokhwa/latest/nokhwa/backends/capture/struct.V4LCaptureDevice.html), comportement quand caméra occupée, consultée le 12 septembre 2026.
[^ort]: `ort`, [documentation 2.0.0-rc.13](https://docs.rs/ort/latest/ort/), binding Rust ONNX Runtime et sessions, consultée le 12 septembre 2026.
[^zeroize]: RustCrypto, [zeroize 1.9.0](https://docs.rs/zeroize/latest/zeroize/), garanties et limites de l’effacement mémoire, consultée le 12 septembre 2026.
[^ultraface]: Linzaer, [Ultra-Light-Fast-Generic-Face-Detector — modèle Slim 320 ONNX](https://github.com/Linzaer/Ultra-Light-Fast-Generic-Face-Detector-1MB/blob/master/models/onnx/version-slim-320_without_postprocessing.onnx), taille indiquée 1,04 Mo ; licence et hash à vérifier dans le paquet, consulté le 12 septembre 2026.
[^mobilefacenet]: Chen, Liu, Gao, Han, [*MobileFaceNets: Efficient CNNs for Accurate Real-Time Face Verification on Mobile Devices*](https://arxiv.org/abs/1804.07573), 2018.
[^nist-63b]: NIST, [SP 800-63B Digital Identity Guidelines](https://pages.nist.gov/800-63-4/sp800-63b.html), biométrie, FMR et PAD facial, consultée le 12 septembre 2026.
[^nist-blog]: NIST, [Facing the Facts to Keep Our Biometrics Secure](https://www.nist.gov/blogs/taking-measure/facing-facts-keep-our-biometrics-secure), 2 octobre 2024.
[^nist-pad]: NIST, [IR 8491 — Face Analysis Technology Evaluation, Part 10](https://nvlpubs.nist.gov/nistpubs/ir/2023/NIST.IR.8491.pdf), septembre 2023.
