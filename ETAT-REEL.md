# ETAT-REEL — ApiVault_Proxy

Document de référence factuel : ce qui existe vraiment dans le dépôt, à la date
du 18/10/2026, après le sprint 0 d'assainissement.

## Version

- Version applicative : **0.4.1** (`Cargo.toml`, `tauri.conf.json`, en-tête `/health`).
- Binaire : `Proxy-Externe/src-tauri/target/release/apivault-proxy.exe`.
- Crate : `apivault-proxy` (nom technique), produit : `ApiVault_Proxy`.
- Édition Rust : 2021.

## Ce qui est implémenté

### Passerelle HTTP (127.0.0.1 uniquement)

| Route | Méthode | Rôle |
|---|---|---|
| `/` | GET | Présentation en texte |
| `/health`, `/health/liveliness`, `/health/readiness` | GET | État, version, alias, cibles |
| `/v1/models`, `/models` | GET | Liste des alias au format OpenAI |
| `/v1/models/{id}` | GET | Détail d'un alias |
| `/v1/chat/completions`, `/chat/completions` | POST | Appel avec bascule automatique |
| `/admin/pool` | GET | Vue du pool (états, erreurs, pauses) |
| `/admin/requalifier` | POST | Remet les cibles en test |
| `/admin/rafraichir` | POST | Recharge les clés et les adresses |

- Authentification `Authorization: Bearer …` ou `X-API-Key`, comparaison à temps constant.
- **Fail-closed** : sans jeton configuré, toute requête reçoit 401.
- Alias : `gratuit`, `gratuit-secours`, `defaut`, `rapide`.
- Bascule ordonnée selon l'alias, au plus 3 cibles (1 tentative + 2 bascules).
- Budget : 45 s pour les tentatives/bascules, 300 s maximum par tentative.
  Le flux a ensuite son propre budget de 900 s, qui démarre après les bascules :
  le pire cas est donc 45 s + 900 s = 945 s, et non 900 s pour la requête
  complète.
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
- Fournisseurs et clés : saisie, test, effacement avec double confirmation (2,5 s).
- Alias et cibles : clic sur un modèle = modèle par défaut, étoile, persistance
  dans `cles.env` (`PROXY_MODELE_PREFERE`).
- Journal, pool (requalifier), démarrage avec Windows.
- Bandeau fixe en haut, infobulles personnalisées, FR/EN.
- Super IA : case désactivée, mention « bientôt ».

### Configuration

- `cles.env` (UTF-8 BOM, CRLF) : `PROXY_PORT`, `PROXY_MASTER_KEY`,
  `PROXY_MODELE_PREFERE`, `OPENROUTER_KEY`, `DEEPSEEK_KEY`, `NVIDIA_KEY`,
  `*_URLS`.
- `%APPDATA%\ApiVault_Proxy\` : `journal.log`, état du pool.
- Surcharge du chemin de configuration par `APIVAULT_CLE_ENV` (utilisée par les
  tests d'intégration).

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

# Compilation de production
cargo build --release
```

## Compteurs au 18/10/2026

- 20 tests unitaires (modules `config`, `pool`, `serveur`).
- 7 tests d'intégration (`tests/integration.rs`) sur la passerelle réelle.
- Exécutable release : voir la taille mesurée dans le rapport de sprint.
