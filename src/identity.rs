//! The person's identity: one Ed25519 key, made once on this computer, the same for every
//! team server (see docs/collaboration-serveur.md). A server knows the public half; to connect,
//! the app signs what the server asks (a fresh challenge), so no password ever travels.
//!
//! The secret half is kept in the data folder (`.identite`, readable by this user only).
//! Losing it means joining the servers again with a new invitation.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

fn key_path() -> std::path::PathBuf {
    crate::store::data_dir().join(".identite")
}

/// This computer's key, made the first time it is asked for.
pub fn key() -> Result<SigningKey, String> {
    let path = key_path();
    if let Ok(text) = std::fs::read_to_string(&path) {
        return from_hex(text.trim()).ok_or_else(|| {
            crate::tr!("clé d'identité illisible", "unreadable identity key").into()
        });
    }
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| e.to_string())?;
    let k = SigningKey::from_bytes(&seed);
    std::fs::create_dir_all(crate::store::data_dir()).map_err(|e| e.to_string())?;
    // Written once, privately (0600 from the start, see store::write_atomic).
    crate::store::write_atomic(&path, hex(&seed).as_bytes()).map_err(|e| e.to_string())?;
    // Two processes making it at once: keep what is on disk, so they agree.
    Ok(std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| from_hex(t.trim()))
        .unwrap_or(k))
}

/// The public half, as the servers know it (64 hexadecimal characters).
pub fn public_hex(k: &SigningKey) -> String {
    hex(k.verifying_key().as_bytes())
}

/// The signature of `msg`, in hexadecimal.
pub fn sign_hex(k: &SigningKey, msg: &[u8]) -> String {
    hex(&k.sign(msg).to_bytes())
}

/// Does `sig_hex` sign `msg` with the key `public_hex`?
pub fn verify(public_hex: &str, msg: &[u8], sig_hex: &str) -> bool {
    let (Some(pk), Some(sig)) = (unhex::<32>(public_hex), unhex::<64>(sig_hex)) else {
        return false;
    };
    let Ok(vk) = VerifyingKey::from_bytes(&pk) else {
        return false;
    };
    vk.verify(msg, &Signature::from_bytes(&sig)).is_ok()
}

/// Random bytes as hexadecimal (challenges, invitation tokens).
pub fn random_hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    // A failure leaves zeros: callers check the length of what they get, never its secrecy…
    // so refuse to hand out a predictable token instead.
    if getrandom::getrandom(&mut b).is_err() {
        panic!("no randomness available from the system");
    }
    hex(&b)
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex<const N: usize>(s: &str) -> Option<[u8; N]> {
    let s = s.trim();
    if s.len() != N * 2 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn from_hex(s: &str) -> Option<SigningKey> {
    unhex::<32>(s).map(|seed| SigningKey::from_bytes(&seed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_proves_the_key_and_nothing_else() {
        let a = SigningKey::from_bytes(&[7u8; 32]);
        let b = SigningKey::from_bytes(&[8u8; 32]);
        let sig = sign_hex(&a, b"defi-123");
        assert!(verify(&public_hex(&a), b"defi-123", &sig));
        assert!(!verify(&public_hex(&a), b"defi-124", &sig));
        assert!(!verify(&public_hex(&b), b"defi-123", &sig));
        assert!(!verify("zz", b"defi-123", &sig));
        assert!(!verify(&public_hex(&a), b"defi-123", "00"));
    }

    #[test]
    fn hex_round_trips() {
        let k = SigningKey::from_bytes(&[42u8; 32]);
        assert_eq!(
            from_hex(&hex(&[42u8; 32])).unwrap().to_bytes(),
            k.to_bytes()
        );
        assert_eq!(random_hex(16).len(), 32);
        assert_ne!(random_hex(16), random_hex(16));
    }
}
