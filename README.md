# Orchestra

**Un tableau de bord de bureau pour suivre vos projets et les IA qui travaillent dessus**
(Claude Code, Codex, Gemini CLI, l'app Claude…). Tout reste sur votre ordinateur : pas de
compte, pas d'abonnement, pas de serveur d'entreprise.

Orchestra réunit au même endroit :

- **des pages et des bases de données** : blocs, `/` pour choisir un type, glisser-déposer,
  tableaux, Kanban, listes ;
- **des projets IA** : dossier du code, statut, état des lieux, étapes à suivre, sessions
  Claude Code / Codex en direct, terminal intégré, docs du projet (README, `docs/`, `CLAUDE.md`…)
  reprises en pages ;
- **l'orchestration** : une IA avance seule sur un projet, dans une copie du dépôt, avec des
  limites sur l'utilisation de l'abonnement ;
- **les versions** : enregistrer (commit), envoyer (push), publier une version GitHub ;
- **une boîte de réception** : questions des IA, fins de session, rapports, et tous les
  messages de l'application (les erreurs s'affichent aussi dans une petite fenêtre) ;
- **le travail à plusieurs** : dossier partagé, connecteur maison, ou en direct, chiffré.

Les IA lisent et écrivent dans Orchestra par **MCP** : au démarrage, l'application se branche
d'elle-même à celles qu'elle trouve sur la machine.

---

## Installer

Téléchargez le fichier de votre système dans les
[publications GitHub](../../releases) : `.dmg` (macOS), `.exe` (Windows), `.deb` (Linux).

L'application n'est pas signée : la première fois, macOS la bloque. Ouvrez Réglages Système →
Confidentialité et sécurité → « Ouvrir quand même », ou dans le Terminal :

```sh
xattr -dr com.apple.quarantine "/Applications/Orchestra.app"
```

Une fois installée, Orchestra se met à jour seule depuis les publications du dépôt.

## Compiler depuis les sources

Il faut [Rust](https://rustup.rs) (édition 2024, donc Rust 1.85 ou plus récent).

```sh
cargo run --release          # lance la fenêtre
cargo test                   # les tests
packaging/build-macos.sh     # construit Orchestra.app (macOS)
```

Sous Linux, installez d'abord les bibliothèques de fenêtrage :
`libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgl1-mesa-dev`.

## Ligne de commande

```text
orchestra ouvrir                    ouvre la fenêtre (onglets de la dernière fois)
orchestra orchestra://page/<id>     ouvre la fenêtre sur cette page
orchestra mcp                       serveur MCP (stdio) pour brancher une IA
orchestra statusline                ligne d'état de Claude Code (relève l'utilisation de l'abonnement)
orchestra export <dossier>          exporte toutes les pages en Markdown
orchestra import <fichier.md>       importe un fichier Markdown comme page
orchestra sync                      synchronise une fois avec le branchement configuré
orchestra serveur [port] [code]     point de rencontre en direct, sans fenêtre (port 7878)
orchestra aide | version
```

Les données sont dans `$ORCHESTRA_DIR`, sinon dans le dossier de données de l'utilisateur
(`~/Library/Application Support/orchestra` sur macOS), fichier `workspace.json`.

## Brancher une IA

Rien à faire en général : au démarrage, Orchestra s'ajoute comme serveur MCP à Claude Code,
Codex, Gemini CLI et l'app Claude quand ils sont installés (une copie `.bak` des fichiers de
réglages est gardée). Réglages → IA (MCP) permet de brancher ou débrancher chacune.

À la main, pour Claude Code :

```sh
claude mcp add orchestra -- /chemin/vers/orchestra mcp
```

Une fois branchée, une IA peut appeler `get_project_state`, mettre à jour l'état des lieux,
cocher les étapes, poser une question dans la boîte de réception… Voir
[docs/ia-et-mcp.md](docs/ia-et-mcp.md).

## Documentation

| Page | Contenu |
|---|---|
| [docs/architecture.md](docs/architecture.md) | Les modules du code et comment ils s'articulent |
| [docs/ia-et-mcp.md](docs/ia-et-mcp.md) | Les outils MCP, le branchement automatique, ce que les IA ont le droit de faire |
| [docs/mode-autonome.md](docs/mode-autonome.md) | L'orchestration : étapes, limites, copies du dépôt, rapports |
| [docs/notifications.md](docs/notifications.md) | Boîte de réception, fenêtres d'erreur, confirmations |
| [MECHANICS.md](MECHANICS.md) | La fiche des règles : tailles, raccourcis, couleurs, données |
| [examples/connecteur-exemple.sh](examples/connecteur-exemple.sh) | Un connecteur de synchronisation maison |

## Publier une version

Le bouton « Publier une version » d'un projet (ou un tag `vX.Y.Z` poussé à la main) déclenche
le workflow [`.github/workflows/release.yml`](.github/workflows/release.yml), qui construit
les apps macOS, Windows et Linux et les joint à la publication GitHub. Les tests tournent à
chaque push sur `main`.

## Licence

MIT.
