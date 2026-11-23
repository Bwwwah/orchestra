# Collaboration : espaces et serveur auto-hébergeable

Plan de la version collaborative (branche `collaboratif`, dépôt privé). Rien de tout cela n'est
dans `main` tant que ce n'est pas prêt.

## Ce qu'il y avait avant

Trois modes, tous sans compte :

* **Dossier partagé** et **commande maison** : chacun publie une copie entière de son espace,
  tout le monde fusionne les copies des autres.
* **En direct** : un poste (ou `orchestra serveur`) relaie les messages, chiffrés avec un code
  commun (SPAKE2 + ChaCha20-Poly1305, `secure.rs`).

Fusion bloc par bloc, la modification la plus récente gagne (`collab::merge_page`, horloge
hybride). Seuls les projets cochés « partagés » sortent.

Limites pour une équipe : un code commun (pas d'identité, pas de révocation, pas de droits),
l'espace relu et réécrit en entier à chaque message, une reconnexion qui renvoie tout, un
protocole TCP maison qui ne passe pas derrière un proxy HTTPS, deux personnes dans le même
paragraphe : la dernière écrase l'autre.

**Décision :** le dossier partagé et la commande maison sont retirés (trop à maintenir, remplacés
par les espaces). Le mode en direct n'est pas retiré : il devient l'hébergement d'un espace
depuis l'appli (principe 8) ; son code (relais, garde contre les essais de code, présence,
discussion) sert de base au serveur.

## Les principes

1. **Des espaces.** `Mon espace` est local et privé. Chaque serveur rejoint est un autre espace,
   qu'on choisit dans le sélecteur en haut de la barre latérale. Chaque espace a son propre
   dossier : rien d'un espace ne peut partir vers un autre. Dans un espace partagé, tout est
   partagé avec ses membres (des droits par projet viendront plus tard).
2. **Une identité par clé.** Orchestra crée une clé sur la machine : c'est le compte, le même
   pour tous les serveurs. On rejoint un serveur par un lien d'invitation (adresse + jeton à
   usage unique) ; le serveur garde la clé publique et le nom. Sauvegarde de la clé et second
   appareil : à prévoir.
3. **Des réglages personnels communs à tous les espaces** : abonnements IA, limites par IA,
   langue, thème… Ils sortent du fichier de l'espace.
4. **Une règle d'IA par espace**, fixée par l'administrateur :
   * *IA libres* : ses propres IA, et celles de l'espace s'il y en a ;
   * *IA de l'espace seulement* ;
   * *Sans IA*.
   Plus une liste d'IA permises. Tous les lancements d'IA (Automatique, orchestration, état des
   lieux, questions) ne proposent que les IA permises ; le serveur MCP refuse d'écrire dans un
   espace qui n'autorise pas l'IA qui l'appelle.
5. **Des IA fournies par l'espace** : un profil donné par le serveur (outil — Claude Code, Codex,
   Vibe… —, adresse du modèle de l'entreprise — vLLM, Ollama, LiteLLM, Bedrock… —, modèle, clé
   personnelle par membre, révocable). L'agent tourne sur le poste (il lit les fichiers), ses
   appels au modèle partent chez l'entreprise. Branché sur le mode API existant (`sync::api_env`).
   Orchestra ne fait pas de passerelle d'IA lui-même : il distribue la configuration et les clés.
6. **Écriture à plusieurs dans le même paragraphe** : un CRDT (Loro de préférence — texte riche,
   liste déplaçable, arbre ; yrs en alternative) pour le texte et l'ordre des blocs des pages
   des espaces partagés. Le modèle `Page`/`Block` reste la vue sur laquelle s'appuient MCP,
   export et IA ; chaque frappe est traduite en insertions / suppressions.
7. **Un seul programme** : `orchestra serveur` devient le serveur d'équipe. SQLite (comptes,
   invitations, projets, membres, rôles, journal des modifications), WebSocket derrière le HTTPS
   de l'hébergeur (Caddy, Traefik…), un salon par projet, asynchrone (tokio) pour tenir des
   centaines de connexions. La reprise après coupure vient du CRDT (« ce qui t'a manqué depuis
   la version X »).

8. **Les petites équipes sans serveur : n'importe quel Orchestra peut héberger un espace.**
   « Partager cet espace depuis mon ordinateur » fait tourner le même serveur dans l'appli. Un
   seul protocole, deux façons d'héberger (l'appli, ou `orchestra serveur` / Docker), mêmes
   identités, invitations, règle d'IA et écriture simultanée : pas de double maintenance. Un
   espace hébergé sur un poste peut être déplacé sur un serveur quand l'équipe grandit.
   Contrepartie : le poste qui héberge doit être allumé pour que les autres se synchronisent
   (chacun continue sur sa copie entre-temps) et joignable (même réseau, Tailscale,
   WireGuard…). À réexaminer après le CRDT : un mode « dossier synchronisé » (Syncthing,
   Nextcloud) où personne n'a besoin d'être en ligne, devenu simple et sûr avec les
   modifications CRDT.

## Ce qu'Orchestra ne peut pas garantir

Orchestra *respecte* la règle d'IA d'un espace, il ne peut pas l'*imposer* : la personne garde
la main sur sa machine et peut lancer une IA sur le même dossier en dehors d'Orchestra. Le
serveur garantit seulement ce qui passe par lui : les IA de l'espace ne marchent qu'avec une clé
qu'il délivre et révoque, leurs appels peuvent être tracés, un membre retiré perd l'accès aux
pages et aux IA. Une interdiction stricte relève des outils de l'entreprise (gestion des postes,
accès au code). La documentation d'hébergement doit le dire.

## Étapes

1. **Espaces en local, sans réseau** : registre des espaces, un dossier par espace, sélecteur,
   réglages personnels sortis de l'espace, MCP qui écrit dans l'espace du projet de son dossier
   (sinon `Mon espace`, jamais un espace partagé par défaut), règle d'IA par espace réglable à la
   main pour la tester.
2. **Serveur** : identité par clé, invitations, WebSocket, SQLite. Un espace partagé est la copie
   locale d'un serveur ; sa règle d'IA vient du serveur. D'abord avec la fusion bloc par bloc
   actuelle, pour un circuit complet au plus vite.
3. **IA d'espace** : profils et clés par membre distribués par le serveur.
4. **Écriture simultanée** (CRDT), puis les curseurs des autres.
5. **Rôles, révocation, quotas**, image Docker, guide d'hébergement, sauvegardes.
   **Hébergement depuis l'appli** (le même serveur, dans la fenêtre) et déplacement d'un espace
   vers un serveur ; le mode en direct à code commun disparaît alors, remplacé par lui.

## Avancement

* [x] Plan consigné.
* [x] Dossier partagé et commande maison retirés.
* [ ] Étape 1 — espaces en local.
  * [x] Registre des espaces (`espaces.json`), un dossier par espace (`src/space.rs`) ; `Mon
    espace` garde le dossier de données actuel, donc rien à migrer.
  * [x] Chaque programme choisit son espace (`store::use_space`) ; sans choix, `Mon espace`.
  * [x] Règle d'IA par espace (modèle et tests ; pas encore appliquée aux lancements).
  * [ ] Sélecteur d'espace dans la fenêtre (le bouton « Mon espace ⌄ » en bas de la barre
    latérale). À régler : que faire des IA, terminaux et sessions en cours au changement
    d'espace (proposition : changement refusé tant qu'une IA travaille).
  * [ ] Réglages personnels communs à tous les espaces.
  * [ ] Le serveur MCP écrit dans l'espace qui contient le projet de son dossier.
  * [ ] Règle d'IA appliquée à tous les lancements d'IA et au serveur MCP.
