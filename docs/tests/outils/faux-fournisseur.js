// Faux fournisseur pour la recette d'ApiVault Proxy.
// Ecoute sur 127.0.0.1:3999 et se comporte comme une API compatible OpenAI.
// Aucun acces reseau externe. Ecrit chaque corps recu dans un journal.
//
// Utilisation : node faux-fournisseur.js
//   GET  /v1/models              -> liste de modeles (contient modele-test)
//   POST /v1/chat/completions    -> reponse JSON avec usage 17 / 23,
//                                   ou flux SSE SANS bloc usage (cas estime)
//   POST /v1/lent                -> attend 5 s puis repond (marge de test)
//
// Journal : %TEMP%\recette-apivault\corps-recus.log
// (le meme dossier que le bac a sable de RECETTE-INTERNE.md ; surchargeable
//  avec la variable APIVAULT_RECETTE_JOURNAL)

const http = require("http");
const fs = require("fs");
const path = require("path");

const PORT = 3999;
const journal = process.env.APIVAULT_RECETTE_JOURNAL
  || path.join(process.env.TEMP || ".", "recette-apivault", "corps-recus.log");

function noter(entree) {
  try {
    fs.mkdirSync(path.dirname(journal), { recursive: true });
    fs.appendFileSync(journal, JSON.stringify(entree) + "\n", "utf8");
  } catch (e) {
    console.error("journal impossible:", e.message);
  }
}

function lireCorps(req) {
  return new Promise((resolve) => {
    const morceaux = [];
    req.on("data", (c) => morceaux.push(c));
    req.on("end", () => resolve(Buffer.concat(morceaux).toString("utf8")));
  });
}

const serveur = http.createServer(async (req, res) => {
  const url = req.url.split("?")[0];

  if (req.method === "GET" && url === "/v1/models") {
    noter({ quand: new Date().toISOString(), methode: req.method, url });
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify({
      object: "list",
      data: [{ id: "modele-test", object: "model", owned_by: "recette" }]
    }));
    return;
  }

  if (req.method === "POST" && (url === "/v1/chat/completions" || url === "/v1/lent")) {
    const brut = await lireCorps(req);
    let corps = {};
    try { corps = JSON.parse(brut.replace(/^\uFEFF/, "")); } catch (e) { /* corps non JSON */ }
    noter({ quand: new Date().toISOString(), methode: req.method, url, corps: brut });

    if (url === "/v1/lent") {
      await new Promise((r) => setTimeout(r, 5000));
    }

    if (corps.stream === true) {
      // Flux SSE sans bloc usage : c'est le cas qui doit etre estime par le proxy.
      res.writeHead(200, {
        "content-type": "text/event-stream; charset=utf-8",
        "cache-control": "no-cache",
        connection: "keep-alive"
      });
      const morceaux = [
        { id: "chatcmpl-recette", object: "chat.completion.chunk", model: "modele-test",
          choices: [{ index: 0, delta: { role: "assistant", content: "Bonjour" }, finish_reason: null }] },
        { id: "chatcmpl-recette", object: "chat.completion.chunk", model: "modele-test",
          choices: [{ index: 0, delta: { content: " depuis le faux fournisseur." }, finish_reason: null }] },
        { id: "chatcmpl-recette", object: "chat.completion.chunk", model: "modele-test",
          choices: [{ index: 0, delta: {}, finish_reason: "stop" }] }
      ];
      for (const m of morceaux) {
        res.write("data: " + JSON.stringify(m) + "\n\n");
        await new Promise((r) => setTimeout(r, 40));
      }
      res.write("data: [DONE]\n\n");
      res.end();
      return;
    }

    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify({
      id: "chatcmpl-recette",
      object: "chat.completion",
      created: Math.floor(Date.now() / 1000),
      model: "modele-test",
      choices: [{ index: 0, message: { role: "assistant", content: "Bonjour depuis le faux fournisseur." }, finish_reason: "stop" }],
      usage: { prompt_tokens: 17, completion_tokens: 23, total_tokens: 40 }
    }));
    return;
  }

  noter({ quand: new Date().toISOString(), methode: req.method, url, note: "route inconnue" });
  res.writeHead(404, { "content-type": "application/json" });
  res.end(JSON.stringify({ error: { message: "route inconnue", type: "invalid_request_error" } }));
});

serveur.listen(PORT, "127.0.0.1", () => {
  console.log("faux fournisseur pret sur http://127.0.0.1:" + PORT);
  console.log("journal: " + journal);
});
