//! Encryption of the real-time connection, with nothing more than the shared code.
//!
//! 1. Both ends run SPAKE2 with the code: a password-authenticated key exchange. A strong
//!    shared key comes out of a short code, and someone who listens to the exchange learns
//!    nothing that would let them try codes offline (one guess per connection, at most).
//! 2. HKDF turns that key into one key per direction.
//! 3. Every message is then sealed with ChaCha20-Poly1305: unreadable and tamper-evident.
//!    Nonces are message counters, so a replayed or reordered message is refused too.
//!
//! A wrong code gives different keys: the first message can't be opened and the
//! connection stops with "code incorrect".

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;
use spake2::{Ed25519Group, Identity, Password, Spake2};
use std::io::{Read, Write};

/// Largest message accepted (a big page with its database).
const MAX_FRAME: usize = 32 * 1024 * 1024;

pub struct Sender {
    cipher: ChaCha20Poly1305,
    counter: u64,
}

pub struct Receiver {
    cipher: ChaCha20Poly1305,
    counter: u64,
}

fn nonce(counter: u64) -> Nonce {
    let mut n = [0u8; 12];
    n[4..].copy_from_slice(&counter.to_be_bytes());
    Nonce::from(n)
}

fn write_raw(w: &mut impl Write, bytes: &[u8]) -> std::io::Result<()> {
    w.write_all(&(bytes.len() as u32).to_be_bytes())?;
    w.write_all(bytes)?;
    w.flush()
}

fn read_raw(r: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, crate::tr!("message trop long", "message too long")));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

/// Run the key exchange over a fresh connection. `hosting` picks which derived key each side
/// sends with. Both ends must use the same code.
pub fn handshake(stream: &mut (impl Read + Write), code: &str, hosting: bool) -> std::io::Result<(Sender, Receiver)> {
    let (state, outbound) = Spake2::<Ed25519Group>::start_symmetric(&Password::new(code.as_bytes()), &Identity::new(b"orchestra/direct/1"));
    write_raw(stream, &outbound)?;
    let inbound = read_raw(stream)?;
    let shared = state.finish(&inbound).map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, crate::tr!("échange de clés invalide", "invalid key exchange")))?;
    let hk = Hkdf::<Sha256>::new(Some(b"orchestra"), &shared);
    let mut host_key = [0u8; 32];
    let mut peer_key = [0u8; 32];
    hk.expand(b"host->peer", &mut host_key).map_err(|_| std::io::Error::other("hkdf"))?;
    hk.expand(b"peer->host", &mut peer_key).map_err(|_| std::io::Error::other("hkdf"))?;
    let (send_key, recv_key) = if hosting { (host_key, peer_key) } else { (peer_key, host_key) };
    Ok((
        Sender { cipher: ChaCha20Poly1305::new(Key::from_slice(&send_key)), counter: 0 },
        Receiver { cipher: ChaCha20Poly1305::new(Key::from_slice(&recv_key)), counter: 0 },
    ))
}

impl Sender {
    pub fn send(&mut self, w: &mut impl Write, plain: &[u8]) -> std::io::Result<()> {
        let sealed = self.cipher.encrypt(&nonce(self.counter), plain).map_err(|_| std::io::Error::other("chiffrement"))?;
        self.counter += 1;
        write_raw(w, &sealed)
    }
}

/// Why a message couldn't be read.
#[derive(Debug, PartialEq)]
pub enum RecvError {
    /// The connection closed or failed.
    Closed,
    /// The message can't be opened: wrong code, or altered on the way.
    Refused,
}

impl Receiver {
    pub fn recv(&mut self, r: &mut impl Read) -> Result<Vec<u8>, RecvError> {
        let sealed = read_raw(r).map_err(|_| RecvError::Closed)?;
        let plain = self.cipher.decrypt(&nonce(self.counter), sealed.as_slice()).map_err(|_| RecvError::Refused)?;
        self.counter += 1;
        Ok(plain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};

    fn pair(code_a: &str, code_b: &str) -> ((Sender, Receiver, TcpStream), (Sender, Receiver, TcpStream)) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let cb = code_b.to_string();
        let t = std::thread::spawn(move || {
            let mut s = TcpStream::connect(addr).unwrap();
            let (tx, rx) = handshake(&mut s, &cb, false).unwrap();
            (tx, rx, s)
        });
        let (mut s, _) = l.accept().unwrap();
        let (tx, rx) = handshake(&mut s, code_a, true).unwrap();
        ((tx, rx, s), t.join().unwrap())
    }

    #[test]
    fn same_code_talks_both_ways() {
        let ((mut htx, mut hrx, mut hs), (mut ptx, mut prx, mut ps)) = pair("loutre-42", "loutre-42");
        ptx.send(&mut ps, b"bonjour").unwrap();
        assert_eq!(hrx.recv(&mut hs).unwrap(), b"bonjour");
        htx.send(&mut hs, b"salut").unwrap();
        htx.send(&mut hs, b"encore").unwrap();
        assert_eq!(prx.recv(&mut ps).unwrap(), b"salut");
        assert_eq!(prx.recv(&mut ps).unwrap(), b"encore");
    }

    #[test]
    fn wrong_code_is_refused() {
        let ((_, mut hrx, mut hs), (mut ptx, _, mut ps)) = pair("loutre-42", "loutre-43");
        ptx.send(&mut ps, b"bonjour").unwrap();
        assert_eq!(hrx.recv(&mut hs), Err(RecvError::Refused));
    }

    #[test]
    fn nothing_readable_on_the_wire() {
        let mut wire: Vec<u8> = vec![];
        let key = ChaCha20Poly1305::new(Key::from_slice(&[7u8; 32]));
        let mut s = Sender { cipher: key, counter: 0 };
        s.send(&mut wire, b"page secrete").unwrap();
        assert!(!wire.windows(7).any(|w| w == b"secrete"));
    }
}
