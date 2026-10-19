# ApiVault Proxy 1.0.4 — Fournisseurs locaux, port de secours, refus lisible

[Français](RELEASE-NOTES-v1.0.4.md) | [English](RELEASE-NOTES-v1.0.4.en.md)

Version de praticité. L'API ne change pas ; le comportement s'adapte à trois
situations qui bloquaient ou inquiétaient sans raison.

## Fournisseurs locaux (option)

- Nouvelle option « Autoriser le HTTP vers mon réseau local », **décochée par
  défaut**. Une fois cochée, le HTTP non chiffré est accepté vers une adresse
  privée littérale : `10.x`, `172.16` à `31.x`, `192.168.x`, lien local, et
  `100.64.x` (Tailscale).
- Jamais vers Internet, jamais vers un nom de domaine : sans chiffrement, la clé
  circulerait en clair. `https://` reste accepté partout.
- La règle s'applique aussi au bouton « tester » : aucune clé n'est envoyée vers
  une adresse refusée.
- Pour un Ollama sur un VPS : tunnel SSH, reverse proxy HTTPS, ou Tailscale.
  Trois chemins sûrs, aucun ne demande de désactiver quoi que ce soit.

## Port occupé

- Si le port 3100 est pris, la passerelle essaie 3101, 3102… jusqu'à 3109.
- Le port retenu est **mémorisé** : au démarrage suivant, c'est lui qui est
  essayé en premier, donc les clients configurés continuent de fonctionner.
- Une **bulle Windows** prévient immédiatement, même fenêtre fermée, et un
  bandeau dans la fenêtre rappelle l'adresse à mettre à jour.
- Si les dix ports sont pris, le message dit quoi faire au lieu de seulement
  constater l'échec.

## Refus de plafond

Le `429 plafond_atteint` porte maintenant les chiffres utiles :

```json
{"error":{"type":"insufficient_quota","code":"plafond_atteint",
"utilise":4820000,"plafond":5000000,
"periode":"2026-10","reinitialisation":"2026-11-01"}}
```

## Vérification de version

- Au démarrage, une lecture de la dernière version publiée sur GitHub.
- Une seule requête, **sans jeton et sans aucune donnée envoyée**, 5 secondes
  maximum, échec silencieux. Aucun téléchargement automatique.
- Désactivable dans les réglages.

## Ce que le compteur ne fera jamais

Il compte des **tokens**, jamais des euros. La facturation reste chez le
fournisseur, seul à connaître ses prix et ses paliers. Un montant calculé ici ne
serait qu'une hypothèse déguisée en mesure.

## Vérifications

- 88 tests, 0 échec.
- Essai réel : port 3100 occupé → démarrage sur 3101, bascule journalisée et
  mémorisée ; plafond à 10 → `429` avec les quatre champs, sans appel amont.
- Archive : `apivault-proxy-1.0.4-win64.zip` — **3 577 561 octets**.

## Empreintes

```
SHA-256  apivault-proxy-1.0.4-win64.zip   1ED8C64E5D9DD7BC982275E9D6106B29A9BB3C2D79A98A144E963E529FEE9F83
```
