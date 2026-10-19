# ApiVault Proxy 1.0.2 — Audit fixes

[Français](RELEASE-NOTES-v1.0.2.md) | **English**

Corrective release from the external audit. No business feature change; fixes for
consistency, security and performance.

## Fixes

- `/v1/models` and `/v1/models/{id}` use effective aliases: a provider added from
  the UI is visible without a restart.
- Upstream addresses must use **HTTPS**; `http://` is accepted only for
  `127.0.0.1` / `localhost`.
- The monthly cap is kept in memory after the first load, instead of re-reading the
  whole month's JSONL on every request.
- `masquer()` is safe with non-ASCII characters.
- The token is checked before acquiring the upstream semaphore.
- Token comparison no longer returns early on a length mismatch.
- `journal.log` is rotated to `journal.log.1` above 2 MB; known keys are redacted
  before writing.
- `X-Request-Id` moves to 16 hexadecimal characters.

## Documented limitation

Keys are not zeroized in memory. Protection remains that of a Windows session;
this is now explicit in `ETAT-REEL`.

## Verification

- 88 tests, 0 failures.
- Existing non-regression acceptance kept.
- Archive: `apivault-proxy-1.0.2-win64.zip` — **3,444,874 bytes**.

## Checksums

```
SHA-256  apivault-proxy-1.0.2-win64.zip   5E39AA857DDE1609C72DF49430F566ADD13056F3287F463D66028D7FBCBCC63E
```
