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

## Ce qui est en place : l'entrée obsolète est réinitialisée

Orchestra retient la dernière version qui avait la permission (`screen_granted_version` dans les
réglages). Au lancement d'une session autonome, si la permission manque alors qu'une version
**précédente** l'avait, l'entrée laissée par la mise à jour est obsolète : Orchestra lance
`tccutil reset ScreenCapture app.orchestra.desktop` (seulement l'entrée d'Orchestra, pas celles des
autres apps), puis redemande la permission. macOS affiche alors sa fenêtre normale au lieu d'une case
cochée qui ne marche pas. Rien n'est réinitialisé à la première installation, ni si la permission a été
retirée à la main pour la version en cours, ni hors d'`Orchestra.app` (`cargo run`).

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

## Developer ID : ce qu'il reste à faire (une fois)

Choix fait le 20/11/2026 : abonnement Apple Developer, donc certificat **Developer ID Application**.
Tout est prêt dans les scripts ; il manque le certificat et les secrets, que seul le titulaire du
compte peut créer.

1. **Créer le certificat** sur le Mac : Xcode › Réglages › Comptes › (le compte) › Gérer les
   certificats › « + » › *Developer ID Application*. (Ou developer.apple.com › Certificates, avec une
   demande de signature faite dans Trousseaux d'accès.)
   **Aucun service à cocher** : un Developer ID Application ne demande ni identifiant d'app
   (*Identifiers*) ni capacités (*Capabilities* : iCloud, Push…). Si la page *Identifiers* propose
   des services, c'est pour l'App Store : ne rien créer là. Sur *Certificates*, choisir seulement
   *Developer ID Application* (pas *Installer*, pas *Apple Distribution*) ; si on demande le profil
   « G2 Sub-CA », garder celui proposé par défaut.
2. **L'exporter** : Trousseaux d'accès › Mes certificats › clic droit sur
   « Developer ID Application: … » › Exporter… › format .p12, avec un mot de passe.
3. **Créer un mot de passe pour app** : account.apple.com › Connexion et sécurité › Mots de passe
   pour app.
4. **Ajouter les secrets** dans GitHub › dépôt › Settings › Secrets and variables › Actions :

   | Secret | Valeur |
   |---|---|
   | `MACOS_CERT_P12` | `base64 -i certificat.p12 \| pbcopy`, puis coller |
   | `MACOS_CERT_PASSWORD` | le mot de passe du .p12 |
   | `MACOS_SIGN_IDENTITY` | le nom exact : `Developer ID Application: Prénom Nom (ABCDE12345)` |
   | `APPLE_ID` | l'adresse du compte Apple |
   | `APPLE_TEAM_ID` | l'identifiant d'équipe (10 caractères, developer.apple.com › Membership) |
   | `APPLE_APP_PASSWORD` | le mot de passe pour app de l'étape 3 |

À la version suivante, `build-macos.sh` signe avec ce certificat (hardened runtime,
`packaging/entitlements.plist` : seulement le droit Apple Events, pour osascript), puis
`notarize-macos.sh` envoie l'app à Apple, attend la notarisation et agrafe le ticket. Le .zip des
mises à jour et le .dmg contiennent alors l'app notarisée : plus de « Ouvrir quand même », et la
permission « Enregistrement de l'écran » est conservée d'une version à l'autre (après l'avoir accordée
une dernière fois à la première version signée).

Une fois les secrets en place, mettre `MACOS_REQUIRE_IDENTITY: "1"` dans le workflow pour qu'une
version ne puisse plus partir signée ad hoc par erreur.

La première version signée avec le certificat demandera la permission une dernière fois ; ensuite elle
sera conservée.

## Mises à jour : même développeur Apple exigé

Une fois l'app installée signée avec un Developer ID, la mise à jour intégrée (`src/update.rs`,
`signature_allows`) vérifie la nouvelle version avant de la mettre en place : signature valide pour
tout le bundle (`codesign --verify --deep --strict`) **et** même identifiant d'équipe
(`TeamIdentifier`) que l'app installée. Sinon la mise à jour est refusée et l'ancienne app reste.
Une app installée signée ad hoc accepte encore n'importe quelle version (avec l'empreinte SHA-256),
pour que la première version signée Developer ID puisse s'installer.
