# Orchestra — fiche des règles

## Window

| Value | Build |
|---|---|
| Default size | 1320 × 860 |
| Minimum size | 640 × 480 |
| Quick search panel | 755 wide, at most 700 high |

### Zoom

Zoom goes from 0.5 to 4.9 in steps of 0.1, default 1. Bound to CmdOrCtrl+Plus / CmdOrCtrl+= /
CmdOrCtrl+- / CmdOrCtrl+0. The zoom factor is saved in the settings and rounded to one decimal so
repeated steps don't drift.

### Tabs

Tab strip 36 px high. Tab width = available width ÷ tab count, clamped to [36, 200]. Each tab has
its own back/forward history, and the tabs open last time come back at start.

### Menu shortcuts

| Shortcut | Action |
|---|---|
| CmdOrCtrl+T | new tab |
| CmdOrCtrl+W | close tab |
| CmdOrCtrl+Shift+T | reopen last closed tab |
| Ctrl+Tab / Ctrl+Shift+Tab | next / previous tab |
| Cmd+Option+Right/Left, Cmd+Shift+] / [ (macOS only) | next / previous tab |
| CmdOrCtrl+1…8, CmdOrCtrl+9 | tab n / last tab |
| CmdOrCtrl+[ / CmdOrCtrl+] | history back / forward |
| CmdOrCtrl+\ | show/hide sidebar |
| CmdOrCtrl+L | copy link to current page |
| CmdOrCtrl+Alt+L | copy link with the page title |
| CmdOrCtrl+Plus, =, -, 0 | zoom in, in, out, reset |
| CmdOrCtrl+Z / Shift+Z | undo / redo |
| Cmd/Ctrl+P or K | search |
| Cmd/Ctrl+N | new page |

Copied links use `notes-libres://page/<id>`. Running `notes-libres notes-libres://page/<id>`
opens the window on that page.

### Colours (design tokens)

| Token | Light | Dark | Used for |
|---|---|---|---|
| background.primary | rgba(255,255,255) | rgba(25,25,25) | page, windows |
| background.secondary | rgba(249,248,247) | rgba(32,32,32) | sidebar |
| background.tertiary | rgba(240,239,237) | rgba(56,56,54) | callout and code blocks, tab bar |
| background.interactive | rgba(244,243,243) | rgba(38,38,38) | hover |
| text.primary | rgba(44,44,43) | rgba(240,239,237) | text |
| text.secondary | rgba(125,122,117) | rgba(173,169,163) | faint text |
| text.tertiary | rgba(161,158,153) | rgba(125,122,117) | ticked to-do |
| border.primary | rgba(230,229,227) | rgba(56,56,54) | lines |
| blue 600 / 400 | rgba(35,131,226,1) / 0.35 | same | links / text selection |

## Data

* One workspace = pages (nested by `parent`), each page = ordered blocks, optionally one database.
* Stored as one JSON file: `$NOTES_LIBRES_DIR/workspace.json`, default `<user data dir>/notes-libres/workspace.json`.
* Saved atomically (temp file + rename) 400 ms after the last edit; on quit too.
* The window re-reads the file when it changes on disk (checked every 1.2 s while there are no unsaved edits) — this is how AI edits made through MCP appear.
* No account, no page/block/upload limits, no plan, no upgrade prompt, no trial.

## Blocks

Kinds: text, heading 1/2/3, to-do, bulleted, numbered, toggle, quote, callout, code, divider, sub-page link.

* Enter: new block below; text after the cursor moves into it. List kinds (to-do, bullet, numbered) continue the list; Enter on an empty list item turns it into text.
* Shift+Enter: line break inside the block. In code blocks Enter is a line break.
* Backspace on an empty block: a non-text block becomes text; an empty text block is deleted and focus goes to the previous block.
* Up/Down at the start/end of a one-line block: move to the previous/next block.
* `/` at the start of a block opens the block menu, filtered by the words typed; Enter picks the first match; Esc cancels.
* Markdown shortcuts at the start of a text block: `# ` `## ` `### ` headings, `- ` `* ` bullet, `[] ` `[ ] ` to-do, `1. ` numbered, `> ` fold-out menu, `" ` quote, `! ` callout, ```` ``` ```` code, `---` divider.
* Block handle `⋮⋮`: turn into, move up/down, duplicate, delete.

## Pages

* Sidebar tree with expand arrows, `+` adds a sub-page; right-click: add sub-page, duplicate, trash.
* Favourites section, breadcrumb, icon picker, page title, full-width option, word count.
* Trash: moving a page moves its sub-pages; restore puts a page whose parent is still trashed at the top level; delete forever leaves a tombstone so syncs don't bring it back.
* Quick search: Cmd/Ctrl+P or Cmd/Ctrl+K, titles + block text + database cells, Enter opens the first result. Cmd/Ctrl+N: new page.
* Markdown export (one page or all) and import (window or command line).

## Databases

* Column types: text, number, select (options), checkbox, date (`AAAA-MM-JJ`). The first column is the row title.
* New database: columns Nom / Statut (À faire, En cours, Terminé) / Date and one row.
* Views: table (edit cells, rename/retype/delete columns), Kanban (grouped by the first select column, ◀ ▶ to move a card), list.
* Filter: rows where any cell contains the text (case-insensitive). Sort: by one column, numbers numerically (`,` accepted as decimal point), other columns alphabetically ignoring case.
* Each row can be opened as its own page (created on first open, child of the database page).

## Collaboration — the connector you plug in yourself

* Connectors: none, shared folder, own command. Rust code can add more by implementing `Connector` (pull / push).
* Shared folder: each person writes `<name>.json` (personal settings removed) and reads everyone else's.
* Own command: `<command> pull` prints a workspace JSON (or a JSON list of them, or nothing); `<command> push` receives the workspace JSON on standard input. A non-zero exit is reported as a failure.
* Merge: page by page, the copy with the later `updated` time wins; tombstones newer than a page remove it.
* Auto-sync every 30 s when ticked; `notes-libres sync` runs one sync (for cron etc.).

## AI — MCP

* `notes-libres mcp`: JSON-RPC 2.0 over stdin/stdout, one message per line, protocol version `2025-06-18` (echoes the client's requested version).
* Tools: list_pages, search_pages, read_page, create_page, append_to_page, replace_page_content, update_block, update_page, trash_page, restore_page, create_database, query_database, add_database_row, update_database_row, delete_database_row. Pages may be named by id or title.
* Resources: every page as `page://<id>` in Markdown.


### Font and look

* Font stack `ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI Variable Display",
  "Segoe UI", Helvetica, "Apple Color Emoji", Arial, sans-serif`. The build loads the system font (SF on macOS, Segoe UI on Windows, Inter on
  Linux when present) and uses its weight axis at 600 for titles and headings.
* Menu shadow: light `outline.lg` = `0 20px 24px rgba(25,25,25,0.05), 0 5px 8px
  rgba(25,25,25,0.029), 0 0 0 1px rgba(42,28,0,0.07)` The build
  uses one blurred shadow and a 1 px border of the same colours.
* Hover tint: `background.light.interactiveTranslucent` rgba(33,27,23,0.05), dark
  rgba(255,255,255,0.056). Sidebar labels use `text.accentPrimary` (95,94,89) / (188,186,182).
* Drop line while dragging a block: palette blue 500, rgba(35,131,226,0.57).

### Layout

* Page column 708 px of text (900 px page minus 96 px padding each side), with a 44 px block
  gutter on the left. Full width keeps 96 px padding.
* Page icon 78 px, title 40 px semi-bold, "Ajouter une icône" shown only on hover.
* Body 16 px; headings 30 / 24 / 20 px semi-bold, with 22 / 16 / 10 px above them.
* Sidebar rows 28 px, 14 px labels, rounded 6 px hover. Page icons turn into the expand
  arrow on hover, and "+" appears at the right.
* Database table: 34 px rows, title column 280 px, others 180 px, check boxes 100 px; grey
  headers with a type symbol, 1 px border-colour grid, "OUVRIR" on hover, "+ Nouveau" row,
  and select options as coloured tags.

### Block drag and drop

* On hover, a "+" (add a block below, opening the "/" menu) and a six-dot handle appear in
  the gutter. Click the handle for the block menu (turn into, duplicate, delete). Drag it to
  move the block.
* While dragging: the block is dimmed in place, a faint copy follows the pointer, and a 4 px
  blue line marks the drop position. The page scrolls when the pointer is within 40 px of the
  top or bottom edge. Escape cancels.
* Drop position = the number of blocks whose middle is above the pointer. Dropping just above
  or below the block itself changes nothing.
* Checked by simulated mouse tests: drag down, drag up, drop in place.


### Main window

* Window: the sidebar runs the full height beside the macOS window buttons. Tabs sit only above the
  page. The build hides the title bar and lets the empty part of the tab strip move the window
  (double-click maximises).
* Sidebar 268 px wide (resizable, remembered). Rows about 31 px apart, and the search box is
  32 px high with a ⌘K badge. Order: "Accueil" pill (always shaded) with two icon shortcuts,
  Nouvelle page, a second item, Projets, Orchestration, Boîte de réception (no Bibliothèque
  any more), the sections, then the bottom items, a card and
  the workspace switcher. Line icons, not emoji, except page icons.
* Tabs at most 200 px wide. Tab strip 36 px tall, with thin lines between tabs; the
  open tab joins the page.
* Page bar: breadcrumb, then lock and "Privée" ⌄ (or "Partagée" when a connector is set). On
  the right: lock and "Partager" (opens Collaboration), copy link, star, •••.
* Page: icon top about 95 px below the page bar and about 36 px between icon and title, both
  as measured. Title 40 px bold (700), first block about 20 px below the title. One-line blocks
  are about 31 px apart: 24 px lines plus 6 px.
* Done to-dos are struck through in tertiary text. Inline `**bold**`, `*italic*`, `` `code` ``
  (red on a grey wash) and `~~strike~~` are drawn styled, with their markers hidden except while
  editing.
* Tab / Shift+Tab nests and un-nests a block (at most one level under the block above, 26 px
  per level). Dragging a block carries its nested blocks.
* Theme follows the system by default; Paramètres offers
  Système / Clair / Sombre.


### "/" menu

* A floating window 375 px wide: a heading ("Blocs de base", or "Résultats filtrés" once something
  is typed after "/"), then 32 px rows with a line icon, the name in 16 px, and the Markdown
  shortcut on the right in tertiary grey.
* Past 8½ rows it scrolls. At the bottom, after a line, sits "Fermer le menu · esc".
* It sits below the block, or above it when there's no room. Colours: window #252525, chosen row #313131, border #383836. The typed "/…" text gets a
  soft rounded background.
* Order: Texte, Titre 1–3, Liste à puces, Liste numérotée, Liste de tâches, Menu dépliant, Page,
  Encadré, Citation, Séparateur, Code.
* ↑/↓ choose, Entrée applies, Échap closes and keeps the text. The pointer chooses on hover and
  applies on click.
* "> " makes a fold-out menu and `" ` a quote.

### Settings window

* A large window over the app, which is darkened to 44 % black: 66 px margins, at most 1190 × 725.
  Colours: window #191919, 1 px border #383836, left column 240 px
  #202020, open item #2C2C2C.
* Left column: a search box, then the groups Application (Préférences, Raccourcis clavier),
  Espace de travail (Général, Barre latérale, Importer et exporter) and Fonctionnalités (IA (MCP),
  Collaboration).
* Right side: a 26 px bold title with a subtitle, sections with a line under them, and rows of
  name (semi-bold 14) / description (13, grey) / control. The controls are drop-down lists with a
  #30302E border and a chevron, switches (#4E4F4F off, accent blue on), fields and buttons.
* × closes, as does Échap or a click outside.

### Sidebar bottom

* The bottom block's height is measured each frame, so the page list never runs into it. 18 px are
  kept under the workspace switcher.


### Settings window on small windows

* Margins around the window are 66 px from 1000 px wide, 24 px from 600 px and 8 px below that.
  Vertically, 66 px from 800 px high and 16 px below. The window is at most 1190 × 725 and never
  larger than the app's window; its content is clipped to it.
* The left column is 26 % of the width, between 200 and 240 px. Under 720 px it shows icons
  only, with the name on hover.
* The content column is at most 800 px wide, with padding of 68 / 40 / 20 px depending on the
  width.
* A row puts its control on the right when the column is at least 460 px wide, with the text
  wrapping beside it; otherwise the control goes under the text.
* Check: at 800 × 722 every control stays inside the window.

### Real-time collaboration ("En direct")

* One person hosts (from the window, or with `notes-libres serveur [port] [code]` on a machine
  that stays on). The others join with the address and a shared code; a wrong code is refused.
  The default port is 7878.
* One JSON message per line over TCP. The host relays each message to the others. Messages:
  hello, pages (the changed pages and the pages deleted for good), presence (who is on which
  page, sent when the open page changes and every 5 s, dropped after 16 s of silence), bye.
* Changes leave within about 250 ms of a pause in typing, and the window saves 400 ms after an
  edit. Edits made by the AI through MCP, or by an import, are sent too: anything newer than
  what was last sent goes out.
* Merging is block by block. Each block carries the time of its last edit. Blocks deleted
  locally are recorded as deleted, with the time. A block edited on both sides keeps the most
  recent edit, and blocks added on either side are kept. The page's own settings and block order
  come from the copy edited last.
* A hybrid clock (never going backwards, never behind a time seen from another computer) keeps
  "most recent" meaningful when clocks differ.
* After a merge the window doesn't echo back unless it kept something the sender lacks.
* A joiner retries every 3 s after losing the connection, then sends everything again.
* The headless server keeps its own merged copy in its data folder and gives it to whoever
  joins.
* The page bar shows "En direct" in green, and round faces with initials for the people on the
  same page.
* Plain TCP without encryption, meant for one network. At a distance, use a private network
  (Tailscale, ZeroTier, WireGuard) or your own tunnel.
* Checks:
  * Tests: two people editing different blocks of one page keep both edits, in either merge
    order; added blocks are kept and deleted ones stay deleted; pages go both ways over a real
    connection; a wrong code is refused.
  * The real program, `serveur` with two scripted participants: Bob received Alice's page, the
    intruder was refused, and the server kept the page.

### AI server (MCP) check

A scripted session against the real program, as Claude Desktop does it, covering:
initialize (protocol 2025-06-18), initialized, tools/list (14 tools), create_page with
Markdown, search_pages, read_page, append_to_page, create_database, add_database_row,
query_database with a filter, an unknown page (tool error), an unknown tool (-32602),
resources/list, ping, an unknown method (-32601), malformed JSON (-32700), a batch, and
trash_page. All 17 replies came back correct, with nothing on the error output and exit code 0.
`replace_page_content` now marks the replaced blocks deleted, so a live merge doesn't bring them
back.

### Version for a friend (macOS)

* Built with `sh packaging/build-macos.sh` into `dist/Notes-libres-<version>-macOS.zip`.
* One universal program (Apple chip and Intel) inside "Notes libres.app", with Info.plist
  (macOS 11+, the local-network message for live collaboration), its own icon, a signature
  (packaging/sign-macos.sh), and LISEZMOI.txt (how to open it the first time, how to work together).
* With a Developer ID certificate and the Apple secrets (docs/permissions-macos.md) it is signed,
  notarised and stapled: it opens at once. Without them it is signed ad hoc and the first launch
  needs « Ouvrir quand même » in Réglages Système → Confidentialité et sécurité.
* Updates: once the installed app has a Developer ID, a new version is installed only if it is
  validly signed by the same Apple team (update.rs, signature_allows).
* Started without a terminal (as from Finder), it opens the window: checked.

## Command line

| Input | Output | Exit |
|---|---|---|
| (none), from a terminal | nothing | 1 |
| (none), from Finder or a shortcut | opens the window | 0 when closed |
| `--help` | nothing | 1 |
| `--version` | nothing | 1 |
| `ouvrir` (or `open`) | opens the window from a terminal | 0 when closed |
| `aide` / `help` / `-h` | help text | 0 |
| `version` / `-V` | `notes-libres 0.1.0` | 0 |
| `notes-libres://page/<id>` | opens the window on that page | 0 when closed |
| `serveur [port] [code]` | headless live-collaboration meeting point; prints its address | runs until stopped |
| unknown option | message + help on stderr | 2 |

"From a terminal" means both standard input and standard output are a terminal.
