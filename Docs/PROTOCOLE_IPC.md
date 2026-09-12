# Spécification du Protocole IPC v1

> Crate : `crates/protocol` (`soos-protocol`)  
> Statut : Implémenté et testé  

---

## 1. Vue d'ensemble

Le protocole IPC assure la communication bidirectionnelle entre le module non privilégié `pam_soos.so` (exécuté dans le contexte du processus appelant PAM : `sudo`, `login`, `gdm`, etc.) et le démon privilégié `soos-daemon` (exécuté en tant que `root`).

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

Chaque message échangé sur le socket Unix Stream est préfixé d'un en-tête fixe de 8 octets :

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|       Magic (0x534F4F53 = "SOOS")     |    Version (1)        |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                       Payload Length (u32)                    |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                                                               |
|                   Payload sérialisé (Postcard)                |
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

### Constantes de sécurité
- **Magic** : `0x534F4F53` (ASCII `"SOOS"`). Tout message ayant un magic différent est rejeté immédiatement.
- **Version actuelle** : `1`.
- **Taille maximale de message (`MAX_MESSAGE_SIZE`)** : `65 536` octets (64 Ko). Évite toute attaque par déni de service / épuisement mémoire via de fausses longueurs de payload.
- **Taille maximale du nom de service PAM (`MAX_SERVICE_LEN`)** : `64` octets.

---

## 3. Types de données

### `Request`
Envoyée par le module PAM au démon root pour solliciter une authentification faciale :
- `version: u8` : Version du protocole (doit valoir 1).
- `request_id: [u8; 32]` : Identifiant cryptographique unique (256 bits) généré aléatoirement par le module.
- `target_uid: u32` : UID cible à authentifier.
- `service: String` : Nom du service PAM appelant (`"sudo"`, `"su"`, `"gdm-password"`...). Tronqué ou rejeté si > 64 octets.
- `timeout_ms: u32` : Délai alloué au démon (borné entre 50ms et 500ms, défaut 250ms).

### `Response`
Envoyée par le démon au module PAM :
- `version: u8` : Version du protocole.
- `request_id: [u8; 32]` : Doit correspondre exactement au `request_id` de la requête initiale.
- `verdict: Verdict` :
  - `Allow` : Visage authentifié avec succès, score > seuil.
  - `Deny` : Visage non reconnu, vivant non confirmé, ou tentative suspecte.
  - `ProtocolError` : Erreur de communication ou de trame.
  - `Unavailable` : Matériel indisponible, démon surchargé, timeout dépassé.
- `reason_class: ReasonClass` : Catégorie d'erreur pour audit interne (jamais exposée à l'utilisateur non root).
- `expiry_timestamp_ns: u64` : Timestamp de validité courte.

### Effacement mémoire (`Zeroize`)
La structure `Response` implémente le trait `Zeroize` de manière à ce que les identifiants de requête et données sensibles soient effacés à la destruction (`drop`).

---

## 4. Invariants stricts

1. **Aucun secret transitant sur le socket** : Ni mot de passe PAM, ni embedding biométrique, ni image caméra ne transite sur le socket IPC.
2. **Rejet silencieux sans blocage** : Tout verdict différent de `Allow` (`Deny`, `ProtocolError`, `Unavailable`) ou toute rupture de communication entraîne un retour immédiat `PAM_IGNORE` côté PAM, laissant la main aux modules suivants (`pam_unix.so`).
3. **Usage unique** : Une réponse n'est valide que pour un seul `request_id` et ne peut être rejouée.
