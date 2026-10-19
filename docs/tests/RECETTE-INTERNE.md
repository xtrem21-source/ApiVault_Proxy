# Recette interne — 9 contrôles sur le binaire réel

[Français](RECETTE-INTERNE.md) | [English](RECETTE-INTERNE.en.md)

Cette recette ne remplace pas les tests automatisés : elle prouve ce qu'eux ne
peuvent pas prouver, **sur l'exécutable publié**, dans un bac à sable, avec un faux
fournisseur local. Aucune vraie clé n'est utilisée, aucune donnée réelle n'est
touchée.

Durée : environ une heure, compilation comprise.

## Prérequis

| Outil | Version | Pourquoi |
|---|---|---|
| Windows 10/11 | — | DPAPI, `icacls`, WebView2 |
| Node.js | 18 ou plus | le faux fournisseur |
| Rust / Cargo | stable | compiler le binaire |
| PowerShell | 5.1 ou 7 | les commandes ci-dessous |

## 1. Préparer le bac à sable

```powershell
$bac = "$env:TEMP\recette-apivault"
Remove-Item $bac -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory "$bac\donnees" -Force | Out-Null
```

Deux variables suffisent à isoler complètement l'application :

```powershell
$env:APPDATA = "$bac\donnees"
$env:APIVAULT_CLE_ENV = "$bac\donnees\ApiVault_Proxy\cles.env"
```

Le service refusera de démarrer si le port 3100 est déjà pris :

```powershell
Get-NetTCPConnection -LocalPort 3100 -State Listen -ErrorAction SilentlyContinue
```

Une sortie non vide signifie qu'une autre instance tourne. **Ne la tue pas** :
arrête-la proprement, ou choisis une autre machine.

## 2. Lancer le faux fournisseur

```powershell
node docs\tests\outils\faux-fournisseur.js
```

Il écoute sur `127.0.0.1:3999` et sait faire trois choses :

| Route | Réponse |
|---|---|
| `GET /v1/models` | une liste contenant `modele-test` |
| `POST /v1/chat/completions` sans `stream` | une réponse JSON avec `usage: 17 / 23` |
| `POST /v1/chat/completions` avec `stream: true` | un flux SSE **sans bloc `usage`** |

Il journalise chaque corps reçu dans `%TEMP%\recette-apivault\corps-recus.log`.

## 3. Écrire la configuration de recette

Créer `%APPDATA%` du bac avec un `cles.env` en clair : l'application le chiffrera
au premier démarrage, ce qui est justement le contrôle C1.

```powershell
$d = "$bac\donnees\ApiVault_Proxy"
New-Item -ItemType Directory $d -Force | Out-Null
@"
PROXY_PORT=3100
PROXY_MASTER_KEY=jeton-de-recette-123456
PROXY_FOURNISSEURS=faux
FAUX_URLS=http://127.0.0.1:3999/v1
FAUX_MODELES=modele-test
FAUX_KEY=sk-faux-recette
"@ | Set-Content "$d\cles.env" -Encoding ASCII
```

## 4. Compiler et démarrer

```powershell
cd Proxy-Externe\src-tauri
cargo build --release
.\target\release\apivault-proxy.exe
```

Attendre que `/health` réponde :

```powershell
Invoke-RestMethod http://127.0.0.1:3100/health
```

## 5. Les neuf contrôles

| # | Contrôle | Commande | Attendu |
|---|---|---|---|
| **C1** | Le fichier de clés est chiffré au démarrage | `Get-Content "$d\cles.env" -TotalCount 1` | `APIVAULT-COFFRE-1`, et `Select-String sk-faux` ne trouve rien |
| **C2** | `Host` étranger refusé | `curl.exe -s -o NUL -w "%{http_code}" -H "Host: evil.example" http://127.0.0.1:3100/health` | `403` |
| **C3** | Administration protégée | `curl.exe -s -o NUL -w "%{http_code}" http://127.0.0.1:3100/admin/metriques` | `401` |
| **C4** | Comptage exact hors flux | `POST /v1/chat/completions` puis lire le journal | `tokens_in: 17`, `tokens_out: 23`, `estime: false` |
| **C5** | Estimation non nulle en flux | même appel avec `"stream": true` | `estime: true`, `tokens_in > 0`, `tokens_out > 0` |
| **C6** | Plafond avant l'appel amont | relancer avec `PROXY_PLAFOND_MENSUEL` bas | `429`, code `plafond_atteint`, **aucune ligne de plus** dans le journal du faux fournisseur |
| **C7** | Journal tolérant | ajouter une ligne finissant par l'octet `0xFF`, refaire un appel | `lignes_illisibles: 1`, totaux intacts, plafond toujours actif |
| **C8** | BOM accepté | envoyer un corps précédé de `EF BB BF` | `200`, pas `400` |
| **C9** | Propreté | arrêter l'application et le faux fournisseur | aucun processus restant, ports 3100 et 3999 libres |

Exemple pour C4 :

```powershell
$jeton = 'jeton-de-recette-123456'
$corps = '{"model":"modele-test","messages":[{"role":"user","content":"bonjour"}]}'
curl.exe -s -X POST http://127.0.0.1:3100/v1/chat/completions `
  -H "Authorization: Bearer $jeton" -H "Content-Type: application/json" -d $corps
Get-Content "$d\metriques-$(Get-Date -Format yyyy-MM).jsonl" -Tail 1
```

Exemple pour C5 :

```powershell
$corps = '{"model":"modele-test","stream":true,"messages":[{"role":"user","content":"bonjour"}]}'
curl.exe -s -N -X POST http://127.0.0.1:3100/v1/chat/completions `
  -H "Authorization: Bearer $jeton" -H "Content-Type: application/json" -d $corps
Get-Content "$d\metriques-$(Get-Date -Format yyyy-MM).jsonl" -Tail 1
```

## 6. Résultats de la dernière exécution

| Contrôle | Résultat | Preuve |
|---|---|---|
| C1 | PASS | en-tête `APIVAULT-COFFRE-1`, 583 octets, aucune trace de `sk-faux` |
| C2 | PASS | `403` `{"code":"hote_refuse"}` |
| C3 | PASS | `401` sans jeton, `200` avec |
| C4 | PASS | `"tokens_in":17,"tokens_out":23,"estime":false` |
| C5 | PASS | `"tokens_in":27,"tokens_out":132,"estime":true,"flux":true` |
| C6 | PASS | `429 plafond_atteint`, journal amont inchangé (3 lignes avant, 3 après) |
| C7 | PASS | `lignes_illisibles:1`, `utilise:199` conservé, `429` toujours rendu |
| C8 | PASS | `200` |
| C9 | PASS | 0 processus, ports libres |

## 7. Nettoyage

```powershell
Get-Process apivault-proxy -ErrorAction SilentlyContinue | Stop-Process
Get-Process node -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowTitle -eq '' } | Stop-Process
Remove-Item $bac -Recurse -Force
```

Vérifier ensuite qu'aucun port n'écoute :

```powershell
Get-NetTCPConnection -LocalPort 3100,3999 -State Listen -ErrorAction SilentlyContinue
```

## 8. Ce que cette recette ne prouve pas

- Le rendu visuel et les notifications : ils demandent un œil humain.
- Le comportement sous charge : il n'y a qu'un client dans cette recette.
- La coupure de courant, le disque plein, l'ACL refusée : il faudrait un outil de panne.
- La justesse de l'estimation : elle est non nulle, pas forcément exacte.
