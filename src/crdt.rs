//! Several people writing in the same paragraph (docs/collaboration-serveur.md, step 4).
//!
//! In a team space, each block's text also lives in a small Loro document (a CRDT), kept in
//! the block itself (`Block::crdt`, base64). Two people editing the same block no longer have
//! one edit win over the other: both edits are merged character by character.
//!
//! The editor, the AIs (MCP), imports and exports keep working on `Block::text`, as before.
//! Before a page leaves, or when two copies of a block are merged, the text is brought into
//! the document (`reconcile`): Loro works out what changed since the document's own text and
//! records it as an edit of this program. The merge then joins both documents; the text is
//! the merged document's.
//!
//! Blocks without a document (spaces of this computer, live mode) keep the old rule: the most
//! recent edit of the block wins.

use crate::model::Block;
use base64::Engine;
use loro::{ExportMode, LoroDoc, UpdateOptions};

const TEXT: &str = "t";

/// The name an edit is recorded under: made from the block, the state of its document before
/// and the text it is brought to. The same edit recorded twice (one copy of a block merged
/// twice, before its document was kept) gets the same name, so Loro takes it once; two
/// different edits never share a name.
fn peer_for(id: &str, doc: &LoroDoc, text: &str) -> u64 {
    use sha2::{Digest, Sha256};
    let mut f: Vec<(u64, i32)> = doc
        .oplog_frontiers()
        .iter()
        .map(|x| (x.peer, x.counter))
        .collect();
    f.sort_unstable();
    let mut h = Sha256::new();
    h.update(id.as_bytes());
    h.update([0]);
    for (p, c) in f {
        h.update(p.to_le_bytes());
        h.update(c.to_le_bytes());
    }
    h.update([0]);
    h.update(text.as_bytes());
    let d = h.finalize();
    let mut n = [0u8; 8];
    n.copy_from_slice(&d[..8]);
    // Loro reserves the largest value.
    u64::from_le_bytes(n) % (u64::MAX - 1) + 1
}

/// The document a block starts with, the same on every computer for the same block and text:
/// two people who give a document to the same block at once get the same one (its edits have
/// the same names), so its text isn't doubled when they meet.
fn first_doc(b: &Block) -> LoroDoc {
    use sha2::{Digest, Sha256};
    let doc = LoroDoc::new();
    let h = Sha256::digest(format!("{}\u{0}{}", b.id, b.text).as_bytes());
    let mut n = [0u8; 8];
    n.copy_from_slice(&h[..8]);
    let _ = doc.set_peer_id(u64::from_le_bytes(n) % (u64::MAX - 1) + 1);
    if !b.text.is_empty() {
        let _ = doc.get_text(TEXT).insert(0, &b.text);
    }
    doc.commit();
    doc
}

fn load(b: &Block) -> LoroDoc {
    b.crdt
        .as_deref()
        .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
        .and_then(|bytes| {
            let d = LoroDoc::new();
            d.import(&bytes).ok().map(|_| d)
        })
        .unwrap_or_else(|| first_doc(b))
}

fn save(doc: &LoroDoc) -> Option<String> {
    doc.export(ExportMode::Snapshot)
        .ok()
        .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Blocks known to have their text in their document (by a fingerprint of id, document and
/// text): bringing them up to date again is then free, without opening the document.
fn known() -> &'static std::sync::Mutex<std::collections::HashSet<u64>> {
    static K: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<u64>>> =
        std::sync::OnceLock::new();
    K.get_or_init(Default::default)
}

fn fingerprint(b: &Block) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    b.id.hash(&mut h);
    b.crdt.hash(&mut h);
    b.text.hash(&mut h);
    h.finish()
}

fn remember_known(b: &Block) {
    let mut k = known().lock().unwrap_or_else(|e| e.into_inner());
    if k.len() > 200_000 {
        k.clear();
    }
    k.insert(fingerprint(b));
}

/// Bring the block's text into its document (making one if it has none). Returns true when
/// the document changed.
pub fn reconcile(b: &mut Block) -> bool {
    if b.crdt.is_some()
        && known()
            .lock()
            .map(|k| k.contains(&fingerprint(b)))
            .unwrap_or(false)
    {
        return false;
    }
    let changed = reconcile_now(b);
    remember_known(b);
    changed
}

fn reconcile_now(b: &mut Block) -> bool {
    let had = b.crdt.is_some();
    let doc = load(b);
    let text = doc.get_text(TEXT);
    if text.to_string() == b.text && had {
        return false;
    }
    if text.to_string() != b.text {
        let _ = doc.set_peer_id(peer_for(&b.id, &doc, &b.text));
        let _ = text.update(&b.text, UpdateOptions::default());
        doc.commit();
    }
    match save(&doc) {
        Some(s) => {
            b.crdt = Some(s);
            true
        }
        None => false,
    }
}

/// `ours` and `theirs`, two copies of one block, merged: the other settings from the copy
/// edited last (as before), the text from both documents joined. Copies without documents
/// keep the old rule (the copy edited last wins).
pub fn merged(ours: &Block, theirs: &Block) -> Block {
    let mut out = if theirs.updated > ours.updated {
        theirs.clone()
    } else {
        ours.clone()
    };
    if ours.crdt.is_none() && theirs.crdt.is_none() {
        return out;
    }
    // Only one copy has a document (the other comes from before, or from an older version):
    // there is no common start to merge from, so the copy edited last wins, as before, but
    // through the document (written over it, never added to it: no doubled text).
    if ours.crdt.is_none() != theirs.crdt.is_none() {
        let (with, without) = if ours.crdt.is_some() {
            (ours, theirs)
        } else {
            (theirs, ours)
        };
        let mut x = with.clone();
        if without.updated > with.updated {
            x.text = without.text.clone();
        }
        reconcile(&mut x);
        out.text = x.text;
        out.crdt = x.crdt;
        out.updated = ours.updated.max(theirs.updated);
        return out;
    }
    let (mut a, mut b) = (ours.clone(), theirs.clone());
    reconcile(&mut a);
    reconcile(&mut b);
    let (da, db) = (load(&a), load(&b));
    // The same state, one copy lightened (`compact`): the lighter one, as is.
    if da.oplog_frontiers() == db.oplog_frontiers() {
        let lighter = match (&a.crdt, &b.crdt) {
            (Some(x), Some(y)) if y.len() < x.len() => b.crdt.clone(),
            _ => a.crdt.clone(),
        };
        out.text = a.text.clone();
        out.crdt = lighter;
        out.updated = ours.updated.max(theirs.updated);
        return out;
    }
    let doc = da;
    let before = doc.oplog_vv();
    let imported = b
        .crdt
        .as_deref()
        .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
        .map(|bytes| doc.import(&bytes).is_ok())
        .unwrap_or(false);
    if !imported {
        // No common start any more (one side was lightened since the other's edit was made):
        // the copy edited last wins, written over the document, as with a copy without one.
        let mut keep = if theirs.updated > ours.updated { b } else { a };
        reconcile(&mut keep);
        out.text = keep.text;
        out.crdt = keep.crdt;
        out.updated = ours.updated.max(theirs.updated);
        return out;
    }
    out.text = doc.get_text(TEXT).to_string();
    // Nothing new from theirs: our document as it was (the same bytes, so no echo).
    out.crdt = if doc.oplog_vv() == before {
        a.crdt
    } else {
        save(&doc).or(a.crdt)
    };
    out.updated = ours.updated.max(theirs.updated);
    out
}

/// Lighten a block's document: its history is dropped, only its current state is kept (a
/// shallow snapshot). For blocks nobody has touched for a while (the server does it, see
/// team.rs): an edit made offline from before would no longer merge character by character,
/// only by the copy edited last. Returns true when it got lighter.
pub fn compact(b: &mut Block) -> bool {
    let Some(old) = b.crdt.clone() else {
        return false;
    };
    let doc = load(b);
    let Ok(bytes) = doc.export(ExportMode::ShallowSnapshot(std::borrow::Cow::Owned(
        doc.oplog_frontiers(),
    ))) else {
        return false;
    };
    let new = base64::engine::general_purpose::STANDARD.encode(bytes);
    if new.len() >= old.len() {
        return false;
    }
    b.crdt = Some(new);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::BlockKind;

    fn with_doc(text: &str) -> Block {
        let mut b = Block::new(BlockKind::Paragraph, text);
        b.id = "bloc".into();
        reconcile(&mut b);
        b
    }

    #[test]
    fn two_people_writing_in_the_same_paragraph_keep_both_edits() {
        let base = with_doc("Le plan de la semaine.");
        let mut ana = base.clone();
        ana.text = "Le plan détaillé de la semaine.".into();
        ana.updated = base.updated + 1;
        let mut bob = base.clone();
        bob.text = "Le plan de la semaine. À valider lundi.".into();
        bob.updated = base.updated + 2;
        let ab = merged(&ana, &bob);
        let ba = merged(&bob, &ana);
        assert_eq!(ab.text, "Le plan détaillé de la semaine. À valider lundi.");
        assert_eq!(ab.text, ba.text, "the same in either order");
        // A third copy merging both again changes nothing.
        let again = merged(&ab, &ba);
        assert_eq!(again.text, ab.text);
    }

    #[test]
    fn a_deletion_and_an_insertion_in_one_block_both_hold() {
        let base = with_doc("un deux trois");
        let mut ana = base.clone();
        ana.text = "un trois".into();
        let mut bob = base.clone();
        bob.text = "un deux trois quatre".into();
        assert_eq!(merged(&ana, &bob).text, "un trois quatre");
    }

    #[test]
    fn a_block_given_a_document_on_two_computers_is_not_doubled() {
        // Neither has a document yet, the same text: each makes one, the same.
        let mut a = Block::new(BlockKind::Paragraph, "Bonjour");
        a.id = "x".into();
        let (mut x, mut y) = (a.clone(), a.clone());
        reconcile(&mut x);
        reconcile(&mut y);
        x.text = "Bonjour à tous".into();
        y.text = "Bonjour !".into();
        // Both inserted at the same place: either order (the same on every computer), each
        // once.
        let m = merged(&x, &y).text;
        assert!(m == "Bonjour à tous !" || m == "Bonjour ! à tous", "{m}");
        assert_eq!(merged(&y, &x).text, m);
        // One side with a document, the other without one yet (newer): written over, once.
        let mut old = a.clone();
        old.text = "Bonjour, l'équipe".into();
        old.updated = a.updated + 10;
        let m = merged(&x, &old);
        assert_eq!(m.text, "Bonjour, l'équipe");
        assert!(m.crdt.is_some());
        // Older: the document's text stays.
        old.updated = 0;
        assert_eq!(merged(&x, &old).text, x.text);
    }

    #[test]
    fn nothing_new_keeps_the_same_document_bytes() {
        let a = with_doc("stable");
        let m = merged(&a, &a.clone());
        assert_eq!(m.crdt, a.crdt);
        assert!(!reconcile(&mut a.clone()));
    }

    #[test]
    fn a_lightened_document_keeps_its_text_and_still_takes_new_edits() {
        let mut b = with_doc("début");
        for i in 0..40 {
            b.text = format!("début {i} {}", "x".repeat(i));
            reconcile(&mut b);
        }
        let full = b.clone();
        let mut light = b.clone();
        assert!(compact(&mut light));
        assert!(light.crdt.as_ref().unwrap().len() < full.crdt.as_ref().unwrap().len());
        assert_eq!(load(&light).get_text(TEXT).to_string(), full.text);
        // Meeting the full copy of the same state: the lighter one is kept, either way round.
        assert_eq!(merged(&full, &light).crdt, light.crdt);
        assert_eq!(merged(&light, &full).crdt, light.crdt);
        // Edits after lightening still merge character by character.
        let (mut ana, mut bob) = (light.clone(), light.clone());
        ana.text = format!("A {}", light.text);
        bob.text = format!("{} B", light.text);
        assert_eq!(merged(&ana, &bob).text, format!("A {} B", light.text));
        assert_eq!(
            merged(&full_with(&full, "Z "), &bob).text.starts_with('Z'),
            true
        );
    }

    fn full_with(b: &Block, prefix: &str) -> Block {
        let mut x = b.clone();
        x.text = format!("{prefix}{}", b.text);
        x
    }

    #[test]
    fn an_old_offline_edit_against_a_lightened_document_falls_back_to_the_latest() {
        let base = with_doc("un deux");
        // Bob went offline from `base`; meanwhile Ana edited, then the server lightened.
        let mut ana = base.clone();
        ana.text = "un deux trois".into();
        ana.updated = base.updated + 5;
        reconcile(&mut ana);
        let mut server = ana.clone();
        compact(&mut server);
        let mut bob = base.clone();
        bob.text = "zéro un deux".into();
        bob.updated = base.updated + 9;
        let m = merged(&server, &bob);
        // Either both edits (if it still merges) or Bob's, edited last: never doubled text.
        assert!(
            m.text == "zéro un deux trois" || m.text == "zéro un deux",
            "{}",
            m.text
        );
    }

    #[test]
    fn blocks_without_documents_keep_the_old_rule() {
        let mut a = Block::new(BlockKind::Paragraph, "ancien");
        a.updated = 1;
        let mut b = a.clone();
        b.text = "récent".into();
        b.updated = 2;
        let m = merged(&a, &b);
        assert_eq!(m.text, "récent");
        assert!(m.crdt.is_none());
    }
}

// ------------------------------------------------------------------ order of a page's blocks

const ORDER: &str = "o";

/// The block ids of an order document, each once (in the rare case of the same block
/// inserted on two sides, the first place is kept).
fn ids_of(doc: &LoroDoc) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    doc.get_movable_list(ORDER)
        .to_vec()
        .into_iter()
        .filter_map(|v| match v {
            loro::LoroValue::String(s) => Some(s.to_string()),
            _ => None,
        })
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

fn order_peer(page: &str, doc: &LoroDoc, target: &[String]) -> u64 {
    peer_for(&format!("ordre:{page}"), doc, &target.join("\u{0}"))
}

fn first_order(page: &crate::model::Page) -> LoroDoc {
    let doc = LoroDoc::new();
    let ids: Vec<String> = page.blocks.iter().map(|b| b.id.clone()).collect();
    let _ = doc.set_peer_id(order_peer(&page.id, &doc, &ids));
    let list = doc.get_movable_list(ORDER);
    for id in &ids {
        let _ = list.push(id.as_str());
    }
    doc.commit();
    doc
}

fn load_order(page: &crate::model::Page) -> LoroDoc {
    page.order_crdt
        .as_deref()
        .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
        .and_then(|bytes| {
            let d = LoroDoc::new();
            d.import(&bytes).ok().map(|_| d)
        })
        .unwrap_or_else(|| first_order(page))
}

/// Bring the order of the page's blocks into its order document (making one if it has none):
/// moves, insertions and removals as edits of their own. Returns true when it changed.
pub fn reconcile_order(page: &mut crate::model::Page) -> bool {
    let had = page.order_crdt.is_some();
    let doc = load_order(page);
    let target: Vec<String> = page.blocks.iter().map(|b| b.id.clone()).collect();
    if ids_of(&doc) == target && had {
        return false;
    }
    if ids_of(&doc) != target {
        let _ = doc.set_peer_id(order_peer(&page.id, &doc, &target));
        let list = doc.get_movable_list(ORDER);
        let wanted: std::collections::HashSet<&String> = target.iter().collect();
        // Out: blocks no longer here (and second copies of one block).
        let mut seen = std::collections::HashSet::new();
        let mut i = list.len();
        let all = list.to_vec();
        while i > 0 {
            i -= 1;
            let id = match &all[i] {
                loro::LoroValue::String(s) => s.to_string(),
                _ => String::new(),
            };
            let first = all[..i]
                .iter()
                .all(|v| !matches!(v, loro::LoroValue::String(s) if s.to_string() == id));
            if !wanted.contains(&id) || !first || !seen.insert(id) {
                let _ = list.delete(i, 1);
            }
        }
        // As few moves as possible: the longest run of blocks already in the right order stays
        // put (so a move made elsewhere on one of them isn't undone); each other block is
        // moved, or put in, right after the one before it.
        let now = ids_of(&doc);
        let place: std::collections::HashMap<&str, usize> = now
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect();
        let present: Vec<usize> = target
            .iter()
            .filter_map(|id| place.get(id.as_str()).copied())
            .collect();
        let stay: std::collections::HashSet<usize> = longest_rising(&present).into_iter().collect();
        for (i, id) in target.iter().enumerate() {
            if place.get(id.as_str()).is_some_and(|p| stay.contains(p)) {
                continue;
            }
            let cur = ids_of(&doc);
            let to = if i == 0 {
                0
            } else {
                cur.iter()
                    .position(|x| x == &target[i - 1])
                    .map(|p| p + 1)
                    .unwrap_or(0)
            };
            match cur.iter().position(|x| x == id) {
                Some(from) => {
                    let to = if from < to { to - 1 } else { to };
                    if from != to {
                        let _ = list.mov(from, to);
                    }
                }
                None => {
                    let _ = list.insert(to, id.as_str());
                }
            }
        }
        doc.commit();
    }
    match save(&doc) {
        Some(s) => {
            page.order_crdt = Some(s);
            true
        }
        None => false,
    }
}

/// The values of the longest strictly rising run in `v` (not necessarily side by side).
fn longest_rising(v: &[usize]) -> Vec<usize> {
    // Patience sorting, with links back to rebuild the run.
    let mut tails: Vec<usize> = Vec::new(); // index in v of the last value of each length
    let mut prev: Vec<Option<usize>> = vec![None; v.len()];
    for (i, &x) in v.iter().enumerate() {
        let k = tails.partition_point(|&t| v[t] < x);
        if k > 0 {
            prev[i] = Some(tails[k - 1]);
        }
        if k == tails.len() {
            tails.push(i);
        } else {
            tails[k] = i;
        }
    }
    let mut out = Vec::new();
    let mut at = tails.last().copied();
    while let Some(i) = at {
        out.push(v[i]);
        at = prev[i];
    }
    out.reverse();
    out
}

/// The order of two copies of a page joined: (block ids in order, its document). None when
/// neither copy has an order document (the old rule then: the copy edited last).
pub fn merged_order(
    ours: &crate::model::Page,
    theirs: &crate::model::Page,
) -> Option<(Vec<String>, Option<String>)> {
    if ours.order_crdt.is_none() && theirs.order_crdt.is_none() {
        return None;
    }
    // Only one with a document: the order of the copy edited last, written over it.
    if ours.order_crdt.is_none() != theirs.order_crdt.is_none() {
        let (with, without) = if ours.order_crdt.is_some() {
            (ours, theirs)
        } else {
            (theirs, ours)
        };
        let mut x = with.clone();
        if without.updated > with.updated {
            x.blocks = without.blocks.clone();
        }
        reconcile_order(&mut x);
        return Some((ids_of(&load_order(&x)), x.order_crdt));
    }
    let (mut a, mut b) = (ours.clone(), theirs.clone());
    reconcile_order(&mut a);
    reconcile_order(&mut b);
    let doc = load_order(&a);
    let before = doc.oplog_vv();
    let ok = b
        .order_crdt
        .as_deref()
        .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
        .map(|bytes| doc.import(&bytes).is_ok())
        .unwrap_or(false);
    if !ok {
        let keep = if theirs.updated > ours.updated { b } else { a };
        return Some((ids_of(&load_order(&keep)), keep.order_crdt));
    }
    let crdt = if doc.oplog_vv() == before {
        a.order_crdt
    } else {
        save(&doc).or(a.order_crdt)
    };
    Some((ids_of(&doc), crdt))
}

#[cfg(test)]
mod order_tests {
    use super::*;
    use crate::model::{BlockKind, Page};

    fn page(ids: &[&str]) -> Page {
        let mut p = Page::new("P", None);
        p.id = "page".into();
        p.blocks = ids
            .iter()
            .map(|id| {
                let mut b = Block::new(BlockKind::Paragraph, *id);
                b.id = id.to_string();
                b
            })
            .collect();
        reconcile_order(&mut p);
        p
    }

    fn reorder(p: &Page, ids: &[&str]) -> Page {
        let mut q = p.clone();
        q.blocks = ids
            .iter()
            .map(|id| {
                p.blocks
                    .iter()
                    .find(|b| b.id == *id)
                    .cloned()
                    .unwrap_or_else(|| {
                        let mut b = Block::new(BlockKind::Paragraph, *id);
                        b.id = id.to_string();
                        b
                    })
            })
            .collect();
        q
    }

    #[test]
    fn two_people_moving_different_blocks_keep_both_moves() {
        let base = page(&["a", "b", "c", "d"]);
        // Ana moves « d » to the top; Bob moves « a » to the bottom.
        let ana = reorder(&base, &["d", "a", "b", "c"]);
        let bob = reorder(&base, &["b", "c", "d", "a"]);
        let (x, _) = merged_order(&ana, &bob).unwrap();
        let (y, _) = merged_order(&bob, &ana).unwrap();
        assert_eq!(x, y, "the same either way");
        assert_eq!(x, ["d", "b", "c", "a"]);
    }

    #[test]
    fn blocks_added_at_once_on_both_sides_are_all_kept_in_place() {
        let base = page(&["a", "b"]);
        let ana = reorder(&base, &["a", "x", "b"]);
        let bob = reorder(&base, &["a", "b", "y"]);
        let (o, _) = merged_order(&ana, &bob).unwrap();
        assert_eq!(o, ["a", "x", "b", "y"]);
    }

    #[test]
    fn any_reordering_is_reached_exactly() {
        let base = page(&["a", "b", "c", "d", "e"]);
        for target in [
            vec!["e", "d", "c", "b", "a"],
            vec!["b", "a", "d", "c", "e"],
            vec!["c", "x", "a", "e"],
            vec!["y"],
            vec!["a", "b", "c", "d", "e", "z"],
        ] {
            let mut p = reorder(&base, &target);
            reconcile_order(&mut p);
            assert_eq!(ids_of(&load_order(&p)), target);
        }
        assert_eq!(longest_rising(&[3, 0, 1, 2]), [0, 1, 2]);
    }

    #[test]
    fn nothing_new_keeps_the_same_document() {
        let base = page(&["a", "b"]);
        let (_, c) = merged_order(&base, &base.clone()).unwrap();
        assert_eq!(c, base.order_crdt);
        assert!(!reconcile_order(&mut base.clone()));
    }
}
