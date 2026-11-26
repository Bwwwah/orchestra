# Les IA et MCP

Orchestra est un **serveur MCP** (`orchestra mcp`, JSON-RPC sur l'entrée / sortie standard,
code dans `src/mcp.rs`). Une IA branchée peut lire et écrire les pages et les projets.

## Branchement automatique

Au démarrage (`src/setup.rs`), Orchestra s'ajoute aux IA trouvées sur la machine :

- **Claude Code** et **Codex** par leur propre commande `mcp add` ;
- **Antigravity CLI** (`agy`, fichier `~/.gemini/config/mcp_config.json`) et l'**app Claude** en ajoutant une entrée à leur fichier de réglages JSON
  (une copie `.bak` est gardée la première fois ; rien d'autre n'est modifié).

Une entrée qui pointe déjà vers ce programme est laissée telle quelle ; une qui pointe vers
une ancienne copie est mise à jour. Une IA débranchée dans Réglages → IA (MCP) n'est plus
rebranchée. Le bouton rond en bas à droite n'apparaît que tant qu'aucune IA n'est branchée.

## Les outils

| Outil | Pour |
|---|---|
| `list_pages`, `search_pages`, `read_page` | trouver et lire une page (en Markdown) |
| `create_page`, `append_to_page`, `replace_page_content`, `update_page`, `update_block`, `trash_page`, `restore_page` | écrire |
| `create_database`, `query_database`, `add_database_row`, `update_database_row`, `delete_database_row` | les bases |
| `list_projects`, `get_project_state` | où en est un projet (par id, titre ou dossier) |
| `create_project`, `update_project` | statut, état des lieux (`done`, `in_progress`, `blockers`) |
| `add_steps`, `update_step` | les étapes, et leur personne responsable (`assignee`) dans un espace d'équipe |
| `list_members` | les membres d'un projet d'équipe (personnes et IA de l'espace ; seule une personne peut être responsable) |
| `link_session` | rattacher une session de travail au projet |
| `ask_human` | poser une question dans la boîte de réception |
| `take_screenshot` | capture de l'écran entier ou de la fenêtre d'une app (`app` = « Orchestra », « Safari »…), pour vérifier une interface. Sur macOS, l'app qui lance l'IA (Terminal, Orchestra…) doit avoir la permission « Enregistrement de l'écran », et System Events la permission « Accessibilité » pour ne capturer qu'une fenêtre |

Habitude conseillée à une IA : `get_project_state` (avec `folder` = son dossier) au début,
`update_step` pendant le travail, `update_project` et `link_session` à la fin.

## Ce que les IA ont le droit de faire

- Les appels lancés par Orchestra pour **lire** un projet (« Remplir avec l'IA »,
  « Suggérer des étapes », regard extérieur, `src/sync.rs`) sont **en lecture seule** :
  Claude Code n'a que `Read`, `Glob`, `Grep`, `git log`, `git status` et `ls` ; Codex tourne
  avec `--sandbox read-only`.
- Dans Réglages → IA (MCP), « Remplir avec l'IA » et « Suggérer des étapes » peuvent être
  confiés à un modèle (Ollama, Mistral ou un autre serveur compatible OpenAI). Il n'a aucun
  outil : Orchestra met dans la demande un résumé du projet (`src/sync/brief.rs`) — titre et
  étapes, noms des fichiers, état de git, début du README, des manifestes, des documents
  texte et des points d'entrée habituels, jamais les fichiers cachés, ceux dont le nom
  évoque un secret ni ceux qu'un lien sort du dossier. Le résumé est court pour un modèle
  sur cet ordinateur. « Depuis un texte » n'envoie que le texte et les images. Si le modèle
  échoue, aucune autre IA ne répond à sa place. Les autres demandes (question rapide,
  regard extérieur…) restent à Claude Code ou Codex, sauf la conversation ci-dessous.
- Un **projet sans code** a sa conversation avec une IA dans le volet de droite
  (`src/app/side_chat.rs`, `src/sync/talk.rs`), sans terminal. L'IA est celle du réglage
  ci-dessus. Chaque message part avec la fiche du projet, le texte de sa page et les
  derniers échanges, dans une taille limitée. Claude Code et Codex peuvent aussi lire le
  dossier du projet (sans dossier, ils tournent dans un dossier vide) ; Claude Code n'est
  autorisé qu'à chercher sur le web, et l'écriture, les commandes et les outils d'Orchestra
  lui sont refusés. L'IA ne modifie rien : les étapes qu'elle propose attendent dans
  l'encadré Étapes. L'historique (200 messages au plus par projet) est rangé dans le dossier
  `conversations` à côté de `workspace.json`, jamais dedans : il n'est pas partagé avec
  l'équipe.
- L'état des lieux peut être écrit par une IA ; les **étapes** sont à vous : une IA les
  propose, vous choisissez.
- En mode autonome, l'IA travaille dans une copie du dépôt et ne pousse jamais (voir
  [mode-autonome.md](mode-autonome.md)).
