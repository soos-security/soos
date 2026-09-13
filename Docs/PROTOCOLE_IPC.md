# Spécification du Protocole IPC v1

> Crate : `crates/protocol` (`soos-protocol`)  
> Statut : Implémenté et testé  

---

## 1. Vue d'ensemble

Le protocole IPC assure la communication bidirectionnelle synchrone entre le module non privilégié `pam_soos.so` (exécuté dans le processus appelant PAM : `sudo`, `login`, `gdm`, etc.) et le démon privilégié `soos-daemon` (exécuté en tant que `root`).

```
┌─────────────────────────┐                     ┌─────────────────────────┐
│       pam_soos.so       │  Request (postcard) │       soos-daemon       │
│  (contexte non root)    │ ──────────────────> │      (processus root)   │
│                         │ <────────────────── │                         │
│  timeout dur : 250ms    │  Response (postcard)│  Caméra V4L2 + IA ort   │
└─────────────────────────┘                     └─────────────────────────┘
```

---

## 2. Format de trame sur le câble (Codec v1)

Chaque message échangé sur le socket Unix Stream est encadré par un préfixe de longueur en Big-Endian sur 4 octets (`u32`), suivi du payload sérialisé au format binaire compact **Postcard** :

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                  Payload Length (u32, Big-Endian)             |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                                                               |
|                  Payload sérialisé (Postcard)                 |
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

### Constantes de sécurité
- **Taille maximale de message (`MAX_MESSAGE_SIZE`)** : `4 096` octets (4 Ko). Tout message déclarant une taille supérieure à 4 Ko est rejeté immédiatement avant allocation mémoire (`CodecError::MessageTooLarge`), protégeant le démon et PAM contre les dénis de service.
- **Taille maximale du nom de service PAM (`MAX_SERVICE_LEN`)** : `64` octets. Rejeté avec `CodecError::PayloadCorrupted` si dépassé.
- **Version actuelle du protocole (`PROTOCOL_VERSION`)** : `1`.

---

## 3. Types de messages

### `Request`
Envoyée par le module PAM au démon root pour solliciter une authentification faciale :
- `kind: RequestKind` : Type de requête (`Authenticate`).
- `request_id: RequestId` : Identifiant cryptographique unique (256 bits, `[u8; 32]`) généré via `getrandom`.
- `uid_hint: u32` : UID déclaré par le client PAM (systématiquement confronté côté démon aux identifiants réels `SO_PEERCRED`).
- `service: String` : Nom du service PAM appelant (`"sudo"`, `"su"`, `"gdm-password"`...). Borné à 64 octets.
- `deadline_monotonic_ns: u64` : Horodatage monotone absolu en nanosecondes. Si le démon dépasse cette échéance, il rend immédiatement `Verdict::Unavailable`.

### `Response`
Rendue par le démon root au module PAM :
- `request_id: RequestId` : Doit correspondre bit-à-bit au `request_id` de la requête initiale.
- `verdict: Verdict` :
  - `Allow` : Visage unique authentifié avec succès (PAD validé, score ≥ seuil).
  - `Deny` : Échec de vérification (pas de visage, plusieurs visages, score insuffisant, PAD négatif).
  - `Unavailable` : Matériel indisponible, modèle non prêt, échéance dépassée.
  - `ProtocolError` : Requête malformée, UID incohérent, rate-limit atteint.
- `reason_class: ReasonClass` : Diagnostic pour observabilité interne du démon (ne doit pas être exploité par PAM pour modifier son comportement).
- `issued_monotonic_ns: u64` : Horodatage de génération de la réponse.
- `expires_monotonic_ns: u64` : Horodatage d'expiration courte (non réutilisable).

### `Event`
Notification asynchrone envoyée par PAM après échec d'authentification classique (ex: mot de passe erroné) :
- `version: u8` : Version du protocole (doit valoir 1).
- `kind: EventKind` : `PasswordFailed`.
- `request_id: Option<RequestId>` : Requête associée le cas échéant.
- `service: String` : Nom du service PAM.
- `timestamp_monotonic_ns: u64` : Timestamp monotone.

### Effacement mémoire (`Zeroize`)
La structure `Response` implémente `Zeroize` : ses identifiants de requête et métadonnées sont écrasés en mémoire lors de la destruction (`drop`).

---

## 4. Invariants de sécurité stricts

1. **Aucun secret transitant sur le socket** : Ni mot de passe PAM, ni embedding biométrique, ni image caméra ne transite sur le socket IPC.
2. **Rejet silencieux sans blocage** : Tout verdict différent de `Allow` (`Deny`, `ProtocolError`, `Unavailable`) ou toute rupture de communication entraîne un retour immédiat `PAM_IGNORE` côté PAM, laissant la main aux modules suivants (`pam_unix.so`).
3. **Usage unique** : Une réponse n'est valide que pour son `request_id` propre et expire immédiatement.
