# Internal acceptance — 9 checks on the real binary

[Français](RECETTE-INTERNE.md) | [English](RECETTE-INTERNE.en.md)

This acceptance run does not replace the automated tests: it proves what they
cannot prove, **on the shipped executable**, in a sandbox, with a local fake
provider. No real key is used, no real data is touched.

Duration: about one hour, build included.

## Prerequisites

| Tool | Version | Why |
|---|---|---|
| Windows 10/11 | — | DPAPI, `icacls`, WebView2 |
| Node.js | 18 or later | the fake provider |
| Rust / Cargo | stable | build the binary |
| PowerShell | 5.1 or 7 | the commands below |

## 1. Prepare the sandbox

```powershell
$bac = "$env:TEMP\recette-apivault"
Remove-Item $bac -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory "$bac\donnees" -Force | Out-Null
```

Two variables are enough to fully isolate the application:

```powershell
$env:APPDATA = "$bac\donnees"
$env:APIVAULT_CLE_ENV = "$bac\donnees\ApiVault_Proxy\cles.env"
```

The service will refuse to start if port 3100 is already taken:

```powershell
Get-NetTCPConnection -LocalPort 3100 -State Listen -ErrorAction SilentlyContinue
```

A non-empty result means another instance is running. **Do not kill it**: stop it
cleanly, or pick another machine.

## 2. Start the fake provider

```powershell
node docs\tests\outils\faux-fournisseur.js
```

It listens on `127.0.0.1:3999` and knows three things:

| Route | Response |
|---|---|
| `GET /v1/models` | a list containing `modele-test` |
| `POST /v1/chat/completions` without `stream` | a JSON reply with `usage: 17 / 23` |
| `POST /v1/chat/completions` with `stream: true` | an SSE stream **with no `usage` block** |

It logs every received body to `%TEMP%\recette-apivault\corps-recus.log`.

## 3. Write the acceptance configuration

Create the sandbox `%APPDATA%` with a plain-text `cles.env`: the application will
encrypt it on first start, which is exactly check C1.

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

## 4. Build and start

```powershell
cd Proxy-Externe\src-tauri
cargo build --release
.\target\release\apivault-proxy.exe
```

Wait until `/health` answers:

```powershell
Invoke-RestMethod http://127.0.0.1:3100/health
```

## 5. The nine checks

| # | Check | Command | Expected |
|---|---|---|---|
| **C1** | The key file is encrypted on start | `Get-Content "$d\cles.env" -TotalCount 1` | `APIVAULT-COFFRE-1`, and `Select-String sk-faux` finds nothing |
| **C2** | Foreign `Host` refused | `curl.exe -s -o NUL -w "%{http_code}" -H "Host: evil.example" http://127.0.0.1:3100/health` | `403` |
| **C3** | Administration protected | `curl.exe -s -o NUL -w "%{http_code}" http://127.0.0.1:3100/admin/metriques` | `401` |
| **C4** | Exact counting off-stream | `POST /v1/chat/completions`, then read the journal | `tokens_in: 17`, `tokens_out: 23`, `estime: false` |
| **C5** | Non-zero estimate on a stream | same call with `"stream": true` | `estime: true`, `tokens_in > 0`, `tokens_out > 0` |
| **C6** | Cap applied before the upstream call | restart with a low `PROXY_PLAFOND_MENSUEL` | `429`, code `plafond_atteint`, **no extra line** in the fake provider's log |
| **C7** | Tolerant journal | append a line ending with byte `0xFF`, make another call | `lignes_illisibles: 1`, totals intact, cap still active |
| **C8** | BOM accepted | send a body prefixed with `EF BB BF` | `200`, not `400` |
| **C9** | Cleanliness | stop the application and the fake provider | no process left, ports 3100 and 3999 free |

Example for C4:

```powershell
$jeton = 'jeton-de-recette-123456'
$corps = '{"model":"modele-test","messages":[{"role":"user","content":"bonjour"}]}'
curl.exe -s -X POST http://127.0.0.1:3100/v1/chat/completions `
  -H "Authorization: Bearer $jeton" -H "Content-Type: application/json" -d $corps
Get-Content "$d\metriques-$(Get-Date -Format yyyy-MM).jsonl" -Tail 1
```

Example for C5:

```powershell
$corps = '{"model":"modele-test","stream":true,"messages":[{"role":"user","content":"bonjour"}]}'
curl.exe -s -N -X POST http://127.0.0.1:3100/v1/chat/completions `
  -H "Authorization: Bearer $jeton" -H "Content-Type: application/json" -d $corps
Get-Content "$d\metriques-$(Get-Date -Format yyyy-MM).jsonl" -Tail 1
```

## 6. Results of the last run

| Check | Result | Evidence |
|---|---|---|
| C1 | PASS | header `APIVAULT-COFFRE-1`, 583 bytes, no trace of `sk-faux` |
| C2 | PASS | `403` `{"code":"hote_refuse"}` |
| C3 | PASS | `401` without a token, `200` with |
| C4 | PASS | `"tokens_in":17,"tokens_out":23,"estime":false` |
| C5 | PASS | `"tokens_in":27,"tokens_out":132,"estime":true,"flux":true` |
| C6 | PASS | `429 plafond_atteint`, upstream log unchanged (3 lines before, 3 after) |
| C7 | PASS | `lignes_illisibles:1`, `utilise:199` preserved, `429` still returned |
| C8 | PASS | `200` |
| C9 | PASS | 0 processes, ports free |

## 7. Cleanup

```powershell
Get-Process apivault-proxy -ErrorAction SilentlyContinue | Stop-Process
Get-Process node -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowTitle -eq '' } | Stop-Process
Remove-Item $bac -Recurse -Force
```

Then check that no port is listening:

```powershell
Get-NetTCPConnection -LocalPort 3100,3999 -State Listen -ErrorAction SilentlyContinue
```

## 8. What this acceptance run does not prove

- The visual rendering and the notifications: they need a human eye.
- Behaviour under load: there is only one client in this run.
- Power loss, a full disk, a refused ACL: that would need a fault tool.
- The accuracy of the estimate: it is non-zero, not necessarily exact.
