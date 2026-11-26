# Architecture

Orchestra est un seul binaire Rust : la fenêtre ([egui](https://github.com/emilk/egui) /
eframe), le serveur MCP (`orchestra mcp`) et les commandes en ligne. Tous lisent et écrivent
le même fichier `workspace.json` ; la fenêtre le relit quand il change sur le disque (une IA
vient d'écrire par MCP, une synchronisation est passée).

```text
            ┌──────────────┐     stdio (JSON-RPC)    ┌──────────────┐
  IA  ◀────▶│ orchestra mcp│────────┐                │  la fenêtre  │
            └──────────────┘        ▼                └──────┬───────┘
                              workspace.json ◀──────────────┘
                                    ▲
                 en direct ───┘   (collab.rs, live.rs)
```

## Les données — `model.rs`, `project.rs`, `store.rs`

- Un **espace de travail** (`Workspace`) = des pages imbriquées (`parent`), chacune faite de
  blocs ordonnés, avec au plus une base de données.
- Une page peut porter un **projet** (`Page::project`, voir `project.rs`) : dossier du code,
  statut, état des lieux (`done` / `in_progress` / `blockers`), étapes, sessions liées. Un
  projet se sauve, se synchronise et s'exporte comme n'importe quelle page.
- La **boîte de réception** (`Workspace::inbox`) et les **sessions autonomes**
  (`Workspace::runs`) vivent aussi dans l'espace de travail.
- `store.rs` : emplacement des données (`$ORCHESTRA_DIR`), chargement, enregistrement
  atomique, export / import Markdown.
- `data_folder.rs` : le dossier des données choisi dans les réglages, appliqué au lancement
  de la fenêtre (copie dans un dossier vide ; l'ancien n'est jamais effacé).

## La fenêtre — `app.rs` et `app/`

`app.rs` tient l'état (`NotesApp`), la boucle par image et l'éditeur de blocs. Le reste est
découpé par zone :

| Module | Rôle |
|---|---|
| `app/chrome.rs` | barre latérale, onglets, en-tête de page, bouton rond IA |
| `app/projects.rs` | la vue d'un projet : cartes état des lieux, étapes, sessions, versions |
| `app/ai_fill.rs` | « Remplir avec l'IA », « Suggérer des étapes », texte → étapes |
| `app/ai_ui.rs` | les IA installées / branchées, l'utilisation de l'abonnement |
| `app/orch_ui.rs` | la page Orchestration et la boîte de réception |
| `app/versions.rs` | enregistrer, envoyer, publier, état des Actions GitHub |
| `app/notices.rs` | messages de l'application : boîte de réception et fenêtres d'erreur |
| `app/term_ui.rs` | le terminal intégré (dock en bas) |
| `app/settings.rs` | la fenêtre des réglages |
| `app/search.rs`, `app/slash.rs`, `app/database.rs`, `app/rich.rs` | recherche, menu `/`, bases, texte riche |
| `app/live_ui.rs`, `app/side.rs` | travail en direct, panneau latéral (discussion d'équipe) |
| `app/icons.rs` | les icônes, dessinées en traits (pas de police d'icônes) |

Le travail long (appel d'une IA, `git`, téléchargement) tourne dans un fil à part ; la fenêtre
relève le résultat à chaque image dans une fonction `…_tick`.

## Les IA

| Module | Rôle |
|---|---|
| `mcp.rs` | le serveur MCP : outils pages, bases, projets, étapes, `ask_human` |
| `setup.rs` | branchement automatique aux IA trouvées sur la machine |
| `sync.rs` | appels « sans tête » à Claude Code / Codex, en lecture seule : état des lieux, étapes proposées, regard extérieur |
| `auto.rs` | le mode autonome (voir [mode-autonome.md](mode-autonome.md)) |
| `sessions.rs` | lecture en direct des fichiers de session de Claude Code et Codex |
| `usage.rs` | utilisation de l'abonnement Claude, via la ligne d'état |
| `terminal.rs` | pseudo-terminal (portable-pty + vt100) |

## Le reste

| Module | Rôle |
|---|---|
| `docs.rs` | README, `docs/`, `CLAUDE.md`, CHANGELOG d'un projet repris en pages, tenus à jour |
| `templates.rs` | les modèles de projet (code, étudiant, équipe, événement) : modules et pages de départ |
| `git.rs` | `git` et `gh` de la personne ; l'application ne manipule jamais de jeton |
| `collab.rs` | fusion de copies de l'espace, page par page puis bloc par bloc |
| `live.rs`, `secure.rs` | travail en direct sur TCP, chiffré (SPAKE2 + ChaCha20-Poly1305) |
| `update.rs` | mise à jour automatique depuis les publications GitHub |
| `i18n.rs` | `tr!` / `trf!` : chaque texte en français et en anglais |
| `shot.rs` | aide au développement : la fenêtre s'enregistre en image (`ORCHESTRA_SHOT`) |

## Conventions

- Chaque texte visible passe par `crate::tr!("français", "english")` (ou `trf!` avec des
  arguments).
- Les messages pour la personne passent par `self.ack`, `self.info` ou `self.fail`
  (voir [notifications.md](notifications.md)), jamais par une barre d'état.
- Pas de symbole Unicode décoratif dans les boutons : une icône de `app/icons.rs`, ou rien.
