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
- Archive: `apivault-proxy-1.0.3-win64.zip` — **3,559,537 bytes**.

## Checksums

```
SHA-256  apivault-proxy-1.0.3-win64.zip   95489ABDFB95A61E875738F5F1E2F0B8A729E6CEEF96C45268E58636EA274F0A
```

The binary is built from a neutral copy with `--remap-path-prefix`: it no longer
contains the personal build path. One neutral string remains,
`C:\build\ApiVault_Proxy\...`, written by Tauri (`CARGO_MANIFEST_DIR`); it
reveals no user name.
