# ApiVault Proxy 1.0.0 — October 19, 2026

[Français](RELEASE-NOTES-v1.0.0.md) | **English**

First public release. This is not a pre-release: the vault, pool, counter and cap
are delivered, audited and acceptance-tested.

## What it brings

**The gateway**
- OpenAI-compatible: `/v1/models`, `/v1/chat/completions`, streaming or complete
  response.
- Listens only on `127.0.0.1:3100`, checks the `Host` header against DNS
  rebinding, and protects administration routes with a token.
- `defaut` alias that tries targets in order and fails over on error.
- Persistent target state (reachable, waiting, down) that survives restarts.

**The vault**
- `cles.env` encrypted at rest (DPAPI, Windows user scope): no key in clear text
  on disk.
- Encrypted `.bak` backup before every change, removed after rotation.
- Automatic migration of an old clear-text `cles.env` at startup.
- Data-folder permissions restricted to the current account.

**Usage**
- One line per request in a monthly append-only journal.
- Input, output and cache tokens; day, week and month aggregates.
- Flagged estimate (`estime: true`) when the provider does not send its usage —
  never a misleading zero.
- Monthly cap checked **before** the upstream call, rejected with
  `429 plafond_atteint`, alert at 80%, disabled by default.

**The interface**
- French / English, light and dark mode, tooltips.
- Control window, management panel, usage, cap.
- Notification icon whose colour follows the real service state.
- Start with Windows, in silent mode.

## Installation

1. Download `apivault-proxy-1.0.0-win64.zip` below and unpack it.
2. Run `Demarrer.bat`.
3. In **Management**, enter an API key: it is encrypted immediately.
4. Click **Copy client configuration** and paste it into your AI client.

No prerequisites: no Docker, no Python, no Node, no system service.

## Verification

- **88** automated tests, 0 warnings.
- Independent audit (separate context, model different from the coder): 3 major
  findings found, fixed and re-verified; 3 false leads refuted.
- Execution acceptance on the real binary in a sandbox: **9/9** — effective
  encryption, `Host` rejected, administration protected, exact accounting in
  non-stream, non-zero estimate in streaming, cap without upstream call, journal
  tolerant to one corrupted byte, BOM accepted, no leftover process.

## Known limitations

- The vault protects against another Windows account or a stolen disk, **not**
  against a program running under your own session.
- No per-alias quota, no prices in euros: the cap is global and expressed in
  tokens.
- No Anthropic API (`/v1/messages`) and no `/v1/embeddings`.
- Cap rejections are not yet written to the journal.
- Changing the cap requires a restart.
- Audio transcription is not implemented.

## Checksums

```
SHA-256  apivault-proxy-1.0.0-win64.zip   0AC297645494DCF5712D3DA09D9D00CB8C2C41870379E1CBB72F94FA189271D2
SHA-256  apivault-proxy.exe              F4EED07585DD621B3C2267D661914B73F223E10E2AF01B40E7B6638A3252F0E8
```

Verification on Windows:

```powershell
Get-FileHash .\apivault-proxy-1.0.0-win64.zip -Algorithm SHA256
```
