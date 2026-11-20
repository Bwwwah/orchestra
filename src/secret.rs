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

/// The code as last read or written by this process (None: not read yet).
static CACHE: Mutex<Option<String>> = Mutex::new(None);

fn use_keychain() -> bool {
    // Tests and separate data folders (ORCHESTRA_DIR) keep theirs in a file, so they never
    // touch the person's Keychain.
    cfg!(target_os = "macos") && !cfg!(test) && std::env::var_os("ORCHESTRA_DIR").is_none()
}

fn file() -> PathBuf {
    crate::store::data_dir().join(".live-code")
}

fn security() -> std::process::Command {
    std::process::Command::new("/usr/bin/security")
}

fn read_store() -> String {
    if use_keychain() {
        security()
            .args(["find-generic-password", "-s", SERVICE, "-a", ACCOUNT, "-w"])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    } else {
        std::fs::read_to_string(file()).map(|s| s.trim().to_string()).unwrap_or_default()
    }
}

fn write_store(code: &str) -> Result<(), String> {
    if code.is_empty() {
        return Err("empty code".into());
    }
    if use_keychain() {
        let o = security()
            .args(["add-generic-password", "-U", "-s", SERVICE, "-a", ACCOUNT, "-l", "Orchestra · code d'accès en direct", "-w", code])
            .output()
            .map_err(|e| e.to_string())?;
        if o.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&o.stderr).trim().to_string()) }
    } else {
        let f = file();
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

/// The live access code ("" when none was ever set).
pub fn live_code() -> String {
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    c.get_or_insert_with(read_store).clone()
}

/// Keep `code` as the live access code (only written when it changed; never empty).
pub fn set_live_code(code: &str) -> Result<(), String> {
    let code = code.trim();
    if live_code() == code {
        return Ok(());
    }
    write_store(code)?;
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some(code.to_string());
    Ok(())
}
