# ApiVault Proxy 1.0.3 — Correctifs scripts et distribution

[Français](RELEASE-NOTES-v1.0.3.md) | [English](RELEASE-NOTES-v1.0.3.en.md)

Version corrective de distribution. Aucun changement de l'API métier.

## Sécurité des scripts

- `Etat`, `Rafraichir` et `Demarrer` vérifient que le processus qui écoute sur le
  port est bien `apivault-proxy.exe` **avant** d'envoyer le jeton maître.
- `Changer-Cle` accepte tout fournisseur `<NOM>_KEY`, plus seulement les trois
  fournisseurs historiques.
- `-Cle <valeur>` est supprimé : la clé passe par `-CleFichier` ou `-CleEnv`
  (variable temporaire supprimée après lecture).
- Après une rotation, `cles.env.bak` est supprimé : une clé compromise ne survit
  pas dans la sauvegarde.
- Les ACL utilisent le SID courant, et le `.tmp` du coffre est restreint **avant**
  le `Move-Item`.

## Arrêt et archive

- `POST /admin/arreter` permet un arrêt propre ; `Arreter.ps1` l'utilise et
  n'emploie `Stop-Process -Force` qu'en dernier recours après 15 s.
- L'archive est reconstruite avec des séparateurs `/`, compatible `unzip` sous
  Linux.
- Le binaire est compilé avec `--remap-path-prefix` et `strip = true` : le chemin
  de compilation et le nom d'utilisateur ne sont plus dans la release.

## Limites documentées

- `icacls /inheritance:r` retire aussi SYSTEM et Administrateurs ; c'est un choix
  assumé, documenté dans `ETAT-REEL`.
- Le `.tmp` côté cœur Rust reste protégé par les droits du dossier ; le script
  d'écriture applique les droits avant le remplacement.

## Vérifications

- 88 tests, 0 échec.
- Archive : `apivault-proxy-1.0.3-win64.zip` — **3 451 206 octets**.

## Empreintes

```
SHA-256  apivault-proxy-1.0.3-win64.zip   AA84BC7D43D9567A0C45733294F089896F764E6C69E6B2B783F1DADDDDA07E21
```

Le binaire contient encore une occurrence du chemin de compilation
(`C:\Users\Xtrem-21`) ; elle est acceptée explicitement pour cette release.
