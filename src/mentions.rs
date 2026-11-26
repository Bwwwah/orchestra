//! « @Nom » in what the others write: a comment, a message of a project's discussion, a
//! page's text. The person named finds it in their inbox. Each computer looks for its own
//! person's name in what it receives (inboxes aren't shared), and only what didn't mention
//! them before is told: the same text received again says nothing.

use std::collections::{BTreeSet, HashSet};

use crate::model::{BlockKind, InboxMsg, Page, Workspace, now_ms};
use crate::project::ProjectMember;

/// The kind of an inbox message about a mention.
pub const KIND: &str = "mention";

/// Characters of the mentioning text kept in the inbox message.
const EXCERPT: usize = 400;

/// Who is looked for.
#[derive(Clone, Debug, Default)]
pub struct Me {
    /// The member id in a team space; empty elsewhere.
    pub id: String,
    /// The name the others write after « @ ».
    pub name: String,
    /// The other people, so that « @Ana Martin » isn't taken for Ana.
    pub others: Vec<String>,
}

impl Me {
    /// The person with member id `id` among the people of a team space; None when its
    /// server didn't name them (yet).
    pub fn member(id: &str, people: &[ProjectMember]) -> Option<Me> {
        let name = people.iter().find(|m| !id.is_empty() && m.id == id)?;
        Some(Me {
            id: id.to_string(),
            name: name.name.clone(),
            others: people
                .iter()
                .filter(|m| m.id != id)
                .map(|m| m.name.clone())
                .collect(),
        })
    }

    /// Whether `text` says « @ » and this person's name, in any case: the full name, or the
    /// first name alone when nobody else bears it. Where several names fit, the longest one
    /// is the one meant.
    pub fn is_named_in(&self, text: &str) -> bool {
        if !text.contains('@') {
            return false;
        }
        let low = |s: &str| s.trim().to_lowercase();
        let first_word = |s: &str| s.split_whitespace().next().unwrap_or("").to_string();
        let name = low(&self.name);
        if name.is_empty() {
            return false;
        }
        let others: Vec<String> = self.others.iter().map(|o| low(o)).collect();
        let first = first_word(&name);
        let mut mine = vec![name];
        if others.iter().all(|o| first_word(o) != first) {
            mine.push(first);
        }
        let text = text.to_lowercase();
        text.match_indices('@').any(|(at, _)| {
            // Not the « @ » of an address.
            if text[..at].chars().next_back().is_some_and(is_word) {
                return false;
            }
            let rest = &text[at + 1..];
            let longest = |names: &[String]| {
                names
                    .iter()
                    .filter(|n| !n.is_empty() && starts_with_name(rest, n))
                    .map(|n| n.len())
                    .max()
            };
            longest(&mine).is_some_and(|n| longest(&others).is_none_or(|o| n >= o))
        })
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `rest` begins with the whole of `name`: « ana » is not in « anabelle » nor « ana-lise ».
fn starts_with_name(rest: &str, name: &str) -> bool {
    let Some(after) = rest.strip_prefix(name) else {
        return false;
    };
    let mut after = after.chars();
    match after.next() {
        None => true,
        Some('-' | '.') => !after.next().is_some_and(is_word),
        Some(c) => !is_word(c),
    }
}

/// What of a page mentions the person: ids of blocks and of comments written by others.
pub fn in_page(page: &Page, me: &Me) -> BTreeSet<String> {
    let blocks = page
        .blocks
        .iter()
        .filter(|b| b.kind != BlockKind::Code)
        .filter(|b| {
            me.is_named_in(&b.text) || (b.kind == BlockKind::Toggle && me.is_named_in(&b.extra))
        })
        .map(|b| b.id.clone());
    let comments = page
        .comments
        .values()
        .filter(|c| !c.deleted && !c.is_by(&me.id, &me.name) && me.is_named_in(&c.text))
        .map(|c| c.id.clone());
    blocks.chain(comments).collect()
}

fn excerpt(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(EXCERPT) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

/// A mention to put in the inbox.
struct Found {
    /// The id of the block, comment or message.
    source: String,
    project: String,
    page: Option<String>,
    title: String,
    text: String,
}

impl Workspace {
    /// After page `id` was received: tell what mentions the person in it and didn't in
    /// `before` (`in_page` of our copy before; empty for a page new here). `removed`: the
    /// inbox messages the person deleted. Returns how many were told.
    pub fn notify_page_mentions(
        &mut self,
        id: &str,
        before: &BTreeSet<String>,
        me: &Me,
        removed: &HashSet<String>,
    ) -> usize {
        let Some(page) = self.page(id).filter(|p| !p.trashed) else {
            return 0;
        };
        let project = self.project_of(id).map(|p| p.id.clone()).unwrap_or_default();
        let title = page.display_title();
        let members = self
            .project_of(id)
            .and_then(|p| p.project.as_ref())
            .map_or(&[][..], |p| &p.members);
        let found: Vec<Found> = in_page(page, me)
            .into_iter()
            .filter(|source| !before.contains(source))
            .filter_map(|source| {
                let (title, text) = match page.comments.get(&source) {
                    Some(c) => {
                        let who = c.author_name(members);
                        (
                            crate::trf!(
                                "{who} vous a mentionné dans un commentaire de « {title} »",
                                "{who} mentioned you in a comment on « {title} »"
                            ),
                            excerpt(&c.text),
                        )
                    }
                    // A block's text has no known author.
                    None => {
                        let b = page.blocks.iter().find(|b| b.id == source)?;
                        let text = if me.is_named_in(&b.text) { &b.text } else { &b.extra };
                        (
                            crate::trf!(
                                "Vous êtes mentionné dans « {title} »",
                                "You are mentioned in « {title} »"
                            ),
                            excerpt(text),
                        )
                    }
                };
                Some(Found {
                    source,
                    project: project.clone(),
                    page: Some(id.to_string()),
                    title,
                    text,
                })
            })
            .collect();
        found
            .into_iter()
            .filter(|f| self.push_mention(f, removed))
            .count()
    }

    /// Among the discussion messages `ids`, just received, tell those of other people that
    /// mention the person. Returns how many were told.
    pub fn notify_chat_mentions(
        &mut self,
        ids: &[String],
        me: &Me,
        removed: &HashSet<String>,
    ) -> usize {
        let mut msgs: Vec<_> = ids
            .iter()
            .filter_map(|id| self.chat.get(id))
            .filter(|m| !m.is_by(&me.id, &me.name) && me.is_named_in(&m.text))
            .collect();
        msgs.sort_by_key(|m| (m.at, &m.id));
        let found: Vec<Found> = msgs
            .into_iter()
            .map(|m| {
                let page = self.page(&m.project);
                let members = page
                    .and_then(|p| p.project.as_ref())
                    .map_or(&[][..], |p| &p.members);
                let who = m.author_name(members);
                let title = page.map(|p| p.display_title()).unwrap_or_default();
                Found {
                    source: m.id.clone(),
                    project: m.project.clone(),
                    page: None,
                    title: crate::trf!(
                        "{who} vous a mentionné dans la discussion de « {title} »",
                        "{who} mentioned you in the discussion of « {title} »"
                    ),
                    text: excerpt(&m.text),
                }
            })
            .collect();
        found
            .into_iter()
            .filter(|f| self.push_mention(f, removed))
            .count()
    }

    /// One message per block, comment or message, whose id comes from it: not told again
    /// while it is in the inbox, nor once deleted from it.
    fn push_mention(&mut self, f: &Found, removed: &HashSet<String>) -> bool {
        let id = format!("{KIND}-{}", f.source);
        if removed.contains(&id) || self.inbox.iter().any(|m| m.id == id) {
            return false;
        }
        self.push_inbox(InboxMsg {
            id,
            at: now_ms(),
            kind: KIND.into(),
            project: f.project.clone(),
            title: f.title.clone(),
            text: f.text.clone(),
            page: f.page.clone(),
            ..Default::default()
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Block, ChatMsg};

    fn me(name: &str, others: &[&str]) -> Me {
        Me {
            id: "moi".into(),
            name: name.into(),
            others: others.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn a_name_after_an_at_sign_is_a_mention() {
        let m = me("Zoé Durand", &["Bob"]);
        for yes in [
            "@Zoé Durand, tu valides ?",
            "à voir avec @zoé durand.",
            "@ZOÉ : ok",
            "(@Zoé)",
            "merci @Zoé's",
        ] {
            assert!(m.is_named_in(yes), "{yes}");
        }
        for no in [
            "Zoé Durand",
            "@Zoéline",
            "@Zoé-Lise",
            "ecrire@zoé.fr",
            "@Zoé.fr",
            "@ Zoé",
            "@Bob",
            "@",
        ] {
            assert!(!m.is_named_in(no), "{no}");
        }
        assert!(!me("  ", &[]).is_named_in("@ x"));
    }

    #[test]
    fn the_other_members_names_are_not_mine() {
        // Two « Ana »: the first name alone names neither of those who have a longer name.
        let durand = me("Ana Durand", &["Ana Martin"]);
        assert!(durand.is_named_in("@Ana Durand"));
        assert!(!durand.is_named_in("@Ana Martin"));
        assert!(!durand.is_named_in("@Ana"));
        let ana = me("Ana", &["Ana Martin"]);
        assert!(ana.is_named_in("@Ana, à toi"));
        assert!(!ana.is_named_in("@Ana Martin, à toi"));
        assert!(ana.is_named_in("@Ana Martin et @Ana"));
    }

    #[test]
    fn a_member_is_known_by_their_id() {
        let people: Vec<ProjectMember> = [("a", "Ana"), ("b", "Bob")]
            .map(|(id, name)| ProjectMember {
                id: id.into(),
                name: name.into(),
                role: Default::default(),
                ai: String::new(),
            })
            .into();
        let bob = Me::member("b", &people).unwrap();
        assert_eq!((bob.name.as_str(), bob.others), ("Bob", vec!["Ana".to_string()]));
        assert!(Me::member("c", &people).is_none());
        assert!(Me::member("", &people).is_none());
    }

    #[test]
    fn only_new_mentions_of_a_received_page_are_told_once() {
        let mut ws = Workspace::default();
        let mut page = Page::new("Plan", None);
        page.blocks = vec![
            Block::new(BlockKind::Paragraph, "@Zoé, déjà là"),
            Block::new(BlockKind::Paragraph, "rien"),
            Block::new(BlockKind::Code, "@Zoé"),
        ];
        let id = page.id.clone();
        ws.pages.push(page);
        let m = me("Zoé", &["Bob"]);
        let before = in_page(ws.page(&id).unwrap(), &m);
        assert_eq!(before.len(), 1);

        let removed = HashSet::new();
        let page = ws.pages.iter_mut().find(|p| p.id == id).unwrap();
        page.blocks[1].text = "pour @Zoé".into();
        let block = page.blocks[1].id.clone();
        let theirs = page.add_comment(&block, "@Zoé ?", "Bob", "bob").unwrap();
        page.add_comment(&block, "note pour @Zoé", "Zoé", "moi").unwrap();
        assert_eq!(ws.notify_page_mentions(&id, &before, &m, &removed), 2);
        assert!(ws.inbox.iter().all(|x| x.kind == KIND
            && !x.read
            && x.page.as_deref() == Some(id.as_str())));
        let comment = ws
            .inbox
            .iter()
            .find(|x| x.id == format!("mention-{theirs}"))
            .unwrap();
        assert!(comment.title.starts_with("Bob "), "{}", comment.title);
        assert_eq!(comment.text, "@Zoé ?");
        assert_eq!(ws.unread(), 2);

        // Received again: nothing more, whether still in the inbox or deleted from it.
        assert_eq!(ws.notify_page_mentions(&id, &before, &m, &removed), 0);
        let removed: HashSet<String> = ws.inbox.drain(..).map(|x| x.id).collect();
        assert_eq!(ws.notify_page_mentions(&id, &before, &m, &removed), 0);
        assert!(ws.inbox.is_empty());
    }

    #[test]
    fn a_discussion_message_naming_the_person_reaches_their_inbox() {
        let mut ws = Workspace::default();
        let page = Page::new("Fête", None);
        let project = page.id.clone();
        ws.pages.push(page);
        let long = format!("@Zoé {}", "é".repeat(600));
        for (id, author, text) in [
            ("1", "Bob", long.as_str()),
            ("2", "Bob", "sans mention"),
            ("3", "Zoé", "je signe @Zoé"),
        ] {
            ws.chat.insert(
                id.into(),
                ChatMsg {
                    id: id.into(),
                    project: project.clone(),
                    author: author.into(),
                    author_id: String::new(),
                    text: text.into(),
                    at: 1,
                },
            );
        }
        let ids = ["1", "2", "3", "absent"].map(String::from);
        let m = me("Zoé", &["Bob"]);
        assert_eq!(ws.notify_chat_mentions(&ids, &m, &HashSet::new()), 1);
        let got = &ws.inbox[0];
        assert_eq!((got.project.as_str(), &got.page), (project.as_str(), &None));
        assert!(got.title.contains("Fête"), "{}", got.title);
        assert_eq!(got.text.chars().count(), EXCERPT + 1);
    }
}
