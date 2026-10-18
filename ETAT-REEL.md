# ETAT-REEL — ApiVault_Proxy

Document de référence factuel : ce qui existe vraiment dans le dépôt, à la date
du 18/10/2026, après le sprint 0 d'assainissement.

## Version

- Version applicative : **0.5.0** (`Cargo.toml`, `tauri.conf.json`, en-tête `/health`).
- Binaire : `Proxy-Externe/src-tauri/target/release/apivault-proxy.exe`.
- Crate : `apivault-proxy` (nom technique), produit : `ApiVault_Proxy`.
- Édition Rust : 2021.

## Ce qui est implémenté

### Passerelle HTTP (127.0.0.1 uniquement)

| Route | Méthode | Rôle |
|---|---|---|
| `/` | GET | Présentation en texte |
| `/health`, `/health/liveliness`, `/health/readiness` | GET | Sonde publique : `status` + `version` seulement, sans jeton |
| `/v1/models`, `/models` | GET | Liste des alias au format OpenAI |
| `/v1/models/{id}` | GET | Détail d'un alias |
| `/v1/chat/completions`, `/chat/completions` | POST | Appel avec bascule automatique |
| `/admin/pool` | GET | Vue du pool (états, erreurs, pauses) |
| `/admin/requalifier` | POST | Remet les cibles en test |
| `/admin/rafraichir` | POST | Recharge les clés et les adresses |

- Authentification `Authorization: Bearer …` ou `X-API-Key`, comparaison à temps constant.
- **Fail-closed** : sans jeton configuré, toute requête reçoit 401.
- **`Host` validé** : seuls `127.0.0.1[:port]` et `localhost[:port]` sont acceptés,
  tout autre en-tête reçoit 403 `hote_refuse` (protection contre le DNS rebinding).
- `/health` ne rend que `status` et `version` sans jeton ; l'interface n'en dépend
  pas (elle utilise la commande Tauri `etat`).
- Erreurs amont **rédigées avant affichage** : les clés connues, les jetons
  `Bearer …` et les paramètres d'URL sensibles (`api_key`, `token`, `secret`,
  `auth`, `pass`, `sig`, `credential`) sont remplacés par `***` dans la vue
  exposée à l'interface (`erreur` et `detail`).
- Port occupé : l'interface affiche « Le port 3100 est déjà utilisé par une autre
  application. Ferme-la, puis relance. » ; la cause technique (`os error 10048`)
  reste dans `journal.log`.
- Alias : `gratuit`, `gratuit-secours`, `defaut`, `rapide`.
- Bascule ordonnée selon l'alias, au plus 3 cibles (1 tentative + 2 bascules).
- Budget : 45 s pour les tentatives/bascules, 300 s maximum par tentative.
  Le flux a ensuite son propre budget de 900 s, qui démarre après les bascules :
  le pire cas est donc 45 s + 900 s = 945 s, et non 900 s pour la requête
  complète.
- Un fournisseur ajouté depuis l'interface est immédiatement disponible : ses
  modèles sont ajoutés à l'alias `defaut` sans redémarrage.
- Diagnostic : commande `diagnostic` (interface) qui renvoie un JSON complet
  (version, port, fournisseurs, alias, 80 dernières lignes de journal). Aucune
  valeur de clé, aucun entête d'authentification et aucune query string n'y
  figure : les secrets connus sont remplacés par `***` et les URL sont coupées
  avant `?` / `#`. Un test unitaire échoue si une clé apparaît.
- Flux SSE borné : 120 s d'inactivité, 900 s au total, puis événement d'erreur
  au format OpenAI et fermeture.
- Aucune redirection amont suivie (`Policy::none` sur les deux clients HTTP).
- Un 401/403 condamne toutes les cibles du fournisseur (« clé invalide ») ;
  un 404 ne condamne que la cible ; un 429 ou 5xx met la cible en pause 60 s.
- Corps limité à 16 MiB, 8 requêtes amont simultanées (sémaphore).
- Sémaphore plein : réponse immédiate 503 `gateway_busy`, sans mise en file
  d'attente.
- `X-Request-Id` (8 caractères hexadécimaux) sur chaque réponse et dans chaque
  ligne de journal rattachée à une requête.

### Interface (fenêtre unique, Tauri 2 / WebView2)

- État du service, démarrage/arrêt, port, jeton, copie de la configuration client.
- Adresse du proxy copiable par double-clic, avec confirmation « copié ».
- Bouton « Copier le diagnostic » (JSON, sans clé).
- Ajout d'un fournisseur (nom, adresse, clé, modèles) avec test de clé avant
  enregistrement ; écriture atomique dans `cles.env`, sauvegarde `.bak`,
  commentaires conservés, clé jamais journalisée.
- Fournisseurs et clés : saisie, test, effacement avec double confirmation (2,5 s).
- Alias et cibles : clic sur un modèle = modèle par défaut, étoile, persistance
  dans `cles.env` (`PROXY_MODELE_PREFERE`). Double-clic = plus de défaut
  (« aucun défaut »), repli sur le premier modèle joignable.
- Au démarrage : aucun test réseau, l'état de la dernière session est repris
  tel quel. Un candidat repris n'est jamais présenté comme joignable (badge
  « repris », « dernier état connu »). Une passe de fond unique, 60 s après le
  lancement, ne teste que les cibles jamais vues (4 en parallèle, 5 s chacune).
  Un échec d'appel réel, ou le bouton « Vérifier », déclenche une vérification.
- Modèle non joignable (panne, pause, épuisé) : badge « non joignable », séparé
  de l'état par « · », et infobulle avec la dernière erreur (rédigée) et son
  ancienneté ; la sélection reste possible.
- Panneau fournisseurs : une seule ligne par modèle même s'il est déclaré par
  plusieurs alias ; la provenance (liste des alias) reste disponible.
- « Aucun modèle disponible » ne s'affiche que si la liste est réellement vide.
- Confirmation d'ajout nominative (« Fournisseur Mistral ajouté. ») et message
  d'échec de clé sans répétition du code HTTP.
- Journal du démarrage en ms : `config`, `pool`, `serveur`, `interface`, `total`.
- Journal, pool (requalifier), démarrage avec Windows.
- Bandeau fixe en haut, infobulles personnalisées, FR/EN.
- Super IA : case désactivée, mention « en cours de développement » (prévue
  pour la V2, sans date), dans le panneau de droite sous les jetons d'accès
  (« Gestion supérieure »).

### Configuration

- `cles.env` (UTF-8 BOM, CRLF) : `PROXY_PORT`, `PROXY_MASTER_KEY`,
  `PROXY_MODELE_PREFERE`, `PROXY_FOURNISSEURS`, `OPENROUTER_KEY`, `DEEPSEEK_KEY`,
  `NVIDIA_KEY`, `*_URLS`, `*_MODELES`.
- Fournisseur ajouté depuis l'interface : format de stockage exact, tout en
  majuscules pour les noms de variables.
  - `PROXY_FOURNISSEURS` : liste des noms ajoutés, séparés par `;`.
  - `<NOM>_URLS` : une ou plusieurs adresses, séparées par `,` ou `;`.
  - `<NOM>_MODELES` : identifiants de modèles, séparés par `,` ou `;`.
  - `<NOM>_KEY` : la clé, en clair (aucun chiffrement, voir plus bas).
  - Le nom saisi est normalisé : minuscules, espaces/`-`/`.` remplacés par `_`,
    2 à 32 caractères, commence par une lettre, `[a-z0-9_]` uniquement.
  - Écriture atomique (`.env.tmp` puis renommage), sauvegarde `cles.env.bak`
    prise une seule fois avant l'ajout, commentaires et autres variables
    conservés.
- `%APPDATA%\ApiVault_Proxy\` : `journal.log`, état du pool.
- Surcharge du chemin de configuration par `APIVAULT_CLE_ENV` (utilisée par les
  tests d'intégration).
- Icônes : `outils/generer-icones.ps1` est obsolète, protégé par `-AncienLogo` ;
  le logo officiel vient de `src-tauri/icons/icon.svg`.

## Ce qui n'est PAS implémenté

- Coffre chiffré (PBKDF2 + AES-256-GCM) : les clés restent en clair dans
  `cles.env`, protégées par les ACL du compte Windows.
- DPAPI : aucun déverrouillage automatique n'est câblé (voir le compromis décrit
  dans `SPECIFICATION.md`, section 9).
- Quotas, coûts, plafonds par jeton.
- Les sept onglets de la spécification : l'interface est une page unique.
- Apprentissage / classes d'équivalence automatiques.
- `/v1/messages` (API Anthropic) et `/v1/embeddings`.
- Super IA.

## Commandes

```powershell
# Tests (unitaires + intégration, ~50 s à cause du test de budget 45 s)
cd Proxy-Externe\src-tauri
cargo test --release

# Un seul fichier de tests d'intégration
cargo test --release --test integration

# Tests de sécurité (Host, /health, port occupé)
cargo test --release --test lot3

# Compilation de production
cargo build --release
```

## Compteurs au 18/10/2026

- 27 tests unitaires (modules `config`, `pool`, `serveur`).
- 7 tests d'intégration (`tests/integration.rs`) sur la passerelle réelle.
- 3 tests de sécurité et de disponibilité (`tests/lot3.rs`) : `/health` minimal,
  `Host` refusé, port occupé.
- Exécutable release : voir la taille mesurée dans le rapport de sprint.
