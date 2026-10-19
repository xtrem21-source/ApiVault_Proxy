# Screenshot guide — ApiVault Proxy

[Français](LISEZ-MOI.md) | **English**

Four screenshots are enough for the GitHub page — they are already referenced in
`README.md`. Two more are suggested as bonus shots: if you add them, they will be
inserted into the gallery.

Put the files **in this folder**, **with these exact names**.

## The four essentials

| Expected name | What to show | Why this screenshot |
|---|---|---|
| `01-fenetre-principale.png` | The full window, including the top bar, with at least one reachable model (green dot) | It is the first image a visitor sees |
| `02-panneau-gestion.png` | The "Management" panel open, one provider expanded | Shows where keys live |
| `03-consommation.png` | The monthly usage panel: input, output, cache, request count | Proves the counter exists and is readable |
| `04-plafond.png` | The cap card with its gauge, preferably in alert state | Shows the guardrail |

## The two bonus shots

| Expected name | What to show |
|---|---|
| `05-badges.png` | A row of models with the "free" / "paid" badges and the default-model star |
| `06-fournisseurs.png` | Provider state: reachable, waiting, down, with the "Test" buttons |

## Before capturing — the point that matters

> ⚠️ **Mask the token.** The "View" button displays `PROXY_MASTER_KEY` in clear
> text. A screenshot that shows it is a key published on GitHub, even if you
> delete it afterwards: it remains in the public history.
>
> Same warning for an API key that might appear in an input field. If needed,
> replace it with `sk-example-masked` before capturing.

Also check that no personal folder name, email address or `C:\Users\...` path is
visible in the image.

## How to do it, simply

1. `Win + Shift + S` → select the area → the screenshot goes to the clipboard.
2. Paste it into Paint (or any image editor) and save as **PNG**.
3. Crop to the window, without the desktop around it.
4. Aim for a width of **1200 to 1600 pixels** and less than **500 KB** per image
   (GitHub displays at most 1200 px wide in the README).
5. Put the file here, with the exact name from the table.

## Once the six images are in place

Tell me: I will check the names, dimensions and size, verify that no token is
visible, then prepare the publication commit and the tag.
