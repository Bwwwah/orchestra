//! The account Claude Code is signed in with, as its own `claude auth status` tells it, and
//! the command that signs in. Signing in happens in the person's browser, run by Claude Code
//! itself: Orchestra never sees nor keeps a password or a token.

use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

/// Signs in to one's Claude account: Claude Code opens the browser and waits for the answer.
pub const LOGIN_COMMAND: &str = "claude auth login";

/// How long `claude auth status` may take. A Claude Code too old to know it could take the
/// words for a question and start answering: it is stopped.
const STATUS_LIMIT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq)]
pub enum Account {
    SignedOut,
    SignedIn {
        /// How it signed in (`claude.ai` for a Claude account), when told.
        method: Option<String>,
        email: Option<String>,
        /// The subscription (`pro`, `max`…), when told.
        plan: Option<String>,
    },
}

impl Account {
    pub fn signed_in(&self) -> bool {
        matches!(self, Account::SignedIn { .. })
    }

    /// For the person: « connectée avec nom@exemple.fr (abonnement Max) ».
    pub fn describe(&self) -> String {
        let Account::SignedIn {
            method,
            email,
            plan,
        } = self
        else {
            return crate::tr!("aucun compte connecté", "no account signed in").into();
        };
        if method.as_deref().is_some_and(|m| m != "claude.ai") {
            return crate::tr!(
                "connectée sans compte Claude (clé API ou autre accès)",
                "signed in without a Claude account (API key or other access)"
            )
            .into();
        }
        let who = match email {
            Some(e) => crate::trf!("connectée avec {e}", "signed in as {e}"),
            None => crate::tr!("compte Claude connecté", "Claude account signed in").into(),
        };
        match plan {
            Some(p) => {
                let p = capitalized(p);
                crate::trf!("{who} (abonnement {p})", "{who} ({p} subscription)")
            }
            None => who,
        }
    }
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|first| first.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// What `claude auth status` printed (JSON). None when it says nothing we understand.
pub fn parse_status(output: &str) -> Option<Account> {
    let json = output.get(output.find('{')?..=output.rfind('}')?)?;
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    if !v.get("loggedIn")?.as_bool()? {
        return Some(Account::SignedOut);
    }
    let text = |key: &str| {
        v.get(key)
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| crate::richtext::one_line(s, 80))
    };
    Some(Account::SignedIn {
        method: text("authMethod"),
        email: text("email"),
        plan: text("subscriptionType"),
    })
}

/// Ask this Claude Code which account it is signed in with (starts it: call it off the UI
/// thread). None when it can't tell (too old, broken, too slow).
pub fn read(bin: &Path, path: &str) -> Option<Account> {
    read_within(bin, path, STATUS_LIMIT)
}

fn read_within(bin: &Path, path: &str, limit: Duration) -> Option<Account> {
    let mut cmd = crate::proc::command(bin);
    cmd.args(["auth", "status"])
        .env("PATH", path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // The account the person signed in with, not a stray API key's.
    for k in crate::sync::API_KEY_VARS
        .iter()
        .chain(&crate::sync::HOST_SESSION_VARS)
    {
        cmd.env_remove(k);
    }
    if let Some(home) = dirs::home_dir() {
        cmd.current_dir(home);
    }
    let mut child = cmd.spawn().ok()?;
    let mut pipe = child.stdout.take()?;
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = String::new();
        let _ = pipe.read_to_string(&mut out);
        let _ = send.send(out);
    });
    let start = Instant::now();
    // The exit code is left aside: it isn't 0 when nobody is signed in.
    while child.try_wait().ok()?.is_none() {
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    parse_status(&receive.recv_timeout(Duration::from_secs(2)).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_in(method: &str, email: Option<&str>, plan: Option<&str>) -> Account {
        Account::SignedIn {
            method: Some(method.into()),
            email: email.map(String::from),
            plan: plan.map(String::from),
        }
    }

    #[test]
    fn the_status_says_who_is_signed_in_or_nothing() {
        assert_eq!(
            parse_status(
                r#"{"loggedIn": true, "authMethod": "claude.ai", "email": "ada@exemple.fr", "subscriptionType": "max"}"#
            ),
            Some(signed_in("claude.ai", Some("ada@exemple.fr"), Some("max")))
        );
        // A warning printed first, empty fields: still read.
        assert_eq!(
            parse_status("attention\n{\"loggedIn\":true,\"authMethod\":\"api_key\",\"email\":\"\"}\n"),
            Some(signed_in("api_key", None, None))
        );
        assert_eq!(
            parse_status(r#"{"loggedIn": false, "authMethod": "none"}"#),
            Some(Account::SignedOut)
        );
        // Anything else: unknown, never « signed out ».
        for other in ["", "error: unknown command", "{}", "{\"loggedIn\":\"yes\"}", "} {"] {
            assert_eq!(parse_status(other), None, "{other:?}");
        }
    }

    #[test]
    fn the_account_is_described_for_the_person() {
        let d = signed_in("claude.ai", Some("ada@exemple.fr"), Some("max")).describe();
        assert!(d.contains("ada@exemple.fr") && d.contains("Max"), "{d}");
        // A key isn't presented as the person's subscription.
        let d = signed_in("api_key", Some("ada@exemple.fr"), Some("max")).describe();
        assert!(!d.contains("Max"), "{d}");
        assert!(!Account::SignedOut.signed_in());
        assert!(signed_in("claude.ai", None, None).signed_in());
    }

    /// A stand-in « claude » running this shell script.
    #[cfg(unix)]
    fn fake(script: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("orch-acc-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("claude");
        std::fs::write(&bin, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, bin)
    }

    #[cfg(unix)]
    #[test]
    fn the_account_is_read_from_claude_even_when_it_ends_in_error() {
        let (dir, bin) = fake(
            "[ \"$1 $2\" = 'auth status' ] || exit 2\necho '{\"loggedIn\":false}'\nexit 1",
        );
        assert_eq!(read(&bin, "/usr/bin:/bin"), Some(Account::SignedOut));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_claude_that_does_not_end_is_stopped() {
        let (dir, bin) = fake("exec sleep 30");
        let start = Instant::now();
        assert_eq!(
            read_within(&bin, "/usr/bin:/bin", Duration::from_millis(300)),
            None
        );
        assert!(start.elapsed() < Duration::from_secs(10));
        let _ = std::fs::remove_dir_all(dir);
    }
}
