# ApiVault Proxy 1.0.4 — Local providers, fallback port, readable refusal

[Français](RELEASE-NOTES-v1.0.4.md) | [English](RELEASE-NOTES-v1.0.4.en.md)

A practicality release. The API is unchanged; behaviour now adapts to three
situations that used to block or worry people for no good reason.

## Local providers (option)

- New option “Allow HTTP to my local network”, **off by default**. Once enabled,
  unencrypted HTTP is accepted to a literal private address: `10.x`, `172.16` to
  `31.x`, `192.168.x`, link-local, and `100.64.x` (Tailscale).
- Never to the internet, never to a domain name: without encryption the key
  would travel in clear text. `https://` remains accepted everywhere.
- The rule also applies to the “test” button: no key is ever sent to a refused
  address.
- For Ollama on a VPS: SSH tunnel, HTTPS reverse proxy, or Tailscale. Three safe
  paths, none of which requires disabling anything.

## Port in use

- If port 3100 is taken, the gateway tries 3101, 3102… up to 3109.
- The chosen port is **remembered**: the next start tries it first, so configured
  clients keep working.
- A **Windows notification** warns immediately, even with the window closed, and
  a banner in the window repeats the address to update.
- If all ten ports are taken, the message says what to do instead of merely
  reporting the failure.

## Cap refusal

`429 plafond_atteint` now carries the useful numbers:

```json
{"error":{"type":"insufficient_quota","code":"plafond_atteint",
"utilise":4820000,"plafond":5000000,
"periode":"2026-10","reinitialisation":"2026-11-01"}}
```

## Version check

- At startup, read the latest version published on GitHub.
- A single request, **no token and no data sent**, 5 seconds maximum, silent
  failure. No automatic download.
- Can be disabled in the settings.

## What the counter will never do

It counts **tokens**, never euros. Billing stays with the provider, the only one
that knows its prices and tiers. A figure computed here would only be a guess
dressed up as a measurement.

## Verification

- 88 tests, 0 failure.
- Real run: port 3100 busy → started on 3101, switch journaled and remembered;
  cap set to 10 → `429` with the four fields, no upstream call.
- Archive: `apivault-proxy-1.0.4-win64.zip` — **3,577,561 bytes**.

## Fingerprints

```
SHA-256  apivault-proxy-1.0.4-win64.zip   1ED8C64E5D9DD7BC982275E9D6106B29A9BB3C2D79A98A144E963E529FEE9F83
```
