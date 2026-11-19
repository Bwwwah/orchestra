//! AI-written text, whichever AI wrote it: Claude, Codex, Gemini, a local model… each has
//! its own Markdown habits (tables, nested lists, `+` bullets, `1)` numbering, HTML tags,
//! `#####` headings, CRLF, very long lines). Everything an AI sends goes through here
//! first: `clean` normalises it, `parse` turns it into a small set of blocks that the app
//! knows how to draw well. So the look never depends on the AI's style.

/// One block of AI text.
#[derive(Clone, Debug, PartialEq)]
pub enum RBlock {
    /// Level 1 to 4 (deeper headings become 4).
    Heading(u8, String),
    Para(String),
    Bullet { indent: u8, text: String },
    Numbered { indent: u8, num: String, text: String },
    Todo { indent: u8, done: bool, text: String },
    Quote(String),
    Code(String),
    /// First row is the header.
    Table(Vec<Vec<String>>),
    Rule,
}

/// Normalise AI text: line endings, HTML tags, invisible characters, runs of blank lines,
/// trailing spaces, and a maximum length (cut at a line boundary, with a note).
pub fn clean(md: &str, max_chars: usize) -> String {
    let md = md.replace("\r\n", "\n").replace('\r', "\n").replace('\t', "    ");
    let md = md.replace(['\u{200b}', '\u{feff}', '\u{00ad}'], "").replace('\u{00a0}', " ");
    let mut out = String::new();
    let mut blank = 0;
    let mut in_code = false;
    for line in md.lines() {
        let is_fence = line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~");
        let line = if in_code || is_fence { line.trim_end().to_string() } else { strip_html(line).trim_end().to_string() };
        if is_fence {
            in_code = !in_code;
        }
        if line.trim().is_empty() && !in_code {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(&line);
        out.push('\n');
    }
    if in_code {
        out.push_str("```\n");
    }
    let out = out.trim().to_string();
    if out.chars().count() <= max_chars {
        return out;
    }
    let mut cut = String::new();
    for line in out.lines() {
        if cut.chars().count() + line.chars().count() + 1 > max_chars {
            break;
        }
        cut.push_str(line);
        cut.push('\n');
    }
    if cut.matches("```").count() % 2 == 1 {
        cut.push_str("```\n");
    }
    cut.push_str("\n_(texte raccourci)_");
    cut
}

/// `<br>` becomes a space, `<details>`/`<summary>`/other tags disappear, entities are decoded.
fn strip_html(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        // A tag is `<` + letter or `/` … `>` on the same line; `a < b` stays as is.
        let is_tag = after[1..].chars().next().map(|c| c.is_ascii_alphabetic() || c == '/' || c == '!').unwrap_or(false);
        match (is_tag, after.find('>')) {
            (true, Some(j)) => {
                let tag = after[1..j].trim().trim_start_matches('/').to_lowercase();
                let name: String = tag.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
                // Inline tags vanish; block tags (br, p, div, summary, li…) leave a space.
                if !matches!(name.as_str(), "b" | "i" | "em" | "strong" | "code" | "span" | "a" | "u" | "kbd" | "sub" | "sup") && !out.ends_with(' ') && !out.is_empty() {
                    out.push(' ');
                }
                rest = &after[j + 1..];
            }
            _ => {
                out.push('<');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
}

/// Inline Markdown the app doesn't draw: links `[t](url)` keep their text, images go,
/// `__bold__` becomes `**bold**`, escaped characters lose their backslash.
pub fn inline(text: &str) -> String {
    let mut s = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // Image ![alt](url): dropped. Link [text](url): text.
        if (c == '[' || (c == '!' && chars.get(i + 1) == Some(&'['))) && !in_code_span(&chars, i) {
            let start = if c == '!' { i + 1 } else { i };
            if let Some(close) = (start + 1..chars.len()).find(|&k| chars[k] == ']') {
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(end) = (close + 2..chars.len()).find(|&k| chars[k] == ')') {
                        if c != '!' {
                            s.extend(&chars[start + 1..close]);
                        }
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        if c == '\\' && i + 1 < chars.len() && "\\`*_{}[]()#+-.!|>~".contains(chars[i + 1]) {
            s.push(chars[i + 1]);
            i += 2;
            continue;
        }
        s.push(c);
        i += 1;
    }
    // __bold__ → **bold** (the app's inline styles use asterisks), only when paired.
    let out = if s.matches("__").count() % 2 == 0 { s.replace("__", "**") } else { s };
    out.trim().to_string()
}

fn in_code_span(chars: &[char], i: usize) -> bool {
    chars[..i].iter().filter(|&&c| c == '`').count() % 2 == 1
}

/// One short line: for step titles and session names. Removes list markers, checkboxes,
/// heading marks and emphasis, keeps the first line, cuts at `max` characters.
pub fn one_line(text: &str, max: usize) -> String {
    let first = clean(text, 10_000).lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
    let mut t = first.trim_start_matches('#').trim().to_string();
    for p in ["- [ ] ", "- [x] ", "- [X] ", "* [ ] ", "[ ] ", "[x] ", "- ", "* ", "+ "] {
        if let Some(r) = t.strip_prefix(p) {
            t = r.to_string();
            break;
        }
    }
    if let Some((n, r)) = t.split_once(['.', ')']) {
        if !n.is_empty() && n.len() <= 3 && n.chars().all(|c| c.is_ascii_digit()) && r.starts_with(' ') {
            t = r.trim().to_string();
        }
    }
    let t = inline(&t).replace("**", "").replace('`', "");
    cut(&t, max)
}

/// Like `one_line` but keeps inline Markdown (`code`, **bold**): for step titles, which
/// are drawn with their styles.
pub fn one_line_md(text: &str, max: usize) -> String {
    let plain_markers = one_line(&text.replace('`', "\u{1}").replace("**", "\u{2}"), usize::MAX);
    cut(&plain_markers.replace('\u{1}', "`").replace('\u{2}', "**"), max)
}

/// A step title: like `one_line_md`, but a text written on several lines is kept whole
/// (its lines joined with spaces) instead of losing everything after the first line.
pub fn step_title(text: &str, max: usize) -> String {
    let joined = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
    one_line_md(&joined, max)
}

fn cut(t: &str, max: usize) -> String {
    let t = t.to_string();
    if t.chars().count() > max { format!("{}…", t.chars().take(max - 1).collect::<String>().trim_end()) } else { t }
}

fn indent_of(line: &str) -> u8 {
    let n = line.chars().take_while(|c| *c == ' ').count();
    ((n + 1) / 2).min(4) as u8
}

fn table_cells(line: &str) -> Vec<String> {
    let t = line.trim().trim_start_matches('|').trim_end_matches('|');
    t.split('|').map(|c| inline(c.trim())).collect()
}

fn is_table_sep(line: &str) -> bool {
    let t = line.trim();
    t.contains('-') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// Turn cleaned Markdown into blocks.
pub fn parse(md: &str) -> Vec<RBlock> {
    let lines: Vec<&str> = md.lines().collect();
    let mut out = vec![];
    let mut para: Vec<String> = vec![];
    let flush = |para: &mut Vec<String>, out: &mut Vec<RBlock>| {
        if !para.is_empty() {
            out.push(RBlock::Para(inline(&para.join(" "))));
            para.clear();
        }
    };
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        let t = raw.trim();
        // Code fence.
        if t.starts_with("```") || t.starts_with("~~~") {
            flush(&mut para, &mut out);
            let fence = &t[..3];
            let mut code = vec![];
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with(fence) {
                code.push(lines[i]);
                i += 1;
            }
            out.push(RBlock::Code(code.join("\n")));
            i += 1;
            continue;
        }
        if t.is_empty() {
            flush(&mut para, &mut out);
            i += 1;
            continue;
        }
        // Table: a row of cells followed by a separator row.
        if t.contains('|') && i + 1 < lines.len() && is_table_sep(lines[i + 1]) {
            flush(&mut para, &mut out);
            let mut rows = vec![table_cells(t)];
            i += 2;
            while i < lines.len() && lines[i].contains('|') && !lines[i].trim().is_empty() {
                rows.push(table_cells(lines[i]));
                i += 1;
            }
            let w = rows.iter().map(Vec::len).max().unwrap_or(0);
            for r in rows.iter_mut() {
                r.resize(w, String::new());
            }
            out.push(RBlock::Table(rows));
            continue;
        }
        // Heading (#…######), or a setext underline after a paragraph line.
        let hashes = t.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            flush(&mut para, &mut out);
            out.push(RBlock::Heading((hashes as u8).min(4), inline(t[hashes..].trim().trim_end_matches('#').trim())));
            i += 1;
            continue;
        }
        if (t.chars().all(|c| c == '=') || t.chars().all(|c| c == '-')) && t.len() >= 3 && para.len() == 1 {
            let text = para.pop().unwrap();
            out.push(RBlock::Heading(if t.starts_with('=') { 1 } else { 2 }, inline(&text)));
            i += 1;
            continue;
        }
        if matches!(t, "---" | "***" | "___") || (t.len() >= 3 && (t.chars().all(|c| c == '-') || t.chars().all(|c| c == '*'))) {
            flush(&mut para, &mut out);
            out.push(RBlock::Rule);
            i += 1;
            continue;
        }
        let indent = indent_of(raw);
        // To-do.
        let todo = ["- [ ] ", "* [ ] ", "+ [ ] "].iter().find_map(|p| t.strip_prefix(p).map(|r| (false, r))).or_else(|| {
            ["- [x] ", "- [X] ", "* [x] ", "* [X] ", "+ [x] "].iter().find_map(|p| t.strip_prefix(p).map(|r| (true, r)))
        });
        if let Some((done, r)) = todo {
            flush(&mut para, &mut out);
            out.push(RBlock::Todo { indent, done, text: inline(r) });
            i += 1;
            continue;
        }
        if let Some(r) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ ")).or_else(|| t.strip_prefix("• ")) {
            flush(&mut para, &mut out);
            out.push(RBlock::Bullet { indent, text: inline(r) });
            i += 1;
            continue;
        }
        if let Some(pos) = t.find(['.', ')']) {
            let (n, r) = t.split_at(pos);
            if !n.is_empty() && n.len() <= 3 && n.chars().all(|c| c.is_ascii_digit()) && r[1..].starts_with(' ') {
                flush(&mut para, &mut out);
                out.push(RBlock::Numbered { indent, num: n.to_string(), text: inline(&r[1..]) });
                i += 1;
                continue;
            }
        }
        if let Some(r) = t.strip_prefix('>') {
            flush(&mut para, &mut out);
            let mut q = vec![inline(r.trim_start_matches('>').trim())];
            i += 1;
            while i < lines.len() && lines[i].trim().starts_with('>') {
                q.push(inline(lines[i].trim().trim_start_matches('>').trim()));
                i += 1;
            }
            out.push(RBlock::Quote(q.join("\n").trim().to_string()));
            continue;
        }
        // A list item's continuation line (indented, right after an item) joins it.
        if raw.starts_with("  ") && para.is_empty() {
            if let Some(RBlock::Bullet { text, .. } | RBlock::Numbered { text, .. } | RBlock::Todo { text, .. }) = out.last_mut() {
                text.push(' ');
                text.push_str(&inline(t));
                i += 1;
                continue;
            }
        }
        para.push(t.to_string());
        i += 1;
    }
    flush(&mut para, &mut out);
    out
}

/// A table as aligned plain text (for places that can only hold text, like a code block).
pub fn table_as_text(rows: &[Vec<String>]) -> String {
    let w = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..w).map(|c| rows.iter().map(|r| r.get(c).map(|x| x.chars().count()).unwrap_or(0)).max().unwrap_or(0).min(40)).collect();
    let mut out = String::new();
    for (k, r) in rows.iter().enumerate() {
        let line: Vec<String> = (0..w)
            .map(|c| {
                let v = r.get(c).cloned().unwrap_or_default().replace("**", "").replace('`', "");
                let v: String = v.chars().take(40).collect();
                format!("{v:<width$}", width = widths[c])
            })
            .collect();
        out.push_str(line.join("  ").trim_end());
        out.push('\n');
        if k == 0 {
            out.push_str(&widths.iter().map(|n| "─".repeat(*n)).collect::<Vec<_>>().join("  "));
            out.push('\n');
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_style() {
        let md = "## État\r\n\r\n\r\n**Fait** : le modèle.\n\n| Fichier | Rôle |\n|---|:--|\n| `a.rs` | coeur |\n\n- point\n  - sous-point\n1) premier\n- [x] fini\n```rust\nfn x() {}\n```";
        let b = parse(&clean(md, 10_000));
        assert_eq!(b[0], RBlock::Heading(2, "État".into()));
        assert_eq!(b[1], RBlock::Para("**Fait** : le modèle.".into()));
        assert_eq!(b[2], RBlock::Table(vec![vec!["Fichier".into(), "Rôle".into()], vec!["`a.rs`".into(), "coeur".into()]]));
        assert_eq!(b[3], RBlock::Bullet { indent: 0, text: "point".into() });
        assert_eq!(b[4], RBlock::Bullet { indent: 1, text: "sous-point".into() });
        assert_eq!(b[5], RBlock::Numbered { indent: 0, num: "1".into(), text: "premier".into() });
        assert_eq!(b[6], RBlock::Todo { indent: 0, done: true, text: "fini".into() });
        assert_eq!(b[7], RBlock::Code("fn x() {}".into()));
    }

    #[test]
    fn other_ai_habits() {
        let md = "<details><summary>Résumé</summary>\nLigne<br>suite &amp; fin\n</details>\n\nTitre\n=====\n\n+ plus\n##### profond\nVoir [la doc](https://x.y) ![img](a.png) et __gras__.";
        let b = parse(&clean(md, 10_000));
        assert_eq!(b[0], RBlock::Para("Résumé Ligne suite & fin".into()));
        assert_eq!(parse(&clean("<summary>A</summary>B", 100))[0], RBlock::Para("A B".into()));
        assert_eq!(b[1], RBlock::Heading(1, "Titre".into()));
        assert_eq!(b[2], RBlock::Bullet { indent: 0, text: "plus".into() });
        assert_eq!(b[3], RBlock::Heading(4, "profond".into()));
        assert_eq!(b[4], RBlock::Para("Voir la doc  et **gras**.".into()));
    }

    #[test]
    fn long_text_is_cut_cleanly() {
        let md = format!("```\n{}\n```", "x\n".repeat(500));
        let c = clean(&md, 100);
        assert!(c.matches("```").count() % 2 == 0);
        assert!(c.ends_with("_(texte raccourci)_"));
    }

    #[test]
    fn step_titles() {
        assert_eq!(one_line("- [ ] **Brancher** le `MCP`\nexplication", 80), "Brancher le MCP");
        assert_eq!(one_line("2. Écrire les tests", 80), "Écrire les tests");
        assert_eq!(one_line(&"a".repeat(20), 10), "aaaaaaaaa…");
        assert_eq!(step_title("Refaire la page **Orchestration**\navec une barre\n\nen haut", 2000), "Refaire la page **Orchestration** avec une barre en haut");
        let long = "mot ".repeat(200);
        assert_eq!(step_title(&long, crate::project::STEP_MAX), long.trim());
        assert_eq!(one_line_md("- [ ] Retirer `USE_MOCK` **vite**", 80), "Retirer `USE_MOCK` **vite**");
    }

    #[test]
    fn a_lone_less_than_stays() {
        assert_eq!(clean("si a < b alors", 100), "si a < b alors");
    }
}
