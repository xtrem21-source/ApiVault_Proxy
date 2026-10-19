# Inventaire des tests et des protocoles

[Français](INVENTAIRE.md) | [English](INVENTAIRE.en.md)

## Le principe : exécuter d'abord, compléter ensuite

La documentation annonce des tests de sécurité, d'intégration et de métriques. Ils
existent : **88 tests**, tous verts. La bonne démarche n'est donc pas de réécrire
ces scénarios, mais de les exécuter, de lire ce qu'ils couvrent réellement, puis
d'ajouter **uniquement ce qui manque**.

Ce document sert à ça : il dit quel test prouve quoi, et ce qui n'est prouvé par
rien.

## Lancer la suite

```powershell
cd Proxy-Externe\src-tauri
cargo test
cargo test -- --list    # affiche les 88 noms, sans exécuter
```

Compter les tests réellement exécutés :

```powershell
cargo test 2>&1 | Select-String 'test result:'
```

## Les suites

| Fichier | Tests | Niveau | Ce que la suite prouve |
|---|---:|---|---|
| `src/coffre.rs` | 3 | unitaire | En-tête du coffre, refus DPAPI avec code Windows, texte simple toléré |
| `src/config.rs` | 6 | unitaire | Découpage des adresses, détection des modèles gratuits, préférence de modèle, sauvegarde |
| `src/pool.rs` | 5 | unitaire | Ordre des cibles, priorité de l'alias, 403 → pause, 404 → cible seule, 401 → fournisseur entier |
| `src/serveur.rs` | 16 | unitaire | Jeton, `Host`, redaction des erreurs, flux borné, substitution des libellés, diagnostic sans clé |
| `tests/integration.rs` | 9 | intégration | 401, bascule sur 401 amont, 429 amont, flux coupé, une seule requête amont par succès, délais, 413, BOM |
| `tests/lot3.rs` | 3 | intégration | Port occupé, `/health` minimal, `Host` étranger refusé |
| `tests/lot4.rs` | 12 | intégration | Chiffrement aller-retour, `.bak` chiffré, rotation, effacement, migration, droits |
| `tests/lot5.rs` | 12 | intégration | Journal par requête, agrégats, trois formes d'`usage`, flux SSE, ligne illisible |
| `tests/lot6.rs` | 15 | intégration | Migration au démarrage, échec d'écriture, plafond (10 scénarios) |
| `tests/lot7.rs` | 7 | intégration | Estimation sans usage, coupure client, horodatage au départ, octet invalide |
| **Total** | **88** | | 30 unitaires + 58 d'intégration |

## Matrice de couverture

| Domaine | Couvert par | Niveau | Ce qui reste |
|---|---|---|---|
| Chiffrement au repos | `coffre::aller_retour_chiffrement`, `le_fichier_ecrit_ne_contient_pas_la_cle_en_clair`, `la_sauvegarde_bak_est_chiffree` | unitaire + intégration | — |
| Migration d'un fichier en clair | `migration_d_un_fichier_en_clair_au_premier_enregistrement`, `migration_apres_chargement_de_la_configuration`, `fichier_interne_en_clair_migre_et_chiffre` | intégration | Coupure pendant la migration : non testable sans outil de panne |
| Rotation et effacement | `la_rotation_d_une_cle_supprime_la_sauvegarde`, `la_sauvegarde_est_supprimee_apres_effacement` | intégration | — |
| Droits du dossier | `restriction_des_droits_au_compte_courant`, `restriction_des_droits_garde_les_fichiers_lisibles`, `restriction_jamais_appliquee_a_un_fichier_externe` | intégration | Refus d'ACL réel non rejoué |
| Jeton d'administration | `bon_jeton_est_accepte`, `mauvais_jeton_est_refuse`, `jeton_absent_dans_la_config_refuse_tout`, `requete_sans_entete_est_refusee`, `a_sans_jeton_et_mauvais_jeton_donnent_401` | unitaire + intégration | Comparaison à temps constant non mesurée (elle existe dans le code) |
| `Host` / DNS rebinding | `b2_hote_etranger_refuse_et_hote_local_accepte` | intégration | — |
| Bornes d'entrée | `g_corps_trop_gros_donne_413`, `h_corps_avec_bom_utf8_est_accepte`, `decoupage_des_adresses` | intégration | — |
| Bascule entre fournisseurs | `b_401_amont_bascule_vers_la_cible_suivante`, `c_429_amont_pause_et_bascule_sans_panic`, `e_succes_ne_genere_qu_une_requete_amont`, `f_amont_lent_bascule_vers_la_cible_saine`, `f_amont_lent_partout_donne_504_dans_le_budget` | intégration | Double facturation d'un amont déjà facturé : non mesurable depuis le proxy |
| Comptage des tokens | `capture_non_flux_ecrit_les_tokens`, `capture_flux_ecrit_les_tokens_a_la_fin`, `usage_des_trois_formes_de_fournisseurs`, `usage_lu_dans_un_flux_sse` | intégration | — |
| Estimation sans usage | `flux_sans_usage_donne_une_estimation_non_nulle`, `non_flux_sans_usage_donne_une_estimation_non_nulle`, `flux_avec_usage_reste_exact` | intégration | Précision de l'estimation non calibrée |
| Journal tolérant | `ligne_illisible_ignoree_et_comptee`, `octet_invalide_ignore_la_ligne_mais_garde_les_totaux`, `octet_invalide_ne_desactive_pas_le_plafond` | intégration | — |
| Plafond mensuel | 10 tests `lot6::plafond::*` | intégration | **Refus non journalisés** (voir plus bas) |
| Coupure client en flux | `flux_coupe_par_le_client_ecrit_une_ligne_unique` | intégration | — |
| Interface HTML/JS | — | — | **Aucun test** |
| Scripts `.bat` / `.ps1` | Recette interne, manuelle | manuel | Pas d'automatisation |
| Concurrence | — | — | **Aucun test** |

## Ce qui n'est couvert par rien

Classé par ce que ça coûte s'il casse.

| Prio | Manque | Pourquoi ça compte |
|---|---|---|
| **P1** | Les refus `429 plafond_atteint` n'écrivent pas de ligne de journal | On ne peut pas prouver après coup qu'un blocage a eu lieu |
| **P1** | Aucun test de concurrence (10 requêtes simultanées, un seul mois) | Le compteur et le plafond sont lus puis écrits : à prouver sous charge |
| **P2** | Aucun test de l'interface (ouverture, panneaux, libellés) | Les régressions visuelles ne sont vues qu'à l'œil |
| **P2** | Aucun test automatisé du dossier plat (`Demarrer.bat` → `Arreter.bat`) | C'est pourtant la forme publiée |
| **P3** | Flux très long, amont muet, disque plein | Difficile à rejouer sans outil de panne |

## Comment ajouter un test

1. **Écrire le test avant le correctif**, et le voir échouer pour la bonne raison.
   Un test qui n'a jamais été rouge ne prouve rien.
2. Pas de vraie clé, pas de réseau : valeurs factices (`sk-apivault`, `sk-ancienne`)
   et faux fournisseur local.
3. Un fichier par lot (`lot8.rs`, `lot9.rs`…), un nom en français qui décrit le
   comportement attendu, pas la fonction testée.
4. Vérifier que le test tombe si on retire le correctif (le désactiver une fois).
5. `cargo test` vert, zéro avertissement, aucune assertion supprimée.

## Où sont les protocoles

Les trois protocoles vivent ensemble, à la racine du dépôt ; ce dossier en est
le mode d'emploi. Aucun n'est une copie de l'autre : ils ne s'adressent pas à
la même personne.

| Protocole | À qui il sert | Durée |
|---|---|---|
| [`PROTOCOLE-TEST-UTILISATEUR.md`](../../PROTOCOLE-TEST-UTILISATEUR.md) | À l'utilisateur, 42 étapes, 8 phases, sans outil | 2 à 3 séances |
| [`PROTOCOLE-TEST.md`](../../PROTOCOLE-TEST.md) | À un auditeur externe, charte et méthode J0 à J6 | une demi-journée |
| [`RECETTE-INTERNE.md`](RECETTE-INTERNE.md) | À celui qui livre, 9 contrôles sur le binaire réel | 1 heure |

Les deux premiers se lisent sans outil. Le troisième demande Node et une
compilation locale ; son faux fournisseur est dans
[`outils/faux-fournisseur.js`](outils/faux-fournisseur.js).

Chaque document existe en français et en anglais (`*.en.md`).
