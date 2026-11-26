//! What every AI run without a window has in common. A question is a `Request`, its answer a
//! `Reply`, and a `Provider` turns one into the other: Claude Code and Codex through their
//! command line (`impl Provider for Agent`, in `sync`), and any other source of answers
//! without the callers changing. Nothing here knows about a command line.
//!
//! Every call goes through `send`, where the space's AI rule is kept; `ask_first` adds the
//! choice of the provider.

use super::{AUTH_ERROR, PER_PROJECT, Weight};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// The provider that answers when Claude Code can't renew its sign-in (`AUTH_ERROR`).
const BACKUP: &str = "codex";

/// One question to an AI that works without a window, read-only.
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    /// The folder the question is about.
    pub folder: &'a str,
    pub prompt: &'a str,
    /// The id of an earlier conversation this prompt carries on (`Reply::session`).
    pub resume: Option<&'a str>,
    /// What the AI may use, written as for Claude Code's `--allowedTools` (Codex runs
    /// read-only whatever this says).
    pub tools: &'a str,
    /// What it may not use even so, written the same way ("": nothing more).
    pub denied_tools: &'a str,
    /// The most exchanges the AI may take to answer.
    pub max_turns: u32,
    /// Image files to look at.
    pub images: &'a [String],
    /// How much thinking it takes: each provider picks its model accordingly.
    pub weight: Weight,
    /// Longest the AI may take before it is stopped.
    pub limit: Duration,
}

impl<'a> Request<'a> {
    /// A full question with no tool, no image and one exchange: set what it needs.
    pub fn new(folder: &'a str, prompt: &'a str) -> Self {
        Request {
            folder,
            prompt,
            resume: None,
            tools: "",
            denied_tools: "",
            max_turns: 1,
            images: &[],
            weight: Weight::Full,
            limit: PER_PROJECT,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reply {
    pub text: String,
    /// A short account of the run (« terminé en 42 s »).
    pub report: String,
    /// The conversation's id, when it can be carried on.
    pub session: Option<String>,
}

pub trait Provider {
    /// A stable name (`claude-code`, `codex`…): what the space's AI rule and `prefer` name.
    fn id(&self) -> &str;
    /// The name shown to the person.
    fn label(&self) -> &str;
    /// Answer `request`, stopping when `cancel` is set. An error is a message people can act
    /// on. Call it through `send` or `ask_first`, never directly.
    fn run(&self, request: &Request, cancel: &AtomicBool) -> Result<Reply, String>;
}

/// Why the space doesn't allow this provider, if it doesn't.
fn refusal(provider: &dyn Provider) -> Option<String> {
    crate::space::refusal(provider.id(), provider.label())
}

/// Send a request to this provider, if the space allows it.
pub fn send(
    provider: &dyn Provider,
    request: &Request,
    cancel: &AtomicBool,
) -> Result<Reply, String> {
    match refusal(provider) {
        Some(why) => Err(why),
        None => provider.run(request, cancel),
    }
}

/// An answer that says something, or the error to show instead.
pub fn answered(reply: Reply) -> Result<Reply, String> {
    if reply.text.trim().is_empty() {
        return Err(crate::tr!("l'IA n'a rien répondu", "the AI answered nothing").into());
    }
    Ok(reply)
}

/// Ask one of these providers: `prefer` when it is there and allowed, else the first allowed
/// one. When Claude Code can't renew its sign-in, Codex answers instead if it is there, unless
/// it was already the one preferred. Returns the answer and who gave it.
pub fn ask_first<'p, P: Provider>(
    providers: &'p [P],
    prefer: Option<&str>,
    request: &Request,
) -> Result<(Reply, &'p P), String> {
    // Only the AIs the space allows; none: say why.
    let allowed: Vec<&P> = providers.iter().filter(|p| refusal(*p).is_none()).collect();
    let Some(first) = allowed
        .iter()
        .copied()
        .find(|p| Some(p.id()) == prefer)
        .or_else(|| allowed.first().copied())
    else {
        return Err(providers.iter().find_map(|p| refusal(p)).unwrap_or_else(|| {
            crate::tr!(
                "Ni Claude Code ni Codex n'ont été trouvés sur cette machine.",
                "Neither Claude Code nor Codex was found on this computer."
            )
            .into()
        }));
    };
    let cancel = AtomicBool::new(false);
    let ask = |p: &'p P| {
        send(p, request, &cancel)
            .and_then(answered)
            .map(|reply| (reply, p))
    };
    let result = ask(first);
    if matches!(&result, Err(e) if e == AUTH_ERROR)
        && prefer != Some(BACKUP)
        && let Some(backup) = allowed
            .iter()
            .copied()
            .find(|p| p.id() == BACKUP && p.id() != first.id())
        && let Ok(other) = ask(backup)
    {
        return Ok(other);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Fake {
        id: &'static str,
        answer: Result<&'static str, &'static str>,
        asked: Cell<u32>,
    }

    impl Provider for Fake {
        fn id(&self) -> &str {
            self.id
        }
        fn label(&self) -> &str {
            self.id
        }
        fn run(&self, _: &Request, _: &AtomicBool) -> Result<Reply, String> {
            self.asked.set(self.asked.get() + 1);
            self.answer
                .map(|a| Reply {
                    text: a.into(),
                    ..Reply::default()
                })
                .map_err(String::from)
        }
    }

    fn fake(id: &'static str, answer: Result<&'static str, &'static str>) -> Fake {
        Fake {
            id,
            answer,
            asked: Cell::new(0),
        }
    }

    /// Who answered and what, or the error.
    fn who(providers: &[Fake], prefer: Option<&str>) -> Result<(String, String), String> {
        ask_first(providers, prefer, &Request::new("/x", "?"))
            .map(|(r, p)| (p.id.to_string(), r.text))
    }

    fn ok(id: &str, text: &str) -> Result<(String, String), String> {
        Ok((id.into(), text.into()))
    }

    #[test]
    fn the_preferred_provider_answers_else_the_first() {
        let all = [fake("claude-code", Ok("a")), fake("local", Ok("b"))];
        assert_eq!(who(&all, Some("local")), ok("local", "b"));
        assert_eq!(who(&all, None), ok("claude-code", "a"));
        assert_eq!(who(&all, Some("codex")), ok("claude-code", "a"));
        assert!(who(&[], None).unwrap_err().contains("Codex"));
    }

    #[test]
    fn codex_answers_when_claude_code_cannot_sign_in() {
        let all = [fake("claude-code", Err(AUTH_ERROR)), fake("codex", Ok("b"))];
        assert_eq!(who(&all, Some("claude-code")), ok("codex", "b"));
        assert_eq!(who(&all, None), ok("codex", "b"));
        // Codex failing too, or answering nothing: Claude Code's own error is the one shown.
        for second in [Err("panne"), Ok("  ")] {
            let all = [fake("claude-code", Err(AUTH_ERROR)), fake("codex", second)];
            assert_eq!(who(&all, None), Err(AUTH_ERROR.into()));
            assert_eq!(all[1].asked.get(), 1);
        }
        // No Codex: nobody else is asked.
        let all = [fake("claude-code", Err(AUTH_ERROR)), fake("local", Ok("b"))];
        assert_eq!(who(&all, None), Err(AUTH_ERROR.into()));
        assert_eq!(all[1].asked.get(), 0);
    }

    #[test]
    fn another_error_or_an_empty_answer_goes_back_as_it_is() {
        let all = [fake("claude-code", Err("panne")), fake("codex", Ok("b"))];
        assert_eq!(who(&all, None), Err("panne".into()));
        assert_eq!(all[1].asked.get(), 0);
        let all = [fake("claude-code", Ok(" \n"))];
        assert!(who(&all, None).is_err());
    }
}
