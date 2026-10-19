# ApiVault Proxy

[Français](README.md) | **English**

**A local, OpenAI-compatible gateway that keeps your API keys on your machine.**

ApiVault Proxy runs in the background on Windows, listens only on
`127.0.0.1:3100`, and exposes a single address to your tools:

```
base_url = http://127.0.0.1:3100/v1
```

Plug in a client, an agent, an extension or a script, and you never touch an API
key inside that client again. Keys live in an encrypted vault on your disk. The
proxy selects the model, fails over when a provider does not respond, counts what
you consume, and can cut off at a monthly cap you set.

No Docker, no Python, no Node, no system service. One executable.

---

## Screenshots

| | |
|:--:|:--:|
| ![Main window](docs/captures/01-fenetre-principale.png) | ![Management panel](docs/captures/02-panneau-gestion.png) |
| **The control window** — service state, reachable models, token | **Management** — keys, endpoints, per-provider test |
| ![Usage](docs/captures/03-consommation.png) | ![Monthly cap](docs/captures/04-plafond.png) |
| **Monthly usage** — input, output, cache, estimated requests | **Monthly cap** — gauge and alert at 80% |
| ![Badges](docs/captures/05-badges.png) | ![Providers](docs/captures/06-fournisseurs.png) |
| **Badges** — free / paid, default model | **States** — reachable, waiting, down |

> Screenshots live in [`docs/captures/`](docs/captures/). See the
> [screenshot guide](docs/captures/LISEZ-MOI.en.md).

---

## What it does

- **A single entry point** — `http://127.0.0.1:3100/v1`, OpenAI-compatible.
- **Encrypted vault** — `cles.env` is encrypted at rest with DPAPI (Windows user
  scope). An encrypted `.bak` backup is kept before every change. No key in clear
  text on disk.
- **Provider pool** — several providers and several models, with a `defaut` alias
  that tries targets in order and fails over automatically on error.
- **Persistent states** — a failing target moves to waiting or down; the state
  survives restarts, and nothing is paid for in a loop.
- **Token counter** — input, output and cache, per request, aggregated by day,
  week and month. A monthly append-only journal (`metriques-AAAA-MM.jsonl`).
- **Monthly cap** — checked **before** the provider call, rejected with `429`
  `plafond_atteint`, alert at 80%. Disabled by default.
- **FR / EN interface**, tooltips, start with Windows, notification icon whose
  colour follows the real service state.
- **Nothing leaves the machine** — the service listens only on the loopback.

---

## Quick start

1. Download the archive from the [latest release](../../releases/latest) and
   unpack it wherever you want.
2. Run `Demarrer.bat`. The control window opens; the service starts.
3. In **Management**, enter your first API key. It is written encrypted to
   `cles.env` and applied immediately, without a restart.
4. Click **Copy client configuration**: the address, token and default model are
   copied to the clipboard.
5. Paste that into your AI client, in place of the provider address.

| File | Purpose |
|---|---|
| `Demarrer.bat` | starts the gateway and opens the window |
| `Etat.bat` | checks the service, token and aliases |
| `Rafraichir.bat` | re-queries every provider |
| `Changer-Cle.bat` | replaces a key outside the UI |
| `Arreter.bat` | stops the gateway |
| `Autostart-Activer.bat` | silent launch at sign-in |

Closing the window does not stop the service: it keeps running in the background.

---

## API

A token is required on `/admin/*`, supplied as `Authorization: Bearer <token>`
(the value is the `PROXY_MASTER_KEY` line in `cles.env`).

| Route | Method | Purpose |
|---|---|---|
| `/health` | GET | service state and version, no token |
| `/v1/models`, `/models` | GET | available aliases, OpenAI format |
| `/v1/models/{id}` | GET | details of one alias |
| `/v1/chat/completions` | POST | relay, SSE stream or complete response |
| `/admin/pool` | GET | detailed alias and target state |
| `/admin/metriques` | GET | usage for the period and cap |
| `/admin/rafraichir` | POST | reset then live query |
| `/admin/requalifier` | POST | reset only, all or one target |

---

## What the vault protects — and what it does not

**It protects against:** another Windows account on the same machine, a stolen or
copied disk, a backup left lying around, a `cles.env` file read by mistake (it is
encrypted), an AI client that would keep your key in clear text in its
configuration file.

**It does not protect against:** a program running **under your own Windows
account** — DPAPI would let it decrypt the file. Full administrator access to your
session, a keylogger, or an already-compromised machine are not covered.

This is a deliberate choice: no master password to type at every start, so no
friction, but protection stops at the boundary of your Windows session.

The service listens only on `127.0.0.1`. It checks the `Host` header to protect
against DNS rebinding, and rejects unauthenticated requests to the administration
routes.

---

## Counter and cap

Every relayed request writes **one line** to
`%APPDATA%\ApiVault_Proxy\metriques-AAAA-MM.jsonl`:

```json
{"horodatage":1760800000,"alias":"defaut","cible":"deepseek","modele":"deepseek-chat",
 "tokens_in":1240,"tokens_out":318,"cache":0,"estime":false,"flux":true,"statut":200}
```

The journal is **append-only**: no counter is reset, and the day / week / month
views are aggregations of the same file.

When a provider does not send its usage (common in streaming), the line is marked
`"estime": true` and tokens are estimated from the actual bytes. The UI reports
it. A zero counter would be a lie; a flagged estimate is not.

The cap is global and monthly, expressed in tokens (`PROXY_PLAFOND_MENSUEL`). It
is checked before the upstream call. If reached, the request is rejected with
`429` and the code `plafond_atteint`, without any provider call.

---

## Tests

```powershell
cd Proxy-Externe\src-tauri
cargo test
```

**88 tests**: 30 unit, 9 integration, plus the security and feature suites
(`lot3` to `lot7`). None touches a real key: tests use fake values and local fake
providers.

The execution recipe against the real binary (sandboxed `APPDATA`, local fake
provider) is described in
[`PROTOCOLE-TEST-UTILISATEUR.en.md`](PROTOCOLE-TEST-UTILISATEUR.en.md) — 42 steps,
reproducible by anyone.

---

## Structure

```
ApiVault_Proxy/
├── Proxy-Externe/          the gateway (Rust) and its UI (Tauri 2)
│   ├── src/index.html      control UI, FR / EN
│   ├── src-tauri/src/      server, pool, vault, metrics, configuration
│   ├── src-tauri/tests/    integration and security suites
│   └── *.bat               start, status, stop, key, autostart
├── docs/captures/          screenshots
├── ETAT-REEL.md            what actually exists, without promises
├── SPECIFICATION.md        original specification (intent document)
└── PROTOCOLE-TEST-UTILISATEUR.md
```

---

## What is not there yet

Said plainly, so nobody discovers it after installing:

- no master password: protection stops at your Windows session;
- no per-alias quotas and no prices in euros — only the global cap exists;
- no Anthropic API (`/v1/messages`) and no `/v1/embeddings`;
- no automatic learning of equivalent models;
- Super IA is not there.

The original specification describes the intent; two points changed along the way
(the vault uses DPAPI, not PBKDF2/AES; audio is not implemented).
[`ETAT-REEL.en.md`](ETAT-REEL.en.md) is the factual reference.

---

## License

MIT — see [LICENSE](LICENSE).
