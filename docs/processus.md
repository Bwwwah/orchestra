# Les processus qu'Orchestra lance

Revue du 20/11/2026 (étape « Windows : vérifier qu'aucun processus ne se relance en boucle »),
faite en lisant le code ; à confirmer sur une machine Windows.

Tous les programmes en ligne de commande passent par `proc::command` : sous Windows, ils tournent
sans fenêtre de console (CREATE_NO_WINDOW). Seul « ouvrir dans un terminal »
(`sessions.rs`, `cmd /C start powershell`) ouvre une fenêtre, exprès.

| Quoi | Quand | Rythme | Fin |
|---|---|---|---|
| `git` (état du dépôt) | page d'un projet ouverte | toutes les 15 s | à chaque fois |
| `gh run list` (GitHub Actions) | page d'un projet ouverte | 20 s si une action tourne, sinon 3 min | à chaque fois |
| Vérification des IA (`where`/`which`, `claude mcp …`) | au lancement, à la demande, pendant une installation | 5 s pendant une installation (20 min au plus) | à chaque fois |
| Vérification des mises à jour (`gh`/`curl`) | 5 s après le lancement | toutes les 6 h | à chaque fois |
| Ligne d'état de Claude Code (`orchestra statusline`) | lancée par Claude Code, pas par Orchestra | à chaque rafraîchissement de Claude Code | à chaque fois ; l'ancienne ligne d'état de la personne passe par `cmd /C` sans fenêtre |
| Serveur MCP (`orchestra mcp`) | lancé par chaque IA branchée | une fois par session d'IA | quand l'IA ferme son entrée (fin de `stdin`) |
| Sessions autonomes (Claude Code dans un pseudo-terminal) | au lancement d'une session, ou reprise automatique après la remise à zéro du quota | une seule reprise par arrêt (`resume_at` remis à 0 avant de relancer, même en cas d'échec) | fin de la session |
| Anti-veille (`caffeinate` / `systemd-inhibit` ; rien sous Windows) | pendant les sessions autonomes | une fois ; **s'il échoue ou s'arrête seul, une nouvelle tentative par minute au plus** (avant : à chaque image, soit 60 par seconde) | suit le processus d'Orchestra |
| Redémarrage après mise à jour | sur « Redémarrer » | une fois | l'ancienne app quitte |

Rien ne relance un processus à chaque image ni ne relance l'app elle-même en boucle. Ce qui
ressemblait à une boucle sous Windows venait des consoles visibles de ces commandes périodiques
(surtout la ligne d'état et `git`), corrigé par `proc::command`.

## Arrêt : à la fermeture, à la mise à jour, à la (dés)installation

- **Un terminal** (`Term::stop`, appelé aussi quand il est détruit) arrête le shell **et tout ce
  qu'il a lancé** : `taskkill /F /T /PID` sous Windows, `kill -TERM -- -<pid>` (le groupe du shell,
  qui mène sa propre session dans le pseudo-terminal) ailleurs. Avant, seul le shell s'arrêtait et
  claude, node… continuaient.
- **En quittant l'app** (`on_exit`) : tous les terminaux et toutes les sessions autonomes sont
  arrêtés (`stop_children`).
- **Au redémarrage après une mise à jour** : pareil, avant `process::exit`, qui ne détruit rien de
  lui-même (avant, terminaux et sessions restaient en vie, accrochés à l'ancienne version).
- **Installateur Windows** (`packaging/orchestra.iss`) : avant d'installer et avant de désinstaller,
  `taskkill /F /T /IM orchestra.exe` arrête toutes les copies d'Orchestra (fenêtre, `orchestra mcp`
  lancés par les IA, `orchestra statusline`) et leurs descendants, puis laisse 0,8 s à Windows pour
  libérer les fichiers ; `CloseApplications=force` en plus.
