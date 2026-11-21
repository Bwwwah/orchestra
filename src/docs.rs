//! A project's documentation as pages under the project, as a tree:
//!
//! ```text
//! 🚀 Mon projet
//!    📘 README                 ← README.md
//!    📚 Documentation          ← docs/ (sub-folders become sub-pages)
//!       📄 Architecture        ← docs/architecture.md
//!       📁 API                 ← docs/api/
//!          📄 Authentification ← docs/api/auth.md
//!    💬 Instructions IA        ← CLAUDE.md, AGENTS.md, GEMINI.md…
//!    📝 Changelog              ← CHANGELOG.md
//! ```
//!
//! The files stay the reference: a page follows its file as long as nobody edits the page
//! in the app. Once edited here, the page is kept as is and shows a banner to go back to the
//! file. A page whose file disappeared goes to the trash, unless it was edited here.
//! Nothing is ever written to the project's files.

use crate::model::{BlockKind, Page, Workspace, now_ms};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Where a page comes from.
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct PageSource {
    /// Absolute path of the file or folder; for a group, the project folder + `#name`.
    pub path: String,
    /// Path shown to people, relative to the project folder.
    pub rel: String,
    #[serde(default)]
    pub is_dir: bool,
    /// Hash of the file when it was last imported, and of the page right after.
    #[serde(default)]
    pub file_hash: u64,
    #[serde(default)]
    pub page_hash: u64,
    #[serde(default)]
    pub imported_at: u64,
    /// File modification time at the last look (skips re-reading unchanged files).
    #[serde(default)]
    pub mtime: u64,
    #[serde(default)]
    pub len: u64,
    /// The file changed while the page had been edited here.
    #[serde(default)]
    pub conflict: bool,
    /// The file is gone but the page was edited here, so it was kept.
    #[serde(default)]
    pub missing: bool,
}

const SKIP_DIRS: [&str; 12] = [
    "node_modules",
    ".git",
    "target",
    "dist",
    "build",
    "vendor",
    ".next",
    ".venv",
    "venv",
    "__pycache__",
    "Pods",
    ".expo",
];
const MAX_FILES: usize = 200;
const MAX_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    File {
        title: String,
        icon: &'static str,
        path: PathBuf,
        rel: String,
    },
    Dir {
        title: String,
        icon: &'static str,
        path: String,
        rel: String,
        is_dir: bool,
        children: Vec<Node>,
    },
}

fn hash<T: Hash>(v: T) -> u64 {
    let mut h = DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

/// What the page says, sub-page links aside (they are added as the tree grows).
pub fn content_hash(p: &Page) -> u64 {
    let parts: Vec<(String, &str, bool, u8)> = p
        .blocks
        .iter()
        .filter(|b| b.kind != BlockKind::PageLink)
        .map(|b| {
            (
                format!("{:?}", b.kind),
                b.text.as_str(),
                b.checked,
                b.indent,
            )
        })
        .collect();
    hash(parts)
}

fn md_name(p: &Path) -> Option<String> {
    let n = p.file_name()?.to_string_lossy().to_string();
    let low = n.to_lowercase();
    (low.ends_with(".md") || low.ends_with(".markdown") || low.ends_with(".mdx")).then_some(n)
}

/// "getting-started.md" → "Getting started"
fn pretty(stem: &str) -> String {
    let s = stem.replace(['-', '_'], " ");
    let s = s.trim();
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).display().to_string()
}

fn walk_docs(root: &Path, dir: &Path, depth: u32, budget: &mut usize) -> Vec<Node> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort_by_key(|p| {
        let n = p
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        // index / README first, then alphabetical.
        (!(n.starts_with("index.") || n.starts_with("readme.")), n)
    });
    let mut out = vec![];
    for p in entries {
        if *budget == 0 {
            break;
        }
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        if p.is_dir() {
            if depth == 0 || SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            let children = walk_docs(root, &p, depth - 1, budget);
            if !children.is_empty() {
                out.push(Node::Dir {
                    title: pretty(&name),
                    icon: "📁",
                    path: p.display().to_string(),
                    rel: rel(root, &p),
                    is_dir: true,
                    children,
                });
            }
        } else if let Some(n) = md_name(&p) {
            if p.metadata().map(|m| m.len() > MAX_BYTES).unwrap_or(true) {
                continue;
            }
            *budget -= 1;
            let stem = n.rsplit_once('.').map(|x| x.0).unwrap_or(&n);
            out.push(Node::File {
                title: pretty(stem),
                icon: "📄",
                rel: rel(root, &p),
                path: p,
            });
        }
    }
    out
}

/// The documentation found in a project folder, in display order.
pub fn discover(folder: &Path) -> Vec<Node> {
    let Ok(rd) = std::fs::read_dir(folder) else {
        return vec![];
    };
    let files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    let find = |names: &[&str]| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = files
            .iter()
            .filter(|p| {
                let n = p
                    .file_name()
                    .map(|n| n.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                names.iter().any(|w| n == *w)
            })
            .cloned()
            .collect();
        v.sort();
        v
    };
    let file = |title: &str, icon: &'static str, p: &PathBuf| Node::File {
        title: title.into(),
        icon,
        path: p.clone(),
        rel: rel(folder, p),
    };
    let mut out = vec![];
    if let Some(p) = find(&["readme.md", "readme.markdown", "readme.mdx"]).first() {
        out.push(file("README", "📘", p));
    }
    let mut budget = MAX_FILES;
    for d in ["docs", "doc", "documentation", "Docs"] {
        let p = folder.join(d);
        if p.is_dir() {
            let children = walk_docs(folder, &p, 4, &mut budget);
            if !children.is_empty() {
                out.push(Node::Dir {
                    title: crate::tr!("Documentation", "Documentation").into(),
                    icon: "📚",
                    path: p.display().to_string(),
                    rel: rel(folder, &p),
                    is_dir: true,
                    children,
                });
            }
            break;
        }
    }
    let ai: Vec<Node> = find(&[
        "claude.md",
        "agents.md",
        "gemini.md",
        ".cursorrules",
        "copilot-instructions.md",
    ])
    .iter()
    .map(|p| {
        let n = p.file_name().unwrap().to_string_lossy().to_string();
        file(&n, "📄", p)
    })
    .collect();
    if !ai.is_empty() {
        out.push(Node::Dir {
            title: crate::tr!("Instructions IA", "AI instructions").into(),
            icon: "💬",
            path: format!("{}#ia", folder.display()),
            rel: String::new(),
            is_dir: false,
            children: ai,
        });
    }
    for (names, title, icon) in [
        (&["changelog.md", "history.md"][..], "Changelog", "📝"),
        (&["architecture.md"][..], "Architecture", "📐"),
        (
            &["contributing.md"][..],
            crate::tr!("Contribuer", "Contributing"),
            "👥",
        ),
        (
            &["todo.md", "roadmap.md"][..],
            crate::tr!("Feuille de route", "Roadmap"),
            "🗺️",
        ),
        (
            &["security.md"][..],
            crate::tr!("Sécurité", "Security"),
            "🔒",
        ),
    ] {
        if let Some(p) = find(names).first() {
            out.push(file(title, icon, p));
        }
    }
    out
}

#[derive(Debug, Default, PartialEq)]
pub struct Stats {
    pub created: usize,
    pub updated: usize,
    pub kept: usize,
    pub removed: usize,
    /// Links to the project's sub-pages taken out of its text.
    pub unlinked: usize,
}

fn child_with_source(ws: &Workspace, parent: &str, path: &str) -> Option<String> {
    ws.pages
        .iter()
        .find(|p| {
            !p.trashed
                && p.parent.as_deref() == Some(parent)
                && p.source.as_ref().is_some_and(|s| s.path == path)
        })
        .map(|p| p.id.clone())
}

/// Replace a page's content with its file's.
fn import(ws: &mut Workspace, id: &str, text: &str, title: &str, keep_h1_title: bool) {
    let parsed = crate::store::page_from_markdown(text, title);
    let Some(p) = ws.page_mut(id) else { return };
    let t = now_ms();
    for b in &p.blocks {
        if b.kind != BlockKind::PageLink {
            p.deleted_blocks.insert(b.id.clone(), t);
        }
    }
    let links: Vec<_> = p
        .blocks
        .iter()
        .filter(|b| b.kind == BlockKind::PageLink)
        .cloned()
        .collect();
    p.blocks = parsed
        .blocks
        .into_iter()
        .filter(|b| !(b.kind == BlockKind::Paragraph && b.text.is_empty()))
        .collect();
    p.blocks.extend(links);
    p.title = if keep_h1_title && !parsed.title.trim().is_empty() && parsed.title != title {
        parsed.title
    } else {
        title.to_string()
    };
    let ph = content_hash(p);
    if let Some(s) = p.source.as_mut() {
        s.file_hash = hash(text);
        s.page_hash = ph;
        s.imported_at = t;
        s.conflict = false;
        s.missing = false;
    }
    p.touch();
}

fn sync_nodes(
    ws: &mut Workspace,
    parent: &str,
    nodes: &[Node],
    seen: &mut Vec<String>,
    st: &mut Stats,
) {
    for n in nodes {
        let (title, icon, key, rel, is_dir) = match n {
            Node::File {
                title,
                icon,
                path,
                rel,
            } => (
                title.as_str(),
                *icon,
                path.display().to_string(),
                rel.clone(),
                false,
            ),
            Node::Dir {
                title,
                icon,
                path,
                rel,
                is_dir,
                ..
            } => (title.as_str(), *icon, path.clone(), rel.clone(), *is_dir),
        };
        seen.push(key.clone());
        let id = match child_with_source(ws, parent, &key) {
            Some(id) => id,
            None => {
                let id = ws.create_page(title, Some(parent.to_string()));
                // Mirrored pages are listed by the project's "Pages" card, not as links in
                // the parent's text.
                if let Some(par) = ws.page_mut(parent) {
                    par.blocks
                        .retain(|b| !(b.kind == BlockKind::PageLink && b.extra == id));
                }
                if let Some(p) = ws.page_mut(&id) {
                    p.icon = icon.to_string();
                    p.blocks.clear();
                    p.source = Some(PageSource {
                        path: key.clone(),
                        rel: rel.clone(),
                        is_dir,
                        ..Default::default()
                    });
                    p.source.as_mut().unwrap().page_hash = content_hash(p);
                }
                st.created += 1;
                id
            }
        };
        match n {
            Node::Dir { children, .. } => sync_nodes(ws, &id, children, seen, st),
            Node::File { path, .. } => {
                let md = std::fs::metadata(path).ok();
                let len = md.as_ref().map(|m| m.len()).unwrap_or(0);
                let mtime = md
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                // Same date and same size: the file wasn't touched, no need to read it.
                let unchanged = ws
                    .page(&id)
                    .and_then(|p| p.source.as_ref())
                    .is_some_and(|s| s.imported_at > 0 && s.mtime == mtime && s.len == len);
                if unchanged {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(path) else {
                    continue;
                };
                if let Some(s) = ws.page_mut(&id).and_then(|p| p.source.as_mut()) {
                    s.mtime = mtime;
                    s.len = len;
                }
                let (src, edited) = {
                    let p = ws.page(&id).unwrap();
                    let s = p.source.clone().unwrap_or_default();
                    let edited = s.imported_at > 0 && content_hash(p) != s.page_hash;
                    (s, edited)
                };
                let fresh = src.imported_at == 0;
                if fresh || (src.file_hash != hash(&text) && !edited) {
                    // Docs pages take their title from the file's first heading.
                    let keep_h1 = icon == "📄";
                    import(ws, &id, &text, title, keep_h1);
                    if !fresh {
                        st.updated += 1;
                    }
                } else if src.file_hash != hash(&text) && edited && !src.conflict {
                    if let Some(s) = ws.page_mut(&id).and_then(|p| p.source.as_mut()) {
                        s.conflict = true;
                    }
                    st.kept += 1;
                }
            }
        }
    }
}

/// Bring a project's documentation pages in line with its folder.
pub fn sync_project(ws: &mut Workspace, project_id: &str) -> Stats {
    let mut st = Stats::default();
    // A project's own sub-pages are listed by its "Pages" card: links to them left in the
    // project's text (by earlier versions) are removed, folder or not.
    let kids: Vec<String> = ws
        .children(Some(project_id))
        .iter()
        .map(|p| p.id.clone())
        .collect();
    if let Some(p) = ws.page_mut(project_id).filter(|p| p.project.is_some()) {
        let before = p.blocks.len();
        p.blocks
            .retain(|b| !(b.kind == BlockKind::PageLink && kids.contains(&b.extra)));
        if p.blocks.len() != before {
            st.unlinked = before - p.blocks.len();
            p.touch();
        }
    }
    let Some(folder) = ws
        .page(project_id)
        .and_then(|p| p.project.as_ref())
        .map(|pr| pr.folder.trim().to_string())
    else {
        return st;
    };
    if folder.is_empty() {
        return st;
    }
    let root = crate::project::normalize_folder(&folder);
    if !root.is_dir() {
        return st;
    }
    let nodes = discover(&root);
    // Links to mirrored pages left by earlier versions: removed from the text.
    let mirrored: Vec<String> = ws
        .pages
        .iter()
        .filter(|p| p.source.is_some())
        .map(|p| p.id.clone())
        .collect();
    let tree = ws.descendants(project_id);
    for p in ws
        .pages
        .iter_mut()
        .filter(|p| p.id == project_id || tree.contains(&p.id))
    {
        let before = p.blocks.len();
        p.blocks
            .retain(|b| !(b.kind == BlockKind::PageLink && mirrored.contains(&b.extra)));
        if p.blocks.len() != before {
            p.touch();
        }
    }
    let mut seen = vec![];
    sync_nodes(ws, project_id, &nodes, &mut seen, &mut st);
    // Pages whose file is gone: trash them, unless edited here.
    let tree = ws.descendants(project_id);
    let gone: Vec<(String, bool)> = ws
        .pages
        .iter()
        .filter(|p| !p.trashed && tree.contains(&p.id))
        .filter_map(|p| {
            let s = p.source.as_ref()?;
            (!seen.contains(&s.path) && s.path.starts_with(&root.display().to_string())).then(
                || {
                    (
                        p.id.clone(),
                        s.imported_at > 0 && content_hash(p) != s.page_hash,
                    )
                },
            )
        })
        .collect();
    for (id, edited) in gone {
        if edited {
            if let Some(s) = ws.page_mut(&id).and_then(|p| p.source.as_mut())
                && !s.missing
            {
                s.missing = true;
                st.kept += 1;
            }
        } else {
            ws.set_trashed(&id, true);
            st.removed += 1;
        }
    }
    st
}

/// Throw away the edits made here and take the file again.
pub fn revert_to_file(ws: &mut Workspace, id: &str) -> Result<(), String> {
    let (path, title, keep) = {
        let p = ws.page(id).ok_or("page introuvable")?;
        let s = p.source.as_ref().ok_or("pas de fichier source")?;
        (s.path.clone(), p.title.clone(), p.icon == "📄")
    };
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    import(ws, id, &text, &title, keep);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_with_docs() -> (Workspace, String, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("tia-docs-{}-{}", std::process::id(), now_ms()));
        std::fs::create_dir_all(root.join("docs/api")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::write(root.join("README.md"), "# Mon projet\n\nUne app.\n").unwrap();
        std::fs::write(root.join("CLAUDE.md"), "Toujours tester.\n").unwrap();
        std::fs::write(root.join("CHANGELOG.md"), "## 1.0\n- début\n").unwrap();
        std::fs::write(
            root.join("docs/getting-started.md"),
            "# Bien démarrer\n\nÉtapes.\n",
        )
        .unwrap();
        std::fs::write(root.join("docs/api/auth.md"), "Jetons.\n").unwrap();
        std::fs::write(root.join("node_modules/x/README.md"), "ignored").unwrap();
        let mut ws = Workspace::default();
        let id = ws.create_project("Mon projet", &root.display().to_string());
        (ws, id, root)
    }

    fn titles(ws: &Workspace, parent: &str) -> Vec<String> {
        ws.pages
            .iter()
            .filter(|p| !p.trashed && p.parent.as_deref() == Some(parent))
            .map(|p| format!("{} {}", p.icon, p.title))
            .collect()
    }

    fn child(ws: &Workspace, parent: &str, title: &str) -> String {
        ws.pages
            .iter()
            .find(|p| !p.trashed && p.parent.as_deref() == Some(parent) && p.title == title)
            .unwrap()
            .id
            .clone()
    }

    #[test]
    fn project_pages_are_not_linked_in_its_text() {
        let mut ws = Workspace::default();
        let id = ws.create_project("Sans dossier", "");
        let kid = ws.create_page("Idées", Some(id.clone()));
        let links = |ws: &Workspace| {
            ws.page(&id)
                .unwrap()
                .blocks
                .iter()
                .filter(|b| b.kind == BlockKind::PageLink)
                .count()
        };
        assert_eq!(links(&ws), 0);
        // A link left by an earlier version goes away at the next sync, even without a folder.
        let mut b = crate::model::Block::new(BlockKind::PageLink, "");
        b.extra = kid;
        ws.page_mut(&id).unwrap().blocks.push(b);
        assert_eq!(sync_project(&mut ws, &id).unlinked, 1);
        assert_eq!(links(&ws), 0);
        // An ordinary page still links to its sub-pages.
        let page = ws.create_page("Notes", None);
        ws.create_page("Sous-page", Some(page.clone()));
        assert_eq!(
            ws.page(&page)
                .unwrap()
                .blocks
                .iter()
                .filter(|b| b.kind == BlockKind::PageLink)
                .count(),
            1
        );
    }

    #[test]
    fn tree_follows_the_folder() {
        let (mut ws, id, root) = project_with_docs();
        let st = sync_project(&mut ws, &id);
        assert_eq!(st.created, 8);
        assert_eq!(
            titles(&ws, &id),
            vec![
                "📘 README",
                "📚 Documentation",
                "💬 Instructions IA",
                "📝 Changelog"
            ]
        );
        let doc = child(&ws, &id, crate::tr!("Documentation", "Documentation"));
        assert_eq!(titles(&ws, &doc), vec!["📁 Api", "📄 Bien démarrer"]);
        let readme = child(&ws, &id, "README");
        assert_eq!(ws.page(&readme).unwrap().blocks[0].text, "Une app.");
        // The project page links to its sub-pages.
        assert_eq!(
            ws.page(&id)
                .unwrap()
                .blocks
                .iter()
                .filter(|b| b.kind == BlockKind::PageLink)
                .count(),
            0
        );

        // Nothing changed: nothing to do.
        assert_eq!(sync_project(&mut ws, &id), Stats::default());

        // The file changes: the page follows.
        std::fs::write(root.join("README.md"), "# Mon projet\n\nUne app mobile.\n").unwrap();
        assert_eq!(sync_project(&mut ws, &id).updated, 1);
        assert_eq!(ws.page(&readme).unwrap().blocks[0].text, "Une app mobile.");

        // Edited here: kept, and flagged when the file changes again.
        ws.page_mut(&readme).unwrap().blocks[0].text = "Ma version.".into();
        std::fs::write(root.join("README.md"), "# Mon projet\n\nAutre.\n").unwrap();
        assert_eq!(sync_project(&mut ws, &id).kept, 1);
        let p = ws.page(&readme).unwrap();
        assert_eq!(p.blocks[0].text, "Ma version.");
        assert!(p.source.as_ref().unwrap().conflict);
        revert_to_file(&mut ws, &readme).unwrap();
        assert_eq!(ws.page(&readme).unwrap().blocks[0].text, "Autre.");
        assert!(!ws.page(&readme).unwrap().source.as_ref().unwrap().conflict);

        // A file removed: its page goes to the trash.
        std::fs::remove_file(root.join("CHANGELOG.md")).unwrap();
        assert_eq!(sync_project(&mut ws, &id).removed, 1);
        assert!(!titles(&ws, &id).iter().any(|t| t.contains("Changelog")));
        let _ = std::fs::remove_dir_all(root);
    }
}
