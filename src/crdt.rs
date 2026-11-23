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
    let doc = load(&a);
    let before = doc.oplog_vv();
    if let Some(bytes) = b
        .crdt
        .as_deref()
        .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
    {
        let _ = doc.import(&bytes);
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
