# ApiVault Proxy 1.0.3 — Script and distribution fixes

[Français](RELEASE-NOTES-v1.0.3.md) | **English**

Corrective distribution release. No business API change.

## Script security

- `Etat`, `Rafraichir` and `Demarrer` check that the process listening on the port
  is really `apivault-proxy.exe` **before** sending the master token.
- `Changer-Cle` accepts any `<NOM>_KEY` provider, not only the three historical
  providers.
- `-Cle <value>` is removed: the key comes from `-CleFichier` or `-CleEnv`
  (temporary variable removed after reading).
- After rotation, `cles.env.bak` is deleted: a compromised key does not survive in
  the backup.
- ACLs use the current SID, and the vault `.tmp` is restricted **before**
  `Move-Item`.

## Stop and archive

- `POST /admin/arreter` provides a graceful stop; `Arreter.ps1` uses it and only
  falls back to `Stop-Process -Force` after 15 s.
- The archive is rebuilt with `/` separators, compatible with Linux `unzip`.
- The binary is built with `--remap-path-prefix` and `strip = true`: the build path
  and username are no longer in the release.

## Documented limitations

- `icacls /inheritance:r` also removes SYSTEM and Administrators; this is a
  deliberate, documented choice in `ETAT-REEL`.
- The Rust core `.tmp` remains protected by folder rights; the write script applies
  file rights before replacement.

## Verification

- 88 tests, 0 failures.
- Archive: `apivault-proxy-1.0.3-win64.zip` — **3,451,206 bytes**.

## Checksums

```
SHA-256  apivault-proxy-1.0.3-win64.zip   AA84BC7D43D9567A0C45733294F089896F764E6C69E6B2B783F1DADDDDA07E21
```

The binary still contains one occurrence of the build path
(`C:\Users\Xtrem-21`); it is explicitly accepted for this release.
