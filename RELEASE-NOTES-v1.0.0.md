# ApiVault Proxy 1.0.0 — 19 octobre 2026

Première version publique. Ce n'est pas une préversion : le coffre, le pool, le
compteur et le plafond sont livrés, audités et recettés.

## Ce qu'elle apporte

**La passerelle**
- Compatible OpenAI : `/v1/models`, `/v1/chat/completions`, en flux ou en réponse
  complète.
- Écoute uniquement sur `127.0.0.1:3100`, en-tête `Host` vérifié contre le DNS
  rebinding, routes d'administration protégées par jeton.
- Alias `defaut` qui essaie les cibles dans l'ordre et bascule sur erreur.
- État des cibles persistant (joignable, en attente, en panne) qui survit au
  redémarrage.

**Le coffre**
- `cles.env` chiffré au repos (DPAPI, portée utilisateur Windows) : plus aucune
  clé en clair sur le disque.
- Sauvegarde `.bak` chiffrée avant chaque modification, supprimée après rotation.
- Migration automatique d'un ancien `cles.env` en clair, au démarrage.
- Droits du dossier de données restreints au seul compte courant.

**La consommation**
- Une ligne par requête dans un journal mensuel en ajout seul.
- Tokens d'entrée, de sortie et de cache ; agrégats jour, semaine, mois.
- Estimation signalée (`estime: true`) quand le fournisseur ne fournit pas son
  décompte — jamais un zéro trompeur.
- Plafond mensuel vérifié **avant** l'appel amont, refus `429 plafond_atteint`,
  alerte à 80 %, désactivé par défaut.

**L'interface**
- Français / anglais, mode clair et sombre, infobulles.
- Fenêtre de contrôle, panneau de gestion, consommation, plafond.
- Icône de notification dont la couleur suit l'état réel du service.
- Démarrage avec Windows, en mode discret.

## Installation

1. Télécharger `apivault-proxy-1.0.0-win64.zip` ci-dessous et le décompresser.
2. Lancer `Demarrer.bat`.
3. Dans **Gestion**, saisir une clé d'API : elle est chiffrée immédiatement.
4. Cliquer sur **Copier la configuration client** et la coller dans le client IA.

Aucun prérequis : ni Docker, ni Python, ni Node, ni service système.

## Vérifications

- **88 tests** automatisés, 0 avertissement.
- Audit indépendant (contexte séparé, modèle différent du codeur) : 3 constats
  majeurs trouvés, corrigés et revérifiés ; 3 fausses pistes réfutées.
- Recette d'exécution sur le binaire réel dans un bac à sable : **9/9** —
  chiffrement effectif, `Host` refusé, administration protégée, comptage exact en
  non-stream, estimation non nulle en flux, plafond sans appel amont, journal
  tolérant à un octet corrompu, BOM accepté, aucun processus résiduel.

## Limites connues

- Le coffre protège contre un autre compte Windows ou un disque volé, **pas**
  contre un programme qui tourne sous ta propre session.
- Pas de quota par alias, pas de prix en euros : le plafond est global et exprimé
  en tokens.
- Pas d'API Anthropic (`/v1/messages`) ni d'`/v1/embeddings`.
- Les refus pour plafond atteint ne sont pas encore écrits dans le journal.
- Un changement de plafond demande un redémarrage.
- La transcription audio n'est pas implémentée.

## Empreintes

```
SHA-256  apivault-proxy-1.0.0-win64.zip   38B975B70918F081385CA3E0CE3A3B776B5CDB5EFBFD66B8897FED93F554B30E
SHA-256  apivault-proxy.exe              F4EED07585DD621B3C2267D661914B73F223E10E2AF01B40E7B6638A3252F0E8
```

Vérification sous Windows :

```powershell
Get-FileHash .\apivault-proxy-1.0.0-win64.zip -Algorithm SHA256
```
