//! Database-encryption session state: the unlocked passphrase (memory only),
//! the OS-keychain "remember on this device" integration, and the idle clock
//! the auto-lock timer reads. The cryptography itself is SQLCipher's
//! (see `Db::open` / `Db::set_encryption`); this module never persists a
//! password anywhere except, on explicit opt-in, the OS keychain.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The passphrase for this session, set by the unlock flow (or the keychain)
/// and read wherever the database (re)opens. Never written to disk.
static SESSION_KEY: Mutex<Option<String>> = Mutex::new(None);

pub fn set_session_key(key: Option<String>) {
    *SESSION_KEY.lock().unwrap() = key;
}

pub fn session_key() -> Option<String> {
    SESSION_KEY.lock().unwrap().clone()
}

// --- OS keychain (macOS Keychain / Windows Credential Manager / kernel
// keyring on Linux) ---

const SERVICE: &str = "zorite";
const ACCOUNT: &str = "database";

fn entry() -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, ACCOUNT)
}

/// The opt-in marker, kept OUTSIDE the keychain (it holds no secret, just
/// "the user chose remember-on-this-device"). Reading the actual keychain
/// item can pop a system permission prompt (always, for unsigned dev
/// builds), so nothing may touch the keychain unless this flag exists —
/// especially not render paths.
fn flag_path() -> std::path::PathBuf {
    crate::paths::data_dir().join(".keychain-remember")
}

pub fn is_remembered() -> bool {
    flag_path().exists()
}

/// The remembered password — only consulted when the user opted in.
pub fn keychain_password() -> Option<String> {
    if !is_remembered() {
        return None;
    }
    entry().ok()?.get_password().ok()
}

pub fn remember_password(password: &str) -> bool {
    let ok = entry().and_then(|e| e.set_password(password)).is_ok();
    if ok {
        let _ = std::fs::write(flag_path(), "");
    }
    ok
}

pub fn forget_password() {
    if let Ok(e) = entry() {
        let _ = e.delete_credential();
    }
    let _ = std::fs::remove_file(flag_path());
}

// --- Idle clock for the auto-lock timer ---

/// Unix seconds of the last user input, updated from the main window's
/// keystroke/mouse observers; the auto-lock timer compares against it.
static LAST_ACTIVITY: AtomicU64 = AtomicU64::new(0);

/// Auto-lock threshold in minutes (0 = off), mirrored from the persisted
/// setting so the timer can read it without a database handle.
static AUTO_LOCK_MINUTES: AtomicU64 = AtomicU64::new(0);

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn touch_activity() {
    LAST_ACTIVITY.store(now_secs(), Ordering::Relaxed);
}

pub fn set_auto_lock_minutes(minutes: u64) {
    AUTO_LOCK_MINUTES.store(minutes, Ordering::Relaxed);
}

pub fn auto_lock_minutes() -> u64 {
    AUTO_LOCK_MINUTES.load(Ordering::Relaxed)
}

/// Whether the idle threshold has passed (and auto-lock is on at all).
pub fn idle_past_threshold() -> bool {
    let minutes = auto_lock_minutes();
    if minutes == 0 {
        return false;
    }
    let last = LAST_ACTIVITY.load(Ordering::Relaxed);
    last != 0 && now_secs().saturating_sub(last) >= minutes * 60
}
