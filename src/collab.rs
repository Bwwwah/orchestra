//! Merging copies of the workspace: what comes from the others (live collaboration) and what
//! another program (the MCP server, an AI) wrote to the file meanwhile.
//!
//! Merging is per page, then block by block: each block keeps its most recent edit, and pages
//! or blocks deleted for good stay deleted (their ids are kept with the time of deletion).

use crate::model::{Block, Page, Workspace};

/// A team space: bring each block's text into its document (crdt.rs), for the pages `ids`
/// (all of them with None). Done when the space is opened and pages arrive (so every block
/// has its document before anyone edits it), and before pages leave (so an edit made here,
/// by the window or an AI, is recorded once, here). Returns true when a document changed.
pub fn give_documents(
    ws: &mut Workspace,
    ids: Option<&std::collections::HashSet<String>>,
    missing_only: bool,
) -> bool {
    if !ws.whole_shared {
        return false;
    }
    let mut changed = false;
    for p in ws
        .pages
        .iter_mut()
        .filter(|p| ids.is_none_or(|s| s.contains(&p.id)))
    {
        for b in p
            .blocks
            .iter_mut()
            .filter(|b| !missing_only || b.crdt.is_none())
        {
            changed |= crate::crdt::reconcile(b);
        }
        if !missing_only || p.order_crdt.is_none() {
            changed |= crate::crdt::reconcile_order(p);
        }
    }
    changed
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
            // A team space's block: both texts merged character by character (crdt.rs).
            Some(o) if o.crdt.is_some() || b.crdt.is_some() => crate::crdt::merged(b, o),
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
    // A team space's page: the order of both copies joined (crdt.rs), every move kept.
    let order = crate::crdt::merged_order(ours, theirs);
    if let Some((ids, _)) = &order {
        let at: HashMap<&str, usize> = ids.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
        // A block the order doesn't know keeps its place after the one before it.
        let mut last = 0usize;
        let mut keyed: Vec<(usize, usize, Block)> = Vec::with_capacity(blocks.len());
        for (n, b) in blocks.into_iter().enumerate() {
            let k = at.get(b.id.as_str()).copied().unwrap_or(last);
            last = k;
            keyed.push((k, n, b));
        }
        keyed.sort_by_key(|(k, n, _)| (*k, *n));
        blocks = keyed.into_iter().map(|(_, _, b)| b).collect();
    }
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
    if let Some((_, crdt)) = order {
        merged.order_crdt = crdt;
    }
    merged.updated = ours.updated.max(theirs.updated);
    *ours = merged;
    serde_json::to_string(&ours).unwrap_or_default() != before
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
