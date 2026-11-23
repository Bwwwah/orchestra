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

use crate::LockOrRecover;
use crate::model::{Page, observe_clock};
use crate::secure;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const DEFAULT_PORT: u16 = 7878;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum LiveMsg {
    /// First message of a peer.
    Hello {
        name: String,
        code: String,
    },
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
    Chat {
        msgs: Vec<crate::model::ChatMsg>,
    },
    Bye {
        name: String,
    },
    /// Local only: the connection is (re)established; send everything.
    Connected,
    /// Local only: the connection dropped (the joiner retries on its own).
    Disconnected {
        reason: String,
    },
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub hosting: bool,
    pub connected: bool,
    pub address: String,
    pub peers: usize,
    pub error: Option<String>,
}

pub type Wake = Arc<dyn Fn() + Send + Sync>;

pub struct Live {
    inbox: Receiver<LiveMsg>,
    out: Arc<dyn Fn(&LiveMsg) + Send + Sync>,
    pub status: Arc<Mutex<Status>>,
    stop: Arc<AtomicBool>,
    listener_addr: Option<std::net::SocketAddr>,
}

impl Live {
    /// A connection made elsewhere (the team server's client, team.rs) behind the same face.
    pub fn from_parts(
        inbox: Receiver<LiveMsg>,
        out: Arc<dyn Fn(&LiveMsg) + Send + Sync>,
        status: Arc<Mutex<Status>>,
        stop: Arc<AtomicBool>,
    ) -> Live {
        Live {
            inbox,
            out,
            status,
            stop,
            listener_addr: None,
        }
    }

    pub fn send(&self, msg: &LiveMsg) {
        (self.out)(msg)
    }

    /// Messages received since the last call.
    pub fn drain(&self) -> Vec<LiveMsg> {
        self.inbox.try_iter().collect()
    }

    /// Stopped for good (a team server that refused us): no new attempt.
    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
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
            let _ = TcpStream::connect_timeout(
                &std::net::SocketAddr::new([127, 0, 0, 1].into(), a.port()),
                Duration::from_millis(200),
            );
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

/// How long writing one message to one peer may take (plus `PEER_MIN_RATE` for a big one).
/// The host writes to every peer in turn, holding the list: a peer that stops reading
/// (asleep, on a dead network) would otherwise block the sending to all of them, and the
/// app with it.
const PEER_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// Bytes per second a peer must at least take, so a big page still reaches a slow link.
const PEER_MIN_RATE: u64 = 64 * 1024;

/// A peer's stream with a deadline for the whole message. The socket's own write timeout
/// starts again at every call that moves a few bytes, so alone it bounds nothing.
struct Deadline<'a> {
    stream: &'a TcpStream,
    until: Instant,
}

impl<'a> Deadline<'a> {
    fn new(stream: &'a TcpStream, len: usize) -> Self {
        Deadline {
            stream,
            until: Instant::now()
                + PEER_WRITE_TIMEOUT
                + Duration::from_secs(len as u64 / PEER_MIN_RATE),
        }
    }
}

impl std::io::Write for Deadline<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Small pieces with a short timeout each, the deadline checked in between: a long
        // timeout on a big write is not kept to (macOS overshot 13 s by 5 s on a partial write).
        let piece = &buf[..buf.len().min(64 * 1024)];
        let mut s = self.stream;
        loop {
            let left = self.until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(std::io::ErrorKind::TimedOut.into());
            }
            self.stream
                .set_write_timeout(Some(left.min(Duration::from_secs(1))))?;
            match s.write(piece) {
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                r => return r,
            }
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut s = self.stream;
        s.flush()
    }
}

/// Send `bytes` to every peer but `skip`. A peer whose write fails or times out is dropped
/// and its connection closed: half a message may have left, the stream can't be used again.
/// Closing it also ends its reading thread, which tells the others it left.
fn send_to_all(clients: &Mutex<Vec<Client>>, bytes: &[u8], skip: Option<u64>) {
    if let Ok(mut list) = clients.lock() {
        list.retain_mut(|c| {
            if Some(c.id) == skip
                || c.tx
                    .send(&mut Deadline::new(&c.stream, bytes.len()), bytes)
                    .is_ok()
            {
                return true;
            }
            let _ = c.stream.shutdown(Shutdown::Both);
            false
        });
    }
}

/// Connections the host keeps at once, those still in their handshake included.
const MAX_CONNECTIONS: usize = 32;
/// Failed handshakes (wrong code, garbage, silence) an address may make before being shut out.
const MAX_FAILURES: u32 = 5;
/// How long failures are remembered, and how long an address that made too many stays shut out.
const FAILURE_WINDOW: Duration = Duration::from_secs(10 * 60);

/// Who may connect to the host: slows down guessing the code and caps the threads a stranger
/// on the network can make it open.
#[derive(Default)]
struct Guard {
    active: usize,
    /// Per address: failures in the current window, and when that window started.
    failures: HashMap<IpAddr, (u32, Instant)>,
}

impl Guard {
    /// Take a connection slot for `ip`, or refuse it.
    fn admit(&mut self, ip: IpAddr, now: Instant) -> bool {
        self.failures
            .retain(|_, (_, since)| now.duration_since(*since) < FAILURE_WINDOW);
        if self
            .failures
            .get(&ip)
            .is_some_and(|(n, _)| *n >= MAX_FAILURES)
            || self.active >= MAX_CONNECTIONS
        {
            return false;
        }
        self.active += 1;
        true
    }

    fn release(&mut self) {
        self.active = self.active.saturating_sub(1);
    }

    fn failed(&mut self, ip: IpAddr, now: Instant) {
        let entry = self.failures.entry(ip).or_insert((0, now));
        entry.0 += 1;
        if entry.0 >= MAX_FAILURES {
            // Shut out for a full window from the last failure.
            entry.1 = now;
        }
    }

    fn succeeded(&mut self, ip: IpAddr) {
        self.failures.remove(&ip);
    }
}

/// Gives back the connection slot when the peer's thread ends, however it ends.
struct Slot(Arc<Mutex<Guard>>);

impl Drop for Slot {
    fn drop(&mut self) {
        if let Ok(mut g) = self.0.lock() {
            g.release();
        }
    }
}

/// Host on `port` (0 picks a free one). Peers must know `code`: it encrypts the connection.
pub fn host(port: u16, code: &str, wake: Wake) -> std::io::Result<Live> {
    let listener = TcpListener::bind(("0.0.0.0", port))?;
    let addr = listener.local_addr()?;
    let (tx, rx) = channel::<LiveMsg>();
    let clients: Arc<Mutex<Vec<Client>>> = Arc::new(Mutex::new(vec![]));
    let stop = Arc::new(AtomicBool::new(false));
    let status = Arc::new(Mutex::new(Status {
        hosting: true,
        connected: true,
        address: format!("{}:{}", local_ip(), addr.port()),
        ..Default::default()
    }));
    let next_id = Arc::new(AtomicU64::new(1));
    let guard = Arc::new(Mutex::new(Guard::default()));
    {
        let (clients, stop, status, code, wake, tx) = (
            clients.clone(),
            stop.clone(),
            status.clone(),
            code.to_string(),
            wake.clone(),
            tx.clone(),
        );
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = conn else { continue };
                let Ok(ip) = stream.peer_addr().map(|a| a.ip()) else {
                    continue;
                };
                if !guard.lock().is_ok_and(|mut g| g.admit(ip, Instant::now())) {
                    let _ = stream.shutdown(Shutdown::Both);
                    continue;
                }
                let slot = Slot(guard.clone());
                let _ = stream.set_nodelay(true);
                let id = next_id.fetch_add(1, Ordering::SeqCst);
                let (clients, status, code, wake, tx, stop) = (
                    clients.clone(),
                    status.clone(),
                    code.clone(),
                    wake.clone(),
                    tx.clone(),
                    stop.clone(),
                );
                std::thread::spawn(move || {
                    serve_peer(id, ip, stream, slot, clients, status, code, wake, tx, stop)
                });
            }
        });
    }
    let out_clients = clients.clone();
    let out: Arc<dyn Fn(&LiveMsg) + Send + Sync> =
        Arc::new(move |msg: &LiveMsg| send_to_all(&out_clients, &bytes_of(msg), None));
    Ok(Live {
        inbox: rx,
        out,
        status,
        stop,
        listener_addr: Some(addr),
    })
}

#[allow(clippy::too_many_arguments)]
fn serve_peer(
    id: u64,
    ip: IpAddr,
    mut stream: TcpStream,
    slot: Slot,
    clients: Arc<Mutex<Vec<Client>>>,
    status: Arc<Mutex<Status>>,
    code: String,
    wake: Wake,
    tx: Sender<LiveMsg>,
    stop: Arc<AtomicBool>,
) {
    let fail = |stream: &TcpStream| {
        if let Ok(mut g) = slot.0.lock() {
            g.failed(ip, Instant::now());
        }
        let _ = stream.shutdown(Shutdown::Both);
    };
    // Key exchange, then the first message must be a Hello that opens with that key.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    // Until it joins; then each message gets its own deadline (see `send_to_all`).
    let _ = stream.set_write_timeout(Some(PEER_WRITE_TIMEOUT));
    let Ok((mut sealer, mut opener)) = secure::handshake(&mut stream, &code, true) else {
        fail(&stream);
        return;
    };
    let Ok(mut reader) = stream.try_clone() else {
        return;
    };
    let name = match opener
        .recv(&mut reader)
        .ok()
        .and_then(|b| serde_json::from_slice::<LiveMsg>(&b).ok())
    {
        Some(LiveMsg::Hello { name, .. }) => name,
        _ => {
            // Sealed with our key: a peer with another code can't open it, and knows why.
            let _ = sealer.send(
                &mut stream,
                &bytes_of(&LiveMsg::Disconnected {
                    reason: crate::tr!("code incorrect", "wrong code").into(),
                }),
            );
            fail(&stream);
            return;
        }
    };
    if let Ok(mut g) = slot.0.lock() {
        g.succeeded(ip);
    }
    let _ = stream.set_read_timeout(None);
    if let Ok(mut list) = clients.lock() {
        if let Ok(s) = stream.try_clone() {
            list.push(Client {
                id,
                stream: s,
                tx: sealer,
            });
        }
        if let Ok(mut st) = status.lock() {
            st.peers = list.len();
        }
    }
    // Each peer has its own keys: what one sends is opened here and sealed again for each other.
    let relay =
        |bytes: &[u8], clients: &Arc<Mutex<Vec<Client>>>| send_to_all(clients, bytes, Some(id));
    let hello = LiveMsg::Hello {
        name: name.clone(),
        code: String::new(),
    };
    relay(&bytes_of(&hello), &clients);
    let _ = tx.send(hello);
    wake();
    while !stop.load(Ordering::SeqCst) {
        let Ok(bytes) = opener.recv(&mut reader) else {
            break;
        };
        let Ok(msg) = serde_json::from_slice::<LiveMsg>(&bytes) else {
            continue;
        };
        if matches!(
            msg,
            LiveMsg::Connected | LiveMsg::Disconnected { .. } | LiveMsg::Hello { .. }
        ) {
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
    let status = Arc::new(Mutex::new(Status {
        address: address.to_string(),
        ..Default::default()
    }));
    let writer: Arc<Mutex<Option<(TcpStream, secure::Sender)>>> = Arc::new(Mutex::new(None));
    {
        let (stop, status, writer, address, name, code) = (
            stop.clone(),
            status.clone(),
            writer.clone(),
            address.to_string(),
            name.to_string(),
            code.to_string(),
        );
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let target = with_port(&address);
                let conn = target
                    .to_socket_addrs()
                    .ok()
                    .and_then(|mut a| a.next())
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            crate::tr!("adresse introuvable", "address not found"),
                        )
                    })
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
                        let hello = LiveMsg::Hello {
                            name: name.clone(),
                            code: String::new(),
                        };
                        let Ok(mut w) = stream.try_clone() else {
                            continue;
                        };
                        if sealer.send(&mut w, &bytes_of(&hello)).is_err() {
                            continue;
                        }
                        *writer.lock_or_recover() = Some((w, sealer));
                        if let Ok(mut st) = status.lock() {
                            st.connected = true;
                            st.error = None;
                        }
                        let _ = tx.send(LiveMsg::Connected);
                        wake();
                        let mut reader = stream;
                        let mut reason =
                            crate::tr!("connexion perdue", "connection lost").to_string();
                        while !stop.load(Ordering::SeqCst) {
                            match opener.recv(&mut reader) {
                                Err(secure::RecvError::Closed) => break,
                                Err(secure::RecvError::Refused) => {
                                    reason = crate::tr!("code incorrect", "wrong code").into();
                                    break;
                                }
                                Ok(bytes) => {
                                    let Ok(msg) = serde_json::from_slice::<LiveMsg>(&bytes) else {
                                        continue;
                                    };
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
                        *writer.lock_or_recover() = None;
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
                            st.error = Some(crate::trf!(
                                "connexion impossible : {e}",
                                "connection failed: {e}"
                            ));
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
            if let Some((s, _)) = writer.lock_or_recover().take() {
                let _ = s.shutdown(Shutdown::Both);
            }
        });
    }
    let out_writer = writer.clone();
    let out: Arc<dyn Fn(&LiveMsg) + Send + Sync> = Arc::new(move |msg: &LiveMsg| {
        let bytes = bytes_of(msg);
        let mut w = out_writer.lock_or_recover();
        if let Some((s, sealer)) = w.as_mut()
            && sealer.send(s, &bytes).is_err()
        {
            *w = None;
        }
    });
    Live {
        inbox: rx,
        out,
        status,
        stop,
        listener_addr: None,
    }
}

/// "machine" → "machine:7878".
pub fn with_port(address: &str) -> String {
    let a = address.trim();
    if a.rsplit_once(':')
        .map(|(_, p)| p.parse::<u16>().is_ok())
        .unwrap_or(false)
    {
        a.to_string()
    } else {
        format!("{a}:{DEFAULT_PORT}")
    }
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
    fn guard_shuts_out_an_address_after_too_many_failures_then_forgives() {
        let mut g = Guard::default();
        let (bad, good): (IpAddr, IpAddr) = ([10, 0, 0, 1].into(), [10, 0, 0, 2].into());
        let t0 = Instant::now();
        for _ in 0..MAX_FAILURES {
            assert!(g.admit(bad, t0));
            g.failed(bad, t0);
            g.release();
        }
        assert!(!g.admit(bad, t0), "too many failures: shut out");
        assert!(g.admit(good, t0), "another address is not affected");
        g.release();
        assert!(
            g.admit(bad, t0 + FAILURE_WINDOW),
            "forgiven after the window"
        );
        g.release();
        // A success wipes the failures of that address.
        g.failed(good, t0);
        g.succeeded(good);
        assert!(!g.failures.contains_key(&good));
    }

    #[test]
    fn guard_caps_simultaneous_connections() {
        let mut g = Guard::default();
        let ip: IpAddr = [10, 0, 0, 3].into();
        let now = Instant::now();
        for _ in 0..MAX_CONNECTIONS {
            assert!(g.admit(ip, now));
        }
        assert!(!g.admit(ip, now));
        g.release();
        assert!(g.admit(ip, now));
    }

    #[test]
    fn host_closes_the_door_after_repeated_bad_handshakes() {
        use std::io::{Read, Write};
        let h = host(0, "secret", Arc::new(|| {})).unwrap();
        let addr = format!("127.0.0.1:{}", h.port().unwrap());
        // Garbage instead of a key exchange: each one is a failure.
        for _ in 0..MAX_FAILURES {
            let mut s = TcpStream::connect(&addr).unwrap();
            let _ = s.write_all(&[0xff; 8]);
            let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
            let _ = s.read(&mut [0u8; 64]); // wait until the host hangs up
        }
        std::thread::sleep(Duration::from_millis(100));
        let mut s = TcpStream::connect(&addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let start = Instant::now();
        let n = s.read(&mut [0u8; 64]).unwrap_or(0);
        assert_eq!(n, 0, "shut out: the host closes at once");
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_peer_that_stops_reading_does_not_block_the_others() {
        use crate::model::{Block, BlockKind};
        let wake: Wake = Arc::new(|| {});
        let h = host(0, "secret", wake.clone()).unwrap();
        let addr = format!("127.0.0.1:{}", h.port().unwrap());
        // A peer that says hello, then never reads again.
        let mut stuck = TcpStream::connect(&addr).unwrap();
        let (mut sealer, _) = secure::handshake(&mut stuck, "secret", false).unwrap();
        sealer
            .send(
                &mut stuck,
                &bytes_of(&LiveMsg::Hello {
                    name: "Endormi".into(),
                    code: String::new(),
                }),
            )
            .unwrap();
        let start = Instant::now();
        while h.status().peers < 1 {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "stuck peer never joined"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        // Big pages until the stuck peer's buffers are full and it is dropped. Each send is
        // timed on its own: none may wait much longer than one message's deadline. Without
        // it, one send would never return, and nobody else would get anything.
        // (Only the stuck peer is there: a live peer of a debug build decodes big pages
        // slowly and would make the timing about it.)
        let mut p = Page::new("Grosse page", None);
        p.blocks = vec![Block::new(BlockKind::Paragraph, "x".repeat(128 * 1024))];
        let msg = LiveMsg::Pages {
            pages: vec![p],
            deleted: Default::default(),
        };
        let big = msg.clone();
        let (done_tx, done_rx) = channel();
        std::thread::spawn(move || {
            let mut slowest = Duration::ZERO;
            for _ in 0..400 {
                let t = Instant::now();
                h.send(&msg);
                slowest = slowest.max(t.elapsed());
                if h.status().peers == 0 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = done_tx.send((h, slowest));
        });
        let (h, slowest) = done_rx
            .recv_timeout(Duration::from_secs(60))
            .expect("sending is stuck");
        // At most the deadline of one message, plus sealing it (measured here) and some slack.
        let len = bytes_of(&big).len();
        let t = Instant::now();
        sealer.send(&mut Vec::new(), &bytes_of(&big)).unwrap();
        let seal = t.elapsed();
        let deadline = PEER_WRITE_TIMEOUT + Duration::from_secs(len as u64 / PEER_MIN_RATE);
        assert!(
            slowest < deadline + seal * 4 + Duration::from_secs(2),
            "one send took {slowest:?} (deadline {deadline:?}, sealing {seal:?})"
        );
        // The stuck peer was dropped and its connection closed; the host still serves others.
        assert_eq!(h.status().peers, 0, "stuck peer still counted");
        let _ = stuck.set_read_timeout(Some(Duration::from_secs(5)));
        let mut sink = vec![];
        assert!(
            std::io::Read::read_to_end(&mut stuck, &mut sink).is_ok(),
            "host closed the stuck connection"
        );
        let good = join(&addr, "Camille", "secret", wake);
        assert!(wait_for(&good, |m| matches!(m, LiveMsg::Connected)).is_some());
        h.send(&LiveMsg::Presence {
            name: "Hôte".into(),
            page: None,
            project: None,
            ai: false,
        });
        assert!(wait_for(&good, |m| matches!(m, LiveMsg::Presence { .. })).is_some());
        drop(stuck);
    }

    #[test]
    fn pages_travel_both_ways_and_wrong_codes_are_refused() {
        let wake: Wake = Arc::new(|| {});
        let h = host(0, "secret", wake.clone()).unwrap();
        let addr = format!("127.0.0.1:{}", h.port().unwrap());
        let j = join(&addr, "Camille", "secret", wake.clone());
        assert!(wait_for(&j, |m| matches!(m, LiveMsg::Connected)).is_some());
        assert!(
            wait_for(
                &h,
                |m| matches!(m, LiveMsg::Hello { name, .. } if name == "Camille")
            )
            .is_some()
        );
        // Joiner → host.
        let p = Page::new("Depuis Camille", None);
        j.send(&LiveMsg::Pages {
            pages: vec![p.clone()],
            deleted: Default::default(),
        });
        let got =
            wait_for(&h, |m| matches!(m, LiveMsg::Pages { .. })).expect("host receives pages");
        assert!(matches!(got, LiveMsg::Pages { pages, .. } if pages[0].title == "Depuis Camille"));
        // Host → joiner.
        h.send(&LiveMsg::Presence {
            name: "Hôte".into(),
            page: Some(p.id.clone()),
            project: None,
            ai: false,
        });
        assert!(wait_for(&j, |m| matches!(m, LiveMsg::Presence { .. })).is_some());
        // A peer with the wrong code is turned away.
        let bad = join(&addr, "Intrus", "faux", wake);
        assert!(
            wait_for(
                &bad,
                |m| matches!(m, LiveMsg::Disconnected { reason } if reason.contains("code"))
            )
            .is_some()
        );
        assert_eq!(h.status().peers, 1);
    }
}
