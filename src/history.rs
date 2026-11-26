//! Versions of the pages, kept on this computer to come back to one.
//!
//! A snapshot is kept at each write by an AI (the page before and after it), and every ten
//! minutes while a page is being edited (the page as it was before the first edit, then as it
//! is ten minutes later). Only what makes a page's content is kept (title, icon, blocks,
//! database), and never twice in a row.
//!
//! One file per snapshot, in `historique/<page id>/` in the space's folder: never in
//! workspace.json (saved whole at each edit), never sent to the others.

use crate::model::{Block, BlockKind, Database, DbView, Page, Workspace};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long a page being edited waits for its next snapshot.
pub const EVERY: Duration = Duration::from_secs(10 * 60);
/// Snapshots kept per page: past it, the oldest go.
const KEPT: usize = 200;

/// What a version of a page holds.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Content {
    pub title: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub blocks: Vec<Block>,
    #[serde(default)]
    pub database: Option<Database>,
}

impl Content {
    /// The page's content, without what changes with no edit of it: times, CRDT documents, a
    /// toggle left open, the way its database is shown.
    pub fn of(page: &Page) -> Content {
        let mut blocks = page.blocks.clone();
        for b in &mut blocks {
            b.updated = 0;
            b.crdt = None;
            if b.kind == BlockKind::Toggle {
                b.checked = false;
            }
        }
        let database = page.database.clone().map(|mut db| {
            db.view = DbView::default();
            db.sort = None;
            db.filter.clear();
            db
        });
        Content {
            title: page.title.clone(),
            icon: page.icon.clone(),
            blocks,
            database,
        }
    }

    /// Nothing worth coming back to: a page just made.
    fn is_blank(&self) -> bool {
        self.title.trim().is_empty()
            && self.database.is_none()
            && self
                .blocks
                .iter()
                .all(|b| b.kind == BlockKind::Paragraph && b.text.is_empty())
    }

    fn key(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn same(&self, other: &Content) -> bool {
        self.key() == other.key()
    }

    /// Puts this content back on `page`, its blocks now (so that the others take it over the
    /// blocks they still hold). The page's other fields stay as they are.
    pub fn apply_to(&self, page: &mut Page, now: u64) {
        page.title = self.title.clone();
        page.icon = self.icon.clone();
        page.blocks = self.blocks.clone();
        for b in &mut page.blocks {
            b.updated = now;
        }
        page.database = self.database.clone();
    }
}

/// Where a snapshot comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum By {
    /// Around a write by an AI.
    Ai,
    /// While the page was being edited.
    Edit,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    /// When it was taken (ms since 1970).
    pub at: u64,
    pub by: By,
    pub content: Content,
}

/// The history folder of the space this process works in.
pub fn dir() -> PathBuf {
    crate::space::dir_of(&crate::store::space_id()).join("historique")
}

/// A page's folder. None for an id that couldn't be a folder's name: ids also come from
/// other people's copies, and must never lead outside the history.
fn page_dir(root: &Path, id: &str) -> Option<PathBuf> {
    let safe = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    safe.then(|| root.join(id))
}

/// The page's snapshot files with their times, oldest first.
fn files(folder: &Path) -> Vec<(u64, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return vec![];
    };
    let mut out: Vec<(u64, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension()? != "json" {
                return None;
            }
            let at = path.file_stem()?.to_str()?.parse().ok()?;
            Some((at, path))
        })
        .collect();
    out.sort();
    out
}

fn read(path: &Path) -> Option<Snapshot> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// The versions of a page, newest first. Empty for an id that has none.
pub fn list(root: &Path, id: &str) -> Vec<Snapshot> {
    let Some(folder) = page_dir(root, id) else {
        return vec![];
    };
    files(&folder).iter().rev().filter_map(|(_, p)| read(p)).collect()
}

/// Keep `page` as it is now, unless it is blank or the same as its latest snapshot. True when
/// one was written. `now`: the time (ms since 1970).
pub fn record_at(root: &Path, page: &Page, by: By, now: u64) -> std::io::Result<bool> {
    let Some(folder) = page_dir(root, &page.id) else {
        return Ok(false);
    };
    let content = Content::of(page);
    if content.is_blank() {
        return Ok(false);
    }
    let known = files(&folder);
    let latest = known.last();
    if latest
        .and_then(|(_, p)| read(p))
        .is_some_and(|s| s.content.same(&content))
    {
        return Ok(false);
    }
    // Two in the same millisecond, or a clock set back: still after the latest.
    let at = latest.map_or(now, |(t, _)| now.max(t + 1));
    let snapshot = Snapshot { at, by, content };
    create_private_dir(&folder)?;
    let bytes = serde_json::to_vec(&snapshot)?;
    crate::store::write_atomic(&folder.join(format!("{at}.json")), &bytes)?;
    for (_, old) in known.iter().take((known.len() + 1).saturating_sub(KEPT)) {
        let _ = std::fs::remove_file(old);
    }
    Ok(true)
}

/// Only its owner reads the history, like the workspace.
fn create_private_dir(folder: &Path) -> std::io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(folder)
}

/// `record_at`, now; a failure is only told: it must never stop the write it goes with.
pub fn record(root: &Path, page: &Page, by: By) {
    if let Err(e) = record_at(root, page, by, crate::model::wall_ms()) {
        eprintln!("orchestra: page version not kept: {e}");
    }
}

/// After an AI's write: each page whose content it changed (or made) is kept as it was
/// before, and as it is now.
pub fn record_ai_write(root: &Path, before: &[Page], after: &Workspace) {
    let before: HashMap<&str, &Page> = before.iter().map(|p| (p.id.as_str(), p)).collect();
    for page in &after.pages {
        match before.get(page.id.as_str()) {
            Some(old) if Content::of(old).same(&Content::of(page)) => {}
            Some(old) => {
                record(root, old, By::Ai);
                record(root, page, By::Ai);
            }
            None => record(root, page, By::Ai),
        }
    }
}

/// A page deleted for good takes its versions with it.
pub fn forget(root: &Path, id: &str) {
    if let Some(folder) = page_dir(root, id)
        && let Err(e) = std::fs::remove_dir_all(folder)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("orchestra: page versions not deleted: {e}");
    }
}

/// The pages being edited, each waiting for its next snapshot.
#[derive(Default)]
pub struct Pending {
    /// Page id -> when its editing began, since its last snapshot.
    since: HashMap<String, Instant>,
}

impl Pending {
    /// `page` is `before` after an edit. At the first one since its last snapshot, how it
    /// was is kept at once: what is about to be lost can be found again.
    pub fn edited(&mut self, root: &Path, before: &Page, page: &Page, now: Instant) {
        if self.since.contains_key(&page.id) || Content::of(before).same(&Content::of(page)) {
            return;
        }
        record(root, before, By::Edit);
        self.since.insert(page.id.clone(), now);
    }

    /// Is `id` already waiting? (Then an edit of it needs no look.)
    pub fn waits(&self, id: &str) -> bool {
        self.since.contains_key(id)
    }

    /// Keep the pages that have waited long enough, as they are now.
    pub fn tick(&mut self, root: &Path, ws: &Workspace, now: Instant) {
        self.keep(root, ws, |since| now.duration_since(since) >= EVERY);
    }

    /// Keep every waiting page now: before quitting, or leaving the space.
    pub fn flush(&mut self, root: &Path, ws: &Workspace) {
        self.keep(root, ws, |_| true);
    }

    fn keep(&mut self, root: &Path, ws: &Workspace, due: impl Fn(Instant) -> bool) {
        self.since.retain(|id, since| {
            if !due(*since) {
                return true;
            }
            if let Some(page) = ws.page(id) {
                record(root, page, By::Edit);
            }
            false
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            Scratch(std::env::temp_dir().join(format!(
                "orchestra-history-{}",
                uuid::Uuid::new_v4().simple()
            )))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn page(text: &str) -> Page {
        let mut p = Page::new("Notes", None);
        p.blocks = vec![Block::new(BlockKind::Paragraph, text)];
        p
    }

    /// The texts of a page's versions, oldest first.
    fn versions(root: &Path, id: &str) -> Vec<String> {
        files(&root.join(id))
            .iter()
            .map(|(_, f)| read(f).unwrap().content.blocks[0].text.clone())
            .collect()
    }

    #[test]
    fn a_version_is_kept_once_and_only_when_the_content_changes() {
        let d = Scratch::new();
        let mut p = page("un");
        assert!(record_at(&d.0, &p, By::Ai, 10).unwrap());
        // Times, a database's view or a document of its own are no new version.
        p.updated += 5;
        p.blocks[0].updated += 5;
        p.blocks[0].crdt = Some("doc".into());
        p.favorite = true;
        assert!(!record_at(&d.0, &p, By::Edit, 20).unwrap());
        p.blocks[0].text = "deux".into();
        // A clock set back: still after the latest.
        assert!(record_at(&d.0, &p, By::Edit, 5).unwrap());
        assert_eq!(versions(&d.0, &p.id), ["un", "deux"]);
        let times: Vec<u64> = files(&d.0.join(&p.id)).iter().map(|f| f.0).collect();
        assert_eq!(times, [10, 11]);
    }

    #[test]
    fn a_blank_page_and_an_odd_id_keep_nothing() {
        let d = Scratch::new();
        let mut blank = page("");
        blank.title.clear();
        assert!(!record_at(&d.0, &blank, By::Ai, 1).unwrap());
        for evil in ["", "..", "../x", "a/b", "/etc"] {
            let mut p = page("x");
            p.id = evil.into();
            assert!(!record_at(&d.0, &p, By::Ai, 1).unwrap(), "{evil}");
            forget(&d.0, evil);
        }
        assert!(!d.0.exists());
    }

    #[test]
    fn the_oldest_go_past_the_limit_and_forgetting_removes_all() {
        let d = Scratch::new();
        let mut p = page("");
        for i in 0..KEPT + 3 {
            p.blocks[0].text = i.to_string();
            record_at(&d.0, &p, By::Edit, i as u64 + 1).unwrap();
        }
        let kept = versions(&d.0, &p.id);
        assert_eq!(kept.len(), KEPT);
        assert_eq!(kept[0], "3");
        assert_eq!(kept.last().unwrap(), &(KEPT + 2).to_string());
        forget(&d.0, &p.id);
        assert!(!d.0.join(&p.id).exists());
    }

    #[test]
    fn an_ai_write_keeps_the_page_before_and_after() {
        let d = Scratch::new();
        let old = page("avant");
        let untouched = page("pareil");
        let before = vec![old.clone(), untouched.clone()];
        let mut ws = Workspace::default();
        let mut changed = old.clone();
        changed.blocks[0].text = "après".into();
        let made = page("neuve");
        ws.pages = vec![changed, untouched.clone(), made.clone()];
        record_ai_write(&d.0, &before, &ws);
        assert_eq!(versions(&d.0, &old.id), ["avant", "après"]);
        assert_eq!(versions(&d.0, &made.id), ["neuve"]);
        assert!(versions(&d.0, &untouched.id).is_empty());
    }

    #[test]
    fn an_edited_page_is_kept_before_then_ten_minutes_later() {
        let d = Scratch::new();
        let start = Instant::now();
        let mut pending = Pending::default();
        let first = page("a");
        let mut now = first.clone();
        now.blocks[0].text = "ab".into();
        pending.edited(&d.0, &first, &now, start);
        assert_eq!(versions(&d.0, &first.id), ["a"]);
        // Further edits wait for the ten minutes.
        let previous = now.clone();
        now.blocks[0].text = "abc".into();
        pending.edited(&d.0, &previous, &now, start + Duration::from_secs(60));
        let mut ws = Workspace::default();
        ws.pages = vec![now.clone()];
        pending.tick(&d.0, &ws, start + EVERY - Duration::from_secs(1));
        assert_eq!(versions(&d.0, &first.id), ["a"]);
        pending.tick(&d.0, &ws, start + EVERY);
        assert_eq!(versions(&d.0, &first.id), ["a", "abc"]);
        assert!(!pending.waits(&first.id));
        // The next edit starts a new wait; quitting keeps it at once.
        let previous = now.clone();
        ws.pages[0].blocks[0].text = "abcd".into();
        pending.edited(&d.0, &previous, &ws.pages[0], start + EVERY);
        assert!(pending.waits(&first.id));
        pending.flush(&d.0, &ws);
        assert_eq!(versions(&d.0, &first.id), ["a", "abc", "abcd"]);
    }

    #[test]
    fn versions_come_newest_first_and_one_can_be_put_back() {
        let d = Scratch::new();
        let mut p = page("un");
        record_at(&d.0, &p, By::Ai, 10).unwrap();
        p.blocks[0].text = "deux".into();
        record_at(&d.0, &p, By::Edit, 20).unwrap();
        let listed: Vec<String> = list(&d.0, &p.id)
            .iter()
            .map(|s| s.content.blocks[0].text.clone())
            .collect();
        assert_eq!(listed, ["deux", "un"]);
        assert!(list(&d.0, "absent").is_empty());
        assert!(list(&d.0, "../x").is_empty());

        // Put the first one back: title, blocks and their new times; the rest of the page stays.
        let old = list(&d.0, &p.id).pop().unwrap();
        let mut current = p.clone();
        current.favorite = true;
        old.content.apply_to(&mut current, 99);
        assert_eq!(current.blocks[0].text, "un");
        assert_eq!(current.blocks[0].updated, 99);
        assert!(current.favorite);
        assert!(Content::of(&current).same(&old.content));
    }

    #[test]
    fn a_change_outside_the_content_starts_no_wait() {
        let d = Scratch::new();
        let mut pending = Pending::default();
        let before = page("a");
        let mut after = before.clone();
        after.favorite = true;
        pending.edited(&d.0, &before, &after, Instant::now());
        assert!(!pending.waits(&before.id));
        assert!(!d.0.exists());
    }
}
