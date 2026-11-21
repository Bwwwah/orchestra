//! Saving and loading the workspace as one JSON file on this computer,
//! plus Markdown export and import.

use crate::model::{Block, BlockKind, ColumnKind, Page, Workspace};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Folder holding the workspace. `ORCHESTRA_DIR` overrides the default.
pub fn data_dir() -> PathBuf {
    if let Ok(d) = std::env::var("ORCHESTRA_DIR")
        && !d.trim().is_empty()
    {
        return PathBuf::from(d);
    }
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("orchestra");
    // The app was called "Tableau IA" before: take its data along the first time.
    let old = base.join("tableau-ia");
    if !dir.exists() && old.is_dir() {
        let _ = std::fs::rename(&old, &dir);
    }
    dir
}

/// This computer's id, made once and kept in the data folder (never in workspace.json,
/// which copies of a shared project exchange): it tells whose AI reserved a step.
pub fn computer_id() -> String {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        if cfg!(test) {
            return crate::model::new_id();
        }
        let path = data_dir().join(".computer-id");
        let known = fs::read_to_string(&path)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if known.len() >= 8 && known.chars().all(|c| c.is_ascii_alphanumeric()) {
            return known;
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let _ = fs::create_dir_all(data_dir());
        let _ = write_atomic(&path, id.as_bytes());
        // Two processes making it at once: read back what is on disk, so they agree.
        fs::read_to_string(&path)
            .map(|s| s.trim().to_string())
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or(id)
    })
    .clone()
}

pub fn workspace_path() -> PathBuf {
    data_dir().join("workspace.json")
}

/// Holds the workspace for one read-then-write: several MCP servers (one per Claude Code
/// session) each load the file, change it and save it whole; without a lock, two at once
/// each keep their own change and the second save drops the first. Released on drop.
pub struct WorkspaceLock(#[allow(dead_code)] Option<fs::File>);

/// Waits for the other processes' read-then-write to end, then holds the workspace until
/// the lock is dropped. A lock that can't be taken (an odd file system) doesn't stop the
/// work: it goes on unlocked, as before.
pub fn lock_workspace() -> WorkspaceLock {
    let path = data_dir().join("workspace.json.lock");
    let file = (|| {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut o = fs::OpenOptions::new();
        o.create(true).truncate(false).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let f = o.open(&path)?;
        f.lock()?;
        Ok::<_, std::io::Error>(f)
    })();
    match file {
        Ok(f) => WorkspaceLock(Some(f)),
        Err(e) => {
            eprintln!("orchestra: workspace not locked ({e})");
            WorkspaceLock(None)
        }
    }
}

pub fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

pub fn load() -> Workspace {
    let path = workspace_path();
    match fs::read_to_string(&path) {
        Ok(s) => match serde_json::from_str::<Workspace>(&s) {
            Ok(mut ws) => {
                fix_icons(&mut ws);
                live_code_in(&mut ws);
                ws
            }
            Err(e) => {
                // Keep the unreadable file aside rather than overwrite it.
                let bak = path.with_extension(format!("json.illisible-{}", crate::model::now_ms()));
                let _ = fs::copy(&path, &bak);
                eprintln!(
                    "orchestra: workspace file unreadable ({e}); a copy was kept at {}",
                    bak.display()
                );
                Workspace::starter()
            }
        },
        Err(_) => Workspace::starter(),
    }
}

/// Icons from older versions that the fonts can't draw (they showed as a box of bars).
fn fix_icons(ws: &mut Workspace) {
    for p in &mut ws.pages {
        let new = match p.icon.trim_end_matches('\u{fe0f}') {
            "🤖" => "💬",
            "🏛" => "📐",
            "🤝" => "👥",
            "🗂" | "🗃" => "📋",
            "🧠" | "🧩" | "🏗" | "🗒" => "📄",
            _ => continue,
        };
        p.icon = new.to_string();
    }
}

/// Write atomically: a temporary file then a rename, so a crash never leaves half a file.
/// The live access code isn't in the file (see secret.rs): put the kept one in, or move
/// there the one an older version wrote in clear (the next save leaves it out).
fn live_code_in(ws: &mut Workspace) {
    let c = &mut ws.settings.collab;
    if c.live_code.trim().is_empty() {
        c.live_code = crate::secret::live_code();
    } else if let Err(e) = crate::secret::set_live_code(&c.live_code) {
        eprintln!("orchestra: live code not moved out of the workspace file: {e}");
    }
}

pub fn save(ws: &Workspace) -> std::io::Result<()> {
    // The access code goes to the Keychain, never to the file. An empty one (a workspace
    // read without it) leaves the kept code alone; a failure doesn't stop the pages' save.
    let code = ws.settings.collab.live_code.trim();
    if !code.is_empty()
        && let Err(e) = crate::secret::set_live_code(code)
    {
        eprintln!("orchestra: live code not kept: {e}");
    }
    let path = workspace_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    write_atomic(
        &path,
        serde_json::to_string_pretty(ws)
            .unwrap_or_default()
            .as_bytes(),
    )
}

/// A new file that only this user may read (0600 on Unix), from its creation: the
/// workspace holds the pages, the projects' folders and the AIs' sessions, and must not be
/// readable by other accounts, not even while it is being written. The rename keeps the
/// mode, so `workspace.json` ends up 0600 too (an older 0644 file is replaced on the next
/// save).
fn private_file() -> fs::OpenOptions {
    let mut o = fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o
}

/// Write `bytes` to `path` through a temporary file with a unique name, then a rename.
/// The app and the MCP server save the same file (and several Claude Code sessions the same
/// usage files): a shared temporary name would let one truncate or rename the other's
/// half-written file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace.json");
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
    let written = (|| {
        let mut f = private_file().open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

pub fn page_to_markdown(ws: &Workspace, page: &Page) -> String {
    let mut out = String::new();
    let icon = if page.icon.is_empty() {
        String::new()
    } else {
        format!("{} ", page.icon)
    };
    out.push_str(&format!("# {}{}\n\n", icon, page.display_title()));
    let mut number = 0;
    for b in &page.blocks {
        if b.kind == BlockKind::Numbered {
            number += 1
        } else {
            number = 0
        }
        let line = match b.kind {
            BlockKind::Paragraph => b.text.clone(),
            BlockKind::Heading1 => format!("## {}", b.text),
            BlockKind::Heading2 => format!("### {}", b.text),
            BlockKind::Heading3 => format!("#### {}", b.text),
            BlockKind::Todo => format!("- [{}] {}", if b.checked { "x" } else { " " }, b.text),
            BlockKind::Bullet => format!("- {}", b.text),
            BlockKind::Numbered => format!("{}. {}", number, b.text),
            BlockKind::Toggle => format!(
                "<details><summary>{}</summary>\n\n{}\n\n</details>",
                b.text, b.extra
            ),
            BlockKind::Quote => b
                .text
                .lines()
                .map(|l| format!("> {l}"))
                .collect::<Vec<_>>()
                .join("\n"),
            BlockKind::Callout => format!("> 💡 {}", b.text),
            BlockKind::Code => format!("```\n{}\n```", b.text),
            BlockKind::Divider => "---".to_string(),
            BlockKind::PageLink => {
                let t = ws
                    .page(&b.extra)
                    .map(|p| p.display_title())
                    .unwrap_or_else(|| crate::tr!("Page supprimée", "Deleted page").into());
                format!("[{}]({}.md)", t, safe_name(&t))
            }
        };
        let pad = "  ".repeat(b.indent as usize);
        let line = line
            .lines()
            .map(|l| format!("{pad}{l}"))
            .collect::<Vec<_>>()
            .join("\n");
        out.push_str(&line);
        out.push_str("\n\n");
    }
    if let Some(db) = &page.database {
        out.push_str(&format!(
            "| {} |\n",
            db.columns
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>()
                .join(" | ")
        ));
        out.push_str(&format!(
            "|{}\n",
            db.columns.iter().map(|_| " --- |").collect::<String>()
        ));
        for i in db.visible_rows() {
            let r = &db.rows[i];
            let cells: Vec<String> = db
                .columns
                .iter()
                .map(|c| {
                    let v = r.cells.get(&c.id).cloned().unwrap_or_default();
                    if c.kind == ColumnKind::Checkbox {
                        if v == "true" {
                            "☑".into()
                        } else {
                            "☐".into()
                        }
                    } else {
                        v.replace('|', "\\|")
                    }
                })
                .collect();
            out.push_str(&format!("| {} |\n", cells.join(" | ")));
        }
        out.push('\n');
    }
    out
}

pub fn safe_name(s: &str) -> String {
    let n: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let n = n.trim().to_string();
    if n.is_empty() {
        crate::tr!("Sans titre", "Untitled").into()
    } else {
        n
    }
}

/// Write every page (not in the trash) as a Markdown file in `dir`. Returns how many.
pub fn export_all(ws: &Workspace, dir: &Path) -> std::io::Result<usize> {
    fs::create_dir_all(dir)?;
    let mut n = 0;
    let mut used = std::collections::HashSet::new();
    for p in ws.pages.iter().filter(|p| !p.trashed) {
        let mut name = safe_name(&p.display_title());
        if !used.insert(name.clone()) {
            name = format!("{name} {}", &p.id[..6]);
        }
        fs::write(dir.join(format!("{name}.md")), page_to_markdown(ws, p))?;
        n += 1;
    }
    Ok(n)
}

/// Turn Markdown text into a page (headings, lists, tasks, quotes, code, tables, dividers).
/// Any AI's Markdown goes through `richtext` first, so odd habits (tables, nested lists,
/// HTML tags, `+` bullets, `1)` numbering…) still give clean blocks.
pub fn page_from_markdown(md: &str, fallback_title: &str) -> Page {
    use crate::richtext::{RBlock, clean, parse, table_as_text};
    let mut page = Page::new(fallback_title, None);
    page.blocks.clear();
    let mut title_taken = false;
    for rb in parse(&clean(md, 200_000)) {
        let block = match rb {
            RBlock::Heading(1, t) if !title_taken => {
                page.title = t;
                title_taken = true;
                continue;
            }
            RBlock::Heading(1 | 2, t) => Block::new(BlockKind::Heading1, t),
            RBlock::Heading(3, t) => Block::new(BlockKind::Heading2, t),
            RBlock::Heading(_, t) => Block::new(BlockKind::Heading3, t),
            RBlock::Para(t) => Block::new(BlockKind::Paragraph, t),
            RBlock::Bullet { indent, text } => {
                indented(Block::new(BlockKind::Bullet, text), indent)
            }
            RBlock::Numbered { indent, text, .. } => {
                indented(Block::new(BlockKind::Numbered, text), indent)
            }
            RBlock::Todo { indent, done, text } => {
                let mut b = indented(Block::new(BlockKind::Todo, text), indent);
                b.checked = done;
                b
            }
            RBlock::Quote(t) => Block::new(BlockKind::Quote, t),
            RBlock::Code(t) => Block::new(BlockKind::Code, t),
            RBlock::Table(rows) => Block::new(BlockKind::Code, table_as_text(&rows)),
            RBlock::Rule => Block::new(BlockKind::Divider, ""),
        };
        page.blocks.push(block);
    }
    if page.blocks.is_empty() {
        page.blocks.push(Block::new(BlockKind::Paragraph, ""));
    }
    page
}

fn indented(mut b: Block, indent: u8) -> Block {
    b.indent = indent.min(3);
    b
}

/// Turn raw Markdown left in a page's blocks (tables written line by line, `**` headings,
/// pasted lists…) back into proper blocks. Sub-page links are kept as they are.
/// Returns how many blocks the page has afterwards.
pub fn reformat_page(page: &mut Page) -> usize {
    let t = crate::model::now_ms();
    let old = std::mem::take(&mut page.blocks);
    let mut out: Vec<Block> = vec![];
    let mut run: Vec<Block> = vec![];
    let flush = |run: &mut Vec<Block>, out: &mut Vec<Block>| {
        if run.is_empty() {
            return;
        }
        // Rebuild the Markdown of the run; table lines stay on adjacent lines.
        let tmp = Page {
            blocks: std::mem::take(run),
            ..Page::new("_", None)
        };
        let md = page_to_markdown(&Workspace::default(), &tmp);
        let mut joined = String::new();
        let lines: Vec<&str> = md.lines().collect();
        for (k, l) in lines.iter().enumerate() {
            joined.push_str(l);
            joined.push('\n');
            let table = |x: &str| x.trim_start().starts_with('|');
            if l.trim().is_empty()
                && k > 0
                && k + 1 < lines.len()
                && table(lines[k - 1])
                && table(lines[k + 1])
            {
                joined.pop();
                joined.pop();
                joined.push('\n');
            }
        }
        let p = page_from_markdown(&joined, "_");
        out.extend(
            p.blocks
                .into_iter()
                .filter(|b| !(b.kind == BlockKind::Paragraph && b.text.is_empty())),
        );
    };
    for b in old.iter().cloned() {
        if b.kind == BlockKind::PageLink {
            flush(&mut run, &mut out);
            out.push(b);
        } else {
            run.push(b);
        }
    }
    flush(&mut run, &mut out);
    for b in &old {
        if !out.iter().any(|n| n.id == b.id) {
            page.deleted_blocks.insert(b.id.clone(), t);
        }
    }
    if out.is_empty() {
        out.push(Block::new(BlockKind::Paragraph, ""));
    }
    page.blocks = out;
    page.touch();
    page.blocks.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_atomic_writes_never_mix_or_leave_temp_files() {
        let dir =
            std::env::temp_dir().join(format!("orchestra-store-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("workspace.json");
        let a = "a".repeat(200_000);
        let b = "b".repeat(300_000);
        std::thread::scope(|s| {
            for i in 0..8 {
                let (path, body) = (&path, if i % 2 == 0 { &a } else { &b });
                s.spawn(move || {
                    for _ in 0..10 {
                        write_atomic(path, body.as_bytes()).unwrap();
                    }
                });
            }
        });
        let got = fs::read_to_string(&path).unwrap();
        assert!(got == a || got == b, "file mixes two writes");
        let left: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, vec![std::ffi::OsString::from("workspace.json")]);
        let _ = fs::remove_dir_all(&dir);
    }

    /// The workspace file, rewritten over an older one readable by all, ends up readable by
    /// its owner only; no temporary file is left.
    #[cfg(unix)]
    #[test]
    fn written_files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("orchestra-mode-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("workspace.json");
        fs::write(&path, "{}").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        write_atomic(&path, b"{\"pages\":[]}").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"pages\":[]}");
        // A brand-new file too.
        let other = dir.join("claude-usage.json");
        write_atomic(&other, b"[]").unwrap();
        assert_eq!(
            fs::metadata(&other).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            2,
            "no temporary file left"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn raw_table_lines_become_one_block() {
        let mut p = Page::new("x", None);
        p.blocks = vec![
            Block::new(BlockKind::Paragraph, "| A | B |"),
            Block::new(BlockKind::Paragraph, "|---|---|"),
            Block::new(BlockKind::Paragraph, "| 1 | 2 |"),
            Block::new(BlockKind::Paragraph, "**Fait** : tout"),
        ];
        reformat_page(&mut p);
        assert_eq!(p.blocks.len(), 2);
        assert_eq!(p.blocks[0].kind, BlockKind::Code);
        assert!(p.blocks[0].text.contains('A') && p.blocks[0].text.contains('2'));
        assert_eq!(p.deleted_blocks.len(), 4);
    }
}
