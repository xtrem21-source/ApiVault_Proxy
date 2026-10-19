# ApiVault Proxy 1.0.2 — Correctifs d'audit

[Français](RELEASE-NOTES-v1.0.2.md) | [English](RELEASE-NOTES-v1.0.2.en.md)

Version corrective issue de l'audit externe. Aucun changement de fonctionnalité
métier ; corrections de cohérence, de sécurité et de performance.

## Corrections

- `/v1/models` et `/v1/models/{id}` utilisent les alias effectifs : un fournisseur
  ajouté depuis l'interface est visible sans redémarrage.
- Les adresses amont doivent être en **HTTPS** ; `http://` n'est accepté que vers
  `127.0.0.1` / `localhost`.
- Le plafond mensuel est tenu en mémoire après le premier chargement, au lieu de
  relire tout le JSONL du mois à chaque requête.
- `masquer()` est sûr avec les caractères non ASCII.
- Le jeton est vérifié avant l'acquisition du sémaphore amont.
- La comparaison du jeton ne retourne plus immédiatement sur une longueur
  différente.
- `journal.log` est rotationné en `journal.log.1` au-delà de 2 Mo ; les clés
  connues sont masquées avant écriture.
- `X-Request-Id` passe à 16 caractères hexadécimaux.

## Limite documentée

Les clés ne sont pas zeroizées en mémoire. La protection reste celle d'une
session Windows ; ce point est désormais explicite dans `ETAT-REEL`.

## Vérifications

- 88 tests, 0 échec.
- Recette de non-régression conservée.
- Archive : `apivault-proxy-1.0.2-win64.zip` — **3 444 874 octets**.

## Empreintes

```
SHA-256  apivault-proxy-1.0.2-win64.zip   5E39AA857DDE1609C72DF49430F566ADD13056F3287F463D66028D7FBCBCC63E
```
