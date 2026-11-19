# Les messages de l'application

Il n'y a plus de barre d'état en bas de la fenêtre. Chaque message a un niveau
(`src/app/notices.rs`) :

| Niveau | Appel | Où il apparaît | Exemples |
|---|---|---|---|
| Confirmation | `self.ack(…)` | petite fenêtre en bas à droite, qui s'efface après 2,5 s ; rien n'est gardé | « Lien copié », « Zoom 110 % » |
| Information | `self.info(…)` | boîte de réception | « Exporté : … », « Claude Code est branché » |
| Erreur | `self.fail(…)` | boîte de réception **et** fenêtre rouge (9 s, ou jusqu'à la fermer) | « Enregistrement impossible : … » |

Cliquer sur une fenêtre d'erreur ouvre la boîte de réception. Survolée, une fenêtre reste
affichée.

Un même message répété (une erreur qui revient à chaque tentative) ne s'empile pas : le
message non lu existant remonte en haut, et la fenêtre d'erreur ne réapparaît pas plus d'une
fois par minute.

Quand le message vient d'un fil de travail ou d'une fonction qui ne tient pas `NotesApp`,
renvoyez un `Notice` (`Notice::Ack`, `Notice::Info`, `Notice::Error`, ou
`Notice::from_result(résultat)`) et appelez `self.notice(n)` ensuite.

Les synchronisations automatiques (toutes les 30 s) sont silencieuses sauf en cas d'échec ;
leur journal reste dans Réglages → Collaboration.
