//! Persisted app settings, stored as plain key/value rows in the `meta`
//! table. This covers the Broadcast (live stream) settings and the folder
//! the "Fix missing files" search starts in.

use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

const KEY_ENABLED: &str = "broadcast_enabled";
const KEY_PORT: &str = "broadcast_port";
const KEY_KEY: &str = "broadcast_key";
const KEY_PUBLIC_BASE: &str = "broadcast_public_base";
const KEY_FIX_MISSING_ROOT: &str = "fix_missing_search_root";

/// Port the stream server listens on until the user picks another one.
/// 5883 spells L-U-T-E on a phone keypad. We avoid 333x because other dev
/// tools grab those generic numbers, and 5883 is in the registered range so the
/// OS never hands it out as a temporary (ephemeral) port.
pub const DEFAULT_BROADCAST_PORT: u16 = 5883;

/// Everything the user can configure about the broadcast stream.
#[derive(Debug, Clone, PartialEq)]
pub struct BroadcastSettings {
    /// Whether the stream server should be running.
    pub enabled: bool,
    /// TCP port the stream server listens on.
    pub port: u16,
    /// Secret URL component listeners need; acts like a password in the link.
    pub key: String,
    /// Public address shown to listeners (e.g. a Caddy/duckdns URL); empty if unset.
    pub public_base: String,
}

/// Makes a new secret listener key: a random UUID written as 32 lowercase
/// hex characters (no dashes).
pub fn generate_listener_key() -> String {
    Uuid::new_v4().simple().to_string()
}

/// True if `k` is exactly 32 lowercase hex characters, the only shape
/// `generate_listener_key` produces. Anything else (empty, short, odd
/// characters) would make a weak or guessable listener link.
fn is_valid_listener_key(k: &str) -> bool {
    k.len() == 32 && k.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// Reads one value from the `meta` table, or `None` if the row is missing.
fn read_meta(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |row| {
        row.get::<_, String>(0)
    })
    .optional()
    .map_err(|e| format!("Failed to read {key}: {e}"))
}

/// Writes one value to the `meta` table, replacing any existing row.
fn write_meta(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        params![key, value],
    )
    .map_err(|e| format!("Failed to write {key}: {e}"))?;
    Ok(())
}

/// Loads the broadcast settings, using defaults for anything missing
/// (off, port 5883, no public base). If no listener key exists yet, a new
/// one is generated and saved immediately so it stays the same on later loads.
/// A stored port that is not a valid number falls back to the default.
pub fn load_broadcast(conn: &Connection) -> Result<BroadcastSettings, String> {
    let enabled = read_meta(conn, KEY_ENABLED)?.as_deref() == Some("1");

    let port = match read_meta(conn, KEY_PORT)? {
        None => DEFAULT_BROADCAST_PORT,
        Some(text) => text.parse::<u16>().unwrap_or_else(|_| {
            log::warn!(
                "Invalid stored {KEY_PORT} {text:?}; using default {DEFAULT_BROADCAST_PORT}"
            );
            DEFAULT_BROADCAST_PORT
        }),
    };

    let key = match read_meta(conn, KEY_KEY)? {
        Some(k) if is_valid_listener_key(&k) => k,
        stored => {
            if stored.is_some() {
                log::warn!("Stored {KEY_KEY} is malformed; replacing it with a new key");
            }
            let k = generate_listener_key();
            write_meta(conn, KEY_KEY, &k)?;
            k
        }
    };

    let public_base = read_meta(conn, KEY_PUBLIC_BASE)?.unwrap_or_default();

    Ok(BroadcastSettings { enabled, port, key, public_base })
}

/// Saves all four broadcast settings, replacing whatever was stored before.
pub fn save_broadcast(conn: &Connection, s: &BroadcastSettings) -> Result<(), String> {
    write_meta(conn, KEY_ENABLED, if s.enabled { "1" } else { "0" })?;
    write_meta(conn, KEY_PORT, &s.port.to_string())?;
    write_meta(conn, KEY_KEY, &s.key)?;
    write_meta(conn, KEY_PUBLIC_BASE, &s.public_base)?;
    Ok(())
}

/// Loads the folder the "Fix missing files" search starts in. Returns `None`
/// if the user never picked one, or if the stored value is empty.
pub fn load_fix_missing_root(conn: &Connection) -> Result<Option<String>, String> {
    Ok(read_meta(conn, KEY_FIX_MISSING_ROOT)?.filter(|root| !root.is_empty()))
}

/// Saves the folder the "Fix missing files" search starts in. Passing an
/// empty string clears it, so the next load returns `None`.
pub fn save_fix_missing_root(conn: &Connection, root: &str) -> Result<(), String> {
    write_meta(conn, KEY_FIX_MISSING_ROOT, root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::init_schema;
    use rusqlite::Connection;

    #[test]
    fn load_defaults_and_persists_a_fresh_key() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let a = load_broadcast(&conn).unwrap();
        assert!(!a.enabled);
        assert_eq!(a.port, 5883);
        assert_eq!(a.key.len(), 32);
        assert!(a.key.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(a.public_base, "");
        let b = load_broadcast(&conn).unwrap();
        assert_eq!(a, b, "key must be stable across loads");
    }

    #[test]
    fn save_then_load_round_trips() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let s = BroadcastSettings {
            enabled: true,
            port: 4000,
            key: "ab".repeat(16),
            public_base: "https://x.example".into(),
        };
        save_broadcast(&conn, &s).unwrap();
        assert_eq!(load_broadcast(&conn).unwrap(), s);
    }

    #[test]
    fn malformed_stored_key_is_replaced_and_persisted() {
        for bad in ["", "abc"] {
            let conn = Connection::open_in_memory().unwrap();
            init_schema(&conn).unwrap();
            write_meta(&conn, KEY_KEY, bad).unwrap();
            let a = load_broadcast(&conn).unwrap();
            assert!(is_valid_listener_key(&a.key), "bad input {bad:?} must be replaced");
            let b = load_broadcast(&conn).unwrap();
            assert_eq!(a.key, b.key, "replacement key must be persisted");
        }
    }

    #[test]
    fn fix_missing_root_defaults_to_none_and_round_trips() {
        let conn = Connection::open_in_memory().unwrap(); init_schema(&conn).unwrap();
        assert_eq!(load_fix_missing_root(&conn).unwrap(), None);
        save_fix_missing_root(&conn, r"Z:\media\audio\music").unwrap();
        assert_eq!(load_fix_missing_root(&conn).unwrap().as_deref(), Some(r"Z:\media\audio\music"));
        save_fix_missing_root(&conn, "").unwrap();
        assert_eq!(load_fix_missing_root(&conn).unwrap(), None);
    }
}
