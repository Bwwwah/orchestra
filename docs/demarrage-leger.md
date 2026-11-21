# Démarrage d'une session autonome : ce qui est chargé

Relevé du 21/11/2026, pour l'étape « Démarrage léger ». Il sert de base pour retirer ce
qui ne sert à rien en mode autonome (étape suivante).

## Mesure

Le premier message de chaque session autonome Claude Code (fichier `.jsonl` de la session,
champ `usage` du premier `assistant`) donne la taille du contexte avant tout travail :

| Session (copie)          | Total   | Lu en cache | Écrit en cache |
|--------------------------|---------|-------------|----------------|
| orchestra-20261121-233056 | 40 576 | 29 358      | 11 216         |
| orchestra-20261121-220655 | 40 573 | 27 364      | 13 207         |
| orchestra-20261121-193759 | 41 813 | 29 358      | 12 453         |
| orchestra-20261121-192333 | 41 752 | 29 358      | 12 392         |
| orchestra-20261121-183911 | 41 359 | 27 364      | 13 993         |
| orchestra-20261121-171703 | 41 549 | 27 364      | 14 183         |
| orchestra-20261121-154139 | 41 046 | 29 358      | 11 686         |
| orchestra-20261121-152534 | 40 924 | 29 358      | 11 564         |

- Le démarrage pèse **40,5 à 41,8 k tokens**, à chaque session.
- **27 à 29 k** sont identiques d'une session à l'autre (lus en cache) : les consignes de
  Claude Code et les définitions de ses outils. Elles ne coûtent qu'un dixième au tarif du
  cache, mais restent dans le contexte de chaque message.
- **11 à 14 k** sont propres à la session : écrits en cache au premier message (plus cher
  qu'une lecture normale).

`/context` donnerait le détail exact, mais c'est une commande interactive : elle ne peut
pas être tapée depuis une session autonome. On peut la lancer à la main dans le terminal
d'une session (« Regarder »). Le détail ci-dessous est donc une **estimation**, faite en lisant
le contexte de démarrage d'une session autonome.

## Ce qui compose ces 40 k (estimation)

Partie commune, en cache :

- **Consignes de Claude Code** : environ 3 à 4 k.
- **Définitions des outils intégrés** : environ 15 à 18 k, dont :
  - **Artifact** (publier des pages sur claude.ai) : sa description fait à elle seule plusieurs
    milliers de tokens (environ 6 à 7 k). Ne sert jamais en mode autonome.
  - **Agent** (sous-agents) et la liste des types d'agents : environ 2 k. Inutile ici.
  - **AskUserQuestion**, **ScheduleWakeup**, **SendFeedback**, **ReportFindings**,
    **ListAgents** : environ 2 à 3 k. Personne ne répond dans le terminal caché ;
    ask_human les remplace.
  - **Bash, Read, Edit, Write, Grep, Glob, Skill, ToolSearch** : environ 5 à 6 k. Utiles.
- **Liste des outils différés** (noms seulement, dont les ~25 outils MCP d'Orchestra) :
  environ 1 k. Les outils MCP sont différés : seuls leurs noms sont chargés.

Partie propre à la session, au premier message :

- **Liste des skills** : environ 35 skills avec leur description (documents, tableurs,
  navigateur, design d'artifacts, plugins…), environ 5 à 7 k. Presque aucune ne sert à une
  étape de code.
- **Consigne de mission d'Orchestra** (`auto::mission_prompt`) : environ 1 k.
- **Mémoire automatique** (MEMORY.md du projet), **git status**, environnement : environ
  1 k.
- **Instructions des serveurs MCP** : Orchestra, et aussi **easel** (un plugin de graphiques)
  malgré `--strict-mcp-config` : il arrive par les plugins des réglages personnels, chargés
  par `--setting-sources user`.

## Pistes pour l'étape suivante

1. Ajouter à `--disallowedTools` (`auto::claude_command`) les outils inutiles en autonome :
   Artifact, Agent, AskUserQuestion, ScheduleWakeup, SendFeedback, ReportFindings,
   ListAgents. Gain estimé : **10 à 12 k** par session.
2. Couper les skills en autonome (`--disable-slash-commands`, à vérifier sur la version de
   Claude Code installée). Gain estimé : **5 à 7 k**.
3. Vérifier s'il faut garder les plugins personnels (`--setting-sources user` les
   charge, avec leurs serveurs MCP et leurs skills).
4. Une fois fait, remesurer avec le tableau ci-dessus (premier `usage` du `.jsonl`) et avec
   `/context` dans le terminal d'une session.
