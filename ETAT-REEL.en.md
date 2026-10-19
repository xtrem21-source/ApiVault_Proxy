# ETAT-REEL — ApiVault_Proxy

[Français](ETAT-REEL.md) | **English**

Factual reference document: what actually exists in the repository, as of
2026-10-19, after lots 4 to 7 bis and the internal acceptance run.

## Version

- Application version: **1.0.0** (`Cargo.toml`, `tauri.conf.json`, `/health` header).
- Binary: `Proxy-Externe/src-tauri/target/release/apivault-proxy.exe`.
- Crate: `apivault-proxy` (technical name), product: `ApiVault_Proxy`.
- Rust edition: 2021.

## What is implemented

### HTTP gateway (127.0.0.1 only)

| Route | Method | Purpose |
|---|---|---|
| `/` | GET | Text presentation |
| `/health`, `/health/liveliness`, `/health/readiness` | GET | Public probe: `status` + `version` only, no token |
| `/v1/models`, `/models` | GET | Alias list in OpenAI format |
| `/v1/models/{id}` | GET | Alias details |
| `/v1/chat/completions`, `/chat/completions` | POST | Call with automatic failover |
| `/admin/pool` | GET | Pool view (states, errors, pauses) |
| `/admin/requalifier` | POST | Puts targets back in test |
| `/admin/rafraichir` | POST | Reloads keys and addresses |
| `/admin/metriques?periode=AAAA-MM` | GET | Input/output/cache totals for a period (master token) |

- `Authorization: Bearer …` or `X-API-Key` authentication, constant-time comparison.
- **Fail-closed**: without a configured token, every request receives 401.
- **`Host` validated**: only `127.0.0.1[:port]` and `localhost[:port]` are accepted;
  any other header receives 403 `hote_refuse` (DNS rebinding protection).
- `/health` returns only `status` and `version` without a token; the UI does not
  depend on it (it uses the Tauri `etat` command).
- Upstream errors **redacted before display**: known keys, `Bearer …` tokens and
  sensitive URL parameters (`api_key`, `token`, `secret`, `auth`, `pass`, `sig`,
  `credential`) are replaced by `***` in the view exposed to the UI (`erreur` and
  `detail`).
- Port busy: the UI shows "Port 3100 is already used by another application.
  Close it, then restart."; the technical cause (`os error 10048`) stays in
  `journal.log`.
- Aliases: `gratuit`, `gratuit-secours`, `defaut`, `rapide`.
- Failover ordered by alias, at most 3 targets (1 attempt + 2 failovers).
- Budget: **900 s total**, **120 s maximum per attempt** (including first byte),
  at most 3 targets. An attempt timeout no longer cuts the request: if the global
  budget allows, it fails over to the next target; 504 only when the budget is
  exhausted (LOT 7, M-3). These two delays can be overridden by
  `APIVAULT_DELAI_CIBLE_MS` / `APIVAULT_BUDGET_BASCULE_MS`, for tests only (without
  the variables, the values above apply). The stream then has its own 900 s
  budget, starting after failovers: worst case is therefore 900 s + 900 s =
  1800 s, not 900 s for the whole request.
- A provider added from the UI is immediately available: its models are added to
  the `defaut` alias without a restart.
- Diagnostic: `diagnostic` command (UI) returns full JSON (version, port,
  providers, aliases, last 80 log lines). No key value, no authentication header
  and no query string appears: known secrets are replaced by `***` and URLs are
  cut before `?` / `#`. A unit test fails if a key appears.
- Bounded SSE stream: 120 s inactivity, 900 s total, then an OpenAI-format error
  event and close.
- No upstream redirect followed (`Policy::none` on both HTTP clients).
- A 401 condemns all targets of the provider ("invalid key", manual fix); a 403
  only pauses the target temporarily, resumed by `reconcilier` (LOT 7, m-3); a 404
  condemns only that target; a 429 or 5xx pauses the target for 60 s.
- Body limited to 16 MiB, 8 simultaneous upstream requests (semaphore).
- Semaphore full: immediate 503 `gateway_busy`, no queueing.
- `X-Request-Id` (8 hexadecimal characters) on every response and in every log
  line attached to a request.

### Token counter (LOT 5)

Measurement and view only: no cap, no blocking, no real spending.

- File `%APPDATA%\ApiVault_Proxy\metriques-AAAA-MM.jsonl`: one JSON line per
  served request, appended at end of file, one file per month.
- **Public and stable format**: LOT 6 reads the same file without migration. Fields
  are added, none removed or renamed.
- Fields, in write order: `id`, `horodatage`, `jeton`, `alias`, `fournisseur`,
  `modele`, `statut`, `latence_ms`, `tokens_in`, `tokens_out`, `tokens_cache`,
  `cout_eur`, `estime`, `tentative`, `flux`.
- `tokens_cache` = `prompt_cache_hit_tokens` (DeepSeek) or
  `cache_read_input_tokens` (Anthropic), 0 when the provider does not report it.
  `cout_eur` remains `null`: no price catalogue exists yet.
- **Estimated mode** (`estime` = `true`): as soon as the provider does not return
  usable `usage`, counters are estimated instead of being set to zero. Single
  rule, 4 bytes per token, rounded up:
  `tokens_in` = bytes of the body sent to the provider / 4,
  `tokens_out` = relayed bytes (stream) or response bytes (non-stream) / 4.
  Zero only when no bytes circulated. An estimated line is therefore never zero
  while text circulated.
  Estimated mode covers two cases: response without a `usage` block, and stream
  broken before the `usage` event (the relayed-byte counter is cumulative, it does
  not depend on the 64 KiB sliding buffer). For a stream without `usage`, the line
  is written when the stream closes, including when the client disconnects.
- SSE stream: the final `usage` is recovered from stream events (64 KiB sliding
  buffer) and the line is written only when the stream closes.
- Aggregation: `metriques::totaliser_fichier(chemin, periode)` is a pure function
  (no interface, no network) that returns input, output, cache, requests,
  estimates and unreadable lines, plus detail by alias and by model. Reusable as
  is by LOT 6.
- `/admin/metriques?periode=AAAA-MM` (master token required) returns these totals;
  missing period = current month, malformed period = 400 `periode_invalide`.
- UI: Tauri `metriques` command (same aggregation) and "Monthly usage" panel
  (requests, input, output, cache, detail).

### Monthly cap (LOT 6)

- Unit: `tokens_in + tokens_out` for the current month. Cache is already included
  in input as returned by the provider: this is conservative, it is billed
  cheaper.
- Global, monthly scope, no per-alias cap. Setting `PROXY_PLAFOND_MENSUEL` in the
  key file (therefore encrypted), 0 = disabled, disabled by default.
- Advance check: before any upstream call, including streaming. If
  `used >= cap`, response 429 `plafond_atteint` with used / cap / period, and
  **no** provider call. The rejected request writes no metrics line.
- Deliberate fail-open: an unreadable journal does not block; the journal says
  "cap not verified". `/health` and `/admin/*` are never blocked.
- No reservation: requests already in flight can exceed the cap.
- Alert at 80%: `/admin/metriques` adds `plafond`, `utilise`, `pourcentage` and
  `niveau` (`desactive` / `ok` / `alerte` / `atteint`). "Monthly cap" card in the
  UI: field, gauge, state, caution text.
- Tauri `sauver_plafond` command: writes the setting then applies it without
  restarting the gateway.

### Interface (single window, Tauri 2 / WebView2)

- Service state, start/stop, port, token, copy of the client configuration.
- Proxy address copyable by double-click, with "copied" confirmation.
- "Copy diagnostic" button (JSON, no key).
- Add a provider (name, address, key, models) with key test before saving;
  atomic write to `cles.env`, `.bak` backup, comments preserved, key never logged.
- Providers and keys: input, test, erase with double confirmation (2.5 s).
- Aliases and targets: clicking a model = default model, star, persistence in
  `cles.env` (`PROXY_MODELE_PREFERE`). Double-click = no default ("no default"),
  fallback to the first reachable model.
- At startup: no network test, the last session state is resumed as is. A resumed
  candidate is never presented as reachable ("resumed", "last known state" badge).
  A single background pass, 60 s after launch, tests only never-seen targets (4 in
  parallel, 5 s each). A real call failure, or the "Check" button, triggers a
  check.
- Unreachable model (down, paused, exhausted): "unreachable" badge, separated from
  the state by " · ", and tooltip with the last (redacted) error and its age;
  selection remains possible.
- Provider panel: one row per model even if declared by several aliases; the
  provenance (alias list) remains available.
- "No model available" is shown only if the list is really empty.
- Named add confirmation ("Provider Mistral added.") and key failure message
  without repeating the HTTP code.
- Startup log in ms: `config`, `pool`, `serveur`, `interface`, `total`.
- Journal, pool (requalify), start with Windows.
- Fixed top bar, custom tooltips, FR/EN.
- Super IA: disabled checkbox, "under development" mention (planned for V2,
  no date), in the right panel under the access tokens ("Higher management").

### Configuration

- `cles.env` (UTF-8 BOM, CRLF): `PROXY_PORT`, `PROXY_MASTER_KEY`,
  `PROXY_MODELE_PREFERE`, `PROXY_FOURNISSEURS`, `OPENROUTER_KEY`, `DEEPSEEK_KEY`,
  `NVIDIA_KEY`, `*_URLS`, `*_MODELES`.
- Provider added from the UI: exact storage format, variable names in uppercase.
  - `PROXY_FOURNISSEURS`: list of added names, separated by `;`.
  - `<NOM>_URLS`: one or more addresses, separated by `,` or `;`.
  - `<NOM>_MODELES`: model identifiers, separated by `,` or `;`.
  - `<NOM>_KEY`: the key, encrypted on disk (see "Vault" below).
  - The entered name is normalised: lowercase, spaces/`-`/`.` replaced by `_`,
    2 to 32 characters, starts with a letter, `[a-z0-9_]` only.
  - Atomic write (`.tmp` then rename), `cles.env.bak` backup taken once before the
    addition, comments and other variables preserved; the backup is encrypted too.
- `%APPDATA%\ApiVault_Proxy\`: `journal.log`, pool state,
  `metriques-AAAA-MM.jsonl` (one line per served request).
- Configuration path override via `APIVAULT_CLE_ENV` (used by integration tests).
- Icons: `outils/generer-icones.ps1` is obsolete, protected by `-AncienLogo`; the
  official logo comes from `src-tauri/icons/icon.svg`.

### Vault (LOT 4)

- `cles.env` is encrypted with DPAPI, **user** scope: the file starts with the line
  `APIVAULT-COFFRE-1` followed by a base64 blob. A file without this header is read
  as clear text (migration) then rewritten encrypted.
- Read by `coffre::lire_texte`, writes always encrypted and atomic (`.tmp` then
  rename). Encrypted `.bak`, removed on rotation or when a key is erased.
- At startup, `config::charger_et_migrer` rewrites an unencrypted `cles.env` as
  encrypted, **only** if it lives in `%APPDATA%\ApiVault_Proxy\`. An external file
  (`APIVAULT_CLE_ENV`) is never touched and the journal says
  "external key file: migration not applied". Atomic write: on failure, the old
  file remains in place.
- At startup, `icacls` restricts the key folder to the current account, but
  **only** if `cles.env` is in `%APPDATA%\ApiVault_Proxy\`; an external key file
  (`APIVAULT_CLE_ENV`) gets the log "rights not applied: external key file". A
  second pass grants explicit rights to existing files: without it, `/T` with the
  inherited `(OI)(CI)F` rule made them unreadable for their own account.
- Honest limitation: DPAPI binds encryption to the Windows account, not to a
  password. Another account or a copy outside the session cannot decrypt, but a
  process running under the same account can read the keys; freed SSD blocks
  (TRIM, snapshots) may retain old versions.

## What is NOT implemented

- Master-password vault (PBKDF2 + AES-256-GCM): current encryption is user DPAPI,
  with no manual unlock.
- Per-alias quotas, costs in euros and per-model prices (`cout_eur` remains
  `null`); only the global monthly cap exists.
- The seven tabs of the specification: the UI is a single page.
- Automatic learning / equivalence classes.
- `/v1/messages` (Anthropic API) and `/v1/embeddings`.
- Super IA.

## Commands

```powershell
# Tests (unit + integration, ~5 s: failover delays are shortened by
# APIVAULT_DELAI_CIBLE_MS / APIVAULT_BUDGET_BASCULE_MS)
cd Proxy-Externe\src-tauri
cargo test --release

# One integration test file
cargo test --release --test integration

# Security tests (Host, /health, busy port)
cargo test --release --test lot3

# Vault and keys on disk (LOT 4)
cargo test --release --test lot4

# Token counter (LOT 5)
cargo test --release --test lot5

# Migration and monthly cap (LOT 6)
cargo test --release --test lot6

# Production build
cargo build --release
```

## Counts as of 2026-10-18

- 29 unit tests (modules `config`, `coffre`, `metriques`, `pool`, `serveur`).
- 7 integration tests (`tests/integration.rs`) against the real gateway.
- 3 security and availability tests (`tests/lot3.rs`): minimal `/health`, `Host`
  rejected, busy port.
- 12 vault tests (`tests/lot4.rs`): round-trip, clear-text migration, no secret in
  clear on disk, encrypted `.bak` then removed, corrupted header, rights
  restriction, external key file never restricted.
- 12 counter tests (`tests/lot5.rs`): stable format, aggregation by period, `usage`
  from the four providers, SSE end-of-stream `usage`, `/admin/metriques` (401
  without token, 400 invalid period), estimated upstream failure.
- 15 LOT 6 tests (`tests/lot6.rs`): startup migration of a clear-text file
  (internal, external, write failure, after loading), restriction that keeps files
  readable, cap disabled / just below / reached without upstream call / stream
  rejected and counted, 80% alert, unreadable journal (fail-open), setting
  persistence, previous month ignored, `/health` and `/admin/pool` never blocked.
- Total: 78, all green (`cargo test --release`).
- Release executable: see the measured size in the sprint report.
