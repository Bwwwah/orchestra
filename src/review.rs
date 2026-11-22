//! Cross review: before a session's branch is merged, another AI than the one that wrote it
//! reads the diff, read-only, and gives its opinion. A problem it calls blocking stops the
//! automatic merge (the person merges by hand); anything else is only an opinion.

use crate::sessions::Tool;

/// What the reviewer may use: reading and git's own history and diff. No shell, no writing,
/// no MCP tool.
const TOOLS: &str = "Read,Glob,Grep,Bash(git log:*),Bash(git show:*),Bash(git diff:*),\
                     Bash(git status:*),Bash(ls:*)";

const MAX_TURNS: u32 = 30;

/// The reviewer's verdict.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Verdict {
    /// Nothing to object to.
    Approved,
    /// Remarks that don't prevent the merge.
    Remarks,
    /// A defect that should be fixed before merging.
    Blocking,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Review {
    pub verdict: Verdict,
    /// The AI that reviewed.
    pub reviewer: String,
    pub text: String,
}

/// The AI to review work written by `author` (a `Tool::name`): one installed that isn't the
/// author. None when the author is the only one.
pub fn reviewer_for(author: &str, installed: &[Tool]) -> Option<Tool> {
    installed.iter().copied().find(|t| t.name() != author)
}

fn prompt(range: &str, title: &str) -> String {
    format!(
        "Tu es relecteur pour Orchestra. Une autre IA a travaillé seule, dans cette copie du dépôt, \
         sur « {title} ». Relis ses changements SANS RIEN MODIFIER : `git diff {range}` (et \
         `git log {range}`), puis lis le code autour si besoin.\n\
         Cherche : bugs, régressions, oublis (tests, cas limites), failles de sécurité (secrets, \
         commandes construites à partir d'entrées, permissions), et code qui ne fait pas ce que \
         son commit annonce. Ne signale pas les goûts de style.\n\n\
         Réponds en français. Ta réponse commence par une seule ligne, exactement l'une de :\n\
         VERDICT: OK\nVERDICT: REMARQUES\nVERDICT: BLOQUANT\n\
         (BLOQUANT seulement pour un défaut qui doit être corrigé avant la fusion), puis, à la ligne, \
         ton avis en quelques phrases ou puces, avec fichier et fonction concernés."
    )
}

/// The verdict line and the text after it. A reply without a readable verdict is kept as
/// remarks: never taken as an approval, never blocking by itself.
pub fn parse(reply: &str) -> (Verdict, String) {
    let reply = reply.trim();
    let mut lines = reply.lines();
    let head = lines
        .by_ref()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .trim_matches(|c: char| c == '*' || c == '`')
        .to_uppercase();
    let rest = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    let Some(v) = head.strip_prefix("VERDICT") else {
        return (Verdict::Remarks, reply.to_string());
    };
    let v = v.trim_start_matches([':', ' ']);
    let verdict = if v.starts_with("BLOQUANT") {
        Verdict::Blocking
    } else if v.starts_with("OK") {
        Verdict::Approved
    } else {
        Verdict::Remarks
    };
    (verdict, rest)
}

/// Have the branch of `r` read by another AI than the one that wrote it. Err: no other AI
/// installed, or it failed: the merge then goes on without a review.
pub fn review_run(r: &crate::model::RunRecord) -> Result<Review, String> {
    let path = crate::sync::shell_path();
    let installed: Vec<Tool> = crate::sync::detect(&path).iter().map(|a| a.tool).collect();
    let tool = reviewer_for(&r.tool, &installed).ok_or(crate::tr!(
        "aucune autre IA installée pour relire",
        "no other AI installed to review"
    ))?;
    let range = crate::auto::review_range(r)?;
    let (reply, used) = crate::sync::ask_with_images(
        &r.worktree,
        prompt(&range, &r.title),
        TOOLS,
        MAX_TURNS,
        Some(tool),
        &[],
        crate::sync::Weight::Full,
    )?;
    let (verdict, text) = parse(&reply);
    Ok(Review {
        verdict,
        reviewer: used.name().to_string(),
        text: crate::richtext::clean(&text, 4000),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reviewer_is_never_the_author() {
        let both = [Tool::ClaudeCode, Tool::Codex];
        assert_eq!(reviewer_for("claude-code", &both), Some(Tool::Codex));
        assert_eq!(reviewer_for("codex", &both), Some(Tool::ClaudeCode));
        assert_eq!(reviewer_for("claude-code", &[Tool::ClaudeCode]), None);
    }

    #[test]
    fn the_verdict_is_read_from_the_first_line() {
        assert_eq!(
            parse("VERDICT: OK\nRien à signaler."),
            (Verdict::Approved, "Rien à signaler.".into())
        );
        assert_eq!(
            parse("**VERDICT : BLOQUANT**\nsrc/a.rs : panique si vide."),
            (Verdict::Blocking, "src/a.rs : panique si vide.".into())
        );
        assert_eq!(parse("VERDICT: REMARQUES\nUn test manque.").0, Verdict::Remarks);
        // No verdict: kept as remarks, neither approved nor blocking.
        assert_eq!(parse("Ça me semble bon.").0, Verdict::Remarks);
    }

    #[test]
    fn the_prompt_names_the_range() {
        let p = prompt("main...orchestra/x", "Étape");
        assert!(p.contains("git diff main...orchestra/x"));
        assert!(p.contains("SANS RIEN MODIFIER"));
    }
}
