# Audit des pages d'Orchestra (20/11/2026)

Tour de chaque écran de l'app, pour préparer les étapes « Design » et « Fonctionnel ».
Sources : une capture de l'app installée (page Orchestration, barre latérale) et la lecture
du code de chaque vue. Les autres écrans n'ont pas pu être ouverts à l'écran (l'IA ne peut
pas cliquer dans l'app) : leurs remarques viennent du code.

Légende : **[D]** design, **[F]** fonctionnel, **[T]** texte. Priorité : ★★★ à faire d'abord.

## Écrans recensés

| Écran | Code | Rôle |
|---|---|---|
| Barre latérale | `app/chrome.rs`, `app/side.rs` | Recherche, Accueil / IA / Personnes, Projets, Orchestration, Boîte de réception, pages, abonnement, Corbeille |
| Accueil | `chrome.rs::home_view` | Présentation, puces d'état, « Pour commencer », pages récentes |
| Projets | `projects.rs::projects_view` | Cartes des projets, puis page d'un projet (état des lieux, étapes, versions, sessions) |
| Orchestration | `orch_ui.rs::orchestra_view` | Usage de l'abonnement, IA en cours, lancement des sessions |
| Boîte de réception | `orch_ui.rs::inbox_view` | Questions et messages des IA |
| Page / éditeur | `app.rs::page_view`, `slash.rs`, `rich.rs` | Blocs, menu « / », sous-pages |
| Base de données | `database.rs` | Vues Tableau, Kanban, Liste |
| Corbeille | `app.rs::trash_view` | Pages supprimées |
| Paramètres | `settings.rs` | Préférences, Raccourcis, Général, Barre latérale, Import/Export, IA, Collaboration |
| Recherche (⌘K) | `search.rs` | Recherche et questions |
| Volet latéral | `side.rs`, `term_ui.rs`, `ai_ui.rs`, `live_ui.rs` | Terminal, IA, sessions en direct |

## Constats

### Toutes les pages
- ★★★ **[T]** Pluriels en « (s) » : 77 occurrences (« 1 sur 3 projet(s) prêt(s) », « 11 session(s) · 77 étape(s) faite(s) »). Un petit helper de pluriel rendrait les textes naturels.
- ★★ **[T]** Nombres décimaux au format anglais en français : « 25.0 % », « 14.3 % » (`usage.rs:385`, `orch_ui.rs:2086`). En français : « 25,0 % ». Dans la même phrase, « 30 % » est sans décimale : harmoniser (pas de décimale au-dessus de 10 %).
- ★ **[D]** 90 couleurs `Color32::from_rgb` codées en dur dans `src/app/` en plus des teintes de `Tone` : risque d'écarts entre thème clair et sombre (c'est ce qui donnait la croix noire sur noir). À ramener dans `Tone` au fil des pages.

### Barre latérale (vue à l'écran)
- ★★ **[F]** « Rapports des IA » déroule un rapport par session (10 pages pour le 20/11). La liste pousse les pages de la personne hors de l'écran. Idée : replier les jours par défaut, ou garder seulement le dernier jour ouvert.
- ★ **[D]** Les icônes IA (✦) et Personnes à côté d'« Accueil » n'ont pas de libellé : une infobulle au minimum.

### Accueil
- ★ **[T]** Le paragraphe de présentation est long (5 lignes) et reste là même une fois tout en place. Il pourrait se raccourcir quand les 4 étapes « Pour commencer » sont faites.
- ★ **[D]** Les cartes « Récemment modifiées » (144 × 144) ne montrent que le titre et la date. Un aperçu de la première ligne aiderait à les reconnaître.

### Orchestration (vue à l'écran)
- ★★ **[D]** Les chemins des projets sont mélangés : `~/Code/Orchestra` mais `/Users/…/Code/bind-react-native`. Abréger partout le dossier personnel en `~`.
- ★★ **[T]** La ligne orange « Depuis 22 h 19 : 25.0 % de la semaine utilisés (part d'un jour : 14.3 %) · plus vite que la part d'un jour » est dense et répète « part d'un jour ». Plus court : « Dernières 24 h : 25 % de la semaine, soit 1,7 jour de rythme ».
- ★ **[D]** Dans « Lancer une session », la ligne du projet « au travail » n'a pas de bouton, alors que les autres ont « Lancer » à la même place : la colonne décale. Une étiquette « En cours » à la place du bouton garderait l'alignement.
- ★ **[T]** « sur main » et « commits » : libellés en minuscules, peu parlants pour un débutant (« Travailler sur main », « Faire des commits »).
- ★ **[F]** Le message de chaque projet (« Avance sur les étapes ouvertes… ») est répété à l'identique sur chaque ligne : un message par défaut commun, modifiable par projet seulement si besoin, allégerait la page.

### Projets / page d'un projet
- ★★★ **[F]** Coller une image dans une étape semble donner des PNG vides (étape déjà ouverte, à reproduire en priorité).
- ★ **[D]** La croix pour supprimer une étape était invisible au survol : corrigé (commit fb07c5d), à voir en vrai.

### Page / éditeur
- ★★ **[F]** Le menu « / » ne propose pas encore : table des matières, lien vers une page existante, image, colonnes (chacun demande un nouveau type de bloc).
- ★ **[D]** Nouvelles entrées du menu « / » (bases Tableau / Liste, Date du jour) et Kanban pas encore vues à l'écran.

### Corbeille
- ★★★ **[F]** « Supprimer définitivement » et « Vider la corbeille » agissent dès le premier clic, sans confirmation, alors que c'est irréversible. Ailleurs, l'app demande un second clic (« Supprimer les terminées ») : appliquer le même principe.
- ★★ **[D]** La page garde le look egui par défaut (titre avec emoji 🗑, boutons encadrés, pas de colonne centrée). Elle détonne à côté de l'Accueil et des Projets : l'aligner sur leur mise en page (colonne centrée, lignes avec survol, icône de page).
- ★ **[F]** Pas de date de suppression ni de page parente affichées : difficile de retrouver une page quand la liste grandit. Une recherche ou au moins un tri par date aiderait.

### Boîte de réception
- ★ **[D]** La croix de suppression d'un message est maintenant toujours visible (commit fb07c5d).

### Paramètres
- Pas de remarque particulière dans le code ; le bouton des notes de version dans les paramètres est une étape déjà ouverte.

## Pistes par étape

- **Design** : Corbeille dans le style de l'app, chemins abrégés en `~`, alignement des lignes « Lancer une session », infobulles des icônes de la barre latérale.
- **Fonctionnel** : confirmation dans la Corbeille, rapports des IA repliés, pluriels et nombres en français, message par défaut commun dans Orchestration.
