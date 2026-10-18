# PROTOCOLE DE TEST — ApiVault Proxy

**Objet** : vérifier le comportement réel de l'application, du point de vue de l'utilisateur.
Pas de relecture de code, pas d'opinion : des faits, des preuves, des OK/KO.

**Destinataires** : auditeur externe (Cline, Reasonix, ou un humain).
**Version testée** : à compléter par l'auditeur (commit + tag + version de l'exe).

---

## Charte de l'audit — à lire avant de commencer

Ces six règles sont **contractuelles**. Un audit qui n'en respecte pas une seule est nul et sera renvoyé.

1. **AUCUNE MODIFICATION. INTERDICTION ABSOLUE.**
   Tu ne modifies, ne corriges, ne reformates et ne « améliores » **aucun fichier** du dépôt — code, configuration, documentation, icônes, `.gitignore`, tout compris. Pas même une faute de frappe évidente, pas même un commentaire. Tu observes, tu exécutes, tu constates, tu rapportes. Si tu vois un défaut, il va dans le rapport ; il ne va **pas** dans le code.
   Corollaire : tu ne lances `cargo fmt`, ni `git commit`, ni `git checkout`, ni `git clean`, ni aucune commande qui écrit dans le dépôt.

2. **UN SEUL FICHIER CRÉÉ : LE RAPPORT.**
   À la fin de l'audit, tu déposes à la **racine du dépôt** un fichier nommé :
   `RAPPORT-AUDIT-<ton-nom>-<AAAAMMJJ>.md`
   (exemple : `RAPPORT-AUDIT-Cline-20261020.md`). C'est le livrable. Rien d'autre n'est ajouté au dépôt. Tes scripts ou fichiers temporaires vont **hors du dépôt** (ou sont supprimés avant de rendre).

3. Chaque test produit : un identifiant, un verdict **OK / KO / NON TESTÉ**, et une **preuve** (sortie de commande, extrait de journal, capture d'écran). Un OK sans preuve vaut NON TESTÉ.

4. Un test que tu n'as pas pu exécuter est **NON TESTÉ**. Jamais OK par déduction, jamais « ça devrait marcher ».

5. **Ne jamais recopier une clé API** dans le rapport, même partiellement. Si une clé apparaît quelque part, c'est un **KO de sécurité** et tu le signales comme bloquant.

6. Environnement de départ : application fermée, port 3100 libre, dépôt propre. Vérifie-le (T0) avant tout le reste.

---

## T0 — Préparation

| ID | Test | Attendu |
|---|---|---|
| T0.1 | `git status --short` et `git log -1 --oneline` | Propre, commit annoncé |
| T0.2 | `Get-Process apivault-proxy` | Aucun processus |
| T0.3 | `netstat -ano \| findstr :3100` | Port libre |
| T0.4 | Sauvegarde de `cles.env` + SHA-256 | Fichier conservé pour comparaison finale |
| T0.5 | `cargo test --release` | Noter le nombre exact de tests et le résultat |

---

## T1 — Démarrage

| ID | Test | Attendu |
|---|---|---|
| T1.1 | Chronomètre : lancer l'exe → interface interactive. **3 mesures.** | ≤ 3 s (à défaut, noter les 3 valeurs) |
| T1.2 | Pendant le démarrage, `Invoke-WebRequest http://127.0.0.1:3100/sante` | Réponse ≤ 2 s, y compris pendant l'initialisation |
| T1.3 | Journal des 5 premières secondes | **Aucun** appel à un fournisseur |
| T1.4 | État affiché sans aucune action de l'utilisateur | Les modèles connus apparaissent, avec un horodatage du type « vérifié il y a X » |
| T1.5 | Bandeau de démarrage | Message d'initialisation visible, puis disparaît de lui-même |
| T1.6 | Deuxième lancement (même configuration) | Temps ≤ premier lancement |
| T1.7 | Lancer alors que le port 3100 est occupé | Message d'erreur clair, aucun crash, aucune fenêtre vide |

---

## T2 — Interface

| ID | Test | Attendu |
|---|---|---|
| T2.1 | Défiler jusqu'au bas du tableau | Le bandeau supérieur reste visible en permanence (capture avant/après) |
| T2.2 | Double-clic sur `http://127.0.0.1:3100/v1`, puis coller dans Notepad | La ligne **entière** est copiée, à l'identique, sans espace parasite |
| T2.3 | Bouton de copie du bloc de code | Copie complète, retour visuel |
| T2.4 | Nom du panneau | « Fournisseurs » (et non « Adresse ») |
| T2.5 | Ajouter un fournisseur (nom, URL, clé de test, modèles) | Apparaît immédiatement, **persiste après redémarrage** |
| T2.6 | Bouton « Tester la clé » | Clé valide → succès ; clé invalide → message lisible, pas de crash |
| T2.7 | Bouton « Copier le diagnostic » | JSON valide, collé dans Notepad ; **aucune clé**, aucune URL contenant un secret |
| T2.8 | Double-clic sur l'étoile d'un modèle par défaut | Sélectionne ; **re-double-clic désélectionne** ; repli sur le premier modèle joignable |
| T2.9 | Sélectionner un modèle non joignable | Sélection possible, avertissement visible, pas de blocage |

---

## T3 — Passerelle

| ID | Test | Attendu |
|---|---|---|
| T3.1 | `GET /v1/models` avec le jeton | Liste des alias, format OpenAI |
| T3.2 | `POST /v1/chat/completions` sans streaming | Réponse complète, `usage` si le fournisseur le renvoie |
| T3.3 | Même appel en streaming | Flux SSE complet, terminé par `[DONE]`, pas de coupure |
| T3.4 | Sans jeton / mauvais jeton | **401**, aucun appel au fournisseur |
| T3.5 | Fournisseur prioritaire en panne | Bascule vers le suivant, réponse 200, journal cohérent |
| T3.6 | Rechercher la valeur exacte de la clé dans les journaux | **Aucune occurrence** |

---

## T4 — Super IA : hors périmètre, développement reporté en V2

**Décision de version** : la Super IA n'est **pas** livrée en V1. Elle est reportée à une V2. Le comportement attendu en V1 est donc uniquement celui d'un **emplacement réservé, inerte et honnêtement étiqueté**. Il n'y a **aucune fonction à tester** ici : on vérifie seulement qu'elle ne fait rien.

| ID | Test | Attendu |
|---|---|---|
| T4.1 | Emplacement du bloc | Panneau de droite, sous les jetons, séparé par un filet, intitulé « Gestion supérieure » |
| T4.2 | État de la case | **Désactivée et non cliquable**, badge « en cours de développement » (et non « bientôt », qui suggérerait une livraison proche) |
| T4.3 | Deux listes déroulantes | Présentes, **inactives**, sans effet au clic |
| T4.4 | Une requête normale, case décochée | **Aucun** appel supplémentaire (journal : un seul appel amont) |
| T4.5 | Tentative de forcer la case ou les listes (clic, clavier, DevTools) | Aucun effet, aucune requête émise, aucun message d'erreur visible |
| T4.6 | Recherche d'un second appel amont dans le journal sur 10 requêtes | **Zéro** appel supplémentaire, quel que soit le réglage |

**KO automatique** : toute trace d'un appel à un modèle de « réflexion », toute case cochable, ou tout texte laissant croire que la fonction est opérationnelle.

---

## T5 — Régressions

| ID | Test | Attendu |
|---|---|---|
| T5.1 | `cargo test --release` | Tous verts, aucun test ignoré, aucun test supprimé par rapport à la version précédente |
| T5.2 | Version de l'exe | Celle annoncée dans le rapport |
| T5.3 | SHA-256 de `cles.env` | Identique à T0.4 |
| T5.4 | `git status --short`, `git tag --points-at HEAD` | Propre, tag présent |

---

## Rapport attendu — livrable obligatoire

Fichier unique, à la racine du dépôt : **`RAPPORT-AUDIT-<nom>-<AAAAMMJJ>.md`**.
Il commence par ce bloc, à recopier tel quel en le complétant :

```
# Rapport d'audit — ApiVault Proxy

- Auditeur : <nom / outil>
- Date : <AAAA-MM-JJ>
- Commit audité : <git log -1 --oneline>
- Tag : <git tag --points-at HEAD>
- Version de l'exe : <version affichée>
- Convention respectée : aucune modification apportée, un seul fichier créé (celui-ci) : OUI / NON
```

Puis un tableau unique :

| ID | Verdict | Preuve | Remarque |
|---|---|---|---|
| T1.1 | OK | 2,8 s / 2,9 s / 2,7 s | — |
| … | … | … | … |

Puis trois sections :

- **Bloquants** — ce qui empêche l'usage normal.
- **Majeurs** — ce qui dégrade l'expérience sans bloquer.
- **Mineurs** — cosmétique, formulations, incohérences.

Une phrase de conclusion : *« Sur les N tests, X OK, Y KO, Z non testés. »*

Et une dernière ligne, obligatoire :

```
Intégrité : git status --short ne montre que ce fichier de rapport.  OUI / NON
```

Si la réponse est NON, l'audit est invalide : l'auditeur a modifié le dépôt.

---

## Ce que l'auditeur ne doit pas faire

- **Modifier un fichier du dépôt, quel qu'il soit, même pour un test trivial** (règle n°1, sans exception).
- Créer un fichier autre que le rapport à la racine.
- Corriger le code « puisque c'était évident ».
- Relancer un scan complet « pour voir » sans le noter.
- Conclure « ça marche » sans preuve exécutée.
- Tester avec une vraie clé facturée : utiliser une clé de test ou un fournisseur gratuit.
- Développer, même partiellement, la Super IA : elle est hors périmètre (V2).
