# Protocole de test — ApiVault Proxy

Document à suivre **par vous**, à votre rythme. Il se découpe en trois séances possibles.
Chaque étape porte un numéro : dans vos notes, écrivez simplement « étape 12 : OK » ou « étape 12 : la fenêtre affiche X ».

| | |
|---|---|
| Version testée | ______________ (affichée dans l'interface) |
| Commit | ______________ |
| Date de début | ______________ |
| Testeur | ______________ |

**Durée** : environ 75 minutes au total. Découpage conseillé : P0–P1 (15 min), P2–P3 (30 min), P4–P7 (30 min).

---

## Règles de passation

1. **Une étape = une action.** Ne regroupez pas, sinon un échec devient impossible à localiser.
2. **Si une étape échoue** : notez la sortie exacte, cochez FAIL, et **continuez**. Ne corrigez rien, ne réinstallez rien. Une étape qui dépend d'une étape en échec sera marquée « non concluante ».
3. **Ne recopiez jamais une clé API** dans vos notes, même partiellement. Écrivez « clé de test ».
4. **« Non testé » est une réponse valable.** Mieux vaut ça qu'un PASS de complaisance.
5. Application **fermée** pour P1. Application **ouverte** pour P2 à P6.

### Point de reprise

| Phase | Contenu | Fait |
|---|---|---|
| P0 | Préparation | [ ] |
| P1 | État et intégrité (app fermée) | [ ] |
| P2 | Le coffre et les clés | [ ] |
| P3 | Le service | [ ] |
| P4 | Compteur et plafond | [ ] |
| P5 | Démarrage et redémarrage | [ ] |
| P6 | Interface | [ ] |
| P7 | Clôture | [ ] |

---

# P0 — Préparation (5 min)

**À avoir sous la main :**

- une **clé de test** chez un fournisseur, que vous pouvez casser volontairement (jamais votre clé principale) ;
- idéalement **deux fournisseurs** configurés, pour tester le repli ;
- le droit de **redémarrer le PC** une fois ;
- l'application **fermée** au départ.

### Étape 1 — Fermer toute instance
Vérifiez qu'aucune fenêtre ApiVault Proxy n'est ouverte, et qu'aucun agent ne travaille sur le dépôt.
**Attendu** : rien qui tourne.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 2 — Noter la version et le commit
Dans un terminal, à la racine du dépôt :
```powershell
git log --oneline -1
```
**Attendu** : un commit, et vous le recopiez dans le tableau en haut de ce document.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

---

# P1 — État et intégrité (10 min, application fermée)

### Étape 3 — Le dépôt est propre
```powershell
git status --short
```
**Attendu** : aucune ligne, **sauf** `?? PROTOCOLE-TEST-UTILISATEUR.md` et éventuellement `?? RAPPORT-...`. Tout autre fichier modifié est à signaler.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 4 — Aucune clé dans l'historique
```powershell
git log -p -15 | Select-String -Pattern 'sk-[A-Za-z0-9]{8,}' | Measure-Object
```
**Attendu** : `Count : 0`.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 5 — Les tests passent
```powershell
cd Proxy-Externe\src-tauri
cargo test --release
```
**Attendu** : `0 failed` partout. Le total doit correspondre au chiffre du dernier rapport.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 6 — Aucun test masqué
```powershell
Select-String -Path tests\*.rs,src\*.rs -Pattern '#\[ignore\]' -Recurse | Measure-Object
```
**Attendu** : `Count : 0`.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 7 — Le binaire se construit
```powershell
cargo build --release
Get-Item target\release\apivault-proxy.exe | Select-Object Length
```
**Attendu** : `Finished`, et une taille proche de celle annoncée dans le rapport.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

> **Fin de la séance 1 possible ici.**

---

# P2 — Le coffre et les clés (15 min, application ouverte)

### Étape 8 — Démarrage
Lancez l'application.
**Attendu** : la fenêtre s'ouvre, les fournisseurs déjà configurés sont là, pas de message d'erreur.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 9 — Le fichier de clés est chiffré
```powershell
Get-Content "$env:APPDATA\ApiVault_Proxy\cles.env" -TotalCount 1
```
**Attendu** : `APIVAULT-COFFRE-1`.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 10 — Aucune clé en clair sur le disque
```powershell
Select-String -Path "$env:APPDATA\ApiVault_Proxy\*" -Pattern 'sk-','DEEPSEEK_KEY','OPENAI_API_KEY' -Recurse | Measure-Object
```
**Attendu** : `Count : 0`.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 11 — Ajouter un fournisseur
Bouton **Fournisseurs** → **Ajouter un fournisseur**. Renseignez : nom, URL (`.../v1`), **clé de test**, modèles.
Cliquez **Tester la clé**, puis **Enregistrer**.
**Attendu** : le test répond, le fournisseur apparaît dans la liste, le service ne tombe pas.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 12 — Une clé fausse est refusée proprement
Ajoutez un second fournisseur avec une **clé volontairement fausse**. Cliquez **Tester la clé**.
**Attendu** : message d'erreur lisible, sans détail interne, **sans la clé**, et le reste de l'application continue de fonctionner.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 13 — Changer une clé API
Sur le fournisseur de l'étape 11, remplacez la clé par une autre clé valide (rotation), enregistrez.
**Attendu** : la nouvelle clé fonctionne ; l'ancienne n'est plus utilisée.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 14 — La sauvegarde ne garde pas l'ancienne clé
Après la rotation de l'étape 13 :
```powershell
Get-ChildItem "$env:APPDATA\ApiVault_Proxy" -Filter "*.bak"
```
Puis, si un `.bak` existe :
```powershell
Get-Content "$env:APPDATA\ApiVault_Proxy\cles.env.bak" -TotalCount 1
```
**Attendu** : soit aucun `.bak`, soit un `.bak` commençant par `APIVAULT-COFFRE-1`. **Jamais** de clé en clair.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 15 — Supprimer une clé
Supprimez le fournisseur à clé fausse de l'étape 12.
**Attendu** : il disparaît, et la sauvegarde associée est supprimée.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 16 — Les droits du dossier
```powershell
icacls "$env:APPDATA\ApiVault_Proxy"
```
**Attendu** : seul **votre compte** apparaît, sans entrée héritée `(I)` pour `BUILTIN\Users` ou `Everyone`.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

> **Fin de la séance 2 possible ici.**

---

# P3 — Le service (15 min)

### Étape 17 — La sonde est muette
```powershell
curl.exe -s http://127.0.0.1:3100/health
```
**Attendu** : `{"status":"ok","version":"..."}` — **deux champs seulement**.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 18 — Un hôte forgé est refusé
```powershell
curl.exe -s -o NUL -w "%{http_code}" -H "Host: evil.example" http://127.0.0.1:3100/health
```
**Attendu** : `403`.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 19 — L'administration exige le jeton
```powershell
curl.exe -s -o NUL -w "%{http_code}" http://127.0.0.1:3100/admin/pool
```
**Attendu** : `401`.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 20 — Le jeton ouvre l'administration
Copiez le **jeton d'accès** depuis l'interface (bouton voir/copier), puis :
```powershell
curl.exe -s -o NUL -w "%{http_code}" -H "Authorization: Bearer <JETON>" http://127.0.0.1:3100/admin/pool
```
**Attendu** : `200`.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 21 — Une vraie requête, en mode normal
Depuis votre client habituel, ou avec la configuration copiée depuis l'interface : une requête simple.
**Attendu** : une réponse correcte du modèle.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 22 — Une vraie requête, en flux
Même requête avec le mode **stream** activé.
**Attendu** : le texte arrive progressivement, sans coupure ni blocage.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 23 — Le repli fonctionne
Sur le fournisseur à clé fausse, envoyez une requête (ou rendez-le prioritaire).
**Attendu** : le service bascule sur l'autre fournisseur, la réponse arrive, l'erreur reste lisible.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

---

# P4 — Compteur et plafond (10 min)

> **LOT 5 et LOT 6 sont livrés** (18 octobre, jusqu'au commit de la recette interne).
> Attends-toi à voir des lignes `estime: true` : c'est normal sur une réponse en flux,
> le fournisseur n'envoie pas toujours son décompte. Les tokens doivent être **non nuls**.

### Étape 24 — Le journal existe
```powershell
Get-ChildItem "$env:APPDATA\ApiVault_Proxy\metriques" | Select-Object Name, Length
Get-Content "$env:APPDATA\ApiVault_Proxy\metriques\metriques-*.jsonl" -TotalCount 3
```
**Attendu** : le fichier du mois, **une ligne = une requête**, avec fournisseur, modèle et des nombres.
**Résultat** : [ ] PASS  [ ] FAIL  [ ] NON TESTÉ — Notes : ______________________

### Étape 25 — Le journal ne contient aucun contenu
```powershell
Select-String -Path "$env:APPDATA\ApiVault_Proxy\metriques\*" -Pattern 'sk-','Bearer ','prompt' -Recurse | Measure-Object
```
**Attendu** : `Count : 0`.
**Résultat** : [ ] PASS  [ ] FAIL  [ ] NON TESTÉ — Notes : ______________________

### Étape 26 — Les chiffres sont cohérents
Comparez le nombre de requêtes envoyées depuis le démarrage avec la vue **session** de l'interface.
**Attendu** : même nombre. Le cache est affiché **à part** (« dont relu du cache »).
**Résultat** : [ ] PASS  [ ] FAIL  [ ] NON TESTÉ — Notes : ______________________

### Étape 27 — Le plafond est désactivé par défaut
Ouvrez la carte de consommation.
**Attendu** : aucun plafond actif tant que vous ne l'avez pas fixé, et une carte visible pour le régler.
**Résultat** : [ ] PASS  [ ] FAIL  [ ] NON TESTÉ — Notes : ______________________

### Étape 28 — Le plafond refuse proprement
Réglez un plafond très bas, puis envoyez une requête.
**Attendu** : refus clair et compréhensible, service toujours debout, message qui dit pourquoi.
**Résultat** : [ ] PASS  [ ] FAIL  [ ] NON TESTÉ — Notes : ______________________

### Étape 29 — L'alerte à 80 %
Réglez un plafond légèrement au-dessus de votre consommation actuelle.
**Attendu** : une alerte apparaît à 80 %, avant le blocage.
**Résultat** : [ ] PASS  [ ] FAIL  [ ] NON TESTÉ — Notes : ______________________

> **Fin de la séance 3 possible ici.**

---

# P5 — Démarrage et redémarrage (10 min)

### Étape 30 — Activer le démarrage automatique
Interface → **Démarrer avec Windows**.
**Attendu** : l'interrupteur s'active sans erreur.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 31 — La tâche appartient à votre compte
```powershell
schtasks /query /tn "ApiVault_Proxy_Externe" /v /fo list
```
**Attendu** : « Run As User » = **votre compte Windows**, pas `SYSTEM`. C'est ce qui permet au coffre de se déchiffrer au démarrage.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 32 — Redémarrer le PC
Redémarrez, reconnectez-vous, attendez une minute sans rien lancer.
**Attendu** : l'application démarre seule.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 33 — Les clés sont toujours lisibles
Après le redémarrage, ouvrez **Fournisseurs**.
**Attendu** : vos fournisseurs sont là, aucun message « clés illisibles », et une requête fonctionne.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 34 — Deux instances
Lancez l'application une seconde fois alors qu'elle tourne déjà.
**Attendu** : message clair « port occupé » ou équivalent, pas de plantage, pas de seconde instance fantôme.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

---

# P6 — Interface (10 min)

### Étape 35 — Le bouton Super IA est inerte
**Attendu** : grisé, libellé « en cours de développement », aucun clic ne déclenche d'appel.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 36 — Le bandeau reste visible
Faites défiler la page.
**Attendu** : le bandeau d'état reste collé en haut.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 37 — Copier l'adresse
Double-cliquez sur l'adresse `http://127.0.0.1:3100/v1`.
**Attendu** : l'adresse est copiée, avec un retour visuel.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 38 — L'état vide
Sur une installation sans fournisseur (ou après avoir tout retiré), ouvrez **Fournisseurs**.
**Attendu** : un message d'accueil, pas une page blanche.
**Résultat** : [ ] PASS  [ ] FAIL  [ ] NON TESTÉ — Notes : ______________________

### Étape 39 — Les infobulles
Survolez le voyant d'état et le temps de reprise.
**Attendu** : une explication en français, compréhensible sans documentation.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

---

# P7 — Clôture (5 min)

### Étape 40 — Remettre l'application dans un état propre
Retirez la clé fausse, remettez le plafond à zéro, désactivez le démarrage automatique si vous ne le voulez pas.
**Attendu** : état de départ retrouvé.
**Résultat** : [ ] PASS  [ ] FAIL — Notes : ______________________

### Étape 41 — Remplir la synthèse
Reportez ci-dessous le nombre de PASS, FAIL et NON TESTÉ.

| | Nombre |
|---|---|
| PASS | ____ |
| FAIL | ____ |
| NON TESTÉ | ____ |

### Étape 42 — Ce que vous m'envoyez
Pour chaque FAIL : **le numéro d'étape**, **ce que vous attendiez**, **ce que vous avez vu**. Une capture d'écran si c'est visuel.
Ne m'envoyez pas de clé, ni de jeton, ni un export de `cles.env`.

---

## Après réception de vos notes

Je classe chaque FAIL en **bloquant / majeur / mineur / cosmétique**, je corrige, et je vous dis quelles étapes il faut refaire — pas les 42. Ensuite seulement : audit externe sur le commit figé, puis publication.
