ApiVault_Proxy - version 1.0.4
==============================

French version: LISEZ-MOI.txt | English

Local OpenAI-compatible gateway.
One standalone executable: no Python, no Docker, no Node.

Installation
------------
Keep this folder in a permanent place (Documents, Programs): automatic start
remembers the executable path. Your data lives in %APPDATA%\ApiVault_Proxy
(encrypted vault, metrics journal).

Port in use
-----------
If 3100 is taken, the gateway moves to 3101, 3102... and keeps the port it
picked. A Windows notification warns you, and the address is repeated in the
window: update it in your client.

Counter
-------
Tokens, never euros: billing stays with the provider, the only one that knows
its prices. A cap refusal states the usage, the limit and the reset date.

Local providers
---------------
An option, off by default, allows unencrypted HTTP to a private address (10.x,
172.16-31.x, 192.168.x, 100.64.x for Tailscale). Never to the internet: without
encryption the key would travel in clear text. https:// is still accepted
everywhere; for a remote Ollama, prefer an SSH tunnel or Tailscale.

Status light
------------
Green           at least one provider answered: the LLM is reachable.
Orange          the service is running but no model is available.
Blinking blue   provider check in progress.
Grey            service stopped.

So the light does not merely say "the application is running"; it says
"a model is answering".

Target pool
-----------
Each provider/model pair has a state:

  healthy     answered; preferred in the alias order
  untested    never queried, or manually put back in play
  paused      rate limit or transient error: automatic return
              (30 s, then 60, 120, 240, 300 s maximum)
  down        key rejected or model not found: manual fix
  exhausted   credits used up: manual fix

A target is never deleted: it changes state.
An expired pause returns to play by itself (reconciliation loop every 30 s) and
the state survives restarts.

No check is performed at startup: the gateway resumes the state recorded in the
last session (%APPDATA%\ApiVault_Proxy\etat-pool.json). It is instant and
consumes no quota. The UI shows "Checked X ago". A real check starts by itself
if a call fails (at most once per minute), and remains available on demand with
"Refresh".

"Refresh" button
----------------
It resets states to "untested" then really queries every provider (one minimal
request). The icon spins during the operation. A target's "Retry" button does
the same for that target alone.

Changing an API key
-------------------
1. In the window: "Management" button.
   A panel lists every provider and lets you enter a new key, erase it, or edit
   its addresses. The key is written to cles.env and applied immediately (no
   restart).

2. Changer-Cle.bat
   Same thing outside the application: OpenRouter / DeepSeek / NVIDIA menu,
   invisible input, update cles.env and restart.
   Non-interactive, for automated use:
       Changer-Cle.bat -Fournisseur deepseek -Cle sk-...
   The supplied key is checked (length, no spaces) before any write.

3. By hand: NOT POSSIBLE. Since 1.0.0, cles.env is an encrypted vault (see "Key
   vault"). Use the "Management" window or Changer-Cle.bat; a script or editor
   that rewrites the file corrupts it, and the gateway then refuses to start.

Key vault
---------
cles.env no longer contains readable text: it starts with the line
`APIVAULT-COFFRE-1`, followed by content encrypted by Windows (DPAPI, current
user account). Consequences:

- the file can only be decrypted by the Windows account that created it;
  copied to another account or machine, it is unreadable;
- no point reading it by hand: the supplied scripts (Etat.bat, Rafraichir.bat,
  Changer-Cle.bat) know how to decrypt it, with the same rights as the app;
- a cles.env.bak copy is created before every change by the application; it
  uses the same encryption;
- to start from scratch, delete cles.env: on the next launch the gateway writes
  a fresh, already encrypted file (new token, empty keys).

Multiple addresses per provider
-------------------------------
A provider can have several addresses. In the "Management" panel, each address
has a checkbox: the checked one is tried first, the others are fallbacks if it
does not answer.

Addresses are also stored in cles.env:

    OPENROUTER_URLS=https://openrouter.ai/api/v1
    DEEPSEEK_URLS=https://api.deepseek.com/v1,https://api.deepseek.com

The address checked in the UI is used by default. If it does not answer (network
error), the next one is tried automatically for the same request.
Without a *_URLS variable, the built-in addresses are used.

Interface
---------
- Fixed top bar: name, service state and default model stay visible while
  scrolling.
- The currently reachable model is shown at the top, in green.
- "EN / FR" button at the top left: switches the whole interface.
- Permanent tooltips explain each element on hover (and with the keyboard);
  these are no longer the browser's native titles.
- "Management" opens the key and address panel.
- Clicking a model row sets it as the default model (star). The choice is kept
  and used on the next start.
- "Copy client configuration" copies the address, token and selected model (or
  "defaut" if none) to the clipboard.
- Each provider has a "Test" button that checks it immediately.
- Each model has a "free" or "paid" badge.
- "Super IA" is visible in disabled mode, marked "soon".
- Erasing a key requires two clicks: the first changes the button to "Confirm",
  the second erases. The message resets after 2.5 s.

Start with Windows
------------------
The "Start with Windows" checkbox creates a Windows scheduled task: the
application starts 20 seconds after sign-in, minimised to the notification area,
without showing a window. The checkbox reads the task's real state, so it cannot
show a setting that does not exist. Clearing it removes the task.

The notification icon colour follows the real state:
  green    the service is running and at least one model answers
  orange   the service is running but no model answers
  red      the service is stopped
A Windows notification warns (at most once every 15 minutes) when no model
answers any more, and the icon tooltip gives the number of reachable models.

Usage
-----
1. Demarrer.bat             starts the gateway and opens the control window
2. Etat.bat                 checks the service, token and aliases
3. Rafraichir.bat           queries every provider again
4. Changer-Cle.bat          replaces an API key and restarts
5. Arreter.bat              stops the gateway
6. Autostart-Activer.bat    automatic start at sign-in
   Autostart-Desactiver.bat removes that automatic start

Address for clients:  http://127.0.0.1:3100/v1
Token: stored in cles.env, line PROXY_MASTER_KEY
The window's "View" button displays the token in clear text (local only).

Closing the control window does not stop the service: it keeps running in the
background. Running Demarrer.bat again (or the application) simply shows the
window of the already active instance.

Administration API (token required)
-----------------------------------
GET  /admin/pool           detailed state of aliases and targets
POST /admin/rafraichir     reset then real query
POST /admin/requalifier    reset only (all or one target)
GET  /health               service state
GET  /v1/models            available aliases

Files
-----
The archive can be moved and run as is: everything is in one folder, with no
installation. Do not move the executable alone.

apivault-proxy.exe         gateway and control window
Demarrer.bat               starts the gateway and opens the window
Etat.bat                   service state, token and aliases
Rafraichir.bat             queries every provider again
Changer-Cle.bat            replaces an API key and restarts
Arreter.bat                stops the gateway
Autostart-Activer.bat      automatic start at sign-in
Autostart-Desactiver.bat   removes that automatic start
README.en.txt              this file (English version)
outils\                    PowerShell scripts called by the .bat files
outils\commun.ps1          shared functions (executable, vault, token)
cles.env                   local API keys, encrypted (do not share)

%APPDATA%\ApiVault_Proxy\journal.log       request log (2 MB max)
%APPDATA%\ApiVault_Proxy\etat-pool.json    target state (persistent)

If Windows blocks a .bat on first launch, click "More info" then "Run anyway":
the scripts are not signed.

For developers
--------------
The source code and build instructions are in the repository (README.md at the
root). The published archive contains only the binary: nothing needs to be
compiled to use it.

Notes
-----
- The service listens only on 127.0.0.1: nothing is visible from the network.
- It never fails over after the first byte: the client sees only one response.
- Down or exhausted targets are not tried until they are requalified: this avoids
  paying for a useless round trip on every call.
- Probing sends a request of a few tokens: it consumes a minimum of quota, on
  purpose.

What's new in 1.0.0
-------------------
- Flat-folder distribution: apivault-proxy.exe next to the .bat files, no
  development tree to preserve.
- cles.env is encrypted (DPAPI, current Windows account): the file is no longer
  readable and can no longer be edited by hand.
- Scripts read the vault: Etat.bat and Rafraichir.bat recover the token, and
  Changer-Cle.bat changes the key without ever breaking the vault.
- Changer-Cle.bat accepts -Fournisseur and -Cle (non-interactive mode).
- Etat.bat and Rafraichir.bat follow PROXY_PORT instead of assuming 3100.
- Usage counter, monthly cap, default model and settings kept across restarts.

What's new in 0.4
-----------------
- Permanent tooltips (custom, not the native title) on fixed and dynamic
  elements, in FR and EN.
- Real double confirmation for erasing a key (state kept between the two
  clicks).
- Clicking a model = default model, with star, highlighted row and persistence
  in cles.env (PROXY_MODELE_PREFERE).
- The preferred model is placed first in the "defaut" and "rapide" aliases; a
  paid model does not enter the free aliases.
- Gemma 4 31B remains present but is a last resort (frequent 429s).
- Fixed top bar with name, state and default model.
- "Super IA" visible but disabled (planned for 0.5).
- Rust unit tests on the alias table, free/paid detection and pool order.
