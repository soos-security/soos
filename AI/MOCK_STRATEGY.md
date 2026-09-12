# Stratégie de Simulation (Environnement de Développement)

L'accès à `/dev/video0` (Webcam) n'est pas toujours disponible (conteneur Docker de test, CI, machine sans webcam). Pour garantir la progression du projet sans friction matérielle, l'IA doit implémenter un système de *Mocking*.

## 1. Feature Flag Cargo
La crate liée à la caméra (`camera-v4l`) devra intégrer un flag de compilation conditionnelle dans son `Cargo.toml` :
`[features] mock-camera = []`

## 2. Le "Dummy Driver"
Si le flag `mock-camera` est actif, le démon ne tentera pas de se lier à `v4l`. À la place, il instanciera une structure `MockCameraManager` qui :
- Générera des images fixes (tableaux de bits 640x480) simulant une capture vidéo.
- Fournira une fausse horloge monotonique pour simuler l'âge de la frame (requis pour les tests de latence < 150ms).

## 3. Fixtures pour la Vision
Pour tester le pipeline d'intelligence artificielle sans caméra, l'IA créera un dossier `tests/fixtures/`. Ce dossier contiendra des images statiques (`.jpg` ou matrices sérialisées) de visages connus et inconnus. Les tests unitaires du composant `vision` chargeront ces fichiers statiques pour valider les étapes de détection (NMS), d'alignement géométrique, et de similarité cosinus.