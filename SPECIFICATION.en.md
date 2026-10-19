# ApiVault_Proxy — Specification

[Français](SPECIFICATION.md) | **English**

Document version: 0.1 — 2026-10-17
Status: intent document (2026-10-17). The implementation diverged on two points:
the vault uses DPAPI, not PBKDF2/AES, and audio is not implemented. See
[ETAT-REEL.en.md](ETAT-REEL.en.md).

---

## 1. One product, one gateway

ApiVault_Proxy is a standalone application: key vault + local OpenAI-compatible
gateway. The proxy is native; it does not rely on any external gateway.

- Model/key pool, equivalence classes, key rotation, quotas.
- Full interface, journal, costs.
- Goal: free volume, equivalent failover, spending control.

The AI client exposes only one setting: `base_url = http://127.0.0.1:3100/v1` and
the token provided by the gateway. No intelligence in the client: it would have
to be debugged twice.

---

## 2. Constraints

- Windows 10/11 x64. One executable, portable by default.
- No Docker, no Python, no Node, no system service.
- The vault reuses the ApiVault format (PBKDF2 + AES-256-GCM), with import of the
  existing encrypted `.vaultbackup`.
- No data leaves the machine, except requests to explicitly configured providers.
- The binary must remain under 12 MB.

---

## 3. Technical architecture

```
Tauri v2 (1 process)
├── Interface (HTML/JS vanilla, i18n FR/EN)
├── Vault            : PBKDF2-SHA256 + AES-256-GCM
├── Gateway          : axum (HTTP) + reqwest (upstream client, rustls)
├── Journal          : rusqlite (embedded SQLite)
├── Notification area: Tauri tray + single instance
└── Unlock           : password | convenience | DPAPI (Windows)
```

Added crates: `tokio`, `axum`, `reqwest` (rustls, no OpenSSL), `rusqlite`
(bundled), `zeroize`, `windows`, `tauri-plugin-single-instance`.

**Lifecycle**

- The server starts with the application if `passerelle.actif = true`.
- Closing the window does not quit: it minimises to the notification area.
- Real quit: stop the server, erase keys from memory, then exit.
- Single instance mandatory: two instances = port conflict.

**Key flow**

Unlock → the front end decrypts → IPC command `passerelle_definir_cles` → Rust
keeps the list in memory (`Mutex`, `Zeroize` on release) → never written to disk,
never logged.

---

## 4. Data model

### 4.1 Vault (encrypted) — version 3

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
          "quota_jour": 50,            // null = unknown
          "defaut": true,
          "statut": "sain",            // sain | en_pause | en_panne | epuise
          "statut_jusqua": null
        }
      ],
      "passerelle": {
        "actif": true,
        "type": "openai",              // openai | anthropic
        "base_url": "https://openrouter.ai/api/v1",
        "support_usage_stream": true   // include_usage accepted?
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
        "empreinte": "sha256:…",       // the clear key is shown only once
        "alias_autorises": ["defaut", "gratuit"],
        "plafond_jour_eur": null,
        "cree_le": "…"
      }
    ]
  }
}
```

### 4.2 Catalogue (clear text, non-secret)

`%APPDATA%\ApiVault_Proxy\catalogue.json` — model cache, TTL 6 h.
Contains: provider, id, input/output prices, context, modalities, tags, class,
level, state. No key.

### 4.3 Journal (SQLite)

`%APPDATA%\ApiVault_Proxy\journal.db`

```
requetes(id, horodatage, jeton, alias, fournisseur, modele, statut,
         latence_ms, tokens_in, tokens_out, cout_eur, estime, tentative)
```

Automatic purge at `journal_jours`. **Never the message content.**

**Implemented in v0.5 (LOT 5)** — the SQLite above remains the target, but the real
file is `metriques-AAAA-MM.jsonl`: one JSON line per served request, append-only,
one file per month. Fields: `id`, `horodatage`, `jeton`, `alias`, `fournisseur`,
`modele`, `statut`, `latence_ms`, `tokens_in`, `tokens_out`, `tokens_cache`,
`cout_eur`, `estime`, `tentative`, `flux`. This format is public and stable: fields
are added, none removed or renamed (LOT 6 reads the same file). No purge exists
yet; `cout_eur` remains `null`.
Aggregation is `metriques::totaliser_fichier`, exposed by `/admin/metriques`.

**Estimated mode (LOT 7)** — when the provider returns no usable `usage`, `estime`
is `true` and counters are **estimated**, never left at zero while text has
circulated: 4 bytes per token, rounded up, `tokens_in` = bytes of the sent body /
4, `tokens_out` = relayed bytes (stream) or response bytes (non-stream) / 4. Zero
only with no bytes at all. This covers responses without a `usage` block and
streams broken before the `usage` event (cumulative byte counter, independent of
the 64 KiB buffer). Estimated mode is declared in the line: totals always separate
estimated and exact requests.

**Implemented in v0.5 (LOT 6)** — the realised cap is not the `plafond_jour_eur`
above (no price exists yet) but a **monthly token cap**:
`tokens_in + tokens_out` for the month, set by `PROXY_PLAFOND_MENSUEL` (0 =
disabled, encrypted with the keys), checked before any upstream call, 429
`plafond_atteint` without a provider call, alert at 80% (`niveau` in
`/admin/metriques`). Fail-open if the journal is unreadable, without reservation.
Per-alias cap and cost in euros remain to be done.

---

## 5. The pool: a state machine (CloudProxy pattern)

Each `(provider, key, model)` triplet carries a state:

| State | Meaning | Exit |
|---|---|---|
| `sain` | usable | on error → `en_pause` / `en_panne` |
| `inconnu` | never tested | after test → `sain` or `en_panne` |
| `en_pause` | quota reached, retry later | at expiry → `inconnu` |
| `en_panne` | dead key or provider down | manual test or after 1 h → `inconnu` |
| `epuise` | credits exhausted (402) | daily reset or user action |

**Reconciliation loop** (every 30 s, `tokio` task):

1. expire `en_pause` / `epuise` that reached their deadline;
2. requalify `inconnu` with a light test (one-token request);
3. put `en_panne` back to `inconnu` after one hour, with exponential backoff;
4. check the invariant: **at least one `sain` candidate per used class**. If the
   invariant falls, show an alert in the UI — never leave an alias silently empty.

Nothing is deleted from the pool: a model that disappears from the catalogue moves
to `inconnu`; it is not erased (it may return).

**Manual reactivation (essential).**
A "Retry" button on a model, on a provider, and a "Requalify all" button on the
pool. It resets the state to `inconnu` and starts the test immediately, without
waiting for the deadline.

Reason: a two-minute Wi-Fi outage must not degrade a model for an hour. Automation
is there to avoid useless waiting, not to lock the user into a decision made for
them. The displayed state must always be contestable with one click.

---

## 6. Resolving a request

1. **Read the request** and detect required capabilities: `tools` present → tag
   `outils`; image in `content` → `vision`; `response_format: json` → `json`;
   estimate tokens → `contexte ≥ N`.
2. **Choose the alias**: `model` field. Unknown or `auto` → alias `defaut`.
   Nonexistent alias → 404 `model_not_found` with the alias list.
3. **Filter candidates**: same class, `niveau ≥ niveau_min`, all required tags
   present, state ≠ `en_panne` / `epuise`, key not exhausted.
4. **Order**: same model family first, then descending level, then free before paid
   (per policy), then descending remaining quota, then increasing median latency.
5. **Try** each candidate, and for each, each authorised key.
6. **Success**: stream, log, decrement quota.
7. **Failure**: classify the error (table §7), apply state, continue.
8. **Exhaustion**: OpenAI error `pool_exhausted`, with the list of attempts
   (provider, model, status) — not a vague message.

### Failover order (4 stages, never more)

| Stage | Attempt |
|---|---|
| 1 | same model, another key |
| 2 | same model, another provider |
| 3 | same class, free |
| 4 | same class, paid — if policy and cap allow |

**Point of no return**: after the first token sent to the client, no failover. End
with a clean error event.

---

## 7. Upstream errors → actions

| Response | Applied state | Action |
|---|---|---|
| 429 | key `en_pause` (1 min → quota reset) | next key |
| 401 | key `en_panne` | next key + alert |
| 403 | target `en_pause` (temporary, automatic resume) | next target |
| 402 | key `epuise` | next key |
| 404 | model `inconnu` | next model |
| 400 (tools/vision) | tag removed from model | next model |
| 400 (other) | — | **no retry**, error returned to client |
| 5xx / timeout / connection | provider `en_pause` 60 s | next model |
| 200 then 1st event = error | like 5xx | next model |

Circuit breaker: 3 consecutive failures on a provider → excluded for 60 s.
Budget: 2 alternatives maximum (3 targets tried), **900 s total**, **120 s per
attempt** including first byte. An attempt timeout does not stop the request: if
the global budget allows, fail over to the next target; only an exhausted budget
makes the gateway return 504.

---

## 8. Exposed API

| Method | Path | Notes |
|---|---|---|
| GET | `/health` | no authentication, for scripts |
| GET | `/v1/models` | returns **the aliases**, never the inventory |
| POST | `/v1/chat/completions` | streaming and non-streaming |
| POST | `/v1/embeddings` | v1.1 |
| POST | `/v1/audio/transcriptions` | v1.1 (Gemini from the vault) |
| POST | `/v1/messages` | v1.2 (Claude Code) |

Authentication: `Authorization: Bearer sk-apivault-…`, constant-time comparison on
the fingerprint. `/v1/models` makes **no upstream call**.

Error format:

```json
{ "error": {
    "message": "pool exhausted for alias « gratuit »",
    "type": "pool_exhausted",
    "code": "pool_exhausted",
    "attempts": [ { "fournisseur": "…", "modele": "…", "statut": 429 } ]
} }
```

Timeouts: connection 10 s, first byte 120 s, inactivity 300 s, total 900 s.
Maximum body: 8 MB (chat), 32 MB (audio).

---

## 9. Security

- Listen on **127.0.0.1 only** (no firewall rule, no Windows prompt).
- Token mandatory; no anonymous mode.
- Reject any request with a non-local `Origin` header (anti-DNS-rebinding).
- No CORS: no browser should be able to call the gateway.
- Keys erased from memory on lock (`zeroize`).
- Journal without content, without authentication header.
- Client tokens stored as fingerprints, displayed only once.
- Body limited, headers rejected above 16 KB.
- Automatic DPAPI unlock accepted as a compromise. Target protection: offline
  access. Out of scope: malicious code running under the user's account.

---

## 10. Interface — seven tabs

1. **State**: started/stopped, port, uptime, requests, last error.
2. **Providers**: active, URL, keys (type, quota, test, state), priority.
3. **Models**: inventory, filters, badges, refresh + diff.
4. **Aliases**: name, class, level, policy, drag-and-drop, default.
5. **Tokens**: project name, allowed aliases, cap, last call.
6. **Journal**: time, alias, real model, status, latency, tokens, cost.
7. **Settings**: port, Windows startup, unlock, cap, purge.

Plus a **mini test chat**, accessible from a discreet link: it checks an alias
without opening an AI client.

---

## 11. Default settings

| Setting | Value |
|---|---|
| Port | 3100 (explicit failure if busy, no silent switch) |
| Alias `defaut` | standard, level ≥ 3, free then paid |
| Alias `gratuit` | standard, level ≥ 3, free only |
| Alias `code` | code, paid only |
| Cap | €0.50/day |
| Journal | 30 days |
| Startup | Windows, minimised, DPAPI unlock |

---

## 12. Development plan

### v0.1 — foundation
Imported vault, 4 providers, **one** alias, `/v1/models`,
`/v1/chat/completions` (streaming + tools), single token.

*Acceptance*: a client sees the alias, a response arrives token by token, a tool
call round-trips, an upstream outage ends cleanly.

### v0.2 — the pool
Classes, levels, tags, multi-key, the 4 stages, state machine, reconciliation,
cost policy, cap.

*Acceptance*: 429 on the first key → second key, invisible to the client; 500 on a
model → failover within the same class, never outside it; cap reached → explicit
refusal, zero spending.

### v0.3 — operations
Full interface, journal, quotas, catalogue diff, key test, notification area,
Windows startup, DPAPI, purge.

*Acceptance*: restart Windows → gateway available without a password; the day's
costs are accurate to ±5%.

### v0.4 — operational interface
Permanent tooltips, double erase confirmation, default model selection
(persisted), fixed name/state/model bar, disabled "Super IA" slot, explicit alias
priority. Gemma 4 31B remains present but is a last resort (frequent 429s). Unit
tests on aliases, free/paid and pool order.

*Acceptance*: the five interface points are usable in FR and EN; the chosen model
is first in `defaut` after restart; a paid model does not pollute free aliases;
`cargo test --release` passes.

### v1.0 — learning
Capabilities learned from errors, reordering by measured latency, suggestions for
new free models.

---

## 13. Out of scope for v1

Semantic cache, prompt rewriting, multi-user, remote access, image generation,
`/v1/messages` (postponed to v1.2), billing.

---

## 14. Known risks

| Risk | Response |
|---|---|
| Free offers change every month | diff + alert, never an alias on a single model |
| "Equivalent" remains approximate | failover before the first token only |
| Some free providers train on your data | dedicated column in the UI |
| Antivirus false positive (exe opening a port) | signature, or documented exception |
| Two instances | single-instance plugin |
| Port busy | explicit failure, "choose another port" button |
