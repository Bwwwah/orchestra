//! Collaboration without a company server: a "connector" you plug in yourself.
//!
//! Two connectors come ready:
//! * Shared folder — any folder that several people sync (Syncthing, Nextcloud,
//!   a network drive, a git checkout...). Each person writes their copy as
//!   `<author>.json`; everyone merges all copies found there.
//! * Command — your own program. It is run as `<command> pull` (print a workspace
//!   JSON on standard output, or nothing) and `<command> push` (receives the
//!   workspace JSON on standard input). Write it in any language to talk to your
//!   own server, a database, a chat bot...
//!
//! To write a connector in Rust instead, implement [`Connector`].
//!
//! Merging is per page: the copy with the latest `updated` time wins, and pages
//! deleted for good stay deleted (their ids are kept in `deleted`).

use crate::model::{Block, ConnectorKind, Page, Workspace};
use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;

pub trait Connector {
    /// Name shown in the window.
    fn name(&self) -> String;
    /// Fetch the other copies of the workspace.
    fn pull(&self) -> Result<Vec<Workspace>, String>;
    /// Publish this copy of the workspace.
    fn push(&self, ws: &Workspace) -> Result<(), String>;
}

pub struct SharedFolder {
    pub folder: PathBuf,
    pub author: String,
}

impl Connector for SharedFolder {
    fn name(&self) -> String {
        crate::trf!(
            "Dossier partagé {}",
            "Shared folder {}",
            self.folder.display()
        )
    }

    fn pull(&self) -> Result<Vec<Workspace>, String> {
        let entries = std::fs::read_dir(&self.folder)
            .map_err(|e| crate::trf!("dossier illisible : {e}", "unreadable folder: {e}"))?;
        let mine = format!("{}.json", author_file(&self.author));
        let mut out = vec![];
        for e in entries.flatten() {
            let p = e.path();
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if !name.ends_with(".json") || name == mine {
                continue;
            }
            if let Ok(s) = std::fs::read_to_string(&p)
                && let Ok(ws) = serde_json::from_str::<Workspace>(&s)
            {
                out.push(ws);
            }
        }
        Ok(out)
    }

    fn push(&self, ws: &Workspace) -> Result<(), String> {
        std::fs::create_dir_all(&self.folder).map_err(|e| e.to_string())?;
        let mut shared = ws.clone();
        shared.settings = Default::default(); // personal settings stay personal
        let path = self
            .folder
            .join(format!("{}.json", author_file(&self.author)));
        let tmp = path.with_extension("json.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_string_pretty(&shared).unwrap_or_default(),
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(tmp, path).map_err(|e| e.to_string())
    }
}

pub struct UserCommand {
    pub command: String,
}

impl UserCommand {
    fn run(&self, arg: &str, input: Option<String>) -> Result<String, String> {
        let shell_line = format!("{} {}", self.command, arg);
        let mut cmd = if cfg!(windows) {
            let mut c = crate::proc::command("cmd");
            c.args(["/C", &shell_line]);
            c
        } else {
            let mut c = crate::proc::command("sh");
            c.args(["-c", &shell_line]);
            c
        };
        cmd.stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| {
            crate::trf!(
                "impossible de lancer la commande : {e}",
                "couldn't run the command: {e}"
            )
        })?;
        if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
            stdin
                .write_all(text.as_bytes())
                .map_err(|e| e.to_string())?;
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(crate::trf!(
                "la commande a échoué ({}) : {}",
                "the command failed ({}): {}",
                out.status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into()),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }
}

impl Connector for UserCommand {
    fn name(&self) -> String {
        crate::trf!("Commande « {} »", "Command « {} »", self.command)
    }

    fn pull(&self) -> Result<Vec<Workspace>, String> {
        let text = self.run("pull", None)?;
        let text = text.trim();
        if text.is_empty() {
            return Ok(vec![]);
        }
        // One workspace, or a JSON array of workspaces.
        if let Ok(list) = serde_json::from_str::<Vec<Workspace>>(text) {
            return Ok(list);
        }
        serde_json::from_str::<Workspace>(text)
            .map(|w| vec![w])
            .map_err(|e| crate::trf!("réponse illisible : {e}", "unreadable answer: {e}"))
    }

    fn push(&self, ws: &Workspace) -> Result<(), String> {
        let mut shared = ws.clone();
        shared.settings = Default::default();
        self.run(
            "push",
            Some(serde_json::to_string(&shared).unwrap_or_default()),
        )
        .map(|_| ())
    }
}

fn author_file(author: &str) -> String {
    let a = crate::store::safe_name(author);
    if author.trim().is_empty() {
        whoami()
    } else {
        a
    }
}

fn whoami() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "moi".into())
}

pub fn connector_for(ws: &Workspace) -> Option<Box<dyn Connector>> {
    let c = &ws.settings.collab;
    match c.connector {
        ConnectorKind::None => None,
        ConnectorKind::SharedFolder if !c.folder.trim().is_empty() => {
            Some(Box::new(SharedFolder {
                folder: PathBuf::from(c.folder.trim()),
                author: c.author.clone(),
            }))
        }
        ConnectorKind::Command if !c.command.trim().is_empty() => Some(Box::new(UserCommand {
            command: c.command.trim().to_string(),
        })),
        ConnectorKind::Live => None,
        _ => None,
    }
}

/// A page new here, received from another person: without what only this computer decides
/// (see `Project::keep_local`).
pub fn received_new(mut p: Page) -> Page {
    if let Some(x) = p.project.as_mut() {
        x.keep_local(None);
    }
    p
}

/// Merge another copy into ours. Returns how many pages changed here.
pub fn merge(ours: &mut Workspace, theirs: &Workspace) -> usize {
    let mut changed = 0;
    // Only deletions of shared pages (or of pages we don't have): never a private page.
    for (id, t) in &ours.accept_remote_deletions(theirs.deleted.clone()) {
        let e = ours.deleted.entry(id.clone()).or_insert(*t);
        if *t > *e {
            *e = *t;
        }
    }
    let deleted = ours.deleted.clone();
    let before = ours.pages.len();
    ours.pages
        .retain(|p| deleted.get(&p.id).map(|t| p.updated > *t).unwrap_or(true));
    changed += before - ours.pages.len();
    // Only pages of shared projects, as in live mode: never one of our private pages, nor a
    // page slipped under one, nor a stray page of theirs.
    for tp in ours.accept_remote(theirs.pages.clone()) {
        if deleted
            .get(&tp.id)
            .map(|t| tp.updated <= *t)
            .unwrap_or(false)
        {
            continue;
        }
        match ours.pages.iter_mut().find(|p| p.id == tp.id) {
            Some(op) => {
                if merge_page(op, &tp) {
                    changed += 1;
                }
            }
            None => {
                ours.pages.push(received_new(tp));
                changed += 1;
            }
        }
    }
    // Discussion messages: everybody's messages, never edited, so a plain union; only those
    // of the projects shared here (now that theirs came in).
    changed += ours.accept_chat(theirs.chat.values().cloned());
    changed
}

/// Merge another copy of a page into ours, block by block. The page's own settings (title, icon,
/// place, database…) come from the copy edited last; each block keeps its most recent edit; blocks
/// deleted on either side stay deleted; blocks added on either side are kept, in the order of the
/// copy edited last, with the other side's new blocks after the block they followed.
/// Returns true when ours changed.
pub fn merge_page(ours: &mut Page, theirs: &Page) -> bool {
    use std::collections::{BTreeMap, HashMap};
    let before = serde_json::to_string(&ours).unwrap_or_default();
    let mut tomb: BTreeMap<String, u64> = ours.deleted_blocks.clone();
    for (id, t) in &theirs.deleted_blocks {
        let e = tomb.entry(id.clone()).or_insert(*t);
        *e = (*e).max(*t);
    }
    let theirs_newer = theirs.updated > ours.updated;
    let (newer, older) = if theirs_newer {
        (theirs.clone(), ours.clone())
    } else {
        (ours.clone(), theirs.clone())
    };
    let older_by_id: HashMap<&str, &Block> =
        older.blocks.iter().map(|b| (b.id.as_str(), b)).collect();
    let alive = |b: &Block| tomb.get(&b.id).map(|t| b.updated > *t).unwrap_or(true);
    // Blocks of the copy edited last, each replaced by the other side's version if more recent.
    let mut blocks: Vec<Block> = newer
        .blocks
        .iter()
        .map(|b| match older_by_id.get(b.id.as_str()) {
            Some(o) if o.updated > b.updated => (*o).clone(),
            _ => b.clone(),
        })
        .collect();
    // Blocks only the other side has: new ones, placed after the block they followed there.
    let present: std::collections::HashSet<String> = blocks.iter().map(|b| b.id.clone()).collect();
    let mut prev: Option<String> = None;
    for b in &older.blocks {
        if !present.contains(&b.id) {
            let at = prev
                .as_ref()
                .and_then(|p| blocks.iter().position(|x| &x.id == p))
                .map(|i| i + 1)
                .unwrap_or(0);
            blocks.insert(at.min(blocks.len()), b.clone());
        }
        prev = Some(b.id.clone());
    }
    blocks.retain(|b| alive(b));
    let mut merged = newer;
    if let Some(p) = merged.project.as_mut() {
        p.keep_local(ours.project.as_ref());
        // The steps are shared: theirs merged into ours, step by step.
        if let Some(tp) = theirs.project.as_ref() {
            p.merge_steps(&tp.steps, &tp.deleted_steps);
        }
    }
    merged.blocks = blocks;
    merged.deleted_blocks = tomb;
    merged.updated = ours.updated.max(theirs.updated);
    *ours = merged;
    serde_json::to_string(&ours).unwrap_or_default() != before
}

/// Pull, merge, then push. Returns a short report for the window.
pub fn sync(ws: &mut Workspace) -> Result<String, String> {
    let conn = connector_for(ws).ok_or_else(|| {
        crate::tr!("aucun branchement configuré", "no connection set up").to_string()
    })?;
    let others = conn.pull()?;
    let mut changed = 0;
    for o in &others {
        changed += merge(ws, o);
    }
    // Only the shared projects (and their discussions) go out.
    conn.push(&ws.shared_copy())?;
    Ok(crate::trf!(
        "{} : {} copie(s) lue(s), {} page(s) mise(s) à jour",
        "{}: {} copy(ies) read, {} page(s) updated",
        conn.name(),
        others.len(),
        changed
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Page;

    fn page_with(texts: &[&str]) -> Page {
        let mut p = Page::new("P", None);
        p.blocks = texts
            .iter()
            .map(|t| Block::new(crate::model::BlockKind::Paragraph, *t))
            .collect();
        p
    }

    #[test]
    fn two_people_editing_different_blocks_keep_both_edits() {
        let base = page_with(&["un", "deux", "trois"]);
        let mut a = base.clone();
        let mut b = base.clone();
        a.blocks[0].text = "un (A)".into();
        a.blocks[0].updated = crate::model::now_ms();
        a.updated = a.blocks[0].updated;
        b.blocks[2].text = "trois (B)".into();
        b.blocks[2].updated = crate::model::now_ms();
        b.updated = b.blocks[2].updated;
        let mut merged = a.clone();
        assert!(merge_page(&mut merged, &b));
        let texts: Vec<&str> = merged.blocks.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(texts, ["un (A)", "deux", "trois (B)"]);
        // And the same the other way round.
        let mut other = b.clone();
        merge_page(&mut other, &a);
        let texts2: Vec<&str> = other.blocks.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(texts, texts2);
    }

    #[test]
    fn received_projects_keep_what_this_computer_decides() {
        let mut ours = Page::new("Projet", None);
        let mut p = crate::project::Project {
            shared: true,
            ..crate::project::Project::new("/Users/moi/code/app")
        };
        p.add_step("Vraie étape", "moi");
        ours.project = Some(p);
        ours.updated = 10;
        let mut theirs = ours.clone();
        theirs.title = "Projet renommé".into();
        theirs.updated = 20;
        let tp = theirs.project.as_mut().unwrap();
        tp.folder = "/".into();
        tp.commit_to_main = true;
        let long = format!("Étape reçue\nsur deux lignes {}", "x".repeat(1000));
        tp.add_step(&long, "autre");
        tp.summary = "Résumé à jour".into();
        let mut merged = ours.clone();
        merge_page(&mut merged, &theirs);
        let mp = merged.project.as_ref().unwrap();
        // The rest of the page follows the copy edited last…
        assert_eq!(merged.title, "Projet renommé");
        assert_eq!(mp.summary, "Résumé à jour");
        // …but not the folder nor « sur main ».
        assert_eq!(mp.folder, "/Users/moi/code/app");
        assert!(!mp.commit_to_main);
        // The steps are shared: ours stays as ours, theirs comes in, on one line, cut short,
        // and marked as received.
        assert_eq!(mp.steps.len(), 2);
        assert_eq!(
            (mp.steps[0].title.as_str(), mp.steps[0].received),
            ("Vraie étape", false)
        );
        assert!(mp.steps[1].received && !mp.steps[1].title.contains('\n'));
        assert!(mp.steps[1].title.chars().count() <= crate::project::RECEIVED_STEP_MAX);
        // A project new here: its steps come in, all marked as received; never its folder.
        let n = received_new(theirs);
        let np = n.project.as_ref().unwrap();
        assert!(np.folder.is_empty() && !np.commit_to_main);
        assert_eq!(np.steps.len(), 2);
        assert!(np.steps.iter().all(|s| s.received));
    }

    /// Steps edited, added and deleted on two computers: each keeps its latest edit, new
    /// ones come in, deleted ones stay deleted, and « received » is never taken from the
    /// other copy.
    #[test]
    fn steps_are_shared_step_by_step() {
        use crate::project::{Project, StepStatus};
        let mut base = Page::new("Projet", None);
        let mut p = Project {
            shared: true,
            ..Project::new("/x")
        };
        let a = p.add_step("A", "moi");
        let b = p.add_step("B", "moi");
        let c = p.add_step("C", "moi");
        base.project = Some(p);
        base.updated = 10;
        let (mut here, mut there) = (base.clone(), base.clone());
        // Here: A done, C deleted. There: B renamed, D added, and it claims D isn't received.
        let hp = here.project.as_mut().unwrap();
        hp.set_step_status(&a, StepStatus::Done);
        hp.remove_step(&c);
        here.updated = 20;
        let tp = there.project.as_mut().unwrap();
        let s = tp.steps.iter_mut().find(|s| s.id == b).unwrap();
        s.title = "B renommée".into();
        s.updated = crate::model::now_ms() + 5;
        let d = tp.add_step("D", "autre");
        there.updated = 30;
        let mut merged = here.clone();
        assert!(merge_page(&mut merged, &there));
        let mp = merged.project.as_ref().unwrap();
        let got: Vec<(&str, bool)> = mp
            .steps
            .iter()
            .map(|s| (s.title.as_str(), s.received))
            .collect();
        assert_eq!(got, [("A", false), ("B renommée", false), ("D", true)]);
        assert_eq!(mp.steps[0].status, StepStatus::Done);
        assert!(mp.deleted_steps.contains_key(&c));
        // And the other way round: C deleted there too, D stays theirs (not received there).
        let mut back = there.clone();
        merge_page(&mut back, &merged);
        let bp = back.project.as_ref().unwrap();
        assert!(!bp.steps.iter().any(|s| s.id == c));
        assert!(!bp.steps.iter().find(|s| s.id == d).unwrap().received);
        assert_eq!(
            bp.steps.iter().find(|s| s.id == a).unwrap().status,
            StepStatus::Done
        );
    }

    /// Two AIs, one per computer, take the same step before seeing each other: both copies
    /// end up giving it to the one that took it first, and the other is told to leave it.
    #[test]
    fn a_step_taken_on_two_computers_goes_to_the_first() {
        use crate::project::{Project, StepStatus};
        let mut base = Page::new("Projet", None);
        let mut p = Project {
            shared: true,
            ..Project::new("/x")
        };
        let a = p.add_step("A", "moi");
        base.project = Some(p);
        let (mut here, mut there) = (base.clone(), base.clone());
        here.project
            .as_mut()
            .unwrap()
            .claim_step(&a, "ordi-ici")
            .unwrap();
        here.updated = crate::model::now_ms();
        // There takes it later: a later edit, yet it must not win.
        there
            .project
            .as_mut()
            .unwrap()
            .claim_step(&a, "ordi-la")
            .unwrap();
        there.updated = crate::model::now_ms();
        let (h, t) = (here.clone(), there.clone());
        merge_page(&mut here, &t);
        merge_page(&mut there, &h);
        for pg in [&here, &there] {
            let s = &pg.project.as_ref().unwrap().steps[0];
            assert_eq!(
                (s.status, s.held_by.as_str()),
                (StepStatus::Doing, "ordi-ici")
            );
        }
        // There, the AI is refused the step; it isn't its next one any more.
        let tp = there.project.as_mut().unwrap();
        assert!(tp.claim_step(&a, "ordi-la").is_err());
        assert!(
            tp.next_step_for("ordi-la", crate::model::now_ms())
                .is_none()
        );
        // Done here: the step comes back done everywhere, reservation cleared.
        here.project
            .as_mut()
            .unwrap()
            .set_step_status(&a, StepStatus::Done);
        merge_page(&mut there, &here);
        let s = &there.project.as_ref().unwrap().steps[0];
        assert_eq!(s.status, StepStatus::Done);
        assert!(s.held_by.is_empty());
    }

    #[test]
    fn added_blocks_are_kept_and_deleted_blocks_stay_deleted() {
        let base = page_with(&["un", "deux"]);
        let mut a = base.clone();
        let mut b = base.clone();
        // A adds a block after "un"; B deletes "deux".
        let nb = Block::new(crate::model::BlockKind::Paragraph, "nouveau (A)");
        a.blocks.insert(1, nb);
        a.updated = crate::model::now_ms();
        let gone = b.blocks.remove(1);
        b.deleted_blocks.insert(gone.id, crate::model::now_ms());
        b.updated = crate::model::now_ms();
        let mut merged = a.clone();
        merge_page(&mut merged, &b);
        let texts: Vec<&str> = merged.blocks.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(texts, ["un", "nouveau (A)"]);
    }

    #[test]
    fn newest_wins_and_deletions_stick() {
        let mut a = Workspace::default();
        let mut p = Page::new("A", None);
        p.project = Some(crate::project::Project {
            shared: true,
            ..crate::project::Project::new("")
        });
        p.updated = 10;
        a.pages.push(p.clone());
        let mut private = Page::new("Privée", None);
        private.updated = 10;
        a.pages.push(private.clone());
        let mut b = a.clone();
        b.pages[0].title = "B".into();
        b.pages[0].updated = 20;
        assert_eq!(merge(&mut a, &b), 1);
        assert_eq!(a.pages[0].title, "B");
        let mut c = a.clone();
        c.pages.clear();
        c.deleted.insert(p.id.clone(), 30);
        // Even knowing its id, they can't delete a private page.
        c.deleted.insert(private.id.clone(), 30);
        merge(&mut a, &c);
        assert_eq!(a.pages.len(), 1);
        assert_eq!(a.pages[0].id, private.id);
        assert!(!a.deleted.contains_key(&private.id));
    }

    /// A copy read from the shared folder (or a command) brings only the shared projects:
    /// not a stray page, not a private project, not an edit of one of our private pages,
    /// not the discussion of a project that isn't shared.
    #[test]
    fn only_shared_projects_come_in_by_sync() {
        let mut ours = Workspace::default();
        let mine = ours.create_page("Notes perso", None);
        ours.page_mut(&mine).unwrap().updated = 10;
        let mut theirs = Workspace::default();
        let shared = theirs.create_project("Partagé", "");
        theirs
            .page_mut(&shared)
            .unwrap()
            .project
            .as_mut()
            .unwrap()
            .shared = true;
        let doc = theirs.create_page("Doc", Some(shared.clone()));
        let stray = theirs.create_page("Page glissée", None);
        let private = theirs.create_project("Privé chez eux", "");
        // Our private page, edited on their side.
        let mut edit = ours.page(&mine).unwrap().clone();
        edit.title = "piratée".into();
        edit.updated = 99;
        theirs.pages.push(edit);
        for (id, pid) in [("ok", &shared), ("non", &private)] {
            theirs.chat.insert(
                id.into(),
                crate::model::ChatMsg {
                    id: id.into(),
                    project: pid.clone(),
                    author: "eux".into(),
                    text: id.into(),
                    at: 1,
                },
            );
        }
        merge(&mut ours, &theirs);
        assert!(ours.page(&shared).is_some() && ours.page(&doc).is_some());
        assert!(ours.page(&stray).is_none() && ours.page(&private).is_none());
        assert_eq!(ours.page(&mine).unwrap().title, "Notes perso");
        assert!(ours.chat.contains_key("ok") && !ours.chat.contains_key("non"));
    }
}
