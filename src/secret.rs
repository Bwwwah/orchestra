//! The live access code, kept out of workspace.json.
//!
//! The code encrypts the live connection (secure.rs): every person needs it in clear, so it
//! can't be hashed. It goes to the macOS Keychain (encrypted by the system, through the
//! `security` tool), or elsewhere to a file only this user may read. It is read once per
//! process and kept in memory.

use std::path::PathBuf;
use std::sync::Mutex;

const SERVICE: &str = "Orchestra";
const ACCOUNT: &str = "live-code";

/// What each account holds as last read or written by this process (absent: not read yet).
static CACHE: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

fn use_keychain() -> bool {
    // Tests and separate data folders (ORCHESTRA_DIR) keep theirs in a file, so they never
    // touch the person's Keychain.
    cfg!(target_os = "macos") && !cfg!(test) && std::env::var_os("ORCHESTRA_DIR").is_none()
}

fn file(account: &str) -> PathBuf {
    crate::store::data_dir().join(format!(".{account}"))
}

fn security() -> std::process::Command {
    std::process::Command::new("/usr/bin/security")
}

fn read_store(account: &str) -> String {
    if use_keychain() {
        security()
            .args(["find-generic-password", "-s", SERVICE, "-a", account, "-w"])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    } else {
        std::fs::read_to_string(file(account))
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    }
}

fn write_store(account: &str, code: &str) -> Result<(), String> {
    if code.is_empty() {
        return Err("empty code".into());
    }
    if use_keychain() {
        let o = security()
            .args([
                "add-generic-password",
                "-U",
                "-s",
                SERVICE,
                "-a",
                account,
                "-l",
                &format!("Orchestra · {account}"),
                "-w",
                code,
            ])
            .output()
            .map_err(|e| e.to_string())?;
        if o.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
        }
    } else {
        let f = file(account);
        if let Some(d) = f.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        crate::store::write_atomic(&f, code.as_bytes()).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

fn get(account: &str) -> String {
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, v)) = c.iter().find(|(a, _)| a == account) {
        return v.clone();
    }
    let v = read_store(account);
    c.push((account.to_string(), v.clone()));
    v
}

fn set(account: &str, value: &str) -> Result<(), String> {
    let value = value.trim();
    if get(account) == value {
        return Ok(());
    }
    write_store(account, value)?;
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    c.retain(|(a, _)| a != account);
    c.push((account.to_string(), value.to_string()));
    Ok(())
}

/// The live access code ("" when none was ever set).
pub fn live_code() -> String {
    get(ACCOUNT)
}

/// Keep `code` as the live access code (only written when it changed; never empty).
pub fn set_live_code(code: &str) -> Result<(), String> {
    set(ACCOUNT, code)
}

/// The account holding the API key named `var` (one of `sync::API_KEY_VARS`).
fn key_account(var: &str) -> String {
    format!("api-key-{var}")
}

/// The API key kept for `var` ("" when none was ever set). Only to be handed to an AI that
/// runs in API mode; subscription sessions never get it.
pub fn api_key(var: &str) -> String {
    get(&key_account(var))
}

/// Keep `key` as the API key for `var` (never empty; Keychain, or a 0600 file elsewhere).
pub fn set_api_key(var: &str, key: &str) -> Result<(), String> {
    set(&key_account(var), key)
}

/// Forget the API key kept for `var` (nothing to do when there is none).
pub fn remove_api_key(var: &str) -> Result<(), String> {
    let account = key_account(var);
    if get(&account).is_empty() {
        return Ok(());
    }
    if use_keychain() {
        let o = security()
            .args(["delete-generic-password", "-s", SERVICE, "-a", &account])
            .stdout(std::process::Stdio::null())
            .output()
            .map_err(|e| e.to_string())?;
        if !o.status.success() {
            return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
        }
    } else {
        std::fs::remove_file(file(&account)).map_err(|e| e.to_string())?;
    }
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    c.retain(|(a, _)| a != &account);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_secret_has_its_own_account_and_file() {
        assert_ne!(file(&key_account("A")), file(&key_account("B")));
        assert_ne!(file(&key_account("A")), file(ACCOUNT));
        assert_eq!(key_account("OPENAI_API_KEY"), "api-key-OPENAI_API_KEY");
    }
}
