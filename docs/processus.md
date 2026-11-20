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

Reste à voir : ce qui continue de tourner une fois l'app fermée (terminaux, serveurs MCP, sessions),
traité par les étapes « Désinstallation ».
