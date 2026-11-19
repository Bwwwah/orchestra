# Le mode autonome (page Orchestration)

Une IA avance **seule** sur un projet, jusqu'à ce que le travail soit fait, qu'elle soit
bloquée ou qu'une limite soit atteinte. Le code est dans `src/auto.rs` et
`src/app/orch_ui.rs`.

## Ce qu'elle fait

1. Orchestra crée une **copie du dépôt** à part (un `git worktree` sur une branche
   `orchestra/auto-…`) : votre copie de travail n'est jamais touchée, et plusieurs projets
   peuvent avancer en même temps.
2. Elle lance **votre propre Claude Code** dans un terminal caché, avec la mission et les
   règles (`auto::mission_prompt`).
3. L'IA lit l'état du projet (`get_project_state`), puis prend **les étapes ouvertes une par
   une** : elle passe l'étape « doing », fait le travail, le vérifie (compilation, tests), puis
   la passe « done ».
4. Si une décision humaine est nécessaire, elle pose la question dans la boîte de réception
   (`ask_human`) et continue sur autre chose.
5. À la fin, un **rapport** est écrit dans la page « Rapports des IA » et un message arrive
   dans la boîte de réception. Vous **fusionnez** la branche ou l'**abandonnez**. Si la case
   **sur main** du projet est cochée, la fusion se fait toute seule une fois Claude Code fermé,
   et seulement si le dossier du projet est sur `main` ou `master` (`auto::merge_into_main`).

La branche est **envoyée sur GitHub** (`origin`) après chaque tour qui a produit des commits,
et en fin de session : on peut suivre le travail sur GitHub pendant qu'il avance. Fusionner
ou abandonner supprime aussi la branche sur GitHub. Tant que la session n'est ni fusionnée
ni abandonnée, sa branche est ouverte dans la copie : git refuse d'y passer depuis votre
dossier (« already used by worktree »), l'encadré Versions l'explique et indique où se
trouve la copie.

## Les étapes : la matière première

Le mode autonome ne fait que les étapes du projet : leur qualité décide de celle du travail.
« Suggérer des étapes » (carte Étapes d'un projet) demande à une IA de lire **le code** —
pas seulement le README — et de proposer 4 à 8 étapes, chacune :

- dans un domaine : **Fonctionnalité**, **Technique** (bug probable, fragilité, tests
  manquants, lenteur, duplication) ou **Sécurité** (secrets, entrées non validées, commandes
  shell construites à partir d'entrées, permissions, dépendances) ;
- faisable et vérifiable par une IA seule en une session ;
- précise : elle nomme le fichier, le module ou la fonction concernés.

Sont écartées : les étapes vagues, la doc, les tâches de routine (commit, push, publier) et
ce qui demande un accès ou une décision humaine. Vous cochez celles à garder ; rien n'est
ajouté sans votre clic. Le texte de la consigne est dans `sync::ask_tasks`.

## Les limites

| Réglage | Par défaut | Effet |
|---|---|---|
| Limite globale | 60 % | part de la fenêtre de 5 h de l'abonnement, tous projets confondus |
| Limite de la semaine | 95 % | au-delà, aucune session ne démarre ni ne continue (barre en haut de la page) |
| Part par projet | 20 % | réglée sur la ligne de chaque projet au lancement ; les parts ensemble ne dépassent jamais la limite globale |
| Tours au plus | 30 | nombre de tours d'une session |
| Commits | non | l'IA peut faire des commits sur sa branche ; Orchestra envoie la branche sur GitHub après chaque tour et en fin de session |
| Rapport du jour | 19 h | heure du rapport d'ensemble quand plusieurs projets ont tourné |

L'utilisation est relevée par la ligne d'état de Claude Code (`orchestra statusline`, voir
`src/usage.rs`). Si elle ne peut pas être mesurée, la session s'arrête par prudence.
