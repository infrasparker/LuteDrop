//! Persisted app settings, stored as plain key/value rows in the `meta`
//! table. This covers the Broadcast (live stream) settings and the folder
//! the "Fix missing files" search starts in.

use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::models::{AutoTagSettings, SceneConfig};

const KEY_ENABLED: &str = "broadcast_enabled";
const KEY_PORT: &str = "broadcast_port";
const KEY_KEY: &str = "broadcast_key";
const KEY_PUBLIC_BASE: &str = "broadcast_public_base";
const KEY_FIX_MISSING_ROOT: &str = "fix_missing_search_root";
const KEY_AUTOTAG_ENABLED: &str = "autotag_enabled";
const KEY_AUTOTAG_URL: &str = "autotag_url";
const KEY_AUTOTAG_TOKEN: &str = "autotag_token";
const KEY_AUTOTAG_LOCAL_PREFIX: &str = "autotag_local_prefix";
const KEY_AUTOTAG_REMOTE_PREFIX: &str = "autotag_remote_prefix";
const KEY_AUTOTAG_SCENE: &str = "autotag_scene_encounter";

/// Where the analyzer host mounts the audio folder until the user says otherwise.
pub const DEFAULT_REMOTE_PREFIX: &str = "/media/audio";

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

/// Loads the analyzer settings. Missing values fall back to: disabled, empty
/// URL and token, empty local prefix, and `/media/audio` as the remote prefix.
pub fn load_autotag(conn: &Connection) -> Result<AutoTagSettings, String> {
    Ok(AutoTagSettings {
        enabled: read_meta(conn, KEY_AUTOTAG_ENABLED)?.as_deref() == Some("1"),
        analyzer_url: read_meta(conn, KEY_AUTOTAG_URL)?.unwrap_or_default(),
        token: read_meta(conn, KEY_AUTOTAG_TOKEN)?.unwrap_or_default(),
        local_prefix: read_meta(conn, KEY_AUTOTAG_LOCAL_PREFIX)?.unwrap_or_default(),
        remote_prefix: read_meta(conn, KEY_AUTOTAG_REMOTE_PREFIX)?
            .unwrap_or_else(|| DEFAULT_REMOTE_PREFIX.to_string()),
    })
}

/// Saves all analyzer settings, replacing whatever was stored before.
pub fn save_autotag(conn: &Connection, s: &AutoTagSettings) -> Result<(), String> {
    write_meta(conn, KEY_AUTOTAG_ENABLED, if s.enabled { "1" } else { "0" })?;
    write_meta(conn, KEY_AUTOTAG_URL, &s.analyzer_url)?;
    write_meta(conn, KEY_AUTOTAG_TOKEN, &s.token)?;
    write_meta(conn, KEY_AUTOTAG_LOCAL_PREFIX, &s.local_prefix)?;
    write_meta(conn, KEY_AUTOTAG_REMOTE_PREFIX, &s.remote_prefix)?;
    Ok(())
}

/// Loads the scene configuration, stored as JSON. A missing or unparseable
/// value gives `SceneConfig::default()`.
pub fn load_scene_config(conn: &Connection) -> Result<SceneConfig, String> {
    let Some(text) = read_meta(conn, KEY_AUTOTAG_SCENE)? else {
        return Ok(SceneConfig::default());
    };
    Ok(serde_json::from_str(&text).unwrap_or_else(|e| {
        log::warn!("Invalid stored {KEY_AUTOTAG_SCENE}: {e}; using default scene config");
        SceneConfig::default()
    }))
}

/// Saves the scene configuration as JSON.
pub fn save_scene_config(conn: &Connection, c: &SceneConfig) -> Result<(), String> {
    let text = serde_json::to_string(c).map_err(|e| format!("Failed to encode scene config: {e}"))?;
    write_meta(conn, KEY_AUTOTAG_SCENE, &text)
}

/// Replaces a saved scene configuration with `SceneConfig::default()`. A
/// database that never saved one already loads the default and is left as is.
/// Used by the v6→v7 migration, hence the rusqlite error type.
pub(crate) fn reset_saved_scene_config(conn: &Connection) -> rusqlite::Result<()> {
    let text = serde_json::to_string(&SceneConfig::default())
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    conn.execute(
        "UPDATE meta SET value = ?1 WHERE key = ?2",
        params![text, KEY_AUTOTAG_SCENE],
    )?;
    Ok(())
}

/// Removes `word` from a saved scene configuration's vocabulary, matching
/// names the way tags are stored (trimmed, lowercase). Every other field of
/// the saved JSON is kept. A database that never saved one, or saved one that
/// does not parse, is left as is: it loads the default anyway. Used by the
/// v9→v10 migration, hence the rusqlite error type.
pub(crate) fn remove_from_saved_vocabulary(conn: &Connection, word: &str) -> rusqlite::Result<()> {
    let text: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [KEY_AUTOTAG_SCENE], |r| r.get(0))
        .optional()?;
    let Some(text) = text else { return Ok(()) };
    let Ok(mut config) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Ok(());
    };
    let Some(words) = config.get_mut("vocabulary").and_then(|v| v.as_array_mut()) else {
        return Ok(());
    };
    let before = words.len();
    words.retain(|w| !matches!(w.as_str(), Some(w) if w.trim().to_lowercase() == word));
    if words.len() == before {
        return Ok(());
    }
    let text = serde_json::to_string(&config)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    conn.execute(
        "UPDATE meta SET value = ?1 WHERE key = ?2",
        params![text, KEY_AUTOTAG_SCENE],
    )?;
    Ok(())
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
    fn autotag_settings_round_trip() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let defaults = load_autotag(&conn).unwrap();
        assert!(!defaults.enabled);
        assert_eq!(defaults.analyzer_url, "");
        assert_eq!(defaults.local_prefix, "");
        assert_eq!(defaults.remote_prefix, "/media/audio");

        let s = AutoTagSettings {
            enabled: true,
            analyzer_url: "http://host:8765".into(),
            token: "secret".into(),
            local_prefix: r"Z:\media\audio".into(),
            remote_prefix: "/mnt/audio".into(),
        };
        save_autotag(&conn, &s).unwrap();
        assert_eq!(load_autotag(&conn).unwrap(), s);
    }

    #[test]
    fn scene_config_defaults_when_missing_or_corrupt() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        assert_eq!(load_scene_config(&conn).unwrap(), SceneConfig::default());

        write_meta(&conn, KEY_AUTOTAG_SCENE, "{not json").unwrap();
        assert_eq!(load_scene_config(&conn).unwrap(), SceneConfig::default());

        let custom = SceneConfig { vocabulary: vec!["folk".into()] };
        save_scene_config(&conn, &custom).unwrap();
        assert_eq!(load_scene_config(&conn).unwrap(), custom);
    }

    #[test]
    fn scene_config_loads_legacy_json_keeping_vocabulary() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        // The shape saved before the config was reduced to its vocabulary.
        let legacy = r#"{"scene":"encounter","gateTag":"encounter",
            "negativeSceneTags":["tavern","mood"],"vocabulary":["brass","Dark","folk"]}"#;
        write_meta(&conn, KEY_AUTOTAG_SCENE, legacy).unwrap();
        assert_eq!(
            load_scene_config(&conn).unwrap(),
            SceneConfig { vocabulary: vec!["brass".into(), "Dark".into(), "folk".into()] }
        );
    }

    #[test]
    fn remove_from_saved_vocabulary_keeps_the_rest() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        // Nothing saved: nothing to do.
        remove_from_saved_vocabulary(&conn, "driving").unwrap();
        assert_eq!(read_meta(&conn, KEY_AUTOTAG_SCENE).unwrap(), None);

        write_meta(
            &conn,
            KEY_AUTOTAG_SCENE,
            r#"{"gateTag":"encounter","vocabulary":["folk"," Driving ","brass","driving"]}"#,
        )
        .unwrap();
        remove_from_saved_vocabulary(&conn, "driving").unwrap();
        assert_eq!(
            load_scene_config(&conn).unwrap(),
            SceneConfig { vocabulary: vec!["folk".into(), "brass".into()] }
        );
        let saved: serde_json::Value =
            serde_json::from_str(&read_meta(&conn, KEY_AUTOTAG_SCENE).unwrap().unwrap()).unwrap();
        assert_eq!(saved["gateTag"], "encounter");

        // An unparseable value is left alone.
        write_meta(&conn, KEY_AUTOTAG_SCENE, "not json").unwrap();
        remove_from_saved_vocabulary(&conn, "driving").unwrap();
        assert_eq!(read_meta(&conn, KEY_AUTOTAG_SCENE).unwrap().as_deref(), Some("not json"));
    }

    #[test]
    fn scene_config_default_vocabulary() {
        let words: Vec<String> = ["folk", "electronic", "playful"]
        .iter()
        .map(|w| w.to_string())
        .collect();
        assert_eq!(SceneConfig::default().vocabulary, words);
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
