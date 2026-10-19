# Test and protocol inventory

[Français](INVENTAIRE.md) | [English](INVENTAIRE.en.md)

## The principle: run first, complete afterwards

The documentation claims security, integration and metrics tests. They exist:
**88 tests**, all green. So the right move is not to rewrite those scenarios, but
to run them, read what they actually cover, and then add **only what is missing**.

That is what this document is for: it says which test proves what, and what is
proven by nothing.

## Running the suite

```powershell
cd Proxy-Externe\src-tauri
cargo test
cargo test -- --list    # lists the 88 names, runs nothing
```

Count the tests that actually ran:

```powershell
cargo test 2>&1 | Select-String 'test result:'
```

## The suites

| File | Tests | Level | What the suite proves |
|---|---:|---|---|
| `src/coffre.rs` | 3 | unit | Vault header, DPAPI refusal with a Windows code, plain text tolerated |
| `src/config.rs` | 6 | unit | Address splitting, free-model detection, model preference, saving |
| `src/pool.rs` | 5 | unit | Target order, alias priority, 403 → pause, 404 → single target, 401 → whole provider |
| `src/serveur.rs` | 16 | unit | Token, `Host`, error redaction, bounded stream, label substitution, diagnostics without a key |
| `tests/integration.rs` | 9 | integration | 401, failover on upstream 401, upstream 429, cut stream, one upstream request per success, delays, 413, BOM |
| `tests/lot3.rs` | 3 | integration | Port in use, minimal `/health`, foreign `Host` refused |
| `tests/lot4.rs` | 12 | integration | Encryption round trip, encrypted `.bak`, rotation, erasure, migration, permissions |
| `tests/lot5.rs` | 12 | integration | One journal line per request, aggregates, three `usage` shapes, SSE stream, unreadable line |
| `tests/lot6.rs` | 15 | integration | Startup migration, write failure, cap (10 scenarios) |
| `tests/lot7.rs` | 7 | integration | Estimate without usage, client cut, timestamp at start, invalid byte |
| **Total** | **88** | | 30 unit + 58 integration |

## Coverage matrix

| Area | Covered by | Level | What remains |
|---|---|---|---|
| Encryption at rest | `coffre::aller_retour_chiffrement`, `le_fichier_ecrit_ne_contient_pas_la_cle_en_clair`, `la_sauvegarde_bak_est_chiffree` | unit + integration | — |
| Migration from a plain-text file | `migration_d_un_fichier_en_clair_au_premier_enregistrement`, `migration_apres_chargement_de_la_configuration`, `fichier_interne_en_clair_migre_et_chiffre` | integration | Interruption during migration: not testable without a fault tool |
| Rotation and erasure | `la_rotation_d_une_cle_supprime_la_sauvegarde`, `la_sauvegarde_est_supprimee_apres_effacement` | integration | — |
| Folder permissions | `restriction_des_droits_au_compte_courant`, `restriction_des_droits_garde_les_fichiers_lisibles`, `restriction_jamais_appliquee_a_un_fichier_externe` | integration | A real ACL refusal has not been replayed |
| Admin token | `bon_jeton_est_accepte`, `mauvais_jeton_est_refuse`, `jeton_absent_dans_la_config_refuse_tout`, `requete_sans_entete_est_refusee`, `a_sans_jeton_et_mauvais_jeton_donnent_401` | unit + integration | Constant-time comparison not measured (it is in the code) |
| `Host` / DNS rebinding | `b2_hote_etranger_refuse_et_hote_local_accepte` | integration | — |
| Input bounds | `g_corps_trop_gros_donne_413`, `h_corps_avec_bom_utf8_est_accepte`, `decoupage_des_adresses` | integration | — |
| Failover between providers | `b_401_amont_bascule_vers_la_cible_suivante`, `c_429_amont_pause_et_bascule_sans_panic`, `e_succes_ne_genere_qu_une_requete_amont`, `f_amont_lent_bascule_vers_la_cible_saine`, `f_amont_lent_partout_donne_504_dans_le_budget` | integration | Double billing of an upstream already billed: not measurable from the proxy |
| Token counting | `capture_non_flux_ecrit_les_tokens`, `capture_flux_ecrit_les_tokens_a_la_fin`, `usage_des_trois_formes_de_fournisseurs`, `usage_lu_dans_un_flux_sse` | integration | — |
| Estimate without usage | `flux_sans_usage_donne_une_estimation_non_nulle`, `non_flux_sans_usage_donne_une_estimation_non_nulle`, `flux_avec_usage_reste_exact` | integration | Accuracy of the estimate not calibrated |
| Tolerant journal | `ligne_illisible_ignoree_et_comptee`, `octet_invalide_ignore_la_ligne_mais_garde_les_totaux`, `octet_invalide_ne_desactive_pas_le_plafond` | integration | — |
| Monthly cap | 10 tests `lot6::plafond::*` | integration | **Refusals not journaled** (see below) |
| Client cut in a stream | `flux_coupe_par_le_client_ecrit_une_ligne_unique` | integration | — |
| HTML/JS interface | — | — | **No test** |
| `.bat` / `.ps1` scripts | Internal acceptance, manual | manual | No automation |
| Concurrency | — | — | **No test** |

## What nothing covers

Ranked by what it costs if it breaks.

| Prio | Gap | Why it matters |
|---|---|---|
| **P1** | `429 plafond_atteint` refusals write no journal line | You cannot prove afterwards that a block happened |
| **P1** | No concurrency test (10 simultaneous requests, one month) | The counter and the cap are read then written: must be proven under load |
| **P2** | No interface test (opening, panels, labels) | Visual regressions are only seen by eye |
| **P2** | No automated test of the flat folder (`Demarrer.bat` → `Arreter.bat`) | Yet that is the published form |
| **P3** | Very long stream, silent upstream, full disk | Hard to replay without a fault tool |

## How to add a test

1. **Write the test before the fix**, and watch it fail for the right reason.
   A test that was never red proves nothing.
2. No real key, no network: fake values (`sk-apivault`, `sk-ancienne`) and a local
   fake provider.
3. One file per batch (`lot8.rs`, `lot9.rs`…), a name in French that describes the
   expected behaviour, not the function under test.
4. Check that the test fails if the fix is removed (disable it once).
5. `cargo test` green, zero warning, no assertion removed.

## Where the protocols live

The three protocols live together, at the root of the repository; this folder is
their user guide. None is a copy of another: they do not address the same person.

| Protocol | Who it serves | Duration |
|---|---|---|
| [`PROTOCOLE-TEST-UTILISATEUR.md`](../../PROTOCOLE-TEST-UTILISATEUR.md) | The user, 42 steps, 8 phases, no tooling | 2 to 3 sessions |
| [`PROTOCOLE-TEST.md`](../../PROTOCOLE-TEST.md) | An external auditor, charter and J0 to J6 method | half a day |
| [`RECETTE-INTERNE.md`](RECETTE-INTERNE.md) | Whoever ships, 9 checks on the real binary | 1 hour |

The first two read without any tooling. The third needs Node and a local build;
its fake provider is in [`outils/faux-fournisseur.js`](outils/faux-fournisseur.js).

Every document exists in French and in English (`*.en.md`).
