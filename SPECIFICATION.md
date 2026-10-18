# ApiVault_Proxy — Spécification

Version du document : 0.1 — 17/10/2026
Statut : à valider avant la première ligne de code.

---

## 1. Deux produits, un seul contrat

### Version A — greffe dans le client existant (temporaire, non négociable)

Une **entrée fournisseur supplémentaire** dans le client IA déjà installé, qui
pointe vers une passerelle locale statique (LiteLLM). Ce client possède déjà les
clés et les fournisseurs ; la passerelle est une **couche de redirection** posée
devant.

- 4 fournisseurs, 3 alias, bascule linéaire.
- Aucun pool, aucune classe, aucun apprentissage, aucun quota.
- Objectif : que ça marche, que ce soit prévisible, qu'on n'y touche plus.
- Durée de vie : jusqu'à ce que la version B soit mûre, puis on change une URL.

### Version B — ApiVault_Proxy (le produit)

Application autonome, coffre + passerelle intelligente.

- Pool de modèles/clés, classes d'équivalence, rotation de clés, quotas.
- Interface complète, journal, coûts.
- Objectif : volume gratuit, bascule équivalente, maîtrise des dépenses.

### Le contrat entre les deux

B expose la **même API OpenAI** que A. Le jour venu :

```
base_url : http://127.0.0.1:4000/v1   →   http://127.0.0.1:3100/v1
clé      : sk-litellm-…                →   sk-apivault-…
```

Aucune réécriture, aucun couplage. **Aucune intelligence dans A** : elle serait
à déboguer deux fois.

---

## 2. Contraintes

- Windows 10/11 x64. Un seul exécutable, portable par défaut.
- Pas de Docker, pas de Python, pas de Node, pas de service système.
- Le coffre réutilise le format ApiVault (PBKDF2 + AES-256-GCM), avec import
  du `.vaultbackup` chiffré existant.
- Aucune donnée ne sort de la machine, sauf les requêtes vers les fournisseurs
  explicitement configurés.
- Le binaire doit rester sous 12 Mo.

---

## 3. Architecture technique

```
Tauri v2 (1 processus)
├── Interface (HTML/JS vanilla, i18n FR/EN)
├── Coffre          : PBKDF2-SHA256 + AES-256-GCM
├── Passerelle      : axum (HTTP) + reqwest (client amont, rustls)
├── Journal         : rusqlite (SQLite embarqué)
├── Zone de notif.  : tray Tauri + instance unique
└── Déverrouillage  : mot de passe | confort | DPAPI (Windows)
```

Crates ajoutés : `tokio`, `axum`, `reqwest` (rustls, sans OpenSSL), `rusqlite`
(bundled), `zeroize`, `windows`, `tauri-plugin-single-instance`.

**Cycle de vie**

- Le serveur démarre avec l'application si `passerelle.actif = true`.
- Fermer la fenêtre ne quitte pas : réduction dans la zone de notification.
- Quitter vraiment : arrêt du serveur, clés effacées de la mémoire, puis sortie.
- Instance unique obligatoire : deux instances = conflit de port.

**Flux des clés**

Déverrouillage → le front déchiffre → commande IPC `passerelle_definir_cles`
→ Rust conserve la liste en mémoire (`Mutex`, `Zeroize` à la libération)
→ jamais écrite sur le disque, jamais journalisée.

---

## 4. Modèle de données

### 4.1 Coffre (chiffré) — version 3

```json
{
  "version": 3,
  "kdf": { "sel": "…", "iterations": 600000, "cle_memorisee": null },
  "fournisseurs": [
    {
      "id": "…",
      "nom": "OpenRouter",
      "urls": [{ "libelle": "OpenAI", "adresse": "https://openrouter.ai/api/v1" }],
      "cles": [
        {
          "id": "…",
          "libelle": "principale",
          "valeur_chiffree": "…",
          "type": "gratuite",          // gratuite | payante
          "quota_jour": 50,            // null = inconnu
          "defaut": true,
          "statut": "sain",            // sain | en_pause | en_panne | epuise
          "statut_jusqua": null
        }
      ],
      "passerelle": {
        "actif": true,
        "type": "openai",              // openai | anthropic
        "base_url": "https://openrouter.ai/api/v1",
        "support_usage_stream": true   // include_usage accepté ?
      }
    }
  ],
  "passerelle": {
    "actif": false,
    "port": 3100,
    "plafond_jour_eur": 0.5,
    "journal_jours": 30,
    "alias": [
      {
        "nom": "defaut",
        "classe": "standard",
        "niveau_min": 3,
        "tags_requis": [],
        "politique": "gratuit_puis_payant",
        "defaut": true
      }
    ],
    "jetons": [
      {
        "id": "…",
        "nom": "client-local",
        "empreinte": "sha256:…",       // la clé en clair n'est affichée qu'une fois
        "alias_autorises": ["defaut", "gratuit"],
        "plafond_jour_eur": null,
        "cree_le": "…"
      }
    ]
  }
}
```

### 4.2 Catalogue (en clair, non secret)

`%APPDATA%\ApiVault_Proxy\catalogue.json` — cache des modèles, TTL 6 h.
Contient : fournisseur, id, prix entrée/sortie, contexte, modalités, tags,
classe, niveau, statut. Aucune clé.

### 4.3 Journal (SQLite)

`%APPDATA%\ApiVault_Proxy\journal.db`

```
requetes(id, horodatage, jeton, alias, fournisseur, modele, statut,
         latence_ms, tokens_in, tokens_out, cout_eur, estime, tentative)
```

Purge automatique à `journal_jours`. **Jamais le contenu des messages.**

---

## 5. Le pool : une machine à états (patron CloudProxy)

Chaque triplet `(fournisseur, clé, modèle)` porte un état :

| État | Sens | Sortie |
|---|---|---|
| `sain` | utilisable | sur erreur → `en_pause` / `en_panne` |
| `inconnu` | jamais testé | après test → `sain` ou `en_panne` |
| `en_pause` | quota atteint, réessai plus tard | à l'échéance → `inconnu` |
| `en_panne` | clé morte ou fournisseur HS | test manuel ou après 1 h → `inconnu` |
| `epuise` | crédits épuisés (402) | reset quotidien ou action utilisateur |

**Boucle de réconciliation** (toutes les 30 s, tâche `tokio`) :

1. expire les `en_pause` / `epuise` arrivés à échéance ;
2. requalifie les `inconnu` par un test léger (une requête d'un token) ;
3. remet en `inconnu` les `en_panne` après une heure, avec backoff exponentiel ;
4. vérifie l'invariant : **au moins un candidat `sain` par classe utilisée**.
   Si l'invariant tombe, alerte visible dans l'interface — on ne laisse jamais
   un alias silencieusement vide.

Rien n'est supprimé du pool : un modèle qui disparaît du catalogue passe en
`inconnu`, il n'est pas effacé (il peut revenir).

**Réactivation manuelle (indispensable).**
Un bouton « Réessayer » sur un modèle, sur un fournisseur, et un
« Tout requalifier » sur le pool. Il remet l'état à `inconnu` et lance le test
immédiatement, sans attendre l'échéance.

Raison : une coupure Wi-Fi de deux minutes ne doit pas dégrader un modèle
pour une heure. L'automatisme est là pour éviter l'attente inutile, pas pour
enfermer l'utilisateur dans une décision prise à sa place. L'état affiché doit
toujours pouvoir être contesté d'un clic.

---

## 6. Résolution d'une requête

1. **Lire la requête** et détecter les capacités exigées :
   `tools` présent → tag `outils` ; image dans `content` → `vision` ;
   `response_format: json` → `json` ; estimer les tokens → `contexte ≥ N`.
2. **Choisir l'alias** : champ `model`. Inconnu ou `auto` → alias `defaut`.
   Alias inexistant → 404 `model_not_found` avec la liste des alias.
3. **Filtrer les candidats** : classe identique, `niveau ≥ niveau_min`,
   tous les tags requis présents, état ≠ `en_panne` / `epuise`,
   clé non épuisée.
4. **Ordonner** : même famille de modèle d'abord, puis niveau décroissant,
   puis gratuit avant payant (selon politique), puis quota restant
   décroissant, puis latence médiane croissante.
5. **Tenter** chaque candidat, et pour chacun chaque clé autorisée.
6. **Succès** : diffuser, journaliser, décrémenter le quota.
7. **Échec** : classer l'erreur (tableau §7), appliquer l'état, continuer.
8. **Épuisement** : erreur OpenAI `pool_exhausted`, avec la liste des
   tentatives (fournisseur, modèle, statut) — pas un message vague.

### Ordre de bascule (4 étages, jamais plus)

| Étage | Tentative |
|---|---|
| 1 | même modèle, autre clé |
| 2 | même modèle, autre fournisseur |
| 3 | même classe, gratuit |
| 4 | même classe, payant — si la politique et le plafond l'autorisent |

**Point de non-retour** : après le premier token envoyé au client, aucune
bascule. On termine par un événement d'erreur propre.

---

## 7. Erreurs amont → actions

| Réponse | État appliqué | Action |
|---|---|---|
| 429 | clé `en_pause` (1 min → reset quota) | clé suivante |
| 401 / 403 | clé `en_panne` | clé suivante + alerte |
| 402 | clé `epuise` | clé suivante |
| 404 | modèle `inconnu` | modèle suivant |
| 400 (tools/vision) | tag retiré du modèle | modèle suivant |
| 400 (autre) | — | **aucun réessai**, erreur rendue au client |
| 5xx / timeout / connexion | fournisseur `en_pause` 60 s | modèle suivant |
| 200 puis 1er événement = erreur | comme 5xx | modèle suivant |

Disjoncteur : 3 échecs consécutifs sur un fournisseur → écarté 60 s.
Budget : 2 alternatives maximum, 45 s au total.

---

## 8. API exposée

| Méthode | Chemin | Notes |
|---|---|---|
| GET | `/health` | sans authentification, pour les scripts |
| GET | `/v1/models` | renvoie **les alias**, jamais l'inventaire |
| POST | `/v1/chat/completions` | streaming et non-streaming |
| POST | `/v1/embeddings` | v1.1 |
| POST | `/v1/audio/transcriptions` | v1.1 (Gemini depuis le coffre) |
| POST | `/v1/messages` | v1.2 (Claude Code) |

Authentification : `Authorization: Bearer sk-apivault-…`, comparaison à temps
constant sur l'empreinte. `/v1/models` ne fait **aucun appel amont**.

Format d'erreur :

```json
{ "error": {
    "message": "pool épuisé pour l'alias « gratuit »",
    "type": "pool_exhausted",
    "code": "pool_exhausted",
    "attempts": [ { "fournisseur": "…", "modele": "…", "statut": 429 } ]
} }
```

Délais : connexion 10 s, premier octet 120 s, inactivité 300 s, total 900 s.
Corps maximal : 8 Mo (chat), 32 Mo (audio).

---

## 9. Sécurité

- Écoute **127.0.0.1 uniquement** (pas de règle pare-feu, pas de prompt Windows).
- Jeton obligatoire ; pas de mode anonyme.
- Refus de toute requête portant un en-tête `Origin` non local (anti-DNS-rebinding).
- Pas de CORS : aucun navigateur ne doit pouvoir appeler la passerelle.
- Clés effacées de la mémoire au verrouillage (`zeroize`).
- Journal sans contenu, sans en-tête d'authentification.
- Jetons clients stockés sous forme d'empreinte, affichés une seule fois.
- Corps limité, en-têtes refusés au-delà de 16 Ko.

---

## 10. Interface — sept onglets

1. **État** : démarré/arrêté, port, durée, requêtes, dernière erreur.
2. **Fournisseurs** : actif, URL, clés (type, quota, test, état), priorité.
3. **Modèles** : inventaire, filtres, badges, rafraîchir + diff.
4. **Alias** : nom, classe, niveau, politique, glisser-déposer, défaut.
5. **Jetons** : nom du projet, alias autorisés, plafond, dernier appel.
6. **Journal** : heure, alias, modèle réel, statut, latence, tokens, coût.
7. **Réglages** : port, démarrage Windows, déverrouillage, plafond, purge.

Plus un **mini-chat de test**, accessible par un lien discret : il sert à
vérifier un alias sans ouvrir de client IA.

---

## 11. Réglages par défaut

| Réglage | Valeur |
|---|---|
| Port | 3100 (échec explicite si occupé, pas de bascule silencieuse) |
| Alias `defaut` | standard, niveau ≥ 3, gratuit puis payant |
| Alias `gratuit` | standard, niveau ≥ 3, gratuit seulement |
| Alias `code` | code, payant seulement |
| Plafond | 0,50 €/jour |
| Journal | 30 jours |
| Démarrage | Windows, minimisé, déverrouillage DPAPI |

---

## 12. Plan de développement

### v0.1 — le socle
Coffre importé, 4 fournisseurs, **un** alias, `/v1/models`,
`/v1/chat/completions` (streaming + outils), jeton unique.

*Acceptation* : un client voit l'alias, une réponse arrive token par token,
un appel d'outil fait l'aller-retour, une coupure amont se termine proprement.

### v0.2 — le pool
Classes, niveaux, tags, multi-clés, les 4 étages, machine à états,
réconciliation, politique de coût, plafond.

*Acceptation* : 429 sur la première clé → deuxième clé, invisible pour le
client ; 500 sur un modèle → bascule même classe, jamais hors classe ;
plafond atteint → refus explicite, zéro dépense.

### v0.3 — l'exploitation
Interface complète, journal, quotas, diff de catalogue, test de clé,
zone de notification, démarrage Windows, DPAPI, purge.

*Acceptation* : redémarrer Windows → passerelle disponible sans mot de passe ;
les coûts du jour sont justes à ±5 %.

### v0.4 — l'interface opérationnelle
Infobulles permanentes, double confirmation d'effacement, choix du modèle par
défaut (persisté), bandeau fixe nom/état/modèle, emplacement « Super IA »
désactivé, priorité d'alias explicite. Gemma 4 31B reste présent mais passe en
dernier recours (429 fréquents). Tests unitaires sur les alias, le gratuit/
payant et l'ordre du pool.

*Acceptation* : les cinq points d'interface sont utilisables en FR et EN ;
le modèle choisi est en tête de `defaut` au redémarrage ; un modèle payant ne
pollue pas les alias gratuits ; `cargo test --release` passe.

### v1.0 — l'apprentissage
Capacités apprises des erreurs, réordonnancement par latence mesurée,
suggestions de nouveaux gratuits.

---

## 13. Hors périmètre v1

Cache sémantique, réécriture de prompts, multi-utilisateurs, accès distant,
génération d'images, `/v1/messages` (reporté en v1.2), facturation.

---

## 14. Risques connus

| Risque | Réponse |
|---|---|
| Les gratuits changent tous les mois | diff + alerte, jamais un alias sur un seul modèle |
| « Équivalent » reste approximatif | bascule avant le premier token seulement |
| Certains gratuits s'entraînent sur vos données | colonne dédiée dans l'interface |
| Faux positif antivirus (exe qui ouvre un port) | signature, ou exception documentée |
| Deux instances | plugin single-instance |
| Port occupé | échec explicite, bouton « choisir un autre port » |

---

## 15. Version A — configuration du client

```
base_url : http://127.0.0.1:4000/v1
clé      : sk-litellm-<généré>
modèles  : gratuit | rapide | defaut
```

- YAML statique, 4 fournisseurs, `fallbacks` linéaires, aucun pool.
- Démarrage à l'ouverture de session (tâche planifiée, fenêtre masquée).
- Clés dans `%APPDATA%\ApiVault_Proxy\cles.env`, ACL restreinte à l'utilisateur
  courant — compromis assumé d'une version temporaire.
- Les 4 fournisseurs du client restent en place : la passerelle est une 5e entrée.
