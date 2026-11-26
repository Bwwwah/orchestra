//! Project templates: what a new project starts with (its modules and its first pages).
//! A template is only applied at creation: nothing remembers it afterwards, and everything
//! it sets can be changed like on any project.

use crate::model::{Column, ColumnKind, Database, DbView, Page, Workspace, new_id};
use crate::project::Module;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Template {
    Code,
    Student,
    Team,
    Event,
}

/// A page a template puts in the new project.
pub struct StarterPage {
    pub title: &'static str,
    pub icon: &'static str,
    pub content: Content,
}

pub enum Content {
    /// Markdown, without a level 1 title (the page has its own).
    Text(&'static str),
    /// An empty database: its view, and its columns after the first one, « Nom ».
    Database(DbView, Vec<Column>),
}

fn column(name: &str, kind: ColumnKind) -> Column {
    Column {
        id: new_id(),
        name: name.to_string(),
        kind,
        options: vec![],
    }
}

fn select(name: &str, options: [&str; 3]) -> Column {
    Column {
        options: options.map(String::from).to_vec(),
        ..column(name, ColumnKind::Select)
    }
}

/// The « Statut » column of a list of things to do.
fn progress() -> Column {
    select(
        crate::tr!("Statut", "Status"),
        [
            crate::tr!("À faire", "To do"),
            crate::tr!("En cours", "In progress"),
            crate::tr!("Terminé", "Done"),
        ],
    )
}

fn text(title: &'static str, icon: &'static str, md: &'static str) -> StarterPage {
    StarterPage {
        title,
        icon,
        content: Content::Text(md),
    }
}

fn database(
    title: &'static str,
    icon: &'static str,
    view: DbView,
    columns: Vec<Column>,
) -> StarterPage {
    StarterPage {
        title,
        icon,
        content: Content::Database(view, columns),
    }
}

impl Template {
    pub const ALL: [Template; 4] = [
        Template::Code,
        Template::Student,
        Template::Team,
        Template::Event,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Template::Code => "Code",
            Template::Student => crate::tr!("Étudiant", "Student"),
            Template::Team => crate::tr!("Équipe", "Team"),
            Template::Event => crate::tr!("Événement", "Event"),
        }
    }

    /// One sentence on what the template is for.
    pub fn description(self) -> &'static str {
        match self {
            Template::Code => crate::tr!(
                "Un logiciel : versions, veille, terminal et sessions autonomes.",
                "A piece of software: versions, security watch, terminal and autonomous sessions."
            ),
            Template::Student => crate::tr!(
                "Un cours, un mémoire ou des révisions : notes, échéances et sources.",
                "A course, a thesis or revision: notes, deadlines and sources."
            ),
            Template::Team => crate::tr!(
                "Un travail à plusieurs : tâches, comptes rendus et règles communes.",
                "Work done together: tasks, meeting notes and shared rules."
            ),
            Template::Event => crate::tr!(
                "Un événement à organiser : programme, tâches, invités et dépenses.",
                "An event to organise: schedule, tasks, guests and expenses."
            ),
        }
    }

    /// The project's icon.
    pub fn icon(self) -> &'static str {
        match self {
            Template::Code => "💻",
            Template::Student => "📚",
            Template::Team => "👥",
            Template::Event => "🎉",
        }
    }

    /// Whether the template makes a code project (`Project::code`). Always said, as a
    /// project without a folder would otherwise count as code.
    fn is_code(self) -> bool {
        self == Template::Code
    }

    /// The modules turned off whatever the kind of project. The code modules are not
    /// listed: they follow `is_code` (see `Module::for_code`).
    fn modules_off(self) -> &'static [Module] {
        match self {
            Template::Code => &[],
            // The autonomous sessions work on a copy of a git repository, and the budget
            // measures what they use.
            Template::Student | Template::Team | Template::Event => {
                &[Module::Autonomous, Module::Budget]
            }
        }
    }

    /// The pages the project starts with, in order, in the language of the window.
    pub fn pages(self) -> Vec<StarterPage> {
        match self {
            Template::Code => vec![
                text(
                    "Architecture",
                    "📐",
                    crate::tr!(
                        "## Vue d'ensemble\nÀ quoi sert le logiciel, et pour qui.\n\n## Composants\n- \n\n## Lancer et tester\n- ",
                        "## Overview\nWhat the software is for, and for whom.\n\n## Components\n- \n\n## Run and test\n- "
                    ),
                ),
                text(
                    crate::tr!("Décisions", "Decisions"),
                    "🧭",
                    crate::tr!(
                        "Une section par choix technique : ce qui est décidé, et pourquoi.\n\n## Première décision\n- Contexte : \n- Choix : \n- Raison : ",
                        "One section per technical choice: what is decided, and why.\n\n## First decision\n- Context: \n- Choice: \n- Reason: "
                    ),
                ),
            ],
            Template::Student => vec![
                text(
                    crate::tr!("Notes de cours", "Course notes"),
                    "📝",
                    crate::tr!(
                        "## Séance du \n### À retenir\n- \n\n### Questions à poser\n- ",
                        "## Class of \n### To remember\n- \n\n### Questions to ask\n- "
                    ),
                ),
                database(
                    crate::tr!("Échéances", "Deadlines"),
                    "📅",
                    DbView::Table,
                    vec![progress(), column("Date", ColumnKind::Date)],
                ),
                text(
                    crate::tr!("Sources", "Sources"),
                    "🔖",
                    crate::tr!(
                        "Les livres, articles et sites utilisés, avec ce qu'on en tire.\n\n- ",
                        "The books, articles and websites used, with what each one brings.\n\n- "
                    ),
                ),
            ],
            Template::Team => vec![
                database(
                    crate::tr!("Tâches", "Tasks"),
                    "📋",
                    DbView::Board,
                    vec![
                        progress(),
                        column(crate::tr!("Responsable", "Owner"), ColumnKind::Text),
                        column("Date", ColumnKind::Date),
                    ],
                ),
                text(
                    crate::tr!("Comptes rendus", "Meeting notes"),
                    "📓",
                    crate::tr!(
                        "## Réunion du \n### Présents\n- \n\n### Décisions\n- \n\n### À faire\n- [ ] ",
                        "## Meeting of \n### Attendees\n- \n\n### Decisions\n- \n\n### To do\n- [ ] "
                    ),
                ),
                text(
                    crate::tr!("Fonctionnement", "How we work"),
                    "📖",
                    crate::tr!(
                        "## Objectif\nCe que l'équipe veut obtenir.\n\n## Rôles\n- \n\n## Règles communes\n- ",
                        "## Goal\nWhat the team wants to achieve.\n\n## Roles\n- \n\n## Shared rules\n- "
                    ),
                ),
            ],
            Template::Event => vec![
                text(
                    crate::tr!("Programme", "Schedule"),
                    "🗓️",
                    crate::tr!(
                        "## L'essentiel\n- Date : \n- Lieu : \n- Nombre de personnes attendues : \n\n## Déroulé\n- ",
                        "## Key facts\n- Date: \n- Place: \n- Expected number of people: \n\n## Running order\n- "
                    ),
                ),
                database(
                    crate::tr!("Tâches", "Tasks"),
                    "📋",
                    DbView::Board,
                    vec![
                        progress(),
                        column(crate::tr!("Responsable", "Owner"), ColumnKind::Text),
                        column("Date", ColumnKind::Date),
                    ],
                ),
                database(
                    crate::tr!("Invités", "Guests"),
                    "✉️",
                    DbView::Table,
                    vec![
                        select(
                            crate::tr!("Réponse", "Answer"),
                            [
                                crate::tr!("En attente", "Waiting"),
                                crate::tr!("Vient", "Coming"),
                                crate::tr!("Ne vient pas", "Not coming"),
                            ],
                        ),
                        column("Contact", ColumnKind::Text),
                    ],
                ),
                database(
                    crate::tr!("Dépenses", "Expenses"),
                    "💶",
                    DbView::Table,
                    vec![
                        column(crate::tr!("Prévu", "Planned"), ColumnKind::Number),
                        column(crate::tr!("Dépensé", "Spent"), ColumnKind::Number),
                    ],
                ),
            ],
        }
    }
}

impl StarterPage {
    fn into_page(self, parent: &str) -> Page {
        let mut page = match self.content {
            Content::Text(md) => crate::store::page_from_markdown(md, self.title),
            Content::Database(view, columns) => {
                let mut page = Page::new(self.title, None);
                page.blocks.clear();
                let mut all = vec![column(crate::tr!("Nom", "Name"), ColumnKind::Text)];
                all.extend(columns);
                page.database = Some(Database {
                    columns: all,
                    rows: vec![],
                    view,
                    sort: None,
                    filter: String::new(),
                });
                page
            }
        };
        page.icon = self.icon.into();
        page.parent = Some(parent.to_string());
        page
    }
}

impl Workspace {
    /// `create_project`, then what `template` sets: the icon, the kind of project, the
    /// modules turned off and the starter pages.
    pub fn create_project_from(&mut self, template: Template, title: &str, folder: &str) -> String {
        let id = self.create_project(title, folder);
        if let Some(p) = self.page_mut(&id) {
            p.icon = template.icon().into();
            if let Some(x) = p.project.as_mut() {
                x.code = Some(template.is_code());
                for module in template.modules_off() {
                    x.set_module(*module, Some(false));
                }
            }
        }
        for starter in template.pages() {
            // Pushed directly: a project lists its pages itself, without a link block.
            self.pages.push(starter.into_page(&id));
        }
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages_of<'a>(ws: &'a Workspace, id: &str) -> Vec<&'a Page> {
        ws.pages
            .iter()
            .filter(|p| p.parent.as_deref() == Some(id))
            .collect()
    }

    #[test]
    fn each_template_sets_its_modules() {
        let mut ws = Workspace::default();
        for template in Template::ALL {
            // Without a folder, where the automatic choice would call every project code.
            let id = ws.create_project_from(template, "P", "");
            let page = ws.page(&id).unwrap();
            assert_eq!(page.icon, template.icon());
            let modules = page.project.as_ref().unwrap().modules();
            if template == Template::Code {
                assert_eq!(modules, Module::ALL);
            } else {
                assert_eq!(modules, [], "{template:?}");
            }
        }
    }

    #[test]
    fn each_template_puts_its_pages_in_the_project() {
        let mut ws = Workspace::default();
        for template in Template::ALL {
            let id = ws.create_project_from(template, "P", "");
            let pages = pages_of(&ws, &id);
            let titles: Vec<&str> = pages.iter().map(|p| p.title.as_str()).collect();
            let expected: Vec<&str> = template.pages().iter().map(|s| s.title).collect();
            assert_eq!(titles, expected);
            assert!(!pages.is_empty());
            for p in &pages {
                assert!(!p.icon.is_empty() && p.project.is_none());
                match &p.database {
                    Some(db) => {
                        assert!(db.rows.is_empty() && db.columns.len() > 1);
                        for c in &db.columns {
                            assert_eq!(c.kind == ColumnKind::Select, !c.options.is_empty());
                        }
                    }
                    // The Markdown became real blocks, not text left as it was written.
                    None => assert!(
                        p.blocks.len() > 1 && p.blocks.iter().all(|b| !b.text.starts_with('#'))
                    ),
                }
            }
            // The project's own text gets no link to them.
            let home = ws.page(&id).unwrap();
            assert!(home.blocks.iter().all(|b| b.extra.is_empty()));
        }
    }

    /// `fix_icons` replaces some icons each time the workspace is read: a template must not
    /// use them.
    #[test]
    fn the_icons_of_a_template_are_kept_when_the_workspace_is_read() {
        let mut ws = Workspace::default();
        for template in Template::ALL {
            ws.create_project_from(template, "P", "");
        }
        let icons = |ws: &Workspace| ws.pages.iter().map(|p| p.icon.clone()).collect::<Vec<_>>();
        let before = icons(&ws);
        crate::store::fix_icons(&mut ws);
        assert_eq!(icons(&ws), before);
    }

    #[test]
    fn two_projects_from_a_template_share_no_id() {
        let mut ws = Workspace::default();
        ws.create_project_from(Template::Event, "A", "");
        ws.create_project_from(Template::Event, "B", "");
        let mut ids = std::collections::HashSet::new();
        for p in &ws.pages {
            assert!(ids.insert(p.id.clone()));
            for b in &p.blocks {
                assert!(ids.insert(b.id.clone()));
            }
            for c in p.database.iter().flat_map(|d| &d.columns) {
                assert!(ids.insert(c.id.clone()));
            }
        }
    }
}
