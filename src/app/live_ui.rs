//! Real-time collaboration in the window: start or stop the connection, merge what others send,
//! send what changed here (pages edited in the window, by the AI through MCP or by an import),
//! and show who is on which page.

use super::*;
use crate::live::{self, LiveMsg};
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct Peer {
    pub page: Option<String>,
    pub seen: Instant,
    /// The shared project they are in, and whether an AI works on it at their place.
    pub project: Option<String>,
    pub ai: bool,
}

/// A stable colour per name, from the tag palette.
pub fn peer_color(name: &str) -> Color32 {
    const COLORS: [[u8; 3]; 8] = [[35, 131, 226], [221, 0, 129], [217, 115, 13], [68, 131, 97], [144, 101, 176], [212, 76, 71], [203, 145, 47], [51, 126, 169]];
    let h = name.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let c = COLORS[(h as usize) % COLORS.len()];
    Color32::from_rgb(c[0], c[1], c[2])
}

impl NotesApp {
    pub(super) fn my_name(&self) -> String {
        let a = self.ws.settings.collab.author.trim();
        if !a.is_empty() {
            return a.to_string();
        }
        std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "Moi".into())
    }

    pub(super) fn start_live(&mut self) {
        self.stop_live();
        if self.ws.settings.collab.live_code.trim().is_empty() {
            self.ws.settings.collab.live_code = crate::model::new_id()[..8].to_string();
        }
        let code = self.ws.settings.collab.live_code.trim().to_string();
        let ctx = self.ctx.clone();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || ctx.request_repaint());
        let c = &self.ws.settings.collab;
        if c.live_host {
            let port = if c.live_port == 0 { live::DEFAULT_PORT } else { c.live_port };
            match live::host(port, &code, wake) {
                Ok(l) => {
                    self.info(crate::trf!("En direct : hébergé à l'adresse {}", "Live: hosted at {}", l.status().address));
                    self.live = Some(l);
                }
                Err(e) => {
                    self.fail(crate::trf!("Impossible d'héberger sur le port {port} : {e}", "Couldn't host on port {port}: {e}"));
                    return;
                }
            }
        } else {
            if c.live_address.trim().is_empty() {
                self.fail(crate::tr!("Indiquez l'adresse de l'hôte.", "Enter the host's address."));
                return;
            }
            let name = self.my_name();
            self.live = Some(live::join(&c.live_address, &name, &code, wake));
        }
        self.ws.settings.collab.live_on = true;
        self.live_sent.clear();
        self.peers.clear();
    }

    pub(super) fn stop_live(&mut self) {
        if let Some(l) = self.live.take() {
            l.send(&LiveMsg::Bye { name: self.my_name() });
        }
        self.ws.settings.collab.live_on = false;
        self.peers.clear();
    }

    pub(super) fn send_everything(&mut self) {
        let Some(l) = &self.live else { return };
        // Only the shared projects leave this computer.
        let out = self.ws.shared_copy();
        l.send(&LiveMsg::Pages { pages: out.pages.clone(), deleted: out.deleted.clone() });
        l.send(&LiveMsg::Chat { msgs: out.chat.values().cloned().collect() });
        for p in &out.pages {
            self.live_sent.insert(p.id.clone(), p.updated);
        }
        self.chat_sent.extend(out.chat.keys().cloned());
        self.live_presence = None;
    }

    /// Once per frame: merge what arrived, send what changed, keep presence fresh.
    pub(super) fn live_tick(&mut self) {
        let Some(l) = &self.live else { return };
        let msgs = l.drain();
        let mut merged_any = false;
        for msg in msgs {
            match msg {
                LiveMsg::Connected => self.send_everything(),
                LiveMsg::Hello { name, .. } => {
                    // Someone joined (we host): give them everything.
                    self.peers.insert(name, live_ui::Peer { page: None, seen: Instant::now(), project: None, ai: false });
                    self.send_everything();
                }
                LiveMsg::Presence { name, page, project, ai } => {
                    self.peers.insert(name, live_ui::Peer { page, seen: Instant::now(), project, ai });
                }
                LiveMsg::Bye { name } => {
                    self.peers.remove(&name);
                }
                LiveMsg::Disconnected { reason } => {
                    self.peers.clear();
                    self.ack(crate::trf!("En direct : {reason}, nouvelle tentative…", "Live: {reason}, trying again…"));
                }
                LiveMsg::Pages { pages, deleted } => {
                    merged_any |= self.merge_remote(pages, deleted);
                }
                LiveMsg::Chat { msgs } => {
                    // Not echoed back; only those of projects shared here come in.
                    self.chat_sent.extend(msgs.iter().map(|m| m.id.clone()));
                    if self.ws.accept_chat(msgs) > 0 {
                        merged_any = true;
                    }
                }
            }
        }
        if merged_any {
            // Remote edits are not ours to stamp or echo: refresh the snapshot, save to disk.
            self.take_snapshot();
            self.dirty_since.get_or_insert_with(Instant::now);
        }
        // Send pages whose last edit hasn't gone out yet (window, AI through MCP, imports).
        let shared = self.ws.shared_ids();
        let changed: Vec<Page> = self
            .ws
            .pages
            .iter()
            .filter(|p| shared.contains(&p.id) && self.live_sent.get(&p.id).map(|t| p.updated > *t).unwrap_or(true))
            .map(crate::project::shareable)
            .collect();
        let new_msgs: Vec<crate::model::ChatMsg> =
            self.ws.chat.values().filter(|m| shared.contains(&m.project) && !self.chat_sent.contains(&m.id)).cloned().collect();
        if !new_msgs.is_empty() {
            if let Some(l) = &self.live {
                l.send(&LiveMsg::Chat { msgs: new_msgs.clone() });
            }
            self.chat_sent.extend(new_msgs.into_iter().map(|m| m.id));
        }
        let deleted_now = self.ws.deleted.len();
        if !changed.is_empty() || deleted_now != self.live_deleted_sent {
            if let Some(l) = &self.live {
                l.send(&LiveMsg::Pages { pages: changed.clone(), deleted: self.ws.deleted.clone() });
            }
            for p in changed {
                self.live_sent.insert(p.id, p.updated);
            }
            self.live_deleted_sent = deleted_now;
        }
        // Presence: when the open page changes, and every 5 s. Only pages of shared projects
        // are named: a private page shows as "elsewhere".
        let here = if self.view == View::Page { self.current.clone().filter(|p| shared.contains(p)) } else { None };
        let due = match &self.live_presence {
            Some((page, at)) => *page != here || at.elapsed() > Duration::from_secs(5),
            None => true,
        };
        if due {
            let project = here.as_deref().and_then(|p| self.ws.project_of(p)).map(|p| p.id.clone());
            let now = crate::model::now_ms();
            let ai = project.as_ref().and_then(|p| self.agents.by_project.get(p)).is_some_and(|v| v.iter().any(|s| s.is_live(now)));
            if let Some(l) = &self.live {
                l.send(&LiveMsg::Presence { name: self.my_name(), page: here.clone(), project, ai });
            }
            self.live_presence = Some((here, Instant::now()));
        }
        self.peers.retain(|_, p| p.seen.elapsed() < Duration::from_secs(16));
        self.ctx.request_repaint_after(Duration::from_millis(250));
    }

    /// Merge pages from another person. Returns true when something changed here.
    fn merge_remote(&mut self, pages: Vec<Page>, deleted: BTreeMap<String, u64>) -> bool {
        let mut changed = false;
        // Only pages of shared projects come in: never one of our private pages, nor a page
        // slipped under one.
        let pages = self.ws.accept_remote(pages);
        // Deletions too: only pages of shared projects, even if they know another id.
        let deleted = self.ws.accept_remote_deletions(deleted);
        for (id, t) in deleted {
            let e = self.ws.deleted.entry(id).or_insert(t);
            *e = (*e).max(t);
        }
        let gone = self.ws.deleted.clone();
        let before = self.ws.pages.len();
        self.ws.pages.retain(|p| gone.get(&p.id).map(|t| p.updated > *t).unwrap_or(true));
        changed |= before != self.ws.pages.len();
        self.live_deleted_sent = self.ws.deleted.len();
        for incoming in pages {
            if gone.get(&incoming.id).map(|t| incoming.updated <= *t).unwrap_or(false) {
                continue;
            }
            let incoming_json = serde_json::to_string(&incoming).unwrap_or_default();
            match self.ws.pages.iter_mut().find(|p| p.id == incoming.id) {
                Some(ours) => {
                    if collab::merge_page(ours, &incoming) {
                        changed = true;
                    }
                    // Only echo back if our merge kept something they don't have.
                    if serde_json::to_string(&*ours).unwrap_or_default() == incoming_json {
                        self.live_sent.insert(ours.id.clone(), ours.updated);
                    } else {
                        self.live_sent.remove(&ours.id);
                    }
                }
                None => {
                    self.live_sent.insert(incoming.id.clone(), incoming.updated);
                    self.ws.pages.push(collab::received_new(incoming));
                    changed = true;
                }
            }
        }
        if let Some(c) = &self.current
            && self.ws.page(c).is_none() {
                self.current = None;
            }
        changed
    }

    /// The people in a shared project right now, and whether their AI is at work.
    pub(super) fn peers_in_project(&self, project: &str) -> Vec<(String, bool)> {
        let mut v: Vec<(String, bool)> = self.peers.iter().filter(|(_, p)| p.project.as_deref() == Some(project)).map(|(n, p)| (n.clone(), p.ai)).collect();
        v.sort();
        v
    }

    /// Small round faces of the people looking at `page`, for the page bar.
    pub(super) fn peers_on(&self, page: &str) -> Vec<String> {
        self.peers.iter().filter(|(_, p)| p.page.as_deref() == Some(page)).map(|(n, _)| n.clone()).collect()
    }
}

/// Draw overlapping round faces with initials (newest on top).
pub fn faces(ui: &mut egui::Ui, names: &[String], bg: Color32) {
    let n = names.len().min(5);
    if n == 0 {
        return;
    }
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0 + (n as f32 - 1.0) * 16.0, 28.0), egui::Sense::hover());
    for (i, name) in names.iter().take(n).enumerate() {
        let c = egui::pos2(rect.left() + 12.0 + i as f32 * 16.0, rect.center().y);
        ui.painter().circle_filled(c, 12.0, bg);
        ui.painter().circle_filled(c, 10.5, peer_color(name));
        let initial: String = name.chars().next().map(|ch| ch.to_uppercase().collect()).unwrap_or_default();
        ui.painter().text(c, egui::Align2::CENTER_CENTER, initial, semibold(11.0), Color32::WHITE);
    }
    resp.on_hover_text(names.join(", "));
}
