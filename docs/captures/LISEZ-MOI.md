# Guide des captures — ApiVault Proxy

Quatre captures suffisent pour la page GitHub — elles sont déjà référencées dans le
`README.md`. Deux autres sont proposées en bonus : si tu les ajoutes, je les insère
dans la galerie.

Dépose les fichiers **dans ce dossier**, **avec ces noms exacts**.

## Les quatre indispensables

| Nom attendu | Ce qu'il faut montrer | Pourquoi cette capture |
|---|---|---|
| `01-fenetre-principale.png` | La fenêtre complète, bandeau du haut compris, avec au moins un modèle joignable (point vert) | C'est la première image que voit un visiteur |
| `02-panneau-gestion.png` | Le panneau « Gestion » ouvert, un fournisseur déplié | Montre où vivent les clés |
| `03-consommation.png` | Le panneau de consommation du mois : entrée, sortie, cache, nombre de requêtes | Prouve que le compteur existe et qu'il est lisible |
| `04-plafond.png` | La carte du plafond avec sa jauge, si possible en état d'alerte | Montre le garde-fou |

## Les deux bonus

| Nom attendu | Ce qu'il faut montrer |
|---|---|
| `05-badges.png` | Une ligne de modèles avec les badges « gratuit » / « payant » et l'étoile du modèle par défaut |
| `06-fournisseurs.png` | L'état des fournisseurs : joignable, en attente, en panne, avec les boutons « Tester » |

## Avant de capturer — le point qui compte

> ⚠️ **Masque le jeton.** Le bouton « Voir » affiche `PROXY_MASTER_KEY` en clair.
> Une capture qui le montre, c'est une clé publiée sur GitHub, même si tu la
> supprimes ensuite : elle reste dans l'historique public.
>
> Même remarque pour une clé d'API qui apparaîtrait dans un champ de saisie.
> Si besoin, remplace-la par `sk-exemple-masque` avant de capturer.

Vérifie aussi qu'aucun nom de dossier personnel, adresse e-mail ou chemin
`C:\Users\...` ne traîne dans l'image.

## Comment faire, simplement

1. `Win + Maj + S` → sélection de la zone → la capture part dans le presse-papiers.
2. Colle-la dans Paint (ou n'importe quel éditeur d'image) et enregistre en **PNG**.
3. Recadre sur la fenêtre, sans le bureau autour.
4. Vise une largeur de **1200 à 1600 pixels** et moins de **500 Ko** par image
   (GitHub affiche au maximum 1200 px de large dans le README).
5. Dépose le fichier ici, avec le nom exact du tableau.

## Une fois les six images en place

Dis-le-moi : je vérifie les noms, les dimensions et le poids, je contrôle qu'aucun
jeton n'est visible, puis je prépare le commit de publication et le tag.
