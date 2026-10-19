# Test protocol — ApiVault Proxy

[Français](PROTOCOLE-TEST-UTILISATEUR.md) | **English**

A document **for you** to follow at your own pace. It is split into three possible
sessions. Each step has a number: in your notes, simply write "step 12: OK" or
"step 12: the window shows X".

| | |
|---|---|
| Tested version | ______________ (shown in the UI) |
| Commit | ______________ |
| Start date | ______________ |
| Tester | ______________ |

**Duration**: about 75 minutes in total. Suggested split: P0–P1 (15 min),
P2–P3 (30 min), P4–P7 (30 min).

---

## Test rules

1. **One step = one action.** Do not group them, otherwise a failure becomes
   impossible to locate.
2. **If a step fails**: write down the exact output, tick FAIL, and **continue**.
   Do not fix anything, do not reinstall anything. A step that depends on a failed
   step will be marked "inconclusive".
3. **Never copy an API key** into your notes, even partially. Write "test key".
4. **"Not tested" is a valid answer.** Better that than a complacent PASS.
5. Application **closed** for P1. Application **open** for P2 to P6.

### Restart point

| Phase | Content | Done |
|---|---|---|
| P0 | Preparation | [ ] |
| P1 | State and integrity (app closed) | [ ] |
| P2 | Vault and keys | [ ] |
| P3 | Service | [ ] |
| P4 | Counter and cap | [ ] |
| P5 | Startup and restart | [ ] |
| P6 | Interface | [ ] |
| P7 | Closing | [ ] |

---

# P0 — Preparation (5 min)

**Have ready:**

- a **test key** at a provider that you can deliberately break (never your main
  key);
- ideally **two providers** configured, to test failover;
- the right to **restart the PC** once;
- the application **closed** at the start.

### Step 1 — Close every instance
Check that no ApiVault Proxy window is open and that no agent is working on the
repository.
**Expected**: nothing running.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 2 — Record the version and commit
In a terminal, at the repository root:
```powershell
git log --oneline -1
```
**Expected**: one commit, and you copy it into the table at the top of this document.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

---

# P1 — State and integrity (10 min, application closed)

### Step 3 — The repository is clean
```powershell
git status --short
```
**Expected**: no line, **except** `?? PROTOCOLE-TEST-UTILISATEUR.md` and possibly
`?? RAPPORT-...`. Any other modified file must be reported.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 4 — No key in history
```powershell
git log -p -15 | Select-String -Pattern 'sk-[A-Za-z0-9]{8,}' | Measure-Object
```
**Expected**: `Count: 0`.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 5 — Tests pass
```powershell
cd Proxy-Externe\src-tauri
cargo test --release
```
**Expected**: `0 failed` everywhere. The total must match the figure in the last
report.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 6 — No hidden tests
```powershell
Select-String -Path tests\*.rs,src\*.rs -Pattern '#\[ignore\]' -Recurse | Measure-Object
```
**Expected**: `Count: 0`.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 7 — The binary builds
```powershell
cargo build --release
Get-Item target\release\apivault-proxy.exe | Select-Object Length
```
**Expected**: `Finished`, and a size close to the one announced in the report.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

> **End of session 1 possible here.**

---

# P2 — The vault and keys (15 min, application open)

### Step 8 — Startup
Launch the application.
**Expected**: the window opens, already-configured providers are there, no error
message.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 9 — The key file is encrypted
```powershell
Get-Content "$env:APPDATA\ApiVault_Proxy\cles.env" -TotalCount 1
```
**Expected**: `APIVAULT-COFFRE-1`.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 10 — No clear key on disk
```powershell
Select-String -Path "$env:APPDATA\ApiVault_Proxy\*" -Pattern 'sk-','DEEPSEEK_KEY','OPENAI_API_KEY' -Recurse | Measure-Object
```
**Expected**: `Count: 0`.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 11 — Add a provider
**Providers** button → **Add a provider**. Fill in: name, URL (`.../v1`), **test
key**, models.
Click **Test key**, then **Save**.
**Expected**: the test answers, the provider appears in the list, the service does
not fall.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 12 — A wrong key is rejected cleanly
Add a second provider with a **deliberately wrong key**. Click **Test key**.
**Expected**: readable error message, no internal detail, **without the key**, and
the rest of the application keeps working.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 13 — Change an API key
On the provider from step 11, replace the key with another valid key (rotation),
save.
**Expected**: the new key works; the old one is no longer used.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 14 — The backup does not keep the old key
After the rotation in step 13:
```powershell
Get-ChildItem "$env:APPDATA\ApiVault_Proxy" -Filter "*.bak"
```
Then, if a `.bak` exists:
```powershell
Get-Content "$env:APPDATA\ApiVault_Proxy\cles.env.bak" -TotalCount 1
```
**Expected**: either no `.bak`, or a `.bak` starting with `APIVAULT-COFFRE-1`.
**Never** a clear key.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 15 — Delete a key
Delete the wrong-key provider from step 12.
**Expected**: it disappears, and the associated backup is deleted.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 16 — Folder permissions
```powershell
icacls "$env:APPDATA\ApiVault_Proxy"
```
**Expected**: only **your account** appears, with no inherited `(I)` entry for
`BUILTIN\Users` or `Everyone`.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

> **End of session 2 possible here.**

---

# P3 — The service (15 min)

### Step 17 — The probe is silent
```powershell
curl.exe -s http://127.0.0.1:3100/health
```
**Expected**: `{"status":"ok","version":"..."}` — **only two fields**.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 18 — A forged host is rejected
```powershell
curl.exe -s -o NUL -w "%{http_code}" -H "Host: evil.example" http://127.0.0.1:3100/health
```
**Expected**: `403`.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 19 — Administration requires the token
```powershell
curl.exe -s -o NUL -w "%{http_code}" http://127.0.0.1:3100/admin/pool
```
**Expected**: `401`.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 20 — The token opens administration
Copy the **access token** from the UI (view/copy button), then:
```powershell
curl.exe -s -o NUL -w "%{http_code}" -H "Authorization: Bearer <TOKEN>" http://127.0.0.1:3100/admin/pool
```
**Expected**: `200`.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 21 — A real request, normal mode
From your usual client, or with the configuration copied from the UI: a simple
request.
**Expected**: a correct model response.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 22 — A real request, streaming
Same request with **stream** mode enabled.
**Expected**: text arrives progressively, without interruption or blocking.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 23 — Failover works
On the wrong-key provider, send a request (or make it the priority provider).
**Expected**: the service fails over to the other provider, the response arrives,
and the error remains readable.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

---

# P4 — Counter and cap (10 min)

> **LOT 5 and LOT 6 are delivered** (October 18, up to the internal acceptance
> commit).
> Expect to see `estime: true` lines: this is normal for a streaming response, the
> provider does not always send its usage. Tokens must be **non-zero**.

### Step 24 — The journal exists
```powershell
Get-ChildItem "$env:APPDATA\ApiVault_Proxy\metriques" | Select-Object Name, Length
Get-Content "$env:APPDATA\ApiVault_Proxy\metriques\metriques-*.jsonl" -TotalCount 3
```
**Expected**: the current month's file, **one line = one request**, with provider,
model and numbers.
**Result**: [ ] PASS  [ ] FAIL  [ ] NOT TESTED — Notes: ______________________

### Step 25 — The journal contains no content
```powershell
Select-String -Path "$env:APPDATA\ApiVault_Proxy\metriques\*" -Pattern 'sk-','Bearer ','prompt' -Recurse | Measure-Object
```
**Expected**: `Count: 0`.
**Result**: [ ] PASS  [ ] FAIL  [ ] NOT TESTED — Notes: ______________________

### Step 26 — The numbers are consistent
Compare the number of requests sent since startup with the **session** view in the
UI.
**Expected**: the same number. Cache is shown **separately** ("including cache
read").
**Result**: [ ] PASS  [ ] FAIL  [ ] NOT TESTED — Notes: ______________________

### Step 27 — The cap is disabled by default
Open the usage card.
**Expected**: no active cap until you set one, and a visible card to set it.
**Result**: [ ] PASS  [ ] FAIL  [ ] NOT TESTED — Notes: ______________________

### Step 28 — The cap rejects cleanly
Set a very low cap, then send a request.
**Expected**: clear, understandable rejection, service still up, message saying
why.
**Result**: [ ] PASS  [ ] FAIL  [ ] NOT TESTED — Notes: ______________________

### Step 29 — The 80% alert
Set a cap slightly above your current usage.
**Expected**: an alert appears at 80%, before blocking.
**Result**: [ ] PASS  [ ] FAIL  [ ] NOT TESTED — Notes: ______________________

> **End of session 3 possible here.**

---

# P5 — Startup and restart (10 min)

### Step 30 — Enable automatic start
UI → **Start with Windows**.
**Expected**: the switch turns on without error.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 31 — The task belongs to your account
```powershell
schtasks /query /tn "ApiVault_Proxy_Externe" /v /fo list
```
**Expected**: "Run As User" = **your Windows account**, not `SYSTEM`. This is what
allows the vault to decrypt at startup.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 32 — Restart the PC
Restart, sign in, wait one minute without launching anything.
**Expected**: the application starts by itself.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 33 — Keys are still readable
After the restart, open **Providers**.
**Expected**: your providers are there, no "keys unreadable" message, and a request
works.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 34 — Two instances
Launch the application a second time while it is already running.
**Expected**: clear "port busy" or equivalent message, no crash, no ghost second
instance.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

---

# P6 — Interface (10 min)

### Step 35 — The Super IA button is inert
**Expected**: greyed out, labelled "under development", no click triggers a call.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 36 — The bar stays visible
Scroll the page.
**Expected**: the status bar stays pinned at the top.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 37 — Copy the address
Double-click the address `http://127.0.0.1:3100/v1`.
**Expected**: the address is copied, with visual feedback.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 38 — The empty state
On an installation without a provider (or after removing all of them), open
**Providers**.
**Expected**: a welcome message, not a blank page.
**Result**: [ ] PASS  [ ] FAIL  [ ] NOT TESTED — Notes: ______________________

### Step 39 — Tooltips
Hover over the status light and the recovery time.
**Expected**: an explanation in French, understandable without documentation.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

---

# P7 — Closing (5 min)

### Step 40 — Return the application to a clean state
Remove the wrong key, set the cap back to zero, disable automatic start if you do
not want it.
**Expected**: initial state restored.
**Result**: [ ] PASS  [ ] FAIL — Notes: ______________________

### Step 41 — Fill in the summary
Report the number of PASS, FAIL and NOT TESTED below.

| | Count |
|---|---|
| PASS | ____ |
| FAIL | ____ |
| NOT TESTED | ____ |

### Step 42 — What you send me
For each FAIL: **the step number**, **what you expected**, **what you saw**. A
screenshot if it is visual.
Do not send me a key, a token, or an export of `cles.env`.

---

## After receiving your notes

I classify each FAIL as **blocking / major / minor / cosmetic**, fix it, and tell
you which steps to redo — not all 42. Only then: external audit on the frozen
commit, then publication.
