# TEST PROTOCOL — ApiVault Proxy

[Français](PROTOCOLE-TEST.md) | **English**

**Purpose**: verify the real behaviour of the application from the user's point of
view. No code review, no opinion: facts, evidence, OK/KO.

**Recipients**: external auditor (Cline, Reasonix, or a human).
**Tested version**: to be filled in by the auditor (commit + tag + exe version).

**Revision of 2026-10-18, after audit #1**: three criteria were poorly written and
produced KO results that were not product defects — T1.2 (`/sante` instead of
`/health`), T1.6 (missing noise tolerance), T5.4 (expected tag on the audit copy).
Corrected here. The KO results from audit #1 remain in its report, untouched: an
audit is not rewritten.

---

## Audit charter — read before starting

These six rules are **contractual**. An audit that violates even one is void and
will be returned.

1. **NO MODIFICATION. ABSOLUTE PROHIBITION.**
   You do not modify, correct, reformat or "improve" **any file** in the
   repository — code, configuration, documentation, icons, `.gitignore`, everything
   included. Not even an obvious typo, not even a comment. You observe, execute,
   record, report. If you see a defect, it goes in the report; it does **not** go
   into the code.
   Corollary: do not run `cargo fmt`, `git commit`, `git checkout`, `git clean`, or
   any command that writes to the repository.

2. **ONE FILE CREATED: THE REPORT.**
   At the end of the audit, place at the **repository root** a file named:
   `RAPPORT-AUDIT-<your-name>-<YYYYMMDD>.md`
   (example: `RAPPORT-AUDIT-Cline-20261020.md`). That is the deliverable. Nothing
   else is added to the repository. Your scripts or temporary files go **outside
   the repository** (or are deleted before submission).

3. Each test produces: an identifier, an **OK / KO / NOT TESTED** verdict, and
   **evidence** (command output, log excerpt, screenshot). An OK without evidence
   counts as NOT TESTED.

4. A test you could not run is **NOT TESTED**. Never OK by deduction, never "it
   should work".

5. **Never copy an API key** into the report, even partially. If a key appears
   anywhere, it is a **security KO** and you report it as blocking.

6. Starting environment: application closed, port 3100 free, clean repository.
   Check it (T0) before anything else.

---

## T0 — Preparation

| ID | Test | Expected |
|---|---|---|
| T0.1 | `git status --short` and `git log -1 --oneline` | Clean, announced commit |
| T0.2 | `Get-Process apivault-proxy` | No process |
| T0.3 | `netstat -ano \| findstr :3100` | Port free |
| T0.4 | Backup of `cles.env` + SHA-256 | File kept for final comparison |
| T0.5 | `cargo test --release` | Record the exact test count and result |

---

## T1 — Startup

| ID | Test | Expected |
|---|---|---|
| T1.1 | Stopwatch: launch the exe → interactive UI. **3 measurements.** | ≤ 3 s (otherwise record the 3 values) |
| T1.2 | During startup, `Invoke-WebRequest http://127.0.0.1:3100/health` | Response ≤ 2 s, including during initialisation |
| T1.3 | Log for the first 5 seconds | **No** provider call |
| T1.4 | State shown without any user action | Known models appear, with a timestamp such as "checked X ago" |
| T1.5 | Startup banner | Initialisation message visible, then disappears by itself |
| T1.6 | Second launch (same configuration) | Time ≤ first launch **+ 10%** (a difference of a few tens of milliseconds is noise, not a defect) |
| T1.7 | Launch while port 3100 is busy | Clear error message, no crash, no empty window |

---

## T2 — Interface

| ID | Test | Expected |
|---|---|---|
| T2.1 | Scroll to the bottom of the table | Top bar remains visible at all times (before/after screenshot) |
| T2.2 | Double-click `http://127.0.0.1:3100/v1`, then paste into Notepad | The **whole** line is copied exactly, with no stray space |
| T2.3 | Copy button for the code block | Full copy, visual feedback |
| T2.4 | Panel name | "Providers" (not "Address") |
| T2.5 | Add a provider (name, URL, test key, models) | Appears immediately, **persists after restart** |
| T2.6 | "Test key" button | Valid key → success; invalid key → readable message, no crash |
| T2.7 | "Copy diagnostic" button | Valid JSON, pasted into Notepad; **no key**, no URL containing a secret |
| T2.8 | Double-click the star of a default model | Selects; **double-click again deselects**; fallback to the first reachable model |
| T2.9 | Select an unreachable model | Selection possible, warning visible, no blocking |

---

## T3 — Gateway

| ID | Test | Expected |
|---|---|---|
| T3.1 | `GET /v1/models` with the token | Alias list, OpenAI format |
| T3.2 | `POST /v1/chat/completions` without streaming | Complete response, `usage` if the provider returns it |
| T3.3 | Same call in streaming | Complete SSE stream, ending with `[DONE]`, no interruption |
| T3.4 | Without token / wrong token | **401**, no provider call |
| T3.5 | Priority provider down | Failover to the next, 200 response, consistent log |
| T3.6 | Search for the exact key value in the logs | **No occurrence** |

---

## T4 — Super IA: out of scope, development postponed to V2

**Version decision**: Super IA is **not** delivered in V1. It is postponed to a V2.
The expected behaviour in V1 is therefore only that of a **reserved, inert and
honestly labelled slot**. There is **no function to test** here: only verify that
it does nothing.

| ID | Test | Expected |
|---|---|---|
| T4.1 | Block location | Right panel, under the tokens, separated by a rule, titled "Higher management" |
| T4.2 | Checkbox state | **Disabled and not clickable**, "under development" badge (not "soon", which would suggest imminent delivery) |
| T4.3 | Two drop-down lists | Present, **inactive**, no effect on click |
| T4.4 | A normal request, checkbox cleared | **No** additional call (log: one upstream call only) |
| T4.5 | Attempt to force the checkbox or lists (click, keyboard, DevTools) | No effect, no request emitted, no visible error message |
| T4.6 | Search for a second upstream call in the log over 10 requests | **Zero** additional calls, whatever the setting |

**Automatic KO**: any trace of a call to a "reasoning" model, any checkable box, or
any text suggesting the function is operational.

---

## T5 — Regressions

| ID | Test | Expected |
|---|---|---|
| T5.1 | `cargo test --release` | All green, no skipped test, no test removed compared with the previous version |
| T5.2 | Exe version | The one announced in the report |
| T5.3 | SHA-256 of `cles.env` | Identical to T0.4 |
| T5.4 | `git status --short`, `git tag --points-at HEAD` | Clean. Tag present **or** absence explained (the audit copy carries an untagged preparation commit: this is normal, the tag remains on the project commit) |

---

## Expected report — mandatory deliverable

Single file, at the repository root: **`RAPPORT-AUDIT-<name>-<YYYYMMDD>.md`**.
It starts with this block, copied as is and completed:

```
# Audit report — ApiVault Proxy

- Auditor: <name / tool>
- Date: <YYYY-MM-DD>
- Audited commit: <git log -1 --oneline>
- Tag: <git tag --points-at HEAD>
- Exe version: <displayed version>
- Convention respected: no modification made, one file created (this one): YES / NO
```

Then a single table:

| ID | Verdict | Evidence | Remark |
|---|---|---|---|
| T1.1 | OK | 2.8 s / 2.9 s / 2.7 s | — |
| … | … | … | … |

Then three sections:

- **Blocking** — what prevents normal use.
- **Major** — what degrades the experience without blocking.
- **Minor** — cosmetic, wording, inconsistencies.

One conclusion sentence: *"Out of N tests, X OK, Y KO, Z not tested."*

And one final mandatory line:

```
Integrity: git status --short shows only this report file.  YES / NO
```

If the answer is NO, the audit is invalid: the auditor modified the repository.

---

## What the auditor must not do

- **Modify any repository file, whatever it is, even for a trivial test** (rule 1, no exception).
- Create a file other than the report at the root.
- Fix the code "because it was obvious".
- Re-run a full scan "to see" without recording it.
- Conclude "it works" without executed evidence.
- Test with a real billed key: use a test key or a free provider.
- Develop, even partially, Super IA: it is out of scope (V2).
