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
| `/admin/metriques?periode=AAAA-MM` | GET | Totaux entrée/sortie/cache d'une période (jeton maître) |

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

### Compteur de tokens (LOT 5)

Mesure et vue uniquement : aucun plafond, aucun blocage, aucune dépense réelle.

- Fichier `%APPDATA%\ApiVault_Proxy\metriques-AAAA-MM.jsonl` : une ligne JSON par
  requête servie, ajout en fin de fichier, un fichier par mois.
- **Format public et stable** : LOT 6 lit le même fichier sans migration. On
  ajoute des champs, on n'en retire ni n'en renomme aucun.
- Champs, dans l'ordre d'écriture : `id`, `horodatage`, `jeton`, `alias`,
  `fournisseur`, `modele`, `statut`, `latence_ms`, `tokens_in`, `tokens_out`,
  `tokens_cache`, `cout_eur`, `estime`, `tentative`, `flux`.
- `tokens_cache` = `prompt_cache_hit_tokens` (DeepSeek) ou
  `cache_read_input_tokens` (Anthropic), 0 quand le fournisseur n'en parle pas.
  `cout_eur` reste `null` : aucun catalogue de prix n'existe encore.
- **Mode estimé** (`estime` = `true`) : dès que le fournisseur ne rend pas
  d'`usage` exploitable, les compteurs sont estimés au lieu d'être mis à zéro.
  Règle unique, à 4 octets par token, arrondie au supérieur :
  `tokens_in` = octets du corps envoyé au fournisseur / 4,
  `tokens_out` = octets relayés (flux) ou octets de la réponse (hors flux) / 4.
  Zéro seulement quand aucun octet n'a circulé. Une ligne estimée n'est donc
  jamais nulle alors que du texte a circulé.
  Le mode estimé couvre deux cas : réponse sans bloc `usage`, et flux rompu
  avant l'événement d'`usage` (le compteur d'octets relayés est cumulatif, il
  ne dépend pas du tampon glissant de 64 KiB). Pour un flux sans `usage`, la
  ligne est écrite à la fermeture du flux, y compris quand le client coupe.
- Flux SSE : l'`usage` final est récupéré dans les événements du flux (tampon
  glissant de 64 KiB) et la ligne n'est écrite qu'à la fermeture du flux.
- Agrégation : `metriques::totaliser_fichier(chemin, periode)` est une fonction
  pure (aucune interface, aucun réseau) qui rend entrées, sorties, cache,
  requêtes, estimées et lignes illisibles, plus le détail par alias et par
  modèle. Réutilisable telle quelle par le LOT 6.
- `/admin/metriques?periode=AAAA-MM` (jeton maître requis) rend ces totaux ;
  période absente = mois courant, période mal formée = 400 `periode_invalide`.
- Interface : commande Tauri `metriques` (même agrégation) et panneau
  « Consommation du mois » (requêtes, entrées, sorties, cache, détail).

### Plafond mensuel (LOT 6)

- Unite : `tokens_in + tokens_out` du mois en cours. Le cache est deja compte
  dans l'entree comme le renvoie le fournisseur : c'est prudent, il est facture
  moins cher.
- Portee globale, mensuelle, sans plafond par alias. Reglage `PROXY_PLAFOND_MENSUEL`
  dans le fichier de cles (donc chiffre), 0 = desactive, desactive par defaut.
- Controle avance : avant tout appel amont, flux compris. Si `utilise >= plafond`,
  reponse 429 `plafond_atteint` avec utilise / plafond / periode, et **aucun**
  appel au fournisseur. La requete refusee n'ecrit pas de ligne de mesures.
- Fail-open assume : un journal illisible ne bloque pas, le journal indique
  « plafond non verifie ». `/health` et `/admin/*` ne sont jamais bloques.
- Pas de reservation : des requetes deja en vol peuvent depasser le plafond.
- Alerte a 80 % : `/admin/metriques` ajoute `plafond`, `utilise`, `pourcentage`
  et `niveau` (`desactive` / `ok` / `alerte` / `atteint`). Carte « Plafond
  mensuel » dans l'interface : champ, jauge, etat, texte de prudence.
- Commande Tauri `sauver_plafond` : ecrit le reglage puis l'applique sans
  redemarrer la passerelle.

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
  - `<NOM>_KEY` : la clé, chiffrée sur disque (voir « Coffre » ci-dessous).
  - Le nom saisi est normalisé : minuscules, espaces/`-`/`.` remplacés par `_`,
    2 à 32 caractères, commence par une lettre, `[a-z0-9_]` uniquement.
  - Écriture atomique (`.tmp` puis renommage), sauvegarde `cles.env.bak`
    prise une seule fois avant l'ajout, commentaires et autres variables
    conservés ; la sauvegarde est chiffrée elle aussi.
- `%APPDATA%\ApiVault_Proxy\` : `journal.log`, état du pool,
  `metriques-AAAA-MM.jsonl` (une ligne par requête servie).
- Surcharge du chemin de configuration par `APIVAULT_CLE_ENV` (utilisée par les
  tests d'intégration).
- Icônes : `outils/generer-icones.ps1` est obsolète, protégé par `-AncienLogo` ;
  le logo officiel vient de `src-tauri/icons/icon.svg`.

### Coffre (LOT 4)

- `cles.env` est chiffré avec DPAPI, portée **utilisateur** : le fichier
  commence par la ligne `APIVAULT-COFFRE-1` suivie d'un blob base64. Un fichier
  sans cet en-tête est lu comme du clair (migration) puis réécrit chiffré.
- Lecture par `coffre::lire_texte`, écriture toujours chiffrée et atomique
  (`.tmp` puis renommage). `.bak` chiffré, supprimé à la rotation ou à
  l'effacement d'une clé.
- Au démarrage, `config::charger_et_migrer` réécrit chiffré un `cles.env` encore
  en clair, **uniquement** s'il vit dans `%APPDATA%\ApiVault_Proxy\`. Un fichier
  externe (`APIVAULT_CLE_ENV`) n'est jamais touché et le journal indique
  « fichier de cles externe : migration non appliquee ». Écriture atomique : en
  cas d'échec, l'ancien fichier reste en place.
- Au démarrage, `icacls` restreint le dossier des clés au compte courant, mais
  **uniquement** si `cles.env` est dans `%APPDATA%\ApiVault_Proxy\` ; un fichier
  de clés externe (`APIVAULT_CLE_ENV`) reçoit le journal
  « droits non appliques : fichier de cles externe ». Un second passage donne un
  droit explicite aux fichiers déjà présents : sans lui, `/T` avec la règle
  héritée `(OI)(CI)F` les rendait illisibles pour leur propre compte.
- Limite honnête : DPAPI lie le chiffrement au compte Windows, pas à un mot de
  passe. Un autre compte ou une copie hors session ne déchiffre pas, mais un
  processus tournant sous le même compte lit les clés ; les blocs libérés d'un
  SSD (TRIM, clichés instantanés) peuvent conserver d'anciennes versions.

## Ce qui n'est PAS implémenté

- Coffre à mot de passe maître (PBKDF2 + AES-256-GCM) : le chiffrement actuel
  est DPAPI utilisateur, sans déverrouillage manuel.
- Quotas par alias, coûts en euros et prix par modèle (`cout_eur` reste `null`) ;
  seul le plafond global mensuel existe.
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

# Coffre et clés sur disque (LOT 4)
cargo test --release --test lot4

# Compteur de tokens (LOT 5)
cargo test --release --test lot5

# Migration et plafond mensuel (LOT 6)
cargo test --release --test lot6

# Compilation de production
cargo build --release
```

## Compteurs au 18/10/2026

- 29 tests unitaires (modules `config`, `coffre`, `metriques`, `pool`, `serveur`).
- 7 tests d'intégration (`tests/integration.rs`) sur la passerelle réelle.
- 3 tests de sécurité et de disponibilité (`tests/lot3.rs`) : `/health` minimal,
  `Host` refusé, port occupé.
- 12 tests du coffre (`tests/lot4.rs`) : aller-retour, migration du clair, aucun
  secret en clair sur disque, `.bak` chiffré puis supprimé, en-tête corrompu,
  restriction des droits, fichier de clés externe jamais restreint.
- 12 tests du compteur (`tests/lot5.rs`) : format stable, agrégation par
  période, `usage` des quatre fournisseurs, `usage` de fin de flux SSE,
  `/admin/metriques` (401 sans jeton, 400 période invalide), échec amont estimé.
- 15 tests du LOT 6 (`tests/lot6.rs`) : migration au démarrage d'un fichier en
  clair (interne, externe, échec d'écriture, après chargement), restriction qui
  garde les fichiers lisibles, plafond désactivé / juste en dessous / atteint
  sans appel amont / flux refusé et compté, alerte à 80 %, journal illisible
  (fail-open), persistance du réglage, mois précédent ignoré, `/health` et
  `/admin/pool` jamais bloqués.
- Total : 78, tous verts (`cargo test --release`).
- Exécutable release : voir la taille mesurée dans le rapport de sprint.
