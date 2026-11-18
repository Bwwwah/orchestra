//! Real-time collaboration with no company server: one computer "hosts" (the app, or the
//! headless `orchestra serveur`), the others "join" it by address. Everyone shares a code;
//! a peer giving the wrong one is turned away.
//!
//! Wire format: one JSON message per line over TCP. The host relays each message to the other
//! peers. Pages are merged block by block on arrival (see `collab::merge_page`), so two people
//! can edit different blocks of one page at the same time; a block edited on both sides keeps
//! the most recent edit.
//!
//! The connection is encrypted with the shared code (see secure.rs): someone on the network
//! sees neither the pages nor the code. To work over the Internet, the host must still be
//! reachable: same private network (Tailscale, ZeroTier, WireGuard…), an open port, or a
//! tunnel of your choice.

use crate::model::{Page, observe_clock};
use crate::secure;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 7878;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum LiveMsg {
    /// First message of a peer.
    Hello { name: String, code: String },
    /// Changed pages and the ids deleted for good.
    Pages {
        pages: Vec<Page>,
        #[serde(default)]
        deleted: BTreeMap<String, u64>,
    },
    /// Who is where.
    /// Who is where: the page (only inside shared projects), its project, and whether an AI
    /// is working on that project on their computer right now.
    Presence {
        name: String,
        page: Option<String>,
        #[serde(default)]
        project: Option<String>,
        #[serde(default)]
        ai: bool,
    },
    /// New messages of the projects' team discussions.
    Chat { msgs: Vec<crate::model::ChatMsg> },
    Bye { name: String },
    /// Local only: the connection is (re)established; send everything.
    Connected,
    /// Local only: the connection dropped (the joiner retries on its own).
    Disconnected { reason: String },
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub hosting: bool,
    pub connected: bool,
    pub address: String,
    pub peers: usize,
    pub error: Option<String>,
}

type Wake = Arc<dyn Fn() + Send + Sync>;

pub struct Live {
    inbox: Receiver<LiveMsg>,
    out: Arc<dyn Fn(&LiveMsg) + Send + Sync>,
    pub status: Arc<Mutex<Status>>,
    stop: Arc<AtomicBool>,
    listener_addr: Option<std::net::SocketAddr>,
}

impl Live {
    pub fn send(&self, msg: &LiveMsg) {
        (self.out)(msg)
    }

    /// Messages received since the last call.
    pub fn drain(&self) -> Vec<LiveMsg> {
        self.inbox.try_iter().collect()
    }

    pub fn status(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// The port actually listened on (useful with port 0).
    pub fn port(&self) -> Option<u16> {
        self.listener_addr.map(|a| a.port())
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so the listening port is released.
        if let Some(a) = self.listener_addr {
            let _ = TcpStream::connect_timeout(&std::net::SocketAddr::new([127, 0, 0, 1].into(), a.port()), Duration::from_millis(200));
        }
    }
}

fn note_clock(msg: &LiveMsg) {
    if let LiveMsg::Pages { pages, .. } = msg {
        for p in pages {
            observe_clock(p.updated);
        }
    }
}

/// This computer's address on the local network, to give to the others.
pub fn local_ip() -> String {
    // Connecting a UDP socket sends nothing; it only picks the outgoing interface.
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("192.168.1.1:9")?;
            s.local_addr()
        })
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".into())
}

struct Client {
    id: u64,
    stream: TcpStream,
    tx: secure::Sender,
}

fn bytes_of(msg: &LiveMsg) -> Vec<u8> {
    serde_json::to_vec(msg).unwrap_or_default()
}

/// Host on `port` (0 picks a free one). Peers must know `code`: it encrypts the connection.
pub fn host(port: u16, code: &str, wake: Wake) -> std::io::Result<Live> {
    let listener = TcpListener::bind(("0.0.0.0", port))?;
    let addr = listener.local_addr()?;
    let (tx, rx) = channel::<LiveMsg>();
    let clients: Arc<Mutex<Vec<Client>>> = Arc::new(Mutex::new(vec![]));
    let stop = Arc::new(AtomicBool::new(false));
    let status = Arc::new(Mutex::new(Status { hosting: true, connected: true, address: format!("{}:{}", local_ip(), addr.port()), ..Default::default() }));
    let next_id = Arc::new(AtomicU64::new(1));
    {
        let (clients, stop, status, code, wake, tx) = (clients.clone(), stop.clone(), status.clone(), code.to_string(), wake.clone(), tx.clone());
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = conn else { continue };
                let _ = stream.set_nodelay(true);
                let id = next_id.fetch_add(1, Ordering::SeqCst);
                let (clients, status, code, wake, tx, stop) = (clients.clone(), status.clone(), code.clone(), wake.clone(), tx.clone(), stop.clone());
                std::thread::spawn(move || serve_peer(id, stream, clients, status, code, wake, tx, stop));
            }
        });
    }
    let out_clients = clients.clone();
    let out: Arc<dyn Fn(&LiveMsg) + Send + Sync> = Arc::new(move |msg: &LiveMsg| {
        let bytes = bytes_of(msg);
        if let Ok(mut list) = out_clients.lock() {
            list.retain_mut(|c| c.tx.send(&mut c.stream, &bytes).is_ok());
        }
    });
    Ok(Live { inbox: rx, out, status, stop, listener_addr: Some(addr) })
}

#[allow(clippy::too_many_arguments)]
fn serve_peer(
    id: u64,
    mut stream: TcpStream,
    clients: Arc<Mutex<Vec<Client>>>,
    status: Arc<Mutex<Status>>,
    code: String,
    wake: Wake,
    tx: Sender<LiveMsg>,
    stop: Arc<AtomicBool>,
) {
    // Key exchange, then the first message must be a Hello that opens with that key.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let Ok((mut sealer, mut opener)) = secure::handshake(&mut stream, &code, true) else {
        let _ = stream.shutdown(Shutdown::Both);
        return;
    };
    let Ok(mut reader) = stream.try_clone() else { return };
    let name = match opener.recv(&mut reader).ok().and_then(|b| serde_json::from_slice::<LiveMsg>(&b).ok()) {
        Some(LiveMsg::Hello { name, .. }) => name,
        _ => {
            // Sealed with our key: a peer with another code can't open it, and knows why.
            let _ = sealer.send(&mut stream, &bytes_of(&LiveMsg::Disconnected { reason: crate::tr!("code incorrect", "wrong code").into() }));
            let _ = stream.shutdown(Shutdown::Both);
            return;
        }
    };
    let _ = stream.set_read_timeout(None);
    if let Ok(mut list) = clients.lock() {
        if let Ok(s) = stream.try_clone() {
            list.push(Client { id, stream: s, tx: sealer });
        }
        if let Ok(mut st) = status.lock() {
            st.peers = list.len();
        }
    }
    // Each peer has its own keys: what one sends is opened here and sealed again for each other.
    let relay = |bytes: &[u8], clients: &Arc<Mutex<Vec<Client>>>| {
        if let Ok(mut list) = clients.lock() {
            list.retain_mut(|c| c.id == id || c.tx.send(&mut c.stream, bytes).is_ok());
        }
    };
    let hello = LiveMsg::Hello { name: name.clone(), code: String::new() };
    relay(&bytes_of(&hello), &clients);
    let _ = tx.send(hello);
    wake();
    while !stop.load(Ordering::SeqCst) {
        let Ok(bytes) = opener.recv(&mut reader) else { break };
        let Ok(msg) = serde_json::from_slice::<LiveMsg>(&bytes) else { continue };
        if matches!(msg, LiveMsg::Connected | LiveMsg::Disconnected { .. } | LiveMsg::Hello { .. }) {
            continue;
        }
        note_clock(&msg);
        relay(&bytes, &clients);
        let _ = tx.send(msg);
        wake();
    }
    if let Ok(mut list) = clients.lock() {
        list.retain(|c| c.id != id);
        if let Ok(mut st) = status.lock() {
            st.peers = list.len();
        }
    }
    let bye = LiveMsg::Bye { name };
    relay(&bytes_of(&bye), &clients);
    let _ = tx.send(bye);
    wake();
}

/// Join the host at `address` ("192.168.1.20:7878" or "nom-de-machine:7878"). Retries every
/// 3 seconds when the connection drops.
pub fn join(address: &str, name: &str, code: &str, wake: Wake) -> Live {
    let (tx, rx) = channel::<LiveMsg>();
    let stop = Arc::new(AtomicBool::new(false));
    let status = Arc::new(Mutex::new(Status { address: address.to_string(), ..Default::default() }));
    let writer: Arc<Mutex<Option<(TcpStream, secure::Sender)>>> = Arc::new(Mutex::new(None));
    {
        let (stop, status, writer, address, name, code) = (stop.clone(), status.clone(), writer.clone(), address.to_string(), name.to_string(), code.to_string());
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let target = with_port(&address);
                let conn = target
                    .to_socket_addrs()
                    .ok()
                    .and_then(|mut a| a.next())
                    .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, crate::tr!("adresse introuvable", "address not found")))
                    .and_then(|a| TcpStream::connect_timeout(&a, Duration::from_secs(5)))
                    .and_then(|mut stream| {
                        let _ = stream.set_nodelay(true);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                        let keys = secure::handshake(&mut stream, &code, false)?;
                        let _ = stream.set_read_timeout(None);
                        Ok((stream, keys))
                    });
                match conn {
                    Ok((stream, (mut sealer, mut opener))) => {
                        let hello = LiveMsg::Hello { name: name.clone(), code: String::new() };
                        let Ok(mut w) = stream.try_clone() else { continue };
                        if sealer.send(&mut w, &bytes_of(&hello)).is_err() {
                            continue;
                        }
                        *writer.lock().unwrap() = Some((w, sealer));
                        if let Ok(mut st) = status.lock() {
                            st.connected = true;
                            st.error = None;
                        }
                        let _ = tx.send(LiveMsg::Connected);
                        wake();
                        let mut reader = stream;
                        let mut reason = crate::tr!("connexion perdue", "connection lost").to_string();
                        while !stop.load(Ordering::SeqCst) {
                            match opener.recv(&mut reader) {
                                Err(secure::RecvError::Closed) => break,
                                Err(secure::RecvError::Refused) => {
                                    reason = crate::tr!("code incorrect", "wrong code").into();
                                    break;
                                }
                                Ok(bytes) => {
                                    let Ok(msg) = serde_json::from_slice::<LiveMsg>(&bytes) else { continue };
                                    if let LiveMsg::Disconnected { reason: r } = &msg {
                                        reason = r.clone();
                                        break;
                                    }
                                    note_clock(&msg);
                                    let _ = tx.send(msg);
                                    wake();
                                }
                            }
                        }
                        *writer.lock().unwrap() = None;
                        if let Ok(mut st) = status.lock() {
                            st.connected = false;
                            st.error = Some(reason.clone());
                        }
                        let _ = tx.send(LiveMsg::Disconnected { reason });
                        wake();
                    }
                    Err(e) => {
                        if let Ok(mut st) = status.lock() {
                            st.connected = false;
                            st.error = Some(crate::trf!("connexion impossible : {e}", "connection failed: {e}"));
                        }
                        wake();
                    }
                }
                for _ in 0..30 {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            if let Some((s, _)) = writer.lock().unwrap().take() {
                let _ = s.shutdown(Shutdown::Both);
            }
        });
    }
    let out_writer = writer.clone();
    let out: Arc<dyn Fn(&LiveMsg) + Send + Sync> = Arc::new(move |msg: &LiveMsg| {
        let bytes = bytes_of(msg);
        let mut w = out_writer.lock().unwrap();
        if let Some((s, sealer)) = w.as_mut() {
            if sealer.send(s, &bytes).is_err() {
                *w = None;
            }
        }
    });
    Live { inbox: rx, out, status, stop, listener_addr: None }
}

/// "machine" → "machine:7878".
pub fn with_port(address: &str) -> String {
    let a = address.trim();
    if a.rsplit_once(':').map(|(_, p)| p.parse::<u16>().is_ok()).unwrap_or(false) { a.to_string() } else { format!("{a}:{DEFAULT_PORT}") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Page;
    use std::time::Instant;

    fn wait_for(live: &Live, pred: impl Fn(&LiveMsg) -> bool) -> Option<LiveMsg> {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            for m in live.drain() {
                if pred(&m) {
                    return Some(m);
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }

    #[test]
    fn pages_travel_both_ways_and_wrong_codes_are_refused() {
        let wake: Wake = Arc::new(|| {});
        let h = host(0, "secret", wake.clone()).unwrap();
        let addr = format!("127.0.0.1:{}", h.port().unwrap());
        let j = join(&addr, "Camille", "secret", wake.clone());
        assert!(wait_for(&j, |m| matches!(m, LiveMsg::Connected)).is_some());
        assert!(wait_for(&h, |m| matches!(m, LiveMsg::Hello { name, .. } if name == "Camille")).is_some());
        // Joiner → host.
        let p = Page::new("Depuis Camille", None);
        j.send(&LiveMsg::Pages { pages: vec![p.clone()], deleted: Default::default() });
        let got = wait_for(&h, |m| matches!(m, LiveMsg::Pages { .. })).expect("host receives pages");
        assert!(matches!(got, LiveMsg::Pages { pages, .. } if pages[0].title == "Depuis Camille"));
        // Host → joiner.
        h.send(&LiveMsg::Presence { name: "Hôte".into(), page: Some(p.id.clone()), project: None, ai: false });
        assert!(wait_for(&j, |m| matches!(m, LiveMsg::Presence { .. })).is_some());
        // A peer with the wrong code is turned away.
        let bad = join(&addr, "Intrus", "faux", wake);
        assert!(wait_for(&bad, |m| matches!(m, LiveMsg::Disconnected { reason } if reason.contains("code"))).is_some());
        assert_eq!(h.status().peers, 1);
    }
}
