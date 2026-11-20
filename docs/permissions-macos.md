# Pourquoi la permission « Enregistrement de l'écran » se perd à chaque mise à jour

Diagnostic du 20/11/2026, fait en lisant le code (`packaging/build-macos.sh`, `packaging/Info.plist`,
`src/update.rs`, `src/screen.rs`).

## En bref

Orchestra est signé **ad hoc** (`codesign --sign -`, `packaging/build-macos.sh:42`). Pour une app
signée ad hoc, macOS mémorise la permission **pour ce binaire précis** (son empreinte, le
« cdhash »). Chaque nouvelle version est un binaire différent, donc une autre empreinte : pour macOS,
c'est une autre app. La case reste cochée dans Réglages Système, mais elle ne s'applique plus à la
version installée.

## Le mécanisme

1. Quand on coche Orchestra dans *Confidentialité et sécurité › Enregistrement de l'écran*, macOS (TCC)
   enregistre l'identifiant de l'app (`app.orchestra.desktop`) **et** son « exigence désignée »
   (designated requirement), qui dit à quoi reconnaître cette app plus tard.
2. Avec un certificat (Developer ID ou certificat personnel), l'exigence désignée est du type
   `identifier "app.orchestra.desktop" and certificate leaf = H"…"` : elle reste vraie pour toutes les
   versions signées avec le même certificat.
3. Avec une signature ad hoc il n'y a pas de certificat : l'exigence désignée se réduit à
   `cdhash H"…"`, l'empreinte du binaire. Elle n'est vraie que pour cette compilation-là.
4. La mise à jour (`src/update.rs:185-208`) remplace `Orchestra.app` par la nouvelle version (même
   identifiant, même chemin, mais nouveau binaire). Au lancement, `CGPreflightScreenCaptureAccess()`
   (`src/screen.rs:56`) compare l'app à l'exigence enregistrée : l'empreinte ne correspond plus, donc
   la réponse est « non », alors que la case paraît cochée.

Ce qui n'est **pas** en cause :
- l'identifiant du bundle : il est fixe (`app.orchestra.desktop`, `packaging/Info.plist:10`) ;
- le chemin : la mise à jour réinstalle au même endroit ;
- l'attribut de quarantaine : il est retiré après la mise à jour (`src/update.rs:206`) et n'a pas
  d'effet sur TCC.

## Qui doit avoir la permission

`take_screenshot` lance `screencapture` depuis le serveur MCP d'Orchestra, lui-même lancé par l'IA.
macOS attribue la capture à l'app « responsable » de cette chaîne de processus :
- pour les sessions autonomes, lancées dans le terminal caché d'Orchestra : **Orchestra** ;
- pour une session ouverte à la main dans Terminal ou iTerm : **ce terminal**.

C'est pourquoi les sessions autonomes perdent la capture à chaque mise à jour, alors qu'une session
lancée depuis Terminal continue de fonctionner.

## Pistes de correction (étapes suivantes)

| Piste | Garde la permission d'une version à l'autre | Coût | Remarque |
|---|---|---|---|
| Certificat **Developer ID** (Apple Developer Program) | Oui, sur tous les Mac | 99 $/an | Permet aussi la notarisation : plus de « Ouvrir quand même » au premier lancement. |
| **Certificat personnel** créé dans Trousseaux d'accès (auto-signé), toujours le même | Oui, sur tous les Mac | Gratuit | Gatekeeper avertit toujours au premier lancement, comme aujourd'hui. Le certificat et sa clé privée doivent être conservés et réutilisés à chaque version. |
| Rester en ad hoc + `tccutil reset ScreenCapture app.orchestra.desktop` après chaque mise à jour, puis redemander | Non : il faut recocher à chaque fois | Gratuit | Solution de secours seulement : elle évite l'entrée « fantôme » cochée mais inopérante. |

## Ce qui est en place : même identifiant, même identité de signature

`packaging/sign-macos.sh` (appelé par `build-macos.sh`) :
- vérifie que l'identifiant du bundle est toujours `app.orchestra.desktop` et s'arrête sinon ;
- signe avec **la même identité** à chaque version, prise dans cet ordre :
  1. `MACOS_CERT_P12` (le certificat et sa clé, fichier .p12 en base64) + `MACOS_CERT_PASSWORD`,
     importés dans un trousseau temporaire, nommés par `MACOS_SIGN_IDENTITY` (« Orchestra » par
     défaut). C'est ce que fait GitHub Actions avec les secrets du dépôt du même nom ;
  2. `MACOS_SIGN_IDENTITY` seul : un certificat déjà présent dans le trousseau du Mac ;
  3. rien : signature ad hoc comme avant, avec un avertissement (`MACOS_REQUIRE_IDENTITY=1` en fait
     une erreur) ;
- affiche l'exigence désignée obtenue et vérifie qu'elle porte bien l'identifiant.

Pour l'activer : créer le certificat (Developer ID ou personnel), l'exporter en .p12, puis dans
GitHub › Settings › Secrets and variables › Actions, ajouter `MACOS_CERT_P12`
(`base64 -i certificat.p12 | pbcopy`), `MACOS_CERT_PASSWORD` et `MACOS_SIGN_IDENTITY` (le nom
exact du certificat, ex. « Developer ID Application: Nom (ÉQUIPE) »). Toujours réutiliser ce même
certificat ensuite.

Dans les deux premiers cas, `build-macos.sh` doit signer avec `codesign --force --deep
--sign "<nom du certificat>"` au lieu de `--sign -` (plus `--options runtime` pour la notarisation
avec un Developer ID), et garder le même identifiant de bundle.
La première version signée avec le certificat demandera la permission une dernière fois ; ensuite elle
sera conservée.
