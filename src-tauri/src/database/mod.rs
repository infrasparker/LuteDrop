pub mod autotag_db;
pub mod backup;
pub mod configurations;
pub mod error;
pub mod hierarchy;
pub mod library;
pub mod mood_flags;
pub mod settings;

pub use error::DbError;

use rusqlite::{params, Connection, Result as SqlResult};
use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;

use crate::models::{
    AnalysisStatus, AppState, AutoTag, Clip, Comp, Configuration, EntryAnalysis, EntryType, Group,
    LibraryEntry, MoodCalibrationInfo, MoodValue, MoodVocabulary, Role, Track, TrackMode, TrackTransition,
};

/// Current schema version. Bump when adding a migration below; fresh
/// databases are stamped with this directly.
pub const SCHEMA_VERSION: i64 = 15;

/// Spacing between sibling `sort_order` values, leaving room for insertions
/// between neighbours without renumbering. Shared by hierarchy + configurations.
pub(crate) const SORT_GAP: i64 = 1000;

/// Current time as milliseconds since the Unix epoch, as a string. Shared
/// timestamp source for library entries and configurations.
pub(crate) fn now_millis() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before epoch")
        .as_millis()
        .to_string()
}

/// Index DDL shared by init_schema (fresh DBs) and the v3→v4 migration
/// (existing DBs). IF NOT EXISTS makes running both harmless.
const INDEX_DDL: &str = "
    CREATE INDEX IF NOT EXISTS idx_groups_configuration_id     ON groups(configuration_id);
    CREATE INDEX IF NOT EXISTS idx_comps_group_id              ON comps(group_id);
    CREATE INDEX IF NOT EXISTS idx_tracks_comp_id              ON tracks(comp_id);
    CREATE INDEX IF NOT EXISTS idx_clips_track_id              ON clips(track_id);
    CREATE INDEX IF NOT EXISTS idx_clips_library_entry_id      ON clips(library_entry_id);
    CREATE INDEX IF NOT EXISTS idx_library_entry_tags_tag_id   ON library_entry_tags(tag_id);
    CREATE INDEX IF NOT EXISTS idx_library_entries_content_hash ON library_entries(content_hash);
";

/// A migration upgrades a database from version N to N+1.
pub type Migration = fn(&Connection) -> SqlResult<()>;

fn migrate_v3_to_v4(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(INDEX_DDL)
}

/// The analysis table, shared by init_schema (fresh DBs) and the v4→v5
/// migration. IF NOT EXISTS makes running both harmless.
const ENTRY_ANALYSIS_DDL: &str = "
    CREATE TABLE IF NOT EXISTS entry_analysis (
        library_entry_id TEXT PRIMARY KEY REFERENCES library_entries(id) ON DELETE CASCADE,
        content_hash     TEXT,
        status           TEXT NOT NULL,
        error            TEXT,
        analyzed_at      TEXT,
        scores_json      TEXT
    );
";

/// The v5–v7 rejections table. Schema v8 folds it into
/// `library_entry_tags.user_vote` and drops it, so only the v4→v5 migration
/// creates it.
const V5_TAG_REJECTIONS_DDL: &str = "
    CREATE TABLE IF NOT EXISTS tag_rejections (
        library_entry_id TEXT NOT NULL REFERENCES library_entries(id) ON DELETE CASCADE,
        tag_id           TEXT NOT NULL REFERENCES tags(id)            ON DELETE CASCADE,
        created_at       TEXT NOT NULL,
        PRIMARY KEY (library_entry_id, tag_id)
    );
";

/// The v8 tag table: each row is one tag on one entry, with two independent
/// layers. `auto_score` is the analyzer's (NULL when it does not tag it);
/// `user_vote` is the user's (1 yes, 0 no, NULL not voted). A row with
/// neither is deleted, never kept.
const LIBRARY_ENTRY_TAGS_V8_COLUMNS: &str = "
        library_entry_id TEXT NOT NULL REFERENCES library_entries(id) ON DELETE CASCADE,
        tag_id           TEXT NOT NULL REFERENCES tags(id)            ON DELETE CASCADE,
        auto_score       REAL,
        user_vote        INTEGER,
        PRIMARY KEY (library_entry_id, tag_id),
        CHECK (auto_score IS NOT NULL OR user_vote IS NOT NULL),
        CHECK (user_vote IN (0, 1) OR user_vote IS NULL)
";

/// Records who applied each tag and, for auto tags, the score behind it.
/// Existing tags were all applied by the user.
fn migrate_v4_to_v5(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "ALTER TABLE library_entry_tags ADD COLUMN source TEXT NOT NULL DEFAULT 'user';
         ALTER TABLE library_entry_tags ADD COLUMN score REAL;",
    )?;
    conn.execute_batch(V5_TAG_REJECTIONS_DDL)?;
    conn.execute_batch(ENTRY_ANALYSIS_DDL)
}

/// Adds the combat / non-combat role columns. Existing entries start with no
/// role.
fn migrate_v5_to_v6(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "ALTER TABLE library_entries ADD COLUMN role TEXT;
         ALTER TABLE library_entries ADD COLUMN role_source TEXT;
         ALTER TABLE library_entries ADD COLUMN role_score REAL;",
    )
}

/// Tag renames in the v7 vocabulary cleanup: (old name, new name).
const V7_TAG_RENAMES: [(&str, &str); 3] = [
    ("synth", "electronic"),
    ("frantic", "driving"),
    ("mischievous", "playful"),
];

/// Tags the v7 cleanup removes outright, with their rejections. `bright` and
/// `dark` are removed too, after user ones become a Bright ↔ Dark step.
const V7_TAG_REMOVALS: [&str; 12] = [
    "brass", "choir", "slow", "heroic", "horror", "lively", "peaceful", "ominous", "majestic",
    "moody", "haunting", "somber",
];

/// Adds the pace and mood columns and cleans the tag vocabulary up for the
/// new scales (spec "Sound Vocabulary" §4). Runs inside the migration
/// runner's transaction, so the columns and the cleanup commit together.
fn migrate_v6_to_v7(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "ALTER TABLE library_entries ADD COLUMN pace TEXT;
         ALTER TABLE library_entries ADD COLUMN pace_source TEXT;
         ALTER TABLE library_entries ADD COLUMN pace_score REAL;
         ALTER TABLE library_entries ADD COLUMN mood_bright REAL;
         ALTER TABLE library_entries ADD COLUMN mood_light REAL;
         ALTER TABLE library_entries ADD COLUMN mood_bright_user TEXT;
         ALTER TABLE library_entries ADD COLUMN mood_light_user TEXT;",
    )?;

    for (old, new) in V7_TAG_RENAMES {
        rename_tag(conn, old, new)?;
    }

    // A user `bright` or `dark` tag becomes the user's Bright ↔ Dark step
    // (both at once → the neutral step). Auto ones leave no step. Ambiance
    // carries no mood values.
    conn.execute(
        "UPDATE library_entries SET mood_bright_user =
             CASE WHEN user_moods.bright AND user_moods.dark THEN 'mid'
                  WHEN user_moods.bright THEN 'high'
                  ELSE 'low' END
         FROM (
             SELECT let.library_entry_id AS entry_id,
                    MAX(t.name = 'bright') AS bright,
                    MAX(t.name = 'dark')   AS dark
             FROM library_entry_tags let JOIN tags t ON t.id = let.tag_id
             WHERE let.source = 'user' AND t.name IN ('bright', 'dark')
             GROUP BY let.library_entry_id
         ) AS user_moods
         WHERE library_entries.id = user_moods.entry_id
           AND library_entries.entry_type = 'music'",
        [],
    )?;

    // Removed tags go with their applications and rejections. Deleted
    // explicitly rather than through ON DELETE CASCADE, which only fires
    // when the connection has foreign keys on.
    for name in V7_TAG_REMOVALS.iter().chain(["bright", "dark"].iter()) {
        conn.execute(
            "DELETE FROM library_entry_tags WHERE tag_id IN (SELECT id FROM tags WHERE name = ?1)",
            [name],
        )?;
        conn.execute(
            "DELETE FROM tag_rejections WHERE tag_id IN (SELECT id FROM tags WHERE name = ?1)",
            [name],
        )?;
        conn.execute("DELETE FROM tags WHERE name = ?1", [name])?;
    }
    // The v7-era sweep: a tag row stays while a rejection points at it.
    conn.execute(
        "DELETE FROM tags
         WHERE id NOT IN (SELECT tag_id FROM library_entry_tags)
           AND id NOT IN (SELECT tag_id FROM tag_rejections)",
        [],
    )?;

    settings::reset_saved_scene_config(conn)?;

    // Queue every analyzed music entry for the next run, which computes the
    // new pace features and mood scores and re-scores the new vocabulary.
    conn.execute(
        "UPDATE entry_analysis SET status = ?1, error = NULL, scores_json = NULL
         WHERE library_entry_id IN (SELECT id FROM library_entries WHERE entry_type = 'music')",
        [AnalysisStatus::Pending.as_str()],
    )?;
    Ok(())
}

/// Renames tag `old` to `new` on every entry and rejection. When `new`
/// doesn't exist yet the tag row is simply renamed. Otherwise rows move onto
/// `new`; where an entry already has `new`, one row is kept, as a user tag if
/// either was. An auto tag that lands on a tag the user rejected on that
/// entry is dropped.
fn rename_tag(conn: &Connection, old: &str, new: &str) -> SqlResult<()> {
    use rusqlite::OptionalExtension;
    let tag_id = |name: &str| -> SqlResult<Option<String>> {
        conn.query_row("SELECT id FROM tags WHERE name = ?1", [name], |r| r.get(0))
            .optional()
    };
    let Some(old_id) = tag_id(old)? else {
        return Ok(());
    };
    let Some(new_id) = tag_id(new)? else {
        conn.execute("UPDATE tags SET name = ?2 WHERE id = ?1", params![old_id, new])?;
        return Ok(());
    };

    // Collisions: a user `old` row promotes the entry's `new` row to user,
    // then the `old` row goes.
    conn.execute(
        "UPDATE library_entry_tags SET source = 'user', score = NULL
         WHERE tag_id = ?2 AND library_entry_id IN (
             SELECT library_entry_id FROM library_entry_tags
             WHERE tag_id = ?1 AND source = 'user')",
        params![old_id, new_id],
    )?;
    conn.execute(
        "DELETE FROM library_entry_tags
         WHERE tag_id = ?1 AND library_entry_id IN (
             SELECT library_entry_id FROM library_entry_tags WHERE tag_id = ?2)",
        params![old_id, new_id],
    )?;
    conn.execute(
        "UPDATE library_entry_tags SET tag_id = ?2 WHERE tag_id = ?1",
        params![old_id, new_id],
    )?;

    conn.execute(
        "INSERT OR IGNORE INTO tag_rejections (library_entry_id, tag_id, created_at)
         SELECT library_entry_id, ?2, created_at FROM tag_rejections WHERE tag_id = ?1",
        params![old_id, new_id],
    )?;
    conn.execute("DELETE FROM tag_rejections WHERE tag_id = ?1", [&old_id])?;

    // The user's rejection outranks an auto tag.
    conn.execute(
        "DELETE FROM library_entry_tags
         WHERE tag_id = ?1 AND source = 'auto' AND library_entry_id IN (
             SELECT library_entry_id FROM tag_rejections WHERE tag_id = ?1)",
        [&new_id],
    )?;

    conn.execute("DELETE FROM tags WHERE id = ?1", [&old_id])?;
    Ok(())
}

/// Splits every facet and tag into an analyzer layer and a user layer
/// (spec "Auto / yours"). Runs inside the migration runner's transaction.
///
/// - Role and pace: the single value + source + score columns become
///   `*_auto`, `*_auto_score` and `*_user`. A user-cleared value (NULL value,
///   user source) carries nothing over, so it reads as "use auto".
/// - Tags: `library_entry_tags` is rebuilt with `auto_score` and `user_vote`;
///   `tag_rejections` becomes `user_vote = 0` and is dropped.
/// - Stored scores are cleared, so the next run re-scores every entry and
///   fills the auto layer of entries that were user-set until now.
fn migrate_v7_to_v8(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "ALTER TABLE library_entries ADD COLUMN role_auto TEXT;
         ALTER TABLE library_entries ADD COLUMN role_auto_score REAL;
         ALTER TABLE library_entries ADD COLUMN role_user TEXT;
         ALTER TABLE library_entries ADD COLUMN pace_auto TEXT;
         ALTER TABLE library_entries ADD COLUMN pace_auto_score REAL;
         ALTER TABLE library_entries ADD COLUMN pace_user TEXT;

         UPDATE library_entries SET
             role_user       = CASE WHEN role_source = 'user' THEN role END,
             role_auto       = CASE WHEN role_source = 'auto' THEN role END,
             role_auto_score = CASE WHEN role_source = 'auto' AND role IS NOT NULL THEN role_score END,
             pace_user       = CASE WHEN pace_source = 'user' THEN pace END,
             pace_auto       = CASE WHEN pace_source = 'auto' THEN pace END,
             pace_auto_score = CASE WHEN pace_source = 'auto' AND pace IS NOT NULL THEN pace_score END;

         ALTER TABLE library_entries DROP COLUMN role;
         ALTER TABLE library_entries DROP COLUMN role_source;
         ALTER TABLE library_entries DROP COLUMN role_score;
         ALTER TABLE library_entries DROP COLUMN pace;
         ALTER TABLE library_entries DROP COLUMN pace_source;
         ALTER TABLE library_entries DROP COLUMN pace_score;",
    )?;

    // Rows pointing at a missing entry or tag (possible only if foreign keys
    // were ever off) are left behind rather than failing the copy.
    conn.execute_batch(&format!(
        "CREATE TABLE library_entry_tags_v8 ({LIBRARY_ENTRY_TAGS_V8_COLUMNS});

         -- Anything but 'auto' was a user tag (TagSource::from_str).
         INSERT INTO library_entry_tags_v8 (library_entry_id, tag_id, auto_score, user_vote)
         SELECT library_entry_id, tag_id,
                CASE WHEN source = 'auto' THEN COALESCE(score, 0.0) END,
                CASE WHEN source = 'auto' THEN NULL ELSE 1 END
         FROM library_entry_tags
         WHERE library_entry_id IN (SELECT id FROM library_entries)
           AND tag_id IN (SELECT id FROM tags);

         -- A rejection is a no vote. Where the entry also holds the tag as a
         -- user tag (only a v7 rename merge can cause that), the user tag
         -- wins: it is what the entry showed before the upgrade.
         INSERT INTO library_entry_tags_v8 (library_entry_id, tag_id, user_vote)
         SELECT library_entry_id, tag_id, 0
         FROM tag_rejections
         WHERE library_entry_id IN (SELECT id FROM library_entries)
           AND tag_id IN (SELECT id FROM tags)
         ON CONFLICT (library_entry_id, tag_id) DO UPDATE SET user_vote = 0
         WHERE user_vote IS NULL;

         DROP TABLE library_entry_tags;
         ALTER TABLE library_entry_tags_v8 RENAME TO library_entry_tags;
         DROP TABLE tag_rejections;
         CREATE INDEX IF NOT EXISTS idx_library_entry_tags_tag_id ON library_entry_tags(tag_id);"
    ))?;

    conn.execute("UPDATE entry_analysis SET scores_json = NULL", [])?;
    Ok(())
}

/// Moves mood to three PAD scales (spec "Three-axis mood"): Bright ↔ Dark
/// stays, Intimate ↔ Grand and Calm ↔ Energetic are added, and Light ↔ Heavy
/// and pace are dropped. Runs inside the migration runner's transaction.
///
/// - The user's Bright ↔ Dark steps are kept: that scale's meaning is
///   unchanged. Nothing is carried from `mood_light_user` or `pace_user`
///   (their meanings don't map onto the new scales), so Intimate ↔ Grand and
///   Calm ↔ Energetic start from the analyzer alone. The pre-migration
///   backup keeps the dropped values.
/// - The analyzer's Bright ↔ Dark z-score, role and tags are untouched.
/// - Stored scores are cleared, so the next run re-scores every entry and
///   fills the new z-scores.
fn migrate_v8_to_v9(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "ALTER TABLE library_entries ADD COLUMN mood_grand REAL;
         ALTER TABLE library_entries ADD COLUMN mood_grand_user TEXT;
         ALTER TABLE library_entries ADD COLUMN mood_energy REAL;
         ALTER TABLE library_entries ADD COLUMN mood_energy_user TEXT;

         ALTER TABLE library_entries DROP COLUMN mood_light;
         ALTER TABLE library_entries DROP COLUMN mood_light_user;
         ALTER TABLE library_entries DROP COLUMN pace_auto;
         ALTER TABLE library_entries DROP COLUMN pace_auto_score;
         ALTER TABLE library_entries DROP COLUMN pace_user;

         UPDATE entry_analysis SET scores_json = NULL;",
    )
}

/// Adds the dark kind (spec "Dark kind") and folds the `driving` tag into
/// Calm ↔ Energetic. Runs inside the migration runner's transaction.
///
/// - New columns: the analyzer's Sad and Scary z-scores and the user's kind.
///   The analyzer's kind is derived from the z-scores when loading.
/// - A `driving` yes vote on a music entry becomes the user's Energetic step,
///   unless the user already set a Calm ↔ Energetic step. Every other
///   `driving` row (analyzer-only, or a no vote) just goes, then the tag
///   itself, and it leaves the saved vocabulary.
/// - Stored scores are cleared, so the next run scores the kinds.
fn migrate_v9_to_v10(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "ALTER TABLE library_entries ADD COLUMN mood_sad REAL;
         ALTER TABLE library_entries ADD COLUMN mood_scary REAL;
         ALTER TABLE library_entries ADD COLUMN dark_kind_user TEXT;

         UPDATE library_entries SET mood_energy_user = 'high'
         WHERE mood_energy_user IS NULL
           AND entry_type = 'music'
           AND id IN (
               SELECT let.library_entry_id
               FROM library_entry_tags let JOIN tags t ON t.id = let.tag_id
               WHERE t.name = 'driving' AND let.user_vote = 1);

         DELETE FROM library_entry_tags
         WHERE tag_id IN (SELECT id FROM tags WHERE name = 'driving');
         DELETE FROM tags WHERE name = 'driving';

         UPDATE entry_analysis SET scores_json = NULL;",
    )?;
    settings::remove_from_saved_vocabulary(conn, "driving")
}

/// Replaces the dark kind with mood pools (spec "Mood pools and the Review
/// board"). Runs inside the migration runner's transaction.
///
/// - New columns: the analyzer's suggested pool, its probability and the
///   runner-up, the user's pool, and when the user reviewed the entry. They
///   start empty: nothing carries over into pools.
/// - The dark kind columns go. The three mood scales, auto and user, stay.
/// - Stored scores are cleared, so the next run scores pools.
///
/// The pool calibration (meta key `pool_calibration`) is absent until the
/// first run that covers enough music computes it.
fn migrate_v10_to_v11(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "ALTER TABLE library_entries ADD COLUMN pool_auto TEXT;
         ALTER TABLE library_entries ADD COLUMN pool_auto_p REAL;
         ALTER TABLE library_entries ADD COLUMN pool_auto_runner TEXT;
         ALTER TABLE library_entries ADD COLUMN pool_user TEXT;
         ALTER TABLE library_entries ADD COLUMN pool_reviewed_at TEXT;

         ALTER TABLE library_entries DROP COLUMN mood_sad;
         ALTER TABLE library_entries DROP COLUMN mood_scary;
         ALTER TABLE library_entries DROP COLUMN dark_kind_user;

         UPDATE entry_analysis SET scores_json = NULL;",
    )
}

/// The mood profile table (schema v12), shared by init_schema (fresh DBs)
/// and the v11→v12 migration. One row per (entry, mood) that has either
/// layer: the analyzer's 0..1 value and the user's level, one of five.
const LIBRARY_ENTRY_MOODS_DDL: &str = "
    CREATE TABLE IF NOT EXISTS library_entry_moods (
        library_entry_id TEXT NOT NULL REFERENCES library_entries(id) ON DELETE CASCADE,
        mood             TEXT NOT NULL,
        auto_score       REAL,
        user_level       REAL,
        PRIMARY KEY (library_entry_id, mood),
        CHECK (user_level IS NULL OR user_level IN (0, 0.25, 0.5, 0.75, 1.0))
    );
";

/// The mood flag table (schema v15), shared by init_schema (fresh DBs) and
/// the v14→v15 migration. One flag per (entry, mood); `lines_sig` is the
/// mood's prompt lines joined with newlines when the flag was set or kept.
const MOOD_FLAGS_DDL: &str = "
    CREATE TABLE IF NOT EXISTS mood_flags (
        library_entry_id TEXT NOT NULL REFERENCES library_entries(id) ON DELETE CASCADE,
        mood             TEXT NOT NULL,
        kind             TEXT NOT NULL CHECK (kind IN ('band','higher','lower')),
        band             TEXT CHECK (band IS NULL OR band IN ('definitely','understandable','stretch','no')),
        bound            TEXT NOT NULL DEFAULT 'in' CHECK (bound IN ('in','at_least','at_most')),
        bar_at_flag      REAL,
        calibrated_at    TEXT,
        lines_sig        TEXT NOT NULL,
        created_at       INTEGER NOT NULL,
        PRIMARY KEY (library_entry_id, mood)
    );
";

/// The analyzer's and user's pool columns schema v12 drops.
const V11_POOL_COLUMNS: [&str; 4] = ["pool_auto", "pool_auto_p", "pool_auto_runner", "pool_user"];

/// The mood scale columns (Bright ↔ Dark, Intimate ↔ Grand, Calm ↔
/// Energetic; analyzer and user) schema v12 drops if they are still there.
const V11_SCALE_COLUMNS: [&str; 6] = [
    "mood_bright",
    "mood_bright_user",
    "mood_grand",
    "mood_grand_user",
    "mood_energy",
    "mood_energy_user",
];

/// True if `table` has a column named `column`.
fn column_exists(conn: &Connection, table: &str, column: &str) -> SqlResult<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
        params![table, column],
        |row| row.get(0),
    )
}

/// Replaces mood pools (and the scales) with the mood profile (spec "Moods:
/// lenses and the mood profile"). Runs inside the migration runner's
/// transaction.
///
/// - New table `library_entry_moods`: per entry and mood, the analyzer's
///   0..1 value and the user's level. It starts empty: a confirmed pool is
///   not a level, so nothing carries over.
/// - The pool columns go (suggestion, probability, runner-up, the user's
///   pool), and so do the six scale columns where still present.
/// - `pool_reviewed_at` is kept, renamed `mood_reviewed_at`, so reviewed
///   entries stay out of the Review queue.
/// - The pool calibration is deleted and the stored scores are cleared, so
///   the next run scores moods and takes a new calibration.
/// - The mood vocabulary (meta `mood_vocabulary`) is seeded with the
///   defaults, unless one is already stored.
/// - Role and tags are untouched. The pre-migration backup keeps everything
///   dropped here.
fn migrate_v11_to_v12(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(LIBRARY_ENTRY_MOODS_DDL)?;
    for column in V11_POOL_COLUMNS.into_iter().chain(V11_SCALE_COLUMNS) {
        if column_exists(conn, "library_entries", column)? {
            conn.execute_batch(&format!("ALTER TABLE library_entries DROP COLUMN {column}"))?;
        }
    }
    conn.execute_batch(
        "ALTER TABLE library_entries RENAME COLUMN pool_reviewed_at TO mood_reviewed_at;

         DELETE FROM meta WHERE key = 'pool_calibration';

         UPDATE entry_analysis SET scores_json = NULL;",
    )?;
    seed_mood_vocabulary(conn)
}

/// Stores the default mood vocabulary unless one is stored already. Run by
/// the v11→v12 migration and on a fresh database.
fn seed_mood_vocabulary(conn: &Connection) -> SqlResult<()> {
    let json = settings::mood_vocabulary_json(&MoodVocabulary::defaults())
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
    conn.execute(
        "INSERT OR IGNORE INTO meta (key, value) VALUES (?1, ?2)",
        params![settings::KEY_MOOD_VOCABULARY, json],
    )?;
    Ok(())
}

/// The mood vocabulary exactly as schema v12 seeded it: the JSON
/// `mood_vocabulary_json` wrote for the defaults of that release. v13 compares
/// the stored row with it byte for byte.
const V12_DEFAULT_MOOD_VOCABULARY_JSON: &str = r#"{"moods":[{"id":"peaceful","name":"Peaceful","prompts":["benign, pleasant, agreeable music with no threat","music that signals safety, whatever its tempo","friendly and harmless"]},{"id":"sorrowful","name":"Sorrowful","prompts":["reflective, somber, sorrowful music","sad, mournful, melancholic music","wistful, grieving, nostalgic music"]},{"id":"wondrous","name":"Wondrous","prompts":["music full of wonder and awe","sacred, divine, celestial music, a choir in a cathedral","a breathtaking vista, hopeful and vast, slow and soaring"]},{"id":"joyful","name":"Joyful","prompts":["joyful, glad music, any tempo","bright major-key happiness","contented and sunny"]},{"id":"quirky","name":"Quirky","prompts":["quirky, weird, offbeat music","groovy, funky, eccentric and strange","campy, oddball, mischievous tune"]},{"id":"mysterious","name":"Mysterious","prompts":["mysterious, unsettling, eerie music","strange, uncanny, otherworldly sounds","quiet suspense and looming dread, something is about to happen"]},{"id":"grim","name":"Grim","prompts":["horror","dark","heavy"]},{"id":"heroic","name":"Heroic","prompts":["heroic, triumphant, bold orchestral music","a rousing victory anthem, brass and choir soaring","adventurous, noble, uplifting epic music"]}],"energy":{"positive":["energetic","upbeat","powerful"],"negative":["slow","relaxing","calm"]}}"#;

/// New default prompts: every mood, and each side of Energy, becomes one
/// prompt, a cluster of tag words. Runs inside the migration runner's
/// transaction.
///
/// Only a vocabulary still exactly as v12 seeded it is replaced: the stored
/// row must equal `V12_DEFAULT_MOOD_VOCABULARY_JSON` byte for byte, or be
/// missing (a database without the row ran on the v12 defaults, which the
/// loader falls back to). Then the new defaults are written and what the analyzer said about moods is
/// cleared, since it came from the old prompts: the calibration, every
/// `auto_score` (rows with no user level go) and every entry's stored
/// scores, so the next run recalibrates. User-set levels and review times
/// stay. A vocabulary the user edited (or an unreadable row) is left alone. The pre-migration backup keeps the old row.
fn migrate_v12_to_v13(conn: &Connection) -> SqlResult<()> {
    use rusqlite::OptionalExtension;
    let stored: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            params![settings::KEY_MOOD_VOCABULARY],
            |row| row.get(0),
        )
        .optional()?;
    if stored.as_deref().is_some_and(|s| s != V12_DEFAULT_MOOD_VOCABULARY_JSON) {
        return Ok(());
    }
    let json = settings::mood_vocabulary_json(&MoodVocabulary::defaults())
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        params![settings::KEY_MOOD_VOCABULARY, json],
    )?;
    conn.execute("DELETE FROM meta WHERE key = ?1", params![settings::KEY_MOOD_CALIBRATION])?;
    conn.execute_batch(
        "UPDATE library_entry_moods SET auto_score = NULL;
         DELETE FROM library_entry_moods WHERE user_level IS NULL;
         UPDATE entry_analysis SET scores_json = NULL;",
    )
}

/// The `mood_vocabulary` row exactly as v13 seeded it (the v0.16.1 defaults
/// before the set was settled), for the v14 swap.
const V13_DEFAULT_MOOD_VOCABULARY_JSON: &str = r#"{"moods":[{"id":"peaceful","name":"Peaceful","prompts":["relaxing, positive"]},{"id":"sorrowful","name":"Sorrowful","prompts":["sad, sorrowful, mournful, melancholic, somber, wistful, grieving, nostalgic, reflective music"]},{"id":"wondrous","name":"Wondrous","prompts":["hopeful, inspiring"]},{"id":"joyful","name":"Joyful","prompts":["happy, upbeat"]},{"id":"quirky","name":"Quirky","prompts":["quirky, weird, offbeat, groovy, funky, eccentric, strange, campy, oddball, mischievous music"]},{"id":"mysterious","name":"Mysterious","prompts":["mysterious, unsettling, eerie, strange, uncanny, otherworldly, suspenseful, looming, dread music"]},{"id":"grim","name":"Grim","prompts":["horror, dark, heavy"]},{"id":"heroic","name":"Heroic","prompts":["heroic, triumphant, bold, rousing, victorious, adventurous, noble, uplifting, epic orchestral music"]}],"energy":{"positive":["energetic, upbeat, powerful"],"negative":["slow, relaxing, calm"]}}"#;

/// v13 → v14: the mood bars changed meaning. A mood's bar now reads 0.5 one
/// standard deviation above the library average instead of at the average
/// (Energy keeps the average as its middle), so every stored suggestion is on
/// the old scale. Clear the calibration, the suggested values and the cached
/// scores; the next analysis run takes a calibration and fills every profile
/// again on the new scale. Levels set by the user, the vocabulary, roles,
/// tags and review timestamps stay. A vocabulary still exactly the v13
/// defaults is replaced by the settled set (with Battle); an edited, missing
/// or unreadable one is left alone, like v13 did for v12.
fn migrate_v13_to_v14(conn: &Connection) -> SqlResult<()> {
    use rusqlite::OptionalExtension;
    let stored: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            params![settings::KEY_MOOD_VOCABULARY],
            |row| row.get(0),
        )
        .optional()?;
    if stored.as_deref() == Some(V13_DEFAULT_MOOD_VOCABULARY_JSON) {
        let json = settings::mood_vocabulary_json(&MoodVocabulary::defaults())
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = ?2",
            params![json, settings::KEY_MOOD_VOCABULARY],
        )?;
    }
    conn.execute("DELETE FROM meta WHERE key = ?1", params![settings::KEY_MOOD_CALIBRATION])?;
    conn.execute_batch(
        "UPDATE library_entry_moods SET auto_score = NULL;
         DELETE FROM library_entry_moods WHERE user_level IS NULL;
         UPDATE entry_analysis SET scores_json = NULL;",
    )
}

/// v14 → v15: the `mood_flags` table. Nothing existing changes.
fn migrate_v14_to_v15(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(MOOD_FLAGS_DDL)
}

// Ordered migrations: entry `(n, f)` upgrades version n → n+1.
const MIGRATIONS: &[(i64, Migration)] = &[
    (3, migrate_v3_to_v4),
    (4, migrate_v4_to_v5),
    (5, migrate_v5_to_v6),
    (6, migrate_v6_to_v7),
    (7, migrate_v7_to_v8),
    (8, migrate_v8_to_v9),
    (9, migrate_v9_to_v10),
    (10, migrate_v10_to_v11),
    (11, migrate_v11_to_v12),
    (12, migrate_v12_to_v13),
    (13, migrate_v13_to_v14),
    (14, migrate_v14_to_v15),
];

fn read_schema_version(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT value FROM meta WHERE key = 'schema_version'",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|v| v.parse().ok())
    .unwrap_or(0)
}

pub fn run_migrations(conn: &Connection) -> Result<(), String> {
    run_migrations_with(conn, MIGRATIONS, SCHEMA_VERSION)
}

/// Each migration runs on a connection-scoped `unchecked_transaction`: the
/// migrate fn and the version bump execute on `conn` and commit together, so
/// a failed migration rolls back its version stamp.
fn run_migrations_with(
    conn: &Connection,
    migrations: &[(i64, Migration)],
    target: i64,
) -> Result<(), String> {
    let mut version = read_schema_version(conn);
    if version > target {
        return Err(format!(
            "Database schema is v{version}, newer than this app supports (v{target}). \
             Update LuteDrop before opening this library."
        ));
    }
    for (from, migrate) in migrations {
        if version == *from {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("Migration v{from}: failed to open transaction: {e}"))?;
            migrate(conn).map_err(|e| format!("Migration v{from} → v{} failed: {e}", from + 1))?;
            conn.execute(
                "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
                params![(from + 1).to_string()],
            )
            .map_err(|e| format!("Migration v{from}: failed to bump version: {e}"))?;
            tx.commit()
                .map_err(|e| format!("Migration v{from}: commit failed: {e}"))?;
            version = from + 1;
        }
    }
    Ok(())
}

/// Opens (or creates) the SQLite database at `path`, enables WAL mode and
/// foreign-key enforcement, then runs the schema initialiser.
pub fn open_db(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path)
        .map_err(|e| format!("Failed to open database: {e}"))?;

    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
        .map_err(|e| format!("Failed to set PRAGMAs: {e}"))?;

    // A database without a `meta` table is fresh: init_schema creates the
    // current schema, so stamp it SCHEMA_VERSION directly. An existing
    // database keeps (or is backfilled to) its stored version and is walked
    // forward by run_migrations. All pre-runner databases are v3.
    let fresh: bool = !conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='meta'",
            [],
            |row| row.get::<_, i64>(0).map(|c| c > 0),
        )
        .map_err(|e| format!("Failed to inspect database: {e}"))?;

    // Migrations rewrite the user's library (v7 cleans up their tags, v8
    // splits every facet and tag into auto and user layers, v9 drops the old
    // Light/Heavy and pace values, v10 folds the `driving` tag into
    // Calm ↔ Energetic, v11 drops the dark kind, v12 drops the pools and the
    // mood scales, v13 may replace the default mood words and clear mood
    // suggestions), so an existing database behind the current schema is copied first, before anything changes it. No copy,
    // no migration.
    if !fresh {
        let on_disk = match read_schema_version(&conn) {
            0 => 3, // pre-runner database; see the stamp below
            v => v,
        };
        if on_disk < SCHEMA_VERSION {
            let copy = backup::perform_pre_migration_backup(
                &conn,
                &backup::backup_dir_for(path),
                on_disk,
            )
            .map_err(|e| format!("Not upgrading the database: the pre-upgrade backup failed: {e}"))?;
            log::info!("Pre-migration backup written to {copy:?}");
        }
    }

    init_schema(&conn).map_err(|e| format!("Failed to initialise schema: {e}"))?;

    // A pre-runner database has no schema_version row; it is backfilled to 3
    // (the last pre-migration schema) so run_migrations walks it forward. A
    // fresh DB is stamped SCHEMA_VERSION directly.
    let stamp = if fresh { SCHEMA_VERSION } else { 3 };
    conn.execute(
        "INSERT OR IGNORE INTO meta (key, value) VALUES ('schema_version', ?1)",
        params![stamp.to_string()],
    )
    .map_err(|e| format!("Failed to stamp schema version: {e}"))?;
    if fresh {
        seed_mood_vocabulary(&conn).map_err(|e| format!("Failed to seed the mood vocabulary: {e}"))?;
    }

    run_migrations(&conn)?;

    Ok(conn)
}

/// Creates all application tables (idempotent – uses `IF NOT EXISTS`).
pub fn init_schema(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS library_entries (
            id           TEXT PRIMARY KEY,
            file_path    TEXT NOT NULL UNIQUE,
            content_hash TEXT,
            name         TEXT NOT NULL,
            duration     REAL,
            sample_rate  INTEGER,
            file_size    INTEGER,
            volume       REAL NOT NULL DEFAULT 1.0,
            trim_start   REAL,
            trim_end     REAL,
            loop_start   REAL,
            loop_end     REAL,
            created_at   TEXT NOT NULL,
            entry_type   TEXT NOT NULL DEFAULT 'music',
            role_auto        TEXT,
            role_auto_score  REAL,
            role_user        TEXT,
            mood_reviewed_at TEXT
        );

        CREATE TABLE IF NOT EXISTS tags (
            id   TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE
        );

        CREATE TABLE IF NOT EXISTS configurations (
            id         TEXT PRIMARY KEY,
            name       TEXT NOT NULL,
            created_at TEXT NOT NULL,
            sort_order INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS groups (
            id               TEXT PRIMARY KEY,
            name             TEXT NOT NULL,
            sort_order       INTEGER NOT NULL,
            configuration_id TEXT NOT NULL DEFAULT '' REFERENCES configurations(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS comps (
            id         TEXT PRIMARY KEY,
            group_id   TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
            name       TEXT NOT NULL,
            sort_order INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS tracks (
            id                 TEXT PRIMARY KEY,
            comp_id            TEXT NOT NULL REFERENCES comps(id) ON DELETE CASCADE,
            name               TEXT NOT NULL,
            volume             REAL NOT NULL DEFAULT 1.0,
            mode_type          TEXT NOT NULL DEFAULT 'sequence',
            repeat             INTEGER DEFAULT 0,
            transition_type    TEXT DEFAULT 'gapless',
            transition_gap_min REAL,
            transition_gap_max REAL,
            fade_in            REAL NOT NULL DEFAULT 0.0,
            fade_out           REAL NOT NULL DEFAULT 0.0,
            sort_order         INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS clips (
            id               TEXT PRIMARY KEY,
            track_id         TEXT NOT NULL REFERENCES tracks(id)          ON DELETE CASCADE,
            library_entry_id TEXT NOT NULL REFERENCES library_entries(id),
            volume           REAL NOT NULL DEFAULT 1.0,
            sort_order       INTEGER NOT NULL
        );
    ")?;
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS library_entry_tags ({LIBRARY_ENTRY_TAGS_V8_COLUMNS});"
    ))?;
    conn.execute_batch(ENTRY_ANALYSIS_DDL)?;
    conn.execute_batch(LIBRARY_ENTRY_MOODS_DDL)?;
    conn.execute_batch(MOOD_FLAGS_DDL)?;
    conn.execute_batch(INDEX_DDL)
}

/// Ensures at least one configuration exists and active_configuration_id is set.
/// Called after init_schema for fresh databases that skip the v1→v2 migration.
pub fn ensure_default_configuration(conn: &Connection) -> Result<(), String> {
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM configurations", [], |row| row.get(0))
        .map_err(|e| format!("Failed to count configurations: {e}"))?;

    if count > 0 {
        // Also ensure active_configuration_id is set
        let has_active: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM meta WHERE key = 'active_configuration_id'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);
        if !has_active {
            let first_id: String = conn
                .query_row("SELECT id FROM configurations ORDER BY sort_order LIMIT 1", [], |row| row.get(0))
                .map_err(|e| format!("Failed to get first configuration: {e}"))?;
            conn.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('active_configuration_id', ?1)",
                params![first_id],
            ).map_err(|e| format!("Failed to set active configuration: {e}"))?;
        }
        return Ok(());
    }

    use std::time::{SystemTime, UNIX_EPOCH};
    let config_id = Uuid::new_v4().to_string();
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before epoch")
        .as_millis()
        .to_string();

    conn.execute(
        "INSERT INTO configurations (id, name, created_at, sort_order) VALUES (?1, 'Default', ?2, 1000)",
        params![config_id, created_at],
    ).map_err(|e| format!("Failed to insert default configuration: {e}"))?;

    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('active_configuration_id', ?1)",
        params![config_id],
    ).map_err(|e| format!("Failed to set active configuration: {e}"))?;

    Ok(())
}

// ─── State Loading ────────────────────────────────────────────────────────────

/// Reads the active configuration ID from the meta table.
pub fn get_active_configuration_id(conn: &Connection) -> Result<String, String> {
    conn.query_row(
        "SELECT value FROM meta WHERE key = 'active_configuration_id'",
        [],
        |row| row.get(0),
    )
    .map_err(|e| format!("Failed to read active_configuration_id: {e}"))
}

/// Loads all configurations ordered by sort_order.
fn load_configurations(conn: &Connection) -> Result<Vec<Configuration>, String> {
    let mut stmt = conn
        .prepare("SELECT id, name, created_at FROM configurations ORDER BY sort_order ASC")
        .map_err(|e| format!("Failed to prepare load_configurations: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(Configuration {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
            })
        })
        .map_err(|e| format!("Failed to query configurations: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect configurations: {e}"))?;

    Ok(rows)
}

/// Builds the complete nested AppState from SQLite.
/// Called after every mutation to return the updated state to the frontend.
pub fn load_full_state(conn: &Connection) -> Result<AppState, String> {
    let active_configuration_id = get_active_configuration_id(conn)?;
    let configurations = load_configurations(conn)?;
    let mood_vocabulary = settings::load_mood_vocabulary(conn)?;
    let calibration = settings::load_mood_calibration(conn)?;
    let library_entries = load_library_entries(conn, &mood_vocabulary)?;
    let tags = library::list_all_tags(conn)?;

    // Build id → entry reference map for lookups in hierarchy loading.
    let entry_map: HashMap<String, &LibraryEntry> = library_entries
        .iter()
        .map(|e| (e.id.clone(), e))
        .collect();

    let groups = load_groups(conn, &active_configuration_id, &entry_map)?;

    Ok(AppState {
        groups,
        library_entries,
        tags,
        configurations,
        active_configuration_id,
        missing_entry_ids: Vec::new(),
        moods_need_recalibration: crate::autotag::plan::needs_recalibration(calibration.as_ref(), &mood_vocabulary),
        mood_calibration: calibration.map(|c| MoodCalibrationInfo {
            calibrated_at: c.calibrated_at,
            tracks: c.tracks as u32,
            source: match c.source {
                crate::autotag::plan::CalibrationSource::Library => "library",
                crate::autotag::plan::CalibrationSource::Reference => "reference",
            }
            .to_string(),
            reference_set: c.reference_set,
        }),
        mood_vocabulary,
        mood_vocabulary_unreadable: settings::mood_vocabulary_problem(conn)?.is_some(),
    })
}

/// Queries all library_entries ordered by name and fills in tags, auto tags,
/// rejected tags, analysis state and the mood profile (in `vocabulary`'s
/// order) for each.
fn load_library_entries(conn: &Connection, vocabulary: &MoodVocabulary) -> Result<Vec<LibraryEntry>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, file_path, content_hash, name, duration, sample_rate, file_size,
                    volume, trim_start, trim_end, loop_start, loop_end, created_at, entry_type,
                    role_auto, role_auto_score, role_user, mood_reviewed_at
             FROM library_entries
             ORDER BY name ASC",
        )
        .map_err(|e| format!("Failed to prepare load_library_entries: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,           // id
                row.get::<_, String>(1)?,           // file_path
                row.get::<_, Option<String>>(2)?,   // content_hash
                row.get::<_, String>(3)?,           // name
                row.get::<_, Option<f64>>(4)?,      // duration
                row.get::<_, Option<i64>>(5)?,      // sample_rate (INTEGER → i64)
                row.get::<_, Option<i64>>(6)?,      // file_size   (INTEGER → i64)
                row.get::<_, f64>(7)?,              // volume
                row.get::<_, Option<f64>>(8)?,      // trim_start
                row.get::<_, Option<f64>>(9)?,      // trim_end
                row.get::<_, Option<f64>>(10)?,     // loop_start
                row.get::<_, Option<f64>>(11)?,     // loop_end
                row.get::<_, String>(12)?,          // created_at
                row.get::<_, String>(13)?,          // entry_type
                (
                    row.get::<_, Option<String>>(14)?, // role_auto
                    row.get::<_, Option<f64>>(15)?,    // role_auto_score
                    row.get::<_, Option<String>>(16)?, // role_user
                ),
                row.get::<_, Option<String>>(17)?, // mood_reviewed_at
            ))
        })
        .map_err(|e| format!("Failed to query library_entries: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect library_entries: {e}"))?;

    // All entry→tag rows in one query, sorted into the shown tags, the ones
    // shown because of the analyzer, and the ones the user voted no on.
    #[derive(Default)]
    struct EntryTags {
        shown: Vec<String>,
        auto: Vec<AutoTag>,
        rejected: Vec<String>,
    }
    let mut tag_map: HashMap<String, EntryTags> = HashMap::new();
    {
        let mut tstmt = conn
            .prepare(
                "SELECT let.library_entry_id, t.name, let.auto_score, let.user_vote
                 FROM library_entry_tags let
                 JOIN tags t ON t.id = let.tag_id
                 ORDER BY t.name ASC",
            )
            .map_err(|e| format!("Failed to prepare tag map query: {e}"))?;
        let tag_rows = tstmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<f64>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            })
            .map_err(|e| format!("Failed to query tag map: {e}"))?;
        for row in tag_rows {
            let (entry_id, name, auto_score, user_vote) =
                row.map_err(|e| format!("Failed to read tag row: {e}"))?;
            let tags = tag_map.entry(entry_id).or_default();
            match (user_vote, auto_score) {
                (Some(0), _) => tags.rejected.push(name),
                (Some(_), _) => tags.shown.push(name),
                (None, Some(score)) => {
                    tags.auto.push(AutoTag { tag: name.clone(), score });
                    tags.shown.push(name);
                }
                // The table's CHECK rules this out.
                (None, None) => {}
            }
        }
    }

    // Analysis state per entry.
    let mut analysis_map: HashMap<String, EntryAnalysis> = HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT library_entry_id, status, analyzed_at, error FROM entry_analysis")
            .map_err(|e| format!("Failed to prepare analysis query: {e}"))?;
        let analysis_rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    EntryAnalysis {
                        status: AnalysisStatus::from_str(&row.get::<_, String>(1)?),
                        scored_at: row.get(2)?,
                        error: row.get(3)?,
                    },
                ))
            })
            .map_err(|e| format!("Failed to query analysis: {e}"))?;
        for row in analysis_rows {
            let (entry_id, analysis) = row.map_err(|e| format!("Failed to read analysis row: {e}"))?;
            analysis_map.insert(entry_id, analysis);
        }
    }

    // Mood rows per entry: (auto_score, user_level) by mood. Rows with an
    // id outside the vocabulary are ignored.
    type MoodLayers = HashMap<String, (Option<f64>, Option<f64>)>;
    let mut mood_map: HashMap<String, MoodLayers> = HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT library_entry_id, mood, auto_score, user_level FROM library_entry_moods")
            .map_err(|e| format!("Failed to prepare mood query: {e}"))?;
        let mood_rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<f64>>(2)?,
                    row.get::<_, Option<f64>>(3)?,
                ))
            })
            .map_err(|e| format!("Failed to query moods: {e}"))?;
        for row in mood_rows {
            let (entry_id, mood, auto, user) = row.map_err(|e| format!("Failed to read mood row: {e}"))?;
            if vocabulary.contains(&mood) {
                mood_map.entry(entry_id).or_default().insert(mood, (auto, user));
            }
        }
    }

    let mut flags = mood_flags::load_flags(conn, vocabulary)?;

    let mood_ids = vocabulary.ids();
    let mut entries = Vec::with_capacity(rows.len());
    for (id, file_path, content_hash, name, duration, sample_rate, file_size,
         volume, trim_start, trim_end, loop_start, loop_end, created_at, entry_type,
         (role_auto, role_auto_score, role_user), mood_reviewed_at)
        in rows
    {
        let tags = tag_map.remove(&id).unwrap_or_default();
        let analysis = analysis_map.remove(&id);
        let role_auto = role_auto.as_deref().and_then(Role::from_db_str);
        let role_user = role_user.as_deref().and_then(Role::from_db_str);
        let entry_type = EntryType::from_str(&entry_type);
        let stored = mood_map.remove(&id).unwrap_or_default();
        let moods = mood_ids
            .iter()
            .map(|mood| {
                let (auto, user) = stored.get(mood).copied().unwrap_or((None, None));
                let flag = flags.remove(&(id.clone(), mood.clone()));
                MoodValue { mood: mood.clone(), auto, user, flag }
            })
            .collect();
        entries.push(LibraryEntry {
            id,
            file_path,
            content_hash,
            name,
            duration,
            sample_rate: sample_rate.map(|v| v as u32),
            file_size: file_size.map(|v| v as u64),
            volume,
            trim_start,
            trim_end,
            loop_start,
            loop_end,
            created_at,
            tags: tags.shown,
            role: crate::models::effective_role(&entry_type, role_user, role_auto),
            entry_type,
            auto_tags: tags.auto,
            rejected_tags: tags.rejected,
            analysis,
            role_auto,
            // A score without a role says nothing the UI can show.
            role_auto_score: role_auto.and(role_auto_score),
            role_user,
            moods,
            mood_reviewed_at,
        });
    }

    Ok(entries)
}

/// Queries all groups/comps/tracks/clips for the given configuration_id in
/// four flat queries (one per level) instead of a per-parent N+1 cascade,
/// then assembles the nested hierarchy in memory.
fn load_groups(
    conn: &Connection,
    configuration_id: &str,
    entry_map: &HashMap<String, &LibraryEntry>,
) -> Result<Vec<Group>, String> {
    // One query per level, each scoped to the configuration and globally
    // ordered by sort_order — pushing rows into per-parent vecs in query
    // order preserves the per-parent ordering.
    let group_rows: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare("SELECT id, name FROM groups WHERE configuration_id = ?1 ORDER BY sort_order ASC")
            .map_err(|e| format!("Failed to prepare groups query: {e}"))?;
        let rows = stmt
            .query_map(params![configuration_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| format!("Failed to query groups: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to collect groups: {e}"))?;
        rows
    };

    // comps grouped by group_id
    let mut comps_by_group: HashMap<String, Vec<(String, String)>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT c.id, c.group_id, c.name
                 FROM comps c
                 JOIN groups g ON g.id = c.group_id
                 WHERE g.configuration_id = ?1
                 ORDER BY c.sort_order ASC",
            )
            .map_err(|e| format!("Failed to prepare comps query: {e}"))?;
        let rows = stmt
            .query_map(params![configuration_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            })
            .map_err(|e| format!("Failed to query comps: {e}"))?;
        for row in rows {
            let (id, group_id, name) = row.map_err(|e| format!("Failed to read comp row: {e}"))?;
            comps_by_group.entry(group_id).or_default().push((id, name));
        }
    }

    // tracks grouped by comp_id (same tuple shape the old load_tracks used)
    type TrackRow = (String, String, f64, String, Option<i64>, Option<String>, Option<f64>, Option<f64>, f64, f64);
    let mut tracks_by_comp: HashMap<String, Vec<TrackRow>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT t.id, t.comp_id, t.name, t.volume, t.mode_type, t.repeat, t.transition_type,
                        t.transition_gap_min, t.transition_gap_max, t.fade_in, t.fade_out
                 FROM tracks t
                 JOIN comps c ON c.id = t.comp_id
                 JOIN groups g ON g.id = c.group_id
                 WHERE g.configuration_id = ?1
                 ORDER BY t.sort_order ASC",
            )
            .map_err(|e| format!("Failed to prepare tracks query: {e}"))?;
        let rows = stmt
            .query_map(params![configuration_id], |row| {
                Ok((
                    row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,
                    row.get::<_, f64>(3)?, row.get::<_, String>(4)?, row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<String>>(6)?, row.get::<_, Option<f64>>(7)?,
                    row.get::<_, Option<f64>>(8)?, row.get::<_, f64>(9)?, row.get::<_, f64>(10)?,
                ))
            })
            .map_err(|e| format!("Failed to query tracks: {e}"))?;
        for row in rows {
            let (id, comp_id, name, volume, mode_type, repeat, ttype, gmin, gmax, fin, fout) =
                row.map_err(|e| format!("Failed to read track row: {e}"))?;
            tracks_by_comp
                .entry(comp_id)
                .or_default()
                .push((id, name, volume, mode_type, repeat, ttype, gmin, gmax, fin, fout));
        }
    }

    // clips grouped by track_id
    let mut clips_by_track: HashMap<String, Vec<(String, String, f64)>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT cl.id, cl.track_id, cl.library_entry_id, cl.volume
                 FROM clips cl
                 JOIN tracks t ON t.id = cl.track_id
                 JOIN comps c ON c.id = t.comp_id
                 JOIN groups g ON g.id = c.group_id
                 WHERE g.configuration_id = ?1
                 ORDER BY cl.sort_order ASC",
            )
            .map_err(|e| format!("Failed to prepare clips query: {e}"))?;
        let rows = stmt
            .query_map(params![configuration_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, f64>(3)?))
            })
            .map_err(|e| format!("Failed to query clips: {e}"))?;
        for row in rows {
            let (id, track_id, library_entry_id, volume) =
                row.map_err(|e| format!("Failed to read clip row: {e}"))?;
            clips_by_track.entry(track_id).or_default().push((id, library_entry_id, volume));
        }
    }

    // Assemble, resolving clip fields from entry_map exactly as before.
    let mut groups = Vec::with_capacity(group_rows.len());
    for (group_id, group_name) in group_rows {
        let comp_rows = comps_by_group.remove(&group_id).unwrap_or_default();
        let mut comps = Vec::with_capacity(comp_rows.len());
        for (comp_id, comp_name) in comp_rows {
            let track_rows = tracks_by_comp.remove(&comp_id).unwrap_or_default();
            let mut tracks = Vec::with_capacity(track_rows.len());
            for (track_id, name, volume, mode_type, repeat_int, ttype, gmin, gmax, fade_in, fade_out) in track_rows {
                let clip_rows = clips_by_track.remove(&track_id).unwrap_or_default();
                let clips: Vec<Clip> = clip_rows
                    .into_iter()
                    .map(|(id, library_entry_id, volume)| {
                        let (name, file_path, library_volume, trim_start, trim_end) =
                            match entry_map.get(&library_entry_id) {
                                Some(entry) => (entry.name.clone(), entry.file_path.clone(), entry.volume, entry.trim_start, entry.trim_end),
                                None => ("[missing]".to_string(), String::new(), 1.0, None, None),
                            };
                        Clip { id, name, file_path, volume, library_entry_id, library_volume, trim_start, trim_end }
                    })
                    .collect();
                let repeat = repeat_int.map(|v| v != 0).unwrap_or(false);
                let ttype_str = ttype.as_deref().unwrap_or("gapless");
                let mode = build_track_mode(&mode_type, repeat, ttype_str, gmin.unwrap_or(0.0), gmax.unwrap_or(0.0), &clips, entry_map);
                tracks.push(Track { id: track_id, name, clips, volume, mode, fade_in, fade_out });
            }
            comps.push(Comp { id: comp_id, name: comp_name, tracks });
        }
        groups.push(Group { id: group_id, name: group_name, comps });
    }
    Ok(groups)
}

/// Builds a TrackMode from the raw DB columns plus the already-loaded clips.
fn build_track_mode(
    mode_type: &str,
    repeat: bool,
    transition_type: &str,
    gap_min: f64,
    gap_max: f64,
    clips: &[Clip],
    entry_map: &HashMap<String, &LibraryEntry>,
) -> TrackMode {
    // Loop points are SNAPSHOTTED from the first clip's library entry at load
    // time — editing the entry's markers changes track behavior only after
    // the next state reload (deliberate; the engine never re-reads the DB
    // mid-playback).
    let seamless_loops = clips
        .first()
        .and_then(|clip| entry_map.get(&clip.library_entry_id))
        .and_then(|entry| {
            if let (Some(ls), Some(le)) = (entry.loop_start, entry.loop_end) {
                Some((ls, le))
            } else {
                None
            }
        })
        .unwrap_or((0.0, 0.0));
    track_mode_from_parts(mode_type, repeat, transition_type, gap_min, gap_max, seamless_loops)
}

/// Builds the non-Seamless arms (Sequence/Shuffle) shared by both loaders, and
/// the Seamless arm from already-resolved `(loop_start, loop_end)`. Callers
/// differ only in how they source the seamless loop points.
fn track_mode_from_parts(
    mode_type: &str,
    repeat: bool,
    transition_type: &str,
    gap_min: f64,
    gap_max: f64,
    seamless_loops: (f64, f64),
) -> TrackMode {
    match mode_type {
        "seamless" => TrackMode::Seamless { loop_start: seamless_loops.0, loop_end: seamless_loops.1 },
        "shuffle" => TrackMode::Shuffle { repeat, transition: build_transition(transition_type, gap_min, gap_max) },
        _ => TrackMode::Sequence { repeat, transition: build_transition(transition_type, gap_min, gap_max) },
    }
}

/// Builds a TrackTransition from raw DB column values.
fn build_transition(transition_type: &str, gap_min: f64, gap_max: f64) -> TrackTransition {
    match transition_type {
        "gap" => TrackTransition::Gap { min: gap_min, max: gap_max },
        "crossfade" => TrackTransition::Crossfade,
        _ => TrackTransition::Gapless,
    }
}

// ─── Targeted Play-Path Lookups ───────────────────────────────────────────────

/// Row shape shared by the two targeted loaders: a Clip plus the referenced
/// entry's loop markers (needed only for Seamless mode's first clip).
type ClipWithLoops = (Clip, Option<f64>, Option<f64>);

/// `where_sql` is a `&'static str` chosen from exactly two hardcoded literals
/// at the only two call sites below — no user data reaches `format!`.
fn load_clips_resolved(
    conn: &Connection,
    where_sql: &'static str, // "cl.track_id = ?1" | "cl.track_id IN (SELECT id FROM tracks WHERE comp_id = ?1)"
    param: &str,
) -> Result<Vec<(String, ClipWithLoops)>, String> {
    let sql = format!(
        "SELECT cl.id, cl.track_id, cl.library_entry_id, cl.volume,
                le.name, le.file_path, le.volume, le.trim_start, le.trim_end,
                le.loop_start, le.loop_end
         FROM clips cl
         LEFT JOIN library_entries le ON le.id = cl.library_entry_id
         WHERE {where_sql}
         ORDER BY cl.sort_order ASC"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("Failed to prepare resolved clips query: {e}"))?;
    let rows = stmt
        .query_map(params![param], |row| {
            Ok((
                row.get::<_, String>(0)?,          // clip id
                row.get::<_, String>(1)?,          // track_id
                row.get::<_, String>(2)?,          // library_entry_id
                row.get::<_, f64>(3)?,             // clip volume
                row.get::<_, Option<String>>(4)?,  // entry name (None when entry missing)
                row.get::<_, Option<String>>(5)?,  // file_path
                row.get::<_, Option<f64>>(6)?,     // library volume
                row.get::<_, Option<f64>>(7)?,     // trim_start
                row.get::<_, Option<f64>>(8)?,     // trim_end
                row.get::<_, Option<f64>>(9)?,     // loop_start
                row.get::<_, Option<f64>>(10)?,    // loop_end
            ))
        })
        .map_err(|e| format!("Failed to query resolved clips: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect resolved clips: {e}"))?;

    Ok(rows
        .into_iter()
        .map(|(id, track_id, library_entry_id, volume, name, file_path, lib_vol, ts, te, ls, le)| {
            let clip = Clip {
                id,
                name: name.unwrap_or_else(|| "[missing]".to_string()),
                file_path: file_path.unwrap_or_default(),
                volume,
                library_entry_id,
                library_volume: lib_vol.unwrap_or(1.0),
                trim_start: ts,
                trim_end: te,
            };
            (track_id, (clip, ls, le))
        })
        .collect())
}

/// Builds a Track from a raw track row plus its resolved clips.
/// Mirrors `build_track_mode`, but takes the first clip's loop markers
/// directly (already joined) instead of an entry_map.
fn assemble_track(
    row: (String, String, f64, String, Option<i64>, Option<String>, Option<f64>, Option<f64>, f64, f64),
    clips_with_loops: Vec<ClipWithLoops>,
) -> Track {
    let (id, name, volume, mode_type, repeat_int, ttype, gmin, gmax, fade_in, fade_out) = row;
    let first_loops = clips_with_loops.first().map(|(_, ls, le)| (*ls, *le));
    let clips: Vec<Clip> = clips_with_loops.into_iter().map(|(c, _, _)| c).collect();
    let repeat = repeat_int.map(|v| v != 0).unwrap_or(false);
    let ttype_str = ttype.as_deref().unwrap_or("gapless");
    // See build_track_mode's note on the seamless loop points being a
    // load-time snapshot; here they arrive already joined instead of via an
    // entry_map lookup.
    let seamless_loops = match first_loops {
        Some((Some(ls), Some(le))) => (ls, le),
        _ => (0.0, 0.0),
    };
    let mode = track_mode_from_parts(&mode_type, repeat, ttype_str, gmin.unwrap_or(0.0), gmax.unwrap_or(0.0), seamless_loops);
    Track { id, name, clips, volume, mode, fade_in, fade_out }
}

const TRACK_COLUMNS: &str =
    "id, name, volume, mode_type, repeat, transition_type, transition_gap_min, transition_gap_max, fade_in, fade_out";

/// Loads a single track by id with its clips, without loading the rest of
/// the app state. Used on the play/pause/stop path.
pub fn load_track_by_id(conn: &Connection, track_id: &str) -> Result<Track, String> {
    use rusqlite::OptionalExtension;
    let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE id = ?1");
    let row = conn
        .query_row(&sql, params![track_id], |row| {
            Ok((
                row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, f64>(2)?,
                row.get::<_, String>(3)?, row.get::<_, Option<i64>>(4)?, row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<f64>>(6)?, row.get::<_, Option<f64>>(7)?,
                row.get::<_, f64>(8)?, row.get::<_, f64>(9)?,
            ))
        })
        .optional()
        .map_err(|e| format!("Failed to query track: {e}"))?
        .ok_or_else(|| format!("Track not found: {}", track_id))?;
    let clips = load_clips_resolved(conn, "cl.track_id = ?1", track_id)?
        .into_iter()
        .map(|(_, cwl)| cwl)
        .collect();
    Ok(assemble_track(row, clips))
}

/// Loads all tracks for a comp with their clips, without loading the rest of
/// the app state. Used on the play/pause/stop path.
pub fn load_tracks_for_comp(conn: &Connection, comp_id: &str) -> Result<Vec<Track>, String> {
    use rusqlite::OptionalExtension;
    let exists: Option<String> = conn
        .query_row("SELECT id FROM comps WHERE id = ?1", params![comp_id], |r| r.get(0))
        .optional()
        .map_err(|e| format!("Failed to check comp existence: {e}"))?;
    if exists.is_none() {
        return Err(format!("Comp not found: {}", comp_id));
    }

    let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE comp_id = ?1 ORDER BY sort_order ASC");
    let mut stmt = conn.prepare(&sql).map_err(|e| format!("Failed to prepare comp tracks query: {e}"))?;
    let rows = stmt
        .query_map(params![comp_id], |row| {
            Ok((
                row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, f64>(2)?,
                row.get::<_, String>(3)?, row.get::<_, Option<i64>>(4)?, row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<f64>>(6)?, row.get::<_, Option<f64>>(7)?,
                row.get::<_, f64>(8)?, row.get::<_, f64>(9)?,
            ))
        })
        .map_err(|e| format!("Failed to query comp tracks: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect comp tracks: {e}"))?;

    let mut clips_by_track: HashMap<String, Vec<ClipWithLoops>> = HashMap::new();
    for (track_id, cwl) in load_clips_resolved(
        conn,
        "cl.track_id IN (SELECT id FROM tracks WHERE comp_id = ?1)",
        comp_id,
    )? {
        clips_by_track.entry(track_id).or_default().push(cwl);
    }

    Ok(rows
        .into_iter()
        .map(|row| {
            let clips = clips_by_track.remove(&row.0).unwrap_or_default();
            assemble_track(row, clips)
        })
        .collect())
}

// ─────────────────────────────────────────────────────────────────────────────

/// Opens an in-memory SQLite database with the full schema applied.
/// Intended for unit tests only.
#[cfg(test)]
pub fn open_memory_db() -> Connection {
    let conn = Connection::open_in_memory().expect("failed to open in-memory db");
    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .expect("failed to enable foreign keys");
    init_schema(&conn).expect("failed to init schema");
    conn
}

#[cfg(test)]
pub fn ensure_test_config(conn: &Connection) -> String {
    let config_id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT OR IGNORE INTO configurations (id, name, created_at, sort_order) VALUES (?1, 'Test', '0', 1000)",
        params![config_id],
    ).expect("insert test config");
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('active_configuration_id', ?1)",
        params![config_id],
    ).expect("set active config");
    config_id
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index_names(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='index' AND name LIKE 'idx_%' ORDER BY name")
            .unwrap();
        stmt.query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap()
    }

    const EXPECTED_INDEXES: [&str; 7] = [
        "idx_clips_library_entry_id",
        "idx_clips_track_id",
        "idx_comps_group_id",
        "idx_groups_configuration_id",
        "idx_library_entries_content_hash",
        "idx_library_entry_tags_tag_id",
        "idx_tracks_comp_id",
    ];

    #[test]
    fn fresh_database_has_indexes_and_current_stamp() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_db(&dir.path().join("fresh.db")).unwrap();
        assert_eq!(index_names(&conn), EXPECTED_INDEXES);
        assert_eq!(read_schema_version(&conn), SCHEMA_VERSION);
    }

    #[test]
    fn v3_database_gains_indexes_via_migration() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        // init_schema builds the current shape; start from v7's and take the
        // v4-v7 additions away too, so every later step has work to do.
        make_v7_shape(&conn);
        // Simulate a genuine pre-index v3 database.
        for name in EXPECTED_INDEXES {
            conn.execute_batch(&format!("DROP INDEX IF EXISTS {name}")).unwrap();
        }
        drop_v7_columns(&conn);
        conn.execute_batch(
            "ALTER TABLE library_entries DROP COLUMN role;
             ALTER TABLE library_entries DROP COLUMN role_source;
             ALTER TABLE library_entries DROP COLUMN role_score;
             DROP TABLE tag_rejections;
             DROP TABLE entry_analysis;
             DROP TABLE library_entry_tags;
             CREATE TABLE library_entry_tags (
                 library_entry_id TEXT NOT NULL REFERENCES library_entries(id) ON DELETE CASCADE,
                 tag_id           TEXT NOT NULL REFERENCES tags(id)            ON DELETE CASCADE,
                 PRIMARY KEY (library_entry_id, tag_id)
             );",
        )
        .unwrap();
        conn.execute("INSERT INTO meta (key, value) VALUES ('schema_version', '3')", []).unwrap();
        run_migrations(&conn).unwrap();
        assert_eq!(index_names(&conn), EXPECTED_INDEXES);
        assert_eq!(read_schema_version(&conn), SCHEMA_VERSION);
    }

    #[test]
    fn schema_creates_all_tables() {
        let conn = open_memory_db();

        let expected_tables = [
            "meta",
            "library_entries",
            "tags",
            "library_entry_tags",
            "configurations",
            "groups",
            "comps",
            "tracks",
            "clips",
            "library_entry_moods",
        ];

        for table in &expected_tables {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get(0),
                )
                .expect("query failed");
            assert_eq!(count, 1, "table '{}' should exist", table);
        }
    }

    #[test]
    fn schema_is_idempotent() {
        let conn = open_memory_db();
        // Running init_schema a second time must not return an error.
        init_schema(&conn).expect("second call to init_schema should succeed");
    }

    #[test]
    fn schema_version_is_set() {
        // The stamp now happens in open_db (fresh-database detection), not in
        // init_schema's batch, so exercise open_db on a real temp-file DB.
        let dir = tempfile::tempdir().expect("create temp dir");
        let db_path = dir.path().join("lutedrop.db");
        let conn = open_db(&db_path).expect("open_db should succeed on fresh db");
        let version: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |row| row.get(0),
            )
            .expect("schema_version row should exist");
        assert_eq!(version, SCHEMA_VERSION.to_string());
    }

    #[test]
    fn migrations_step_version_and_apply_in_order() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        // init_schema no longer stamps a version (open_db does) — insert one.
        conn.execute("INSERT INTO meta (key, value) VALUES ('schema_version', '1')", []).unwrap();

        fn mig1(c: &Connection) -> rusqlite::Result<()> {
            c.execute_batch("CREATE TABLE mig_one (id TEXT)")
        }
        fn mig2(c: &Connection) -> rusqlite::Result<()> {
            c.execute_batch("CREATE TABLE mig_two (id TEXT)")
        }
        run_migrations_with(&conn, &[(1, mig1 as Migration), (2, mig2 as Migration)], 3).unwrap();

        assert_eq!(read_schema_version(&conn), 3);
        // Both tables exist:
        conn.execute("INSERT INTO mig_one VALUES ('x')", []).unwrap();
        conn.execute("INSERT INTO mig_two VALUES ('x')", []).unwrap();
        // Idempotent: running again is a no-op.
        run_migrations_with(&conn, &[(1, mig1 as Migration), (2, mig2 as Migration)], 3).unwrap();
    }

    #[test]
    fn newer_database_than_app_is_rejected() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn.execute("INSERT INTO meta (key, value) VALUES ('schema_version', '99')", []).unwrap();
        assert!(run_migrations_with(&conn, &[], 3).is_err());
    }

    #[test]
    fn load_full_state_empty_db() {
        let conn = open_memory_db();
        ensure_test_config(&conn);
        let state = load_full_state(&conn).expect("load_full_state should succeed on empty DB");
        assert!(state.groups.is_empty(), "groups should be empty");
        assert!(state.library_entries.is_empty(), "library_entries should be empty");
        assert!(state.tags.is_empty(), "tags should be empty");
        assert_eq!(state.configurations.len(), 1);
    }

    #[test]
    fn load_full_state_with_data() {
        let conn = open_memory_db();

        // Insert a library entry with loop points and tags.
        let entry_id = library::insert_library_entry(
            &conn,
            "/tmp/theme.mp3",
            Some("hash-abc"),
            "Main Theme",
            Some(180.0),
            Some(44100),
            Some(2048),
            "music",
        )
        .expect("insert library entry");

        // Set loop points on the entry.
        library::update_library_entry_loop(&conn, &entry_id, Some(1.5), Some(30.0))
            .expect("update loop");

        // Add tags.
        library::add_tag_to_entry(&conn, &entry_id, "ambient").expect("add ambient");
        library::add_tag_to_entry(&conn, &entry_id, "boss").expect("add boss");

        // Build hierarchy: group → comp → track → clip.
        let config_id = ensure_test_config(&conn);
        let group_id = hierarchy::insert_group(&conn, "Combat", &config_id).expect("insert group");
        let comp_id = hierarchy::insert_comp(&conn, &group_id, "Act 1").expect("insert comp");
        let track_id = hierarchy::insert_track(&conn, &comp_id, "Battle Theme").expect("insert track");
        let _clip_id = hierarchy::insert_clip(&conn, &track_id, &entry_id).expect("insert clip");

        // Set track to seamless mode so loop points are resolved.
        hierarchy::update_track_mode(&conn, &track_id, "seamless", None, None, None, None)
            .expect("set seamless mode");

        // Load full state.
        let state = load_full_state(&conn).expect("load_full_state should succeed");

        // Verify library entries.
        assert_eq!(state.library_entries.len(), 1);
        let entry = &state.library_entries[0];
        assert_eq!(entry.name, "Main Theme");
        assert_eq!(entry.tags, vec!["ambient", "boss"]);
        assert_eq!(entry.loop_start, Some(1.5));
        assert_eq!(entry.loop_end, Some(30.0));

        // Verify all-tags list.
        assert_eq!(state.tags, vec!["ambient", "boss"]);

        // Verify group hierarchy.
        assert_eq!(state.groups.len(), 1);
        let group = &state.groups[0];
        assert_eq!(group.name, "Combat");
        assert_eq!(group.comps.len(), 1, "group should have one comp");

        let comp = &group.comps[0];
        assert_eq!(comp.name, "Act 1");
        assert_eq!(comp.tracks.len(), 1, "comp should have one track");

        let track = &comp.tracks[0];
        assert_eq!(track.name, "Battle Theme");
        assert_eq!(track.clips.len(), 1, "track should have one clip");

        // Verify clip resolution.
        let clip = &track.clips[0];
        assert_eq!(clip.name, "Main Theme");
        assert_eq!(clip.file_path, "/tmp/theme.mp3");
        assert_eq!(clip.library_entry_id, entry_id);

        // Verify seamless mode resolved loop points from library entry.
        match &track.mode {
            crate::models::TrackMode::Seamless { loop_start, loop_end } => {
                assert!(
                    (loop_start - 1.5).abs() < 1e-9,
                    "loop_start should be 1.5, got {loop_start}"
                );
                assert!(
                    (loop_end - 30.0).abs() < 1e-9,
                    "loop_end should be 30.0, got {loop_end}"
                );
            }
            other => panic!("expected Seamless mode, got {other:?}"),
        }
    }

    #[test]
    fn load_full_state_scoped_by_config() {
        let conn = open_memory_db();
        let config_id = ensure_test_config(&conn);

        let config2_id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO configurations (id, name, created_at, sort_order) VALUES (?1, 'Other', '0', 2000)",
            params![config2_id],
        ).expect("insert config2");

        conn.execute(
            "INSERT INTO groups (id, name, sort_order, configuration_id) VALUES ('g1', 'Default Group', 1000, ?1)",
            params![config_id],
        ).expect("insert group in default config");

        conn.execute(
            "INSERT INTO groups (id, name, sort_order, configuration_id) VALUES ('g2', 'Other Group', 1000, ?1)",
            params![config2_id],
        ).expect("insert group in other config");

        let state = load_full_state(&conn).expect("load state");
        assert_eq!(state.groups.len(), 1, "should only see active config's groups");
        assert_eq!(state.groups[0].name, "Default Group");
        assert_eq!(state.active_configuration_id, config_id);
        assert_eq!(state.configurations.len(), 2, "should see all configurations");
    }

    #[test]
    fn load_full_state_returns_ordered_nested_hierarchy_with_tags() {
        let conn = open_memory_db();
        let config_id = ensure_test_config(&conn);

        let e1 = library::insert_library_entry(&conn, "/a.mp3", Some("h1"), "Alpha", None, None, None, "music").unwrap();
        let e2 = library::insert_library_entry(&conn, "/b.mp3", Some("h2"), "Beta", None, None, None, "ambiance").unwrap();
        library::add_tag_to_entry(&conn, &e1, "battle").unwrap();
        library::add_tag_to_entry(&conn, &e1, "ambient").unwrap();
        library::add_tag_to_entry(&conn, &e2, "battle").unwrap();

        let g1 = hierarchy::insert_group(&conn, "G1", &config_id).unwrap();
        let g2 = hierarchy::insert_group(&conn, "G2", &config_id).unwrap();
        let c1 = hierarchy::insert_comp(&conn, &g1, "C1").unwrap();
        let c2 = hierarchy::insert_comp(&conn, &g1, "C2").unwrap();
        let t1 = hierarchy::insert_track(&conn, &c1, "T1").unwrap();
        let t2 = hierarchy::insert_track(&conn, &c1, "T2").unwrap();
        hierarchy::insert_clip(&conn, &t1, &e1).unwrap();
        hierarchy::insert_clip(&conn, &t1, &e2).unwrap();

        let state = load_full_state(&conn).unwrap();

        assert_eq!(state.groups.len(), 2);
        assert_eq!(state.groups[0].name, "G1");
        assert_eq!(state.groups[1].name, "G2");
        assert_eq!(state.groups[0].comps.len(), 2);
        assert_eq!(state.groups[0].comps[0].name, "C1");
        assert_eq!(state.groups[0].comps[1].name, "C2");
        let tracks = &state.groups[0].comps[0].tracks;
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].name, "T1");
        assert_eq!(tracks[1].name, "T2");
        assert_eq!(tracks[0].clips.len(), 2);
        assert_eq!(tracks[0].clips[0].name, "Alpha");
        assert_eq!(tracks[0].clips[1].name, "Beta");
        // Clip resolves entry fields:
        assert_eq!(tracks[0].clips[0].file_path, "/a.mp3");
        // Entries are name-ordered with sorted tags:
        assert_eq!(state.library_entries[0].name, "Alpha");
        assert_eq!(state.library_entries[0].tags, vec!["ambient", "battle"]);
        assert_eq!(state.library_entries[1].tags, vec!["battle"]);
        assert_eq!(state.tags, vec!["ambient", "battle"]);
        // Second comp/track/group are empty but present:
        assert!(state.groups[0].comps[1].tracks.is_empty());
        assert!(state.groups[1].comps.is_empty());
        let _ = (t2, c2, g2);
    }

    #[test]
    fn targeted_track_lookup_matches_full_state() {
        let conn = open_memory_db();
        let config_id = ensure_test_config(&conn);
        let e1 = library::insert_library_entry(&conn, "/x.mp3", None, "X", None, None, None, "music").unwrap();
        let g = hierarchy::insert_group(&conn, "G", &config_id).unwrap();
        let c = hierarchy::insert_comp(&conn, &g, "C").unwrap();
        let t1 = hierarchy::insert_track(&conn, &c, "T1").unwrap();
        let t2 = hierarchy::insert_track(&conn, &c, "T2").unwrap();
        hierarchy::insert_clip(&conn, &t1, &e1).unwrap();

        let state = load_full_state(&conn).unwrap();
        let full_t1 = state.groups[0].comps[0].tracks[0].clone();

        let direct = load_track_by_id(&conn, &t1).unwrap();
        assert_eq!(serde_json::to_value(&direct).unwrap(), serde_json::to_value(&full_t1).unwrap());

        let comp_tracks = load_tracks_for_comp(&conn, &c).unwrap();
        assert_eq!(comp_tracks.len(), 2);
        assert_eq!(comp_tracks[0].id, t1);
        assert_eq!(comp_tracks[1].id, t2);
        assert_eq!(comp_tracks[0].clips.len(), 1);
        assert_eq!(comp_tracks[0].clips[0].file_path, "/x.mp3");

        assert!(load_track_by_id(&conn, "nope").unwrap_err().contains("Track not found"));
        assert!(load_tracks_for_comp(&conn, "nope").unwrap_err().contains("Comp not found"));
    }

    fn table_exists(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
            == 1
    }

    fn has_column(conn: &Connection, table: &str, column: &str) -> bool {
        conn.prepare(&format!("SELECT {column} FROM {table}")).is_ok()
    }

    /// The pool columns schema v11 adds to `library_entries` (v12 drops the
    /// first four and renames the review time).
    const V11_ALL_POOL_COLUMNS: [&str; 5] =
        ["pool_auto", "pool_auto_p", "pool_auto_runner", "pool_user", "pool_reviewed_at"];

    /// The mood scale columns schema v12 drops from `library_entries`.
    const V11_MOOD_COLUMNS: [&str; 6] = V11_SCALE_COLUMNS;

    /// Turns a database `init_schema` just built into the v11 shape: the
    /// three mood scales, auto and user, beside the pools, and no mood table.
    fn make_v11_shape(conn: &Connection) {
        conn.execute_batch(
            "DROP TABLE library_entry_moods;
             ALTER TABLE library_entries RENAME COLUMN mood_reviewed_at TO pool_reviewed_at;
             ALTER TABLE library_entries ADD COLUMN pool_auto TEXT;
             ALTER TABLE library_entries ADD COLUMN pool_auto_p REAL;
             ALTER TABLE library_entries ADD COLUMN pool_auto_runner TEXT;
             ALTER TABLE library_entries ADD COLUMN pool_user TEXT;
             ALTER TABLE library_entries ADD COLUMN mood_bright REAL;
             ALTER TABLE library_entries ADD COLUMN mood_bright_user TEXT;
             ALTER TABLE library_entries ADD COLUMN mood_grand REAL;
             ALTER TABLE library_entries ADD COLUMN mood_grand_user TEXT;
             ALTER TABLE library_entries ADD COLUMN mood_energy REAL;
             ALTER TABLE library_entries ADD COLUMN mood_energy_user TEXT;",
        )
        .unwrap();
    }

    /// Turns a database `init_schema` just built into the v10 shape of
    /// `library_entries`: mood scales and dark kind columns, no pool columns.
    fn make_v10_shape(conn: &Connection) {
        make_v11_shape(conn);
        for column in V11_ALL_POOL_COLUMNS {
            conn.execute_batch(&format!("ALTER TABLE library_entries DROP COLUMN {column}"))
                .unwrap();
        }
        conn.execute_batch(
            "ALTER TABLE library_entries ADD COLUMN mood_sad REAL;
             ALTER TABLE library_entries ADD COLUMN mood_scary REAL;
             ALTER TABLE library_entries ADD COLUMN dark_kind_user TEXT;",
        )
        .unwrap();
    }

    /// Turns a database `init_schema` just built into the v9 shape of
    /// `library_entries`: no dark kind or pool columns.
    fn make_v9_shape(conn: &Connection) {
        make_v10_shape(conn);
        conn.execute_batch(
            "ALTER TABLE library_entries DROP COLUMN mood_sad;
             ALTER TABLE library_entries DROP COLUMN mood_scary;
             ALTER TABLE library_entries DROP COLUMN dark_kind_user;",
        )
        .unwrap();
    }

    /// Turns a database `init_schema` just built into the v8 shape of
    /// `library_entries`: Light ↔ Heavy and pace layers, no Intimate ↔ Grand
    /// or Calm ↔ Energetic columns.
    fn make_v8_shape(conn: &Connection) {
        make_v9_shape(conn);
        conn.execute_batch(
            "ALTER TABLE library_entries DROP COLUMN mood_grand;
             ALTER TABLE library_entries DROP COLUMN mood_grand_user;
             ALTER TABLE library_entries DROP COLUMN mood_energy;
             ALTER TABLE library_entries DROP COLUMN mood_energy_user;
             ALTER TABLE library_entries ADD COLUMN pace_auto TEXT;
             ALTER TABLE library_entries ADD COLUMN pace_auto_score REAL;
             ALTER TABLE library_entries ADD COLUMN pace_user TEXT;
             ALTER TABLE library_entries ADD COLUMN mood_light REAL;
             ALTER TABLE library_entries ADD COLUMN mood_light_user TEXT;",
        )
        .unwrap();
    }

    /// Turns a database `init_schema` just built into the v7 shape of the
    /// tables v8 changes: single-value role and pace columns with a source and
    /// a score, the v5 tag table, and `tag_rejections`.
    fn make_v7_shape(conn: &Connection) {
        make_v8_shape(conn);
        conn.execute_batch(
            "ALTER TABLE library_entries DROP COLUMN role_auto;
             ALTER TABLE library_entries DROP COLUMN role_auto_score;
             ALTER TABLE library_entries DROP COLUMN role_user;
             ALTER TABLE library_entries DROP COLUMN pace_auto;
             ALTER TABLE library_entries DROP COLUMN pace_auto_score;
             ALTER TABLE library_entries DROP COLUMN pace_user;
             ALTER TABLE library_entries ADD COLUMN role TEXT;
             ALTER TABLE library_entries ADD COLUMN role_source TEXT;
             ALTER TABLE library_entries ADD COLUMN role_score REAL;
             ALTER TABLE library_entries ADD COLUMN pace TEXT;
             ALTER TABLE library_entries ADD COLUMN pace_source TEXT;
             ALTER TABLE library_entries ADD COLUMN pace_score REAL;
             DROP TABLE library_entry_tags;
             CREATE TABLE library_entry_tags (
                 library_entry_id TEXT NOT NULL REFERENCES library_entries(id) ON DELETE CASCADE,
                 tag_id           TEXT NOT NULL REFERENCES tags(id)            ON DELETE CASCADE,
                 source           TEXT NOT NULL DEFAULT 'user',
                 score            REAL,
                 PRIMARY KEY (library_entry_id, tag_id));
             CREATE INDEX IF NOT EXISTS idx_library_entry_tags_tag_id ON library_entry_tags(tag_id);",
        )
        .unwrap();
        conn.execute_batch(V5_TAG_REJECTIONS_DDL).unwrap();
    }

    #[test]
    fn v4_database_migrates_tags_to_user_votes() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        // The v4 shape of the tables the migration touches.
        conn.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE library_entries (
                 id TEXT PRIMARY KEY, file_path TEXT NOT NULL UNIQUE, content_hash TEXT,
                 name TEXT NOT NULL, duration REAL, sample_rate INTEGER, file_size INTEGER,
                 volume REAL NOT NULL DEFAULT 1.0, trim_start REAL, trim_end REAL,
                 loop_start REAL, loop_end REAL, created_at TEXT NOT NULL,
                 entry_type TEXT NOT NULL DEFAULT 'music');
             CREATE TABLE tags (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE);
             CREATE TABLE library_entry_tags (
                 library_entry_id TEXT NOT NULL REFERENCES library_entries(id) ON DELETE CASCADE,
                 tag_id           TEXT NOT NULL REFERENCES tags(id)            ON DELETE CASCADE,
                 PRIMARY KEY (library_entry_id, tag_id));
             INSERT INTO meta (key, value) VALUES ('schema_version', '4');
             INSERT INTO library_entries (id, file_path, name, created_at) VALUES ('e1', '/a.mp3', 'A', '0');
             INSERT INTO tags (id, name) VALUES ('t1', 'folk'), ('t2', 'boss');
             INSERT INTO library_entry_tags (library_entry_id, tag_id) VALUES ('e1', 't1'), ('e1', 't2');",
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        assert_eq!(read_schema_version(&conn), SCHEMA_VERSION);
        let rows: Vec<(Option<f64>, Option<i64>)> = conn
            .prepare("SELECT auto_score, user_vote FROM library_entry_tags")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows, vec![(None, Some(1)), (None, Some(1))]);
        assert!(!table_exists(&conn, "tag_rejections"), "v8 folds rejections into votes");
        assert!(table_exists(&conn, "entry_analysis"));
    }

    #[test]
    fn fresh_database_has_autotag_tables() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_db(&dir.path().join("fresh.db")).unwrap();
        assert_eq!(read_schema_version(&conn), SCHEMA_VERSION);
        assert!(table_exists(&conn, "entry_analysis"));
        assert!(!table_exists(&conn, "tag_rejections"));
        // The tag layer columns exist on a fresh database.
        conn.prepare("SELECT auto_score, user_vote FROM library_entry_tags").unwrap();
        assert!(!has_column(&conn, "library_entry_tags", "source"));
    }

    #[test]
    fn library_entry_exposes_tag_layers_and_analysis() {
        use crate::models::{AnalysisStatus, AutoTag};

        let conn = open_memory_db();
        let with_data = library::insert_library_entry(&conn, "/a.mp3", None, "A", None, None, None, "music").unwrap();
        let untouched = library::insert_library_entry(&conn, "/b.mp3", None, "B", None, None, None, "music").unwrap();
        library::add_tag_to_entry(&conn, &with_data, "folk").unwrap();
        let auto = |tag: &str, score: f64| AutoTag { tag: tag.into(), score };
        library::apply_auto_tags(
            &conn,
            &with_data,
            &[auto("brass", 0.82), auto("dark", 0.7), auto("folk", 0.9), auto("drums", 0.6)],
        )
        .unwrap();
        library::reject_auto_tags(&conn, std::slice::from_ref(&with_data), Some(&["dark".to_string()])).unwrap();
        library::keep_auto_tags(&conn, std::slice::from_ref(&with_data), Some(&["drums".to_string()])).unwrap();
        autotag_db::set_analysis(&conn, &with_data, Some("h1"), AnalysisStatus::Failed, Some("bad file")).unwrap();

        let entries = load_library_entries(&conn, &MoodVocabulary::defaults()).unwrap();
        let a = entries.iter().find(|e| e.id == with_data).unwrap();
        assert_eq!(a.tags, vec!["brass", "drums", "folk"]);
        // Only brass is shown because of the analyzer alone; folk and drums
        // are the user's even though the analyzer scores them too.
        assert_eq!(a.auto_tags, vec![auto("brass", 0.82)]);
        assert_eq!(a.rejected_tags, vec!["dark"]);
        let analysis = a.analysis.as_ref().unwrap();
        assert_eq!(analysis.status, AnalysisStatus::Failed);
        assert_eq!(analysis.error.as_deref(), Some("bad file"));

        let b = entries.iter().find(|e| e.id == untouched).unwrap();
        assert!(b.tags.is_empty() && b.auto_tags.is_empty() && b.rejected_tags.is_empty());
        assert!(b.analysis.is_none());
    }

    #[test]
    fn v5_database_migrates_to_current_with_empty_layers() {
        let conn = Connection::open_in_memory().unwrap();
        // The v5 shape of library_entries: no role columns.
        conn.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE library_entries (
                 id TEXT PRIMARY KEY, file_path TEXT NOT NULL UNIQUE, content_hash TEXT,
                 name TEXT NOT NULL, duration REAL, sample_rate INTEGER, file_size INTEGER,
                 volume REAL NOT NULL DEFAULT 1.0, trim_start REAL, trim_end REAL,
                 loop_start REAL, loop_end REAL, created_at TEXT NOT NULL,
                 entry_type TEXT NOT NULL DEFAULT 'music');
             CREATE TABLE tags (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE);
             CREATE TABLE library_entry_tags (
                 library_entry_id TEXT NOT NULL REFERENCES library_entries(id) ON DELETE CASCADE,
                 tag_id           TEXT NOT NULL REFERENCES tags(id)            ON DELETE CASCADE,
                 source           TEXT NOT NULL DEFAULT 'user',
                 score            REAL,
                 PRIMARY KEY (library_entry_id, tag_id));
             INSERT INTO meta (key, value) VALUES ('schema_version', '5');
             INSERT INTO library_entries (id, file_path, name, created_at) VALUES ('e1', '/a.mp3', 'A', '0');",
        )
        .unwrap();
        conn.execute_batch(V5_TAG_REJECTIONS_DDL).unwrap();
        conn.execute_batch(ENTRY_ANALYSIS_DDL).unwrap();

        run_migrations(&conn).unwrap();

        assert_eq!(read_schema_version(&conn), SCHEMA_VERSION);
        let row: (Option<String>, Option<f64>, Option<String>) = conn
            .query_row(
                "SELECT role_auto, role_auto_score, role_user FROM library_entries WHERE id = 'e1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(row, (None, None, None));
    }

    #[test]
    fn fresh_database_is_current() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_db(&dir.path().join("fresh.db")).unwrap();
        assert_eq!(read_schema_version(&conn), SCHEMA_VERSION);
        conn.prepare(
            "SELECT role_auto, role_auto_score, role_user, mood_reviewed_at FROM library_entries",
        )
        .unwrap();
        conn.prepare("SELECT library_entry_id, mood, auto_score, user_level FROM library_entry_moods")
            .unwrap();
        // The default mood vocabulary is seeded, with the v13 words.
        let stored: String = conn
            .query_row("SELECT value FROM meta WHERE key = 'mood_vocabulary'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored, settings::mood_vocabulary_json(&MoodVocabulary::defaults()).unwrap());
        assert_ne!(stored, V12_DEFAULT_MOOD_VOCABULARY_JSON);
        let seeded = serde_json::from_str::<MoodVocabulary>(&stored).unwrap();
        let prompts = |id: &str| seeded.moods.iter().find(|m| m.id == id).unwrap().prompts.clone();
        assert_eq!(prompts("peaceful"), ["relaxing, positive"]);
        assert_eq!(prompts("grim"), ["ominous, dread, fearful, scary, horror, terror, sinister, eldritch, unnatural music"]);
        assert_eq!(prompts("battle"), ["combat, battle, warlike, fierce, clash, war drums, fantasy battle music"]);
        assert!(seeded.moods.iter().all(|m| m.prompts.len() == 1));
        assert_eq!(seeded.energy.positive, ["energetic, exciting, intense, upbeat, powerful"]);
        for old in [
            "role", "role_source", "role_score", "pace", "pace_source", "pace_score",
            "pace_auto", "pace_auto_score", "pace_user", "mood_light", "mood_light_user",
            "mood_sad", "mood_scary", "dark_kind_user",
        ]
        .into_iter()
        .chain(V11_MOOD_COLUMNS)
        .chain(V11_ALL_POOL_COLUMNS)
        {
            assert!(!has_column(&conn, "library_entries", old), "{old} should be gone");
        }
        // Nothing to protect on a fresh database, so no pre-migration copy.
        assert!(!dir.path().join("backups").exists());
    }

    #[test]
    fn library_entry_exposes_role_layers() {
        use crate::models::Role;

        let conn = open_memory_db();
        let auto = library::insert_library_entry(&conn, "/a.mp3", None, "A", None, None, None, "music").unwrap();
        let both = library::insert_library_entry(&conn, "/b.mp3", None, "B", None, None, None, "music").unwrap();
        let user = library::insert_library_entry(&conn, "/c.mp3", None, "C", None, None, None, "music").unwrap();
        let unset = library::insert_library_entry(&conn, "/d.mp3", None, "D", None, None, None, "music").unwrap();
        library::apply_auto_role(&conn, &auto, Some(Role::Combat), 0.9).unwrap();
        library::apply_auto_role(&conn, &both, Some(Role::Combat), 0.8).unwrap();
        library::set_user_role(&conn, &[both.clone(), user.clone()], Some(Role::NonCombat)).unwrap();

        let entries = load_library_entries(&conn, &MoodVocabulary::defaults()).unwrap();
        let get = |id: &str| entries.iter().find(|e| e.id == id).unwrap();
        let layers = |id: &str| {
            let e = get(id);
            (e.role, e.role_auto, e.role_auto_score, e.role_user)
        };
        assert_eq!(layers(&auto), (Some(Role::Combat), Some(Role::Combat), Some(0.9), None));
        // The user's role is shown; the analyzer's stays visible beside it.
        assert_eq!(
            layers(&both),
            (Some(Role::NonCombat), Some(Role::Combat), Some(0.8), Some(Role::NonCombat))
        );
        assert_eq!(layers(&user), (Some(Role::NonCombat), None, None, Some(Role::NonCombat)));
        // Neither layer: Non-combat by default, with no layer behind it.
        assert_eq!(layers(&unset), (Some(Role::NonCombat), None, None, None));
        // Ambiance has no role.
        let amb = library::insert_library_entry(&conn, "/e.ogg", None, "E", None, None, None, "ambiance").unwrap();
        let entries = load_library_entries(&conn, &MoodVocabulary::defaults()).unwrap();
        assert_eq!(entries.iter().find(|e| e.id == amb).unwrap().role, None);
    }

    /// The columns schema v7 adds to `library_entries`.
    const V7_COLUMNS: [&str; 7] = [
        "pace",
        "pace_source",
        "pace_score",
        "mood_bright",
        "mood_light",
        "mood_bright_user",
        "mood_light_user",
    ];

    fn drop_v7_columns(conn: &Connection) {
        for column in V7_COLUMNS {
            conn.execute_batch(&format!("ALTER TABLE library_entries DROP COLUMN {column}"))
                .unwrap();
        }
    }

    /// Writes a file-backed database in the v11 shape, stamped version 11.
    fn create_v11_db(path: &Path) {
        let conn = Connection::open(path).unwrap();
        init_schema(&conn).unwrap();
        make_v11_shape(&conn);
        conn.execute("INSERT INTO meta (key, value) VALUES ('schema_version', '11')", []).unwrap();
    }

    /// Copies the database at `path` aside and walks the copy up to
    /// `target` only, for checking what an intermediate step left behind
    /// (values a later migration drops). The original is not touched.
    fn migrated_copy(path: &Path, target: i64) -> Connection {
        let copy = path.with_extension(format!("v{target}.db"));
        std::fs::copy(path, &copy).unwrap();
        let conn = Connection::open(&copy).unwrap();
        let steps: Vec<(i64, Migration)> =
            MIGRATIONS.iter().copied().filter(|(from, _)| *from < target).collect();
        run_migrations_with(&conn, &steps, target).unwrap();
        assert_eq!(read_schema_version(&conn), target);
        conn
    }

    /// Writes a file-backed database in the v10 shape, stamped version 10.
    fn create_v10_db(path: &Path) {
        let conn = Connection::open(path).unwrap();
        init_schema(&conn).unwrap();
        make_v10_shape(&conn);
        conn.execute("INSERT INTO meta (key, value) VALUES ('schema_version', '10')", []).unwrap();
    }

    /// Writes a file-backed database in the v9 shape, stamped version 9.
    fn create_v9_db(path: &Path) {
        let conn = Connection::open(path).unwrap();
        init_schema(&conn).unwrap();
        make_v9_shape(&conn);
        conn.execute("INSERT INTO meta (key, value) VALUES ('schema_version', '9')", []).unwrap();
    }

    /// Writes a file-backed database in the v8 shape, stamped version 8.
    fn create_v8_db(path: &Path) {
        let conn = Connection::open(path).unwrap();
        init_schema(&conn).unwrap();
        make_v8_shape(&conn);
        conn.execute("INSERT INTO meta (key, value) VALUES ('schema_version', '8')", []).unwrap();
    }

    /// Writes a file-backed database in the v7 shape, stamped version 7.
    fn create_v7_db(path: &Path) {
        let conn = Connection::open(path).unwrap();
        init_schema(&conn).unwrap();
        make_v7_shape(&conn);
        conn.execute("INSERT INTO meta (key, value) VALUES ('schema_version', '7')", []).unwrap();
    }

    /// Writes a file-backed database in the v6 shape: the v7 shape without
    /// the v7 columns, stamped version 6.
    fn create_v6_db(path: &Path) {
        let conn = Connection::open(path).unwrap();
        init_schema(&conn).unwrap();
        make_v7_shape(&conn);
        drop_v7_columns(&conn);
        conn.execute("INSERT INTO meta (key, value) VALUES ('schema_version', '6')", []).unwrap();
    }

    /// (tag name, layer) pairs on an entry, sorted by name. The layer is
    /// "user" (yes vote), "no" (no vote) or "auto" (analyzer only).
    fn entry_tags(conn: &Connection, entry_id: &str) -> Vec<(String, String)> {
        conn.prepare(
            "SELECT t.name, CASE let.user_vote WHEN 1 THEN 'user' WHEN 0 THEN 'no' ELSE 'auto' END
             FROM library_entry_tags let
             JOIN tags t ON t.id = let.tag_id
             WHERE let.library_entry_id = ?1 ORDER BY t.name",
        )
        .unwrap()
        .query_map([entry_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    }

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    fn mood_bright_user(conn: &Connection, entry_id: &str) -> Option<String> {
        conn.query_row(
            "SELECT mood_bright_user FROM library_entries WHERE id = ?1",
            [entry_id],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn mood_energy_user(conn: &Connection, entry_id: &str) -> Option<String> {
        conn.query_row(
            "SELECT mood_energy_user FROM library_entries WHERE id = ?1",
            [entry_id],
            |r| r.get(0),
        )
        .unwrap()
    }

    /// The pre-migration copies taken of a database at `version`.
    fn pre_migration_backups(dir: &Path, version: i64) -> Vec<std::path::PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir.join("backups")) else {
            return Vec::new();
        };
        let prefix = format!("pre-migration-v{version}-");
        entries
            .map(|e| e.unwrap().path())
            .filter(|p| {
                let name = p.file_name().unwrap().to_string_lossy().to_string();
                name.starts_with(&prefix) && name.ends_with(".db")
            })
            .collect()
    }

    /// A v6 library walks through v7 (vocabulary cleanup) and v8 (layers).
    #[test]
    fn v6_database_migrates_with_vocabulary_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lutedrop.db");
        create_v6_db(&path);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                r#"INSERT INTO library_entries (id, file_path, name, created_at, entry_type) VALUES
                     ('A', '/a.mp3', 'A', '0', 'music'), ('B', '/b.mp3', 'B', '0', 'music'),
                     ('C', '/c.mp3', 'C', '0', 'music'), ('D', '/d.mp3', 'D', '0', 'music'),
                     ('E', '/e.mp3', 'E', '0', 'music'), ('R', '/r.ogg', 'Rain', '0', 'ambiance');
                   INSERT INTO tags (id, name) VALUES
                     ('t-synth', 'synth'), ('t-brass', 'brass'), ('t-bright', 'bright'),
                     ('t-heroic', 'heroic'), ('t-skyrim', 'skyrim'), ('t-dark', 'dark'),
                     ('t-mischievous', 'mischievous'), ('t-playful', 'playful'),
                     ('t-frantic', 'frantic'), ('t-driving', 'driving'), ('t-choir', 'choir');
                   INSERT INTO library_entry_tags (library_entry_id, tag_id, source, score) VALUES
                     ('A', 't-synth', 'user', NULL), ('A', 't-brass', 'user', NULL),
                     ('A', 't-bright', 'user', NULL), ('A', 't-heroic', 'user', NULL),
                     ('A', 't-skyrim', 'user', NULL),
                     ('B', 't-bright', 'user', NULL), ('B', 't-dark', 'user', NULL),
                     ('C', 't-dark', 'auto', 0.9), ('C', 't-mischievous', 'user', NULL),
                     ('C', 't-playful', 'user', NULL), ('C', 't-frantic', 'auto', 0.8),
                     ('D', 't-frantic', 'user', NULL), ('D', 't-driving', 'auto', 0.7),
                     ('E', 't-mischievous', 'auto', 0.75);
                   INSERT INTO tag_rejections (library_entry_id, tag_id, created_at) VALUES
                     ('A', 't-choir', '1'), ('C', 't-mischievous', '1'), ('E', 't-playful', '1');
                   INSERT INTO entry_analysis (library_entry_id, content_hash, status, analyzed_at, scores_json) VALUES
                     ('A', 'h-a', 'embedded', '1', '{"stale":1}'), ('D', 'h-d', 'failed', '1', NULL),
                     ('R', 'h-r', 'embedded', '1', '{"keep":1}');
                   INSERT INTO meta (key, value) VALUES ('autotag_scene_encounter',
                     '{"vocabulary":["brass","folk","choir","synth","bright","dark","mischievous"]}');"#,
            )
            .unwrap();
        }

        // The mood scale steps the tags turn into along the way (v7, v10)
        // are dropped again in v12, so check them on a copy walked to v11.
        {
            let v11 = migrated_copy(&path, 11);
            // A's user `bright` became a user step in v7 (see
            // v7_turns_user_bright_dark_tags_into_steps), which v9 keeps.
            assert_eq!(mood_bright_user(&v11, "A").as_deref(), Some("high"));
            // B: user `bright` and user `dark` at once give the mid step.
            assert_eq!(mood_bright_user(&v11, "B").as_deref(), Some("mid"));
            // C: an auto `dark` leaves no step.
            assert_eq!(mood_bright_user(&v11, "C"), None);
            assert_eq!(mood_energy_user(&v11, "C"), None);
            // D: a user `frantic` colliding with an auto `driving` kept it as
            // a user tag in v7, so v10 turns it into an Energetic step.
            assert_eq!(mood_energy_user(&v11, "D").as_deref(), Some("high"));
        }

        let conn = open_db(&path).unwrap();
        assert_eq!(read_schema_version(&conn), SCHEMA_VERSION);
        for column in V11_MOOD_COLUMNS {
            assert!(!has_column(&conn, "library_entries", column), "{column} should be gone");
        }

        // A: synth renamed, removed tags gone (its user `bright` became a
        // step, since dropped).
        assert_eq!(entry_tags(&conn, "A"), pairs(&[("electronic", "user"), ("skyrim", "user")]));
        // B: user `bright` and user `dark` tags are gone too.
        assert!(entry_tags(&conn, "B").is_empty());
        // C: the `mischievous` rejection moved onto `playful` in v7, where
        // the user's own `playful` tag wins in v8. `frantic` became an auto
        // `driving` in v7, which v10 just removes.
        assert_eq!(entry_tags(&conn, "C"), pairs(&[("playful", "user")]));
        // D: its `driving` tag became a step in v10, since dropped.
        assert!(entry_tags(&conn, "D").is_empty());
        // E: an auto tag renamed onto a tag the user rejected is dropped; the
        // rejection is now a no vote.
        assert_eq!(entry_tags(&conn, "E"), pairs(&[("playful", "no")]));

        // Orphan tag rows are swept.
        let names: Vec<String> = conn
            .prepare("SELECT name FROM tags ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(names, ["electronic", "playful", "skyrim"]);

        // The saved vocabulary is replaced with the new default.
        assert_eq!(
            settings::load_scene_config(&conn).unwrap(),
            crate::models::SceneConfig::default()
        );

        // Music entries are queued for re-analysis; ambiance is left alone.
        let status = |id: &str| -> String {
            conn.query_row(
                "SELECT status FROM entry_analysis WHERE library_entry_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(status("A"), "pending");
        assert_eq!(status("D"), "pending");
        assert_eq!(status("R"), "embedded");
        let scores = |id: &str| -> Option<String> {
            conn.query_row(
                "SELECT scores_json FROM entry_analysis WHERE library_entry_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(scores("A"), None, "stale scores are cleared for queued entries");
        assert_eq!(scores("R"), None, "v8 clears every stored score");
        let needing = crate::autotag::entries_needing_analysis(&conn).unwrap();
        assert!(needing.contains(&"A".to_string()));

        // One pre-migration copy of the v6 database was taken, untouched.
        let backups = pre_migration_backups(dir.path(), 6);
        assert_eq!(backups.len(), 1);
        {
            let copy = Connection::open(&backups[0]).unwrap();
            assert_eq!(read_schema_version(&copy), 6);
            let a_rows: i64 = copy
                .query_row("SELECT COUNT(*) FROM library_entry_tags WHERE library_entry_id = 'A'", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(a_rows, 5);
        }

        // Opening the now-current database again takes no further copy.
        drop(conn);
        open_db(&path).unwrap();
        assert_eq!(pre_migration_backups(dir.path(), 6).len(), 1);
        assert!(pre_migration_backups(dir.path(), 7).is_empty());
    }

    /// Every data case of the v7 → v8 split into analyzer and user layers.
    #[test]
    fn v7_database_migrates_to_auto_and_user_layers() {
        use crate::models::{AutoTag, Role};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lutedrop.db");
        create_v7_db(&path);
        {
            let conn = Connection::open(&path).unwrap();
            // Foreign keys off, to plant a rejection of a deleted entry as an
            // old database could hold.
            conn.execute_batch(
                r#"PRAGMA foreign_keys=OFF;
                   INSERT INTO library_entries
                     (id, file_path, name, created_at, entry_type,
                      role, role_source, role_score, pace, pace_source, pace_score) VALUES
                     ('U',  '/u.mp3',  'U',  '0', 'music', 'combat',     'user', NULL, 'moving', 'user', NULL),
                     ('AU', '/au.mp3', 'AU', '0', 'music', 'non_combat', 'auto', 0.2,  'still',  'auto', 0.1),
                     ('CL', '/cl.mp3', 'CL', '0', 'music', NULL,         'user', NULL, NULL,     'user', NULL),
                     ('N',  '/n.mp3',  'N',  '0', 'music', NULL,         NULL,   NULL, NULL,     NULL,   NULL),
                     ('R',  '/r.ogg',  'R',  '0', 'ambiance', NULL,      NULL,   NULL, NULL,     NULL,   NULL);
                   INSERT INTO tags (id, name) VALUES
                     ('t-folk', 'folk'), ('t-brass', 'brass'), ('t-dark', 'dark'),
                     ('t-choir', 'choir'), ('t-drums', 'drums'), ('t-x', 'x');
                   INSERT INTO library_entry_tags (library_entry_id, tag_id, source, score) VALUES
                     ('U', 't-folk',  'user', NULL),
                     ('U', 't-brass', 'auto', 0.8),
                     ('U', 't-dark',  'auto', NULL),
                     ('U', 't-choir', 'auto', 0.6),
                     ('U', 't-x',     'user', NULL);
                   INSERT INTO tag_rejections (library_entry_id, tag_id, created_at) VALUES
                     ('U', 't-drums', '1'),
                     ('U', 't-choir', '1'),
                     ('U', 't-x',     '1'),
                     ('GONE', 't-drums', '1');
                   INSERT INTO entry_analysis (library_entry_id, content_hash, status, analyzed_at, scores_json) VALUES
                     ('U', 'h-u', 'embedded', '1', '{"gate":1}'),
                     ('R', 'h-r', 'embedded', '1', '{"keep":1}');"#,
            )
            .unwrap();
        }

        let conn = open_db(&path).unwrap();
        assert_eq!(read_schema_version(&conn), SCHEMA_VERSION);

        // Old columns and the rejections table are gone; the index is back.
        // Pace goes entirely in v9 (see v7_to_v8_splits_pace_layers).
        for old in ["role", "role_source", "role_score", "pace", "pace_source", "pace_score", "pace_user"] {
            assert!(!has_column(&conn, "library_entries", old), "{old} should be gone");
        }
        assert!(!has_column(&conn, "library_entry_tags", "source"));
        assert!(!table_exists(&conn, "tag_rejections"));
        assert_eq!(index_names(&conn), EXPECTED_INDEXES);

        // Role: (auto, auto score, user) per entry.
        type Layers = (Option<String>, Option<f64>, Option<String>);
        let layers = |facet: &str, id: &str| -> Layers {
            conn.query_row(
                &format!("SELECT {facet}_auto, {facet}_auto_score, {facet}_user FROM library_entries WHERE id = ?1"),
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
        };
        let s = |v: &str| Some(v.to_string());
        // A user role becomes the user layer.
        assert_eq!(layers("role", "U"), (None, None, s("combat")));
        // An auto role becomes the analyzer layer with its score.
        assert_eq!(layers("role", "AU"), (s("non_combat"), Some(0.2), None));
        // "None (yours)" becomes all NULL, i.e. "use auto".
        assert_eq!(layers("role", "CL"), (None, None, None));
        assert_eq!(layers("role", "N"), (None, None, None));

        // Tags: (auto_score, user_vote) per tag on U.
        let tag_row = |tag: &str| -> Option<(Option<f64>, Option<i64>)> {
            use rusqlite::OptionalExtension;
            conn.query_row(
                "SELECT let.auto_score, let.user_vote FROM library_entry_tags let
                 JOIN tags t ON t.id = let.tag_id
                 WHERE let.library_entry_id = 'U' AND t.name = ?1",
                [tag],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .unwrap()
        };
        // A user tag is a yes vote.
        assert_eq!(tag_row("folk"), Some((None, Some(1))));
        // An auto tag keeps its score with no vote; a missing score is 0.0.
        assert_eq!(tag_row("brass"), Some((Some(0.8), None)));
        assert_eq!(tag_row("dark"), Some((Some(0.0), None)));
        // A rejection is a no vote...
        assert_eq!(tag_row("drums"), Some((None, Some(0))));
        // ...beside the analyzer's score when the auto row was still there...
        assert_eq!(tag_row("choir"), Some((Some(0.6), Some(0))));
        // ...but never over the user's own tag.
        assert_eq!(tag_row("x"), Some((None, Some(1))));
        // A rejection of a deleted entry is not carried over.
        let gone: i64 = conn
            .query_row("SELECT COUNT(*) FROM library_entry_tags WHERE library_entry_id = 'GONE'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(gone, 0);

        // What the app sees.
        ensure_test_config(&conn);
        let state = load_full_state(&conn).unwrap();
        let entry = |id: &str| state.library_entries.iter().find(|e| e.id == id).unwrap();
        let u = entry("U");
        assert_eq!((u.role, u.role_user, u.role_auto), (Some(Role::Combat), Some(Role::Combat), None));
        assert_eq!(u.tags, vec!["brass", "dark", "folk", "x"]);
        assert_eq!(
            u.auto_tags,
            vec![AutoTag { tag: "brass".into(), score: 0.8 }, AutoTag { tag: "dark".into(), score: 0.0 }]
        );
        assert_eq!(u.rejected_tags, vec!["choir", "drums"]);
        let au = entry("AU");
        assert_eq!((au.role, au.role_auto_score, au.role_user), (Some(Role::NonCombat), Some(0.2), None));
        assert_eq!(state.tags, vec!["brass", "dark", "folk", "x"]);

        // Every stored score is cleared so the next run re-scores, and fills
        // the analyzer layer of entries that were user-set until now.
        let scores: i64 = conn
            .query_row("SELECT COUNT(*) FROM entry_analysis WHERE scores_json IS NOT NULL", [], |r| r.get(0))
            .unwrap();
        assert_eq!(scores, 0);
        let status: String = conn
            .query_row("SELECT status FROM entry_analysis WHERE library_entry_id = 'U'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "embedded", "the embedding is still good; only scores are redone");

        // A pre-migration copy of the v7 database was taken, untouched.
        let backups = pre_migration_backups(dir.path(), 7);
        assert_eq!(backups.len(), 1);
        let copy = Connection::open(&backups[0]).unwrap();
        assert_eq!(read_schema_version(&copy), 7);
        assert!(table_exists(&copy, "tag_rejections"));
    }

    #[test]
    fn v8_migration_rolls_back_as_a_whole_on_failure() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        make_v7_shape(&conn);
        conn.execute("INSERT INTO meta (key, value) VALUES ('schema_version', '7')", []).unwrap();
        // A table in the way of the rebuild makes the migration fail midway.
        conn.execute_batch("CREATE TABLE library_entry_tags_v8 (x TEXT);").unwrap();

        assert!(run_migrations(&conn).is_err());

        assert_eq!(read_schema_version(&conn), 7);
        assert!(has_column(&conn, "library_entries", "role_source"));
        assert!(!has_column(&conn, "library_entries", "role_auto"));
        assert!(table_exists(&conn, "tag_rejections"));
    }

    #[test]
    fn migration_refuses_without_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lutedrop.db");
        create_v6_db(&path);
        // A file where the backups folder should be makes the copy fail.
        std::fs::write(dir.path().join("backups"), b"not a folder").unwrap();

        assert!(open_db(&path).is_err());

        let conn = Connection::open(&path).unwrap();
        assert_eq!(read_schema_version(&conn), 6);
        assert!(conn.prepare("SELECT pace FROM library_entries").is_err());
    }

    /// The v6 → v7 step alone (v9 then clears the steps): a user `bright` or
    /// `dark` tag becomes the user's Bright ↔ Dark step.
    #[test]
    fn v7_turns_user_bright_dark_tags_into_steps() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lutedrop.db");
        create_v6_db(&path);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "INSERT INTO library_entries (id, file_path, name, created_at, entry_type) VALUES
               ('A', '/a.mp3', 'A', '0', 'music'), ('B', '/b.mp3', 'B', '0', 'music'),
               ('C', '/c.mp3', 'C', '0', 'music'), ('R', '/r.ogg', 'R', '0', 'ambiance');
             INSERT INTO tags (id, name) VALUES ('t-bright', 'bright'), ('t-dark', 'dark');
             INSERT INTO library_entry_tags (library_entry_id, tag_id, source, score) VALUES
               ('A', 't-bright', 'user', NULL),
               ('B', 't-bright', 'user', NULL), ('B', 't-dark', 'user', NULL),
               ('C', 't-dark', 'auto', 0.9),
               ('R', 't-dark', 'user', NULL);",
        )
        .unwrap();
        let up_to_v7: Vec<(i64, Migration)> = MIGRATIONS.iter().copied().filter(|(from, _)| *from < 7).collect();
        run_migrations_with(&conn, &up_to_v7, 7).unwrap();
        assert_eq!(read_schema_version(&conn), 7);

        assert_eq!(mood_bright_user(&conn, "A").as_deref(), Some("high"));
        // Both at once give the neutral step.
        assert_eq!(mood_bright_user(&conn, "B").as_deref(), Some("mid"));
        // An auto tag leaves no step; ambiance carries no mood.
        assert_eq!(mood_bright_user(&conn, "C"), None);
        assert_eq!(mood_bright_user(&conn, "R"), None);
    }

    /// The v7 → v8 step alone (v9 then drops pace): the pace columns split
    /// into layers like role.
    #[test]
    fn v7_to_v8_splits_pace_layers() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        make_v7_shape(&conn);
        conn.execute_batch(
            "INSERT INTO meta (key, value) VALUES ('schema_version', '7');
             INSERT INTO library_entries
               (id, file_path, name, created_at, pace, pace_source, pace_score) VALUES
               ('U',  '/u.mp3',  'U',  '0', 'moving', 'user', NULL),
               ('AU', '/au.mp3', 'AU', '0', 'still',  'auto', 0.1),
               ('CL', '/cl.mp3', 'CL', '0', NULL,     'user', NULL);",
        )
        .unwrap();
        let up_to_v8: Vec<(i64, Migration)> = MIGRATIONS.iter().copied().filter(|(from, _)| *from < 8).collect();
        run_migrations_with(&conn, &up_to_v8, 8).unwrap();
        assert_eq!(read_schema_version(&conn), 8);

        let layers = |id: &str| -> (Option<String>, Option<f64>, Option<String>) {
            conn.query_row(
                "SELECT pace_auto, pace_auto_score, pace_user FROM library_entries WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
        };
        let s = |v: &str| Some(v.to_string());
        assert_eq!(layers("U"), (None, None, s("moving")));
        assert_eq!(layers("AU"), (s("still"), Some(0.1), None));
        assert_eq!(layers("CL"), (None, None, None));
    }

    /// Every data case of the v8 → v9 move to three mood scales (which v12
    /// drops again).
    #[test]
    fn v8_database_migrates_to_three_mood_scales() {
        use crate::models::{AutoTag, Role};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lutedrop.db");
        create_v8_db(&path);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                r#"INSERT INTO library_entries
                     (id, file_path, name, created_at, entry_type, role_auto, role_auto_score, role_user,
                      pace_auto, pace_auto_score, pace_user,
                      mood_bright, mood_light, mood_bright_user, mood_light_user) VALUES
                     ('M', '/m.mp3', 'M', '0', 'music', 'combat', 0.9, 'non_combat',
                      'moving', 0.8, 'still', 1.25, -0.5, 'high', 'low'),
                     ('N', '/n.mp3', 'N', '0', 'music', NULL, NULL, NULL,
                      NULL, NULL, NULL, NULL, NULL, NULL, NULL),
                     ('R', '/r.ogg', 'R', '0', 'ambiance', NULL, NULL, NULL,
                      NULL, NULL, NULL, NULL, NULL, NULL, NULL);
                   INSERT INTO tags (id, name) VALUES ('t-folk', 'folk'), ('t-brass', 'brass'), ('t-drums', 'drums');
                   INSERT INTO library_entry_tags (library_entry_id, tag_id, auto_score, user_vote) VALUES
                     ('M', 't-folk', NULL, 1), ('M', 't-brass', 0.7, NULL), ('M', 't-drums', 0.6, 0);
                   INSERT INTO entry_analysis (library_entry_id, content_hash, status, analyzed_at, scores_json) VALUES
                     ('M', 'h-m', 'embedded', '1', '{"gate":1}'),
                     ('R', 'h-r', 'embedded', '1', '{"keep":1}');"#,
            )
            .unwrap();
        }

        // At v9, Bright ↔ Dark (both layers) is kept and the new scales start
        // empty. v12 drops all three, so check on a copy walked to v9.
        {
            let v9 = migrated_copy(&path, 9);
            type Moods = (Option<f64>, Option<String>, Option<f64>, Option<String>, Option<f64>, Option<String>);
            let moods = |id: &str| -> Moods {
                v9.query_row(
                    "SELECT mood_bright, mood_bright_user, mood_grand, mood_grand_user,
                            mood_energy, mood_energy_user
                     FROM library_entries WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
                )
                .unwrap()
            };
            assert_eq!(moods("M"), (Some(1.25), Some("high".into()), None, None, None, None));
            assert_eq!(moods("N"), (None, None, None, None, None, None));
        }

        let conn = open_db(&path).unwrap();
        assert_eq!(read_schema_version(&conn), SCHEMA_VERSION);

        // Light ↔ Heavy and pace are gone, and so are the scales by v12.
        for old in ["mood_light", "mood_light_user", "pace_auto", "pace_auto_score", "pace_user"]
            .into_iter()
            .chain(V11_MOOD_COLUMNS)
        {
            assert!(!has_column(&conn, "library_entries", old), "{old} should be gone");
        }
        assert_eq!(index_names(&conn), EXPECTED_INDEXES);

        // Role and tags are untouched.
        ensure_test_config(&conn);
        let state = load_full_state(&conn).unwrap();
        let m = state.library_entries.iter().find(|e| e.id == "M").unwrap();
        assert_eq!(
            (m.role, m.role_auto, m.role_auto_score, m.role_user),
            (Some(Role::NonCombat), Some(Role::Combat), Some(0.9), Some(Role::NonCombat))
        );
        assert_eq!(m.tags, vec!["brass", "folk"]);
        assert_eq!(m.auto_tags, vec![AutoTag { tag: "brass".into(), score: 0.7 }]);
        assert_eq!(m.rejected_tags, vec!["drums"]);

        // Every stored score is cleared so the next run re-scores; the
        // embeddings stay good.
        let scores: i64 = conn
            .query_row("SELECT COUNT(*) FROM entry_analysis WHERE scores_json IS NOT NULL", [], |r| r.get(0))
            .unwrap();
        assert_eq!(scores, 0);
        let status: String = conn
            .query_row("SELECT status FROM entry_analysis WHERE library_entry_id = 'M'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "embedded");

        // A pre-migration copy of the v8 database keeps the user's old steps.
        let backups = pre_migration_backups(dir.path(), 8);
        assert_eq!(backups.len(), 1);
        let copy = Connection::open(&backups[0]).unwrap();
        assert_eq!(read_schema_version(&copy), 8);
        let old: (Option<String>, Option<String>, Option<String>) = copy
            .query_row(
                "SELECT mood_bright_user, mood_light_user, pace_user FROM library_entries WHERE id = 'M'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(old, (Some("high".into()), Some("low".into()), Some("still".into())));
    }

    /// Every data case of the v9 → v10 dark kind and `driving` fold.
    #[test]
    fn v9_database_migrates_driving_into_energetic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lutedrop.db");
        create_v9_db(&path);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                r#"INSERT INTO library_entries (id, file_path, name, created_at, entry_type, mood_energy_user) VALUES
                     ('YES', '/y.mp3', 'Y', '0', 'music', NULL),
                     ('KEEP', '/k.mp3', 'K', '0', 'music', 'low'),
                     ('AUTO', '/a.mp3', 'A', '0', 'music', NULL),
                     ('NO', '/n.mp3', 'N', '0', 'music', NULL),
                     ('AMB', '/r.ogg', 'R', '0', 'ambiance', NULL);
                   INSERT INTO tags (id, name) VALUES ('t-driving', 'driving'), ('t-folk', 'folk');
                   INSERT INTO library_entry_tags (library_entry_id, tag_id, auto_score, user_vote) VALUES
                     ('YES', 't-driving', 0.8, 1), ('YES', 't-folk', NULL, 1),
                     ('KEEP', 't-driving', NULL, 1),
                     ('AUTO', 't-driving', 0.7, NULL), ('AUTO', 't-folk', 0.6, NULL),
                     ('NO', 't-driving', 0.9, 0),
                     ('AMB', 't-driving', NULL, 1);
                   INSERT INTO entry_analysis (library_entry_id, content_hash, status, analyzed_at, scores_json) VALUES
                     ('YES', 'h-y', 'embedded', '1', '{"gate":1}');
                   INSERT INTO meta (key, value) VALUES ('autotag_scene_encounter',
                     '{"vocabulary":["folk","driving","electronic"]}');"#,
            )
            .unwrap();
        }

        // At v10, a user yes vote becomes Energetic and an existing user step
        // is kept. v12 drops the scales, so check on a copy walked to v10.
        {
            let v10 = migrated_copy(&path, 10);
            assert_eq!(mood_energy_user(&v10, "YES").as_deref(), Some("high"));
            assert_eq!(mood_energy_user(&v10, "KEEP").as_deref(), Some("low"));
            // An auto-only tag and a no vote are just removed.
            assert_eq!(mood_energy_user(&v10, "AUTO"), None);
            assert_eq!(mood_energy_user(&v10, "NO"), None);
            // Ambiance carries no mood.
            assert_eq!(mood_energy_user(&v10, "AMB"), None);
        }

        let conn = open_db(&path).unwrap();
        assert_eq!(read_schema_version(&conn), SCHEMA_VERSION);
        assert!(!has_column(&conn, "library_entries", "mood_energy_user"));

        // Every `driving` row and the tag itself are gone; other tags stay.
        assert_eq!(entry_tags(&conn, "YES"), pairs(&[("folk", "user")]));
        assert!(entry_tags(&conn, "KEEP").is_empty());
        assert_eq!(entry_tags(&conn, "AUTO"), pairs(&[("folk", "auto")]));
        assert!(entry_tags(&conn, "NO").is_empty());
        assert!(entry_tags(&conn, "AMB").is_empty());
        let driving: i64 = conn
            .query_row("SELECT COUNT(*) FROM tags WHERE name = 'driving'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(driving, 0);

        // `driving` leaves the saved vocabulary.
        assert_eq!(
            settings::load_scene_config(&conn).unwrap().vocabulary,
            ["folk", "electronic"]
        );

        // The new columns start empty; stored scores are cleared.
        ensure_test_config(&conn);
        let state = load_full_state(&conn).unwrap();
        let yes = state.library_entries.iter().find(|e| e.id == "YES").unwrap();
        assert!(yes.moods.iter().all(|m| m.auto.is_none() && m.user.is_none()));
        assert_eq!(yes.mood_reviewed_at, None);
        let scores: i64 = conn
            .query_row("SELECT COUNT(*) FROM entry_analysis WHERE scores_json IS NOT NULL", [], |r| r.get(0))
            .unwrap();
        assert_eq!(scores, 0);

        // A pre-migration copy of the v9 database keeps the tag.
        let backups = pre_migration_backups(dir.path(), 9);
        assert_eq!(backups.len(), 1);
        let copy = Connection::open(&backups[0]).unwrap();
        assert_eq!(read_schema_version(&copy), 9);
        assert_eq!(entry_tags(&copy, "AUTO"), pairs(&[("driving", "auto"), ("folk", "auto")]));
    }

    #[test]
    fn v9_migration_rolls_back_as_a_whole_on_failure() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        make_v8_shape(&conn);
        conn.execute_batch(
            "INSERT INTO meta (key, value) VALUES ('schema_version', '8');
             INSERT INTO library_entries (id, file_path, name, created_at, mood_bright_user)
               VALUES ('M', '/m.mp3', 'M', '0', 'high');
             -- An index on a column v9 drops makes the DROP COLUMN fail.
             CREATE INDEX idx_in_the_way ON library_entries(pace_user);",
        )
        .unwrap();

        assert!(run_migrations(&conn).is_err());

        assert_eq!(read_schema_version(&conn), 8);
        assert!(has_column(&conn, "library_entries", "mood_light"));
        assert!(!has_column(&conn, "library_entries", "mood_grand"));
        let step: Option<String> = conn
            .query_row("SELECT mood_bright_user FROM library_entries WHERE id = 'M'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(step.as_deref(), Some("high"));
    }

    /// Every data case of the v10 → v11 pools upgrade.
    #[test]
    fn v10_database_migrates_to_pools() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lutedrop.db");
        create_v10_db(&path);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                r#"INSERT INTO library_entries (id, file_path, name, created_at, entry_type,
                       role_auto, role_auto_score, role_user,
                       mood_bright, mood_bright_user, mood_grand, mood_grand_user, mood_energy, mood_energy_user,
                       mood_sad, mood_scary, dark_kind_user) VALUES
                     ('M', '/m.mp3', 'M', '5', 'music', 'combat', 0.9, 'non_combat',
                       -1.1, 'low', 0.4, 'high', 0.2, 'mid', 0.7, -0.3, 'scary'),
                     ('N', '/n.mp3', 'N', '6', 'music', NULL, NULL, NULL,
                       0.5, NULL, -0.2, NULL, 1.3, NULL, NULL, NULL, NULL);
                   INSERT INTO entry_analysis (library_entry_id, content_hash, status, analyzed_at, scores_json) VALUES
                     ('M', 'h-m', 'embedded', '1', '{"gate":1}');"#,
            )
            .unwrap();
        }

        // At v11 the user's scale steps and the analyzer's z-scores survive.
        // v12 drops them, so check on a copy walked to v11.
        {
            let v11 = migrated_copy(&path, 11);
            type Moods = (Option<f64>, Option<String>, Option<f64>, Option<String>, Option<f64>, Option<String>);
            let moods = |id: &str| -> Moods {
                v11.query_row(
                    "SELECT mood_bright, mood_bright_user, mood_grand, mood_grand_user,
                            mood_energy, mood_energy_user
                     FROM library_entries WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
                )
                .unwrap()
            };
            let s = |v: &str| Some(v.to_string());
            assert_eq!(moods("M"), (Some(-1.1), s("low"), Some(0.4), s("high"), Some(0.2), s("mid")));
            assert_eq!(moods("N"), (Some(0.5), None, Some(-0.2), None, Some(1.3), None));
        }

        // At v11 the pool columns exist and start empty. v12 drops them, so
        // check on a copy walked to v11.
        {
            let v11 = migrated_copy(&path, 11);
            for column in V11_ALL_POOL_COLUMNS {
                let set: i64 = v11
                    .query_row(&format!("SELECT COUNT({column}) FROM library_entries"), [], |r| r.get(0))
                    .unwrap();
                assert_eq!(set, 0, "{column} should start empty");
            }
        }

        let conn = open_db(&path).unwrap();
        assert_eq!(read_schema_version(&conn), SCHEMA_VERSION);

        // The dark kind columns are gone.
        for old in ["mood_sad", "mood_scary", "dark_kind_user"] {
            assert!(!has_column(&conn, "library_entries", old), "{old} should be gone");
        }

        ensure_test_config(&conn);
        let state = load_full_state(&conn).unwrap();
        let m = state.library_entries.iter().find(|e| e.id == "M").unwrap();
        // The role survives.
        assert_eq!((m.role_auto, m.role_user), (Some(Role::Combat), Some(Role::NonCombat)));
        let n = state.library_entries.iter().find(|e| e.id == "N").unwrap();
        // Nothing carries into moods.
        for e in [m, n] {
            assert!(e.moods.iter().all(|m| m.auto.is_none() && m.user.is_none()));
            assert_eq!(e.mood_reviewed_at, None);
        }
        // Stored scores are cleared so the next run scores moods; no
        // calibration exists yet.
        let scores: i64 = conn
            .query_row("SELECT COUNT(*) FROM entry_analysis WHERE scores_json IS NOT NULL", [], |r| r.get(0))
            .unwrap();
        assert_eq!(scores, 0);
        assert_eq!(settings::load_mood_calibration(&conn).unwrap(), None);

        // A pre-migration copy of the v10 database keeps the dark kind.
        let backups = pre_migration_backups(dir.path(), 10);
        assert_eq!(backups.len(), 1);
        let copy = Connection::open(&backups[0]).unwrap();
        assert_eq!(read_schema_version(&copy), 10);
        let kind: Option<String> = copy
            .query_row("SELECT dark_kind_user FROM library_entries WHERE id = 'M'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(kind.as_deref(), Some("scary"));
    }

    /// Every data case of the v11 → v12 upgrade: pools and the mood scales
    /// go, the mood table arrives empty, and the review time is renamed.
    #[test]
    fn v11_database_moves_from_pools_to_the_mood_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lutedrop.db");
        create_v11_db(&path);
        const CALIBRATION: &str = r#"{"tracks":64,"pools":{"heroic":{"mean":0.2,"sd":0.05}}}"#;
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                r#"INSERT INTO library_entries (id, file_path, name, created_at, entry_type,
                       role_auto, role_auto_score, role_user,
                       mood_bright, mood_bright_user, mood_grand, mood_grand_user, mood_energy, mood_energy_user,
                       pool_auto, pool_auto_p, pool_auto_runner, pool_user, pool_reviewed_at) VALUES
                     ('T', '/t.mp3', 'T', '5', 'music', 'combat', 0.9, 'non_combat',
                       -1.1, 'low', 0.4, 'high', 0.2, 'mid', 'heroic', 0.84, 'awe', 'menacing', '1700000000001'),
                     ('K', '/k.mp3', 'K', '8', 'music', NULL, NULL, NULL,
                       NULL, NULL, NULL, NULL, NULL, NULL, 'somber', 0.7, 'awe', NULL, '1700000000004'),
                     ('U', '/u.mp3', 'U', '9', 'music', NULL, NULL, NULL,
                       NULL, NULL, NULL, NULL, NULL, NULL, 'peaceful', 0.3, 'mysterious', NULL, NULL),
                     ('A', '/a.ogg', 'A', '10', 'ambiance', NULL, NULL, NULL,
                       NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL);
                   INSERT INTO tags (id, name) VALUES ('t-folk', 'folk'), ('t-brass', 'brass');
                   INSERT INTO library_entry_tags (library_entry_id, tag_id, auto_score, user_vote) VALUES
                     ('T', 't-folk', NULL, 1), ('T', 't-brass', 0.7, NULL);
                   INSERT INTO entry_analysis (library_entry_id, content_hash, status, analyzed_at, scores_json) VALUES
                     ('T', 'h-t', 'embedded', '1', '{"pools":1}');"#,
            )
            .unwrap();
            conn.execute(
                "INSERT INTO meta (key, value) VALUES ('pool_calibration', ?1)",
                [CALIBRATION],
            )
            .unwrap();
        }

        let conn = open_db(&path).unwrap();
        assert_eq!(read_schema_version(&conn), SCHEMA_VERSION);

        // The pool and scale columns are gone; the role columns stay, and the
        // review time is renamed.
        for column in V11_MOOD_COLUMNS.into_iter().chain(V11_ALL_POOL_COLUMNS) {
            assert!(!has_column(&conn, "library_entries", column), "{column} should be gone");
        }
        for column in ["role_auto", "role_auto_score", "role_user", "mood_reviewed_at"] {
            assert!(has_column(&conn, "library_entries", column), "{column} should exist");
        }
        assert_eq!(index_names(&conn), EXPECTED_INDEXES);

        // The mood table has its shape and starts empty: a confirmed pool is
        // not a level.
        let columns: Vec<(String, String, bool, i64)> = conn
            .prepare("SELECT name, type, \"notnull\", pk FROM pragma_table_info('library_entry_moods') ORDER BY cid")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let col = |n: &str, t: &str, nn: bool, pk: i64| (n.to_string(), t.to_string(), nn, pk);
        assert_eq!(
            columns,
            vec![
                col("library_entry_id", "TEXT", true, 1),
                col("mood", "TEXT", true, 2),
                col("auto_score", "REAL", false, 0),
                col("user_level", "REAL", false, 0),
            ]
        );
        let rows: i64 = conn.query_row("SELECT COUNT(*) FROM library_entry_moods", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 0);
        // Its level check and cascade work on the migrated database.
        assert!(conn
            .execute("INSERT INTO library_entry_moods (library_entry_id, mood, user_level) VALUES ('T', 'grim', 0.3)", [])
            .is_err());
        conn.execute("INSERT INTO library_entry_moods (library_entry_id, mood, user_level) VALUES ('U', 'grim', 0.5)", [])
            .unwrap();
        library::delete_library_entry(&conn, "U").unwrap();
        let rows: i64 = conn.query_row("SELECT COUNT(*) FROM library_entry_moods", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 0);

        // What the app sees: review times kept under the new name, empty
        // profiles, and role and tags untouched.
        ensure_test_config(&conn);
        let state = load_full_state(&conn).unwrap();
        let entry = |id: &str| state.library_entries.iter().find(|e| e.id == id).unwrap();
        let t = entry("T");
        assert_eq!(t.mood_reviewed_at.as_deref(), Some("1700000000001"));
        assert_eq!(entry("K").mood_reviewed_at.as_deref(), Some("1700000000004"));
        assert_eq!(entry("A").mood_reviewed_at, None);
        for e in [t, entry("K"), entry("A")] {
            assert_eq!(e.moods.iter().map(|m| m.mood.clone()).collect::<Vec<_>>(), MoodVocabulary::defaults().ids());
            assert!(e.moods.iter().all(|m| m.auto.is_none() && m.user.is_none()));
        }
        assert_eq!(
            (t.role, t.role_auto, t.role_auto_score, t.role_user),
            (Some(Role::NonCombat), Some(Role::Combat), Some(0.9), Some(Role::NonCombat))
        );
        assert_eq!(t.tags, vec!["brass", "folk"]);
        assert_eq!(t.auto_tags, vec![crate::models::AutoTag { tag: "brass".into(), score: 0.7 }]);

        // The pool calibration row and the stored scores are gone, so the
        // next run scores moods and takes a mood calibration.
        let calibration: i64 = conn
            .query_row("SELECT COUNT(*) FROM meta WHERE key = 'pool_calibration'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(calibration, 0);
        assert_eq!(settings::load_mood_calibration(&conn).unwrap(), None);
        // The default vocabulary is seeded and shown.
        let stored: String = conn
            .query_row("SELECT value FROM meta WHERE key = 'mood_vocabulary'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(serde_json::from_str::<MoodVocabulary>(&stored).unwrap(), MoodVocabulary::defaults());
        assert_eq!(state.mood_vocabulary, MoodVocabulary::defaults());
        assert_eq!((state.mood_calibration.clone(), state.moods_need_recalibration), (None, false));
        assert!(!state.mood_vocabulary_unreadable);
        let scores: i64 = conn
            .query_row("SELECT COUNT(*) FROM entry_analysis WHERE scores_json IS NOT NULL", [], |r| r.get(0))
            .unwrap();
        assert_eq!(scores, 0);
        let status: String = conn
            .query_row("SELECT status FROM entry_analysis WHERE library_entry_id = 'T'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "embedded", "the embedding is still good; only scores are redone");

        // A pre-migration copy of the v11 database keeps the scales, the
        // pools and the calibration.
        let backups = pre_migration_backups(dir.path(), 11);
        assert_eq!(backups.len(), 1);
        let copy = Connection::open(&backups[0]).unwrap();
        assert_eq!(read_schema_version(&copy), 11);
        assert_eq!(mood_bright_user(&copy, "T").as_deref(), Some("low"));
        assert_eq!(mood_energy_user(&copy, "T").as_deref(), Some("mid"));
        let old: (Option<String>, Option<String>) = copy
            .query_row("SELECT pool_user, pool_auto FROM library_entries WHERE id = 'T'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(old, (Some("menacing".into()), Some("heroic".into())));
        let calibration: String = copy
            .query_row("SELECT value FROM meta WHERE key = 'pool_calibration'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(calibration, CALIBRATION);
    }

    #[test]
    fn v12_migration_tolerates_missing_scale_columns() {
        // "If still present": a v11 database without the scales migrates too.
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        make_v11_shape(&conn);
        for column in V11_MOOD_COLUMNS {
            conn.execute_batch(&format!("ALTER TABLE library_entries DROP COLUMN {column}")).unwrap();
        }
        conn.execute("INSERT INTO meta (key, value) VALUES ('schema_version', '11')", []).unwrap();
        run_migrations(&conn).unwrap();
        assert_eq!(read_schema_version(&conn), SCHEMA_VERSION);
        assert!(has_column(&conn, "library_entries", "mood_reviewed_at"));
        assert!(table_exists(&conn, "library_entry_moods"));
    }

    #[test]
    fn v12_migration_rolls_back_as_a_whole_on_failure() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        make_v11_shape(&conn);
        conn.execute_batch(
            "INSERT INTO meta (key, value) VALUES ('schema_version', '11');
             INSERT INTO library_entries (id, file_path, name, created_at, mood_bright_user, pool_user)
               VALUES ('M', '/m.mp3', 'M', '0', 'high', 'heroic');
             -- An index on the last column v12 drops makes that DROP fail.
             CREATE INDEX idx_in_the_way ON library_entries(mood_energy_user);",
        )
        .unwrap();

        assert!(run_migrations(&conn).is_err());

        assert_eq!(read_schema_version(&conn), 11);
        for column in V11_MOOD_COLUMNS.into_iter().chain(V11_ALL_POOL_COLUMNS) {
            assert!(has_column(&conn, "library_entries", column), "{column} should be back");
        }
        assert!(!table_exists(&conn, "library_entry_moods"));
        assert_eq!(mood_bright_user(&conn, "M").as_deref(), Some("high"));
    }

    /// The defaults as schema v12 shipped them (three sentences per mood
    /// for most, three tag words per side of Energy).
    fn v12_default_vocabulary() -> MoodVocabulary {
        let mood = |id: &str, name: &str, prompts: &[&str]| crate::models::MoodDef {
            id: id.into(),
            name: name.into(),
            prompts: prompts.iter().map(|p| p.to_string()).collect(),
        };
        MoodVocabulary {
            moods: vec![
                mood("peaceful", "Peaceful", &[
                    "benign, pleasant, agreeable music with no threat",
                    "music that signals safety, whatever its tempo",
                    "friendly and harmless",
                ]),
                mood("sorrowful", "Sorrowful", &[
                    "reflective, somber, sorrowful music",
                    "sad, mournful, melancholic music",
                    "wistful, grieving, nostalgic music",
                ]),
                mood("wondrous", "Wondrous", &[
                    "music full of wonder and awe",
                    "sacred, divine, celestial music, a choir in a cathedral",
                    "a breathtaking vista, hopeful and vast, slow and soaring",
                ]),
                mood("joyful", "Joyful", &["joyful, glad music, any tempo", "bright major-key happiness", "contented and sunny"]),
                mood("quirky", "Quirky", &[
                    "quirky, weird, offbeat music",
                    "groovy, funky, eccentric and strange",
                    "campy, oddball, mischievous tune",
                ]),
                mood("mysterious", "Mysterious", &[
                    "mysterious, unsettling, eerie music",
                    "strange, uncanny, otherworldly sounds",
                    "quiet suspense and looming dread, something is about to happen",
                ]),
                mood("grim", "Grim", &["horror", "dark", "heavy"]),
                mood("heroic", "Heroic", &[
                    "heroic, triumphant, bold orchestral music",
                    "a rousing victory anthem, brass and choir soaring",
                    "adventurous, noble, uplifting epic music",
                ]),
            ],
            energy: crate::models::EnergyPrompts {
                positive: vec!["energetic".into(), "upbeat".into(), "powerful".into()],
                negative: vec!["slow".into(), "relaxing".into(), "calm".into()],
            },
        }
    }

    #[test]
    fn v12_default_json_is_what_v12_wrote() {
        assert_eq!(
            settings::mood_vocabulary_json(&v12_default_vocabulary()).unwrap(),
            V12_DEFAULT_MOOD_VOCABULARY_JSON
        );
    }

    /// A v12 database on disk with the given `mood_vocabulary` row, a mood
    /// calibration, stored scores, and two entries: one with only
    /// suggestions, one with a user level beside a suggestion.
    fn create_v12_db(path: &Path, vocabulary_json: &str) {
        drop(open_db(path).unwrap());
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            r#"UPDATE meta SET value = '12' WHERE key = 'schema_version';
               INSERT OR REPLACE INTO meta (key, value) VALUES ('mood_calibration', '{"calibratedAt":"1"}');
               INSERT INTO library_entries (id, file_path, name, created_at, entry_type, mood_reviewed_at) VALUES
                 ('S', '/s.mp3', 'S', '1', 'music', NULL),
                 ('L', '/l.mp3', 'L', '2', 'music', '1700000000000');
               INSERT INTO library_entry_moods (library_entry_id, mood, auto_score, user_level) VALUES
                 ('S', 'grim', 0.4, NULL),
                 ('S', 'energy', 0.6, NULL),
                 ('L', 'peaceful', 0.7, 0.25),
                 ('L', 'heroic', 0.2, NULL);
               INSERT INTO entry_analysis (library_entry_id, content_hash, status, analyzed_at, scores_json) VALUES
                 ('S', 'h-s', 'embedded', '1', '{"mood":1}');"#,
        )
        .unwrap();
        conn.execute("UPDATE meta SET value = ?1 WHERE key = 'mood_vocabulary'", [vocabulary_json])
            .unwrap();
    }


    /// Opens `path` like `open_db` but migrates only up to v13, so the v13
    /// tests check that step alone (v14 clears suggestions again).
    fn open_db_to_v13(path: &Path) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        let version = read_schema_version(&conn);
        if version < 13 {
            backup::perform_pre_migration_backup(&conn, &backup::backup_dir_for(path), version).unwrap();
        }
        run_migrations_with(&conn, &[(12, migrate_v12_to_v13)], 13).unwrap();
        conn
    }

    #[test]
    fn v14_clears_suggestions_and_calibration_but_keeps_levels_and_vocabulary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lutedrop.db");
        let mut edited = v12_default_vocabulary();
        edited.moods[0].name = "Calm".into();
        let edited_json = settings::mood_vocabulary_json(&edited).unwrap();
        create_v12_db(&path, &edited_json);
        drop(open_db_to_v13(&path));
        {
            let c = Connection::open(&path).unwrap();
            assert_eq!(read_schema_version(&c), 13);
            assert!(meta_value(&c, "mood_calibration").is_some());
            assert_eq!(mood_rows(&c).len(), 4);
        }

        let conn = open_db(&path).unwrap();
        assert_eq!(read_schema_version(&conn), SCHEMA_VERSION);
        assert_eq!(meta_value(&conn, "mood_vocabulary").as_deref(), Some(edited_json.as_str()));
        assert_eq!(meta_value(&conn, "mood_calibration"), None);
        assert_eq!(mood_rows(&conn), vec![("L".into(), "peaceful".into(), None, Some(0.25))]);
        assert_eq!(stored_score_count(&conn), 0);
        let reviewed: Option<String> = conn
            .query_row("SELECT mood_reviewed_at FROM library_entries WHERE id = 'L'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(reviewed.as_deref(), Some("1700000000000"));
        assert_eq!(pre_migration_backups(dir.path(), 13).len(), 1);
    }


    fn v13_default_vocabulary() -> MoodVocabulary {
        let mood = |id: &str, name: &str, prompts: &[&str]| crate::models::MoodDef {
            id: id.into(),
            name: name.into(),
            prompts: prompts.iter().map(|p| p.to_string()).collect(),
        };
        MoodVocabulary {
            moods: vec![
                mood("peaceful", "Peaceful", &["relaxing, positive"]),
                mood("sorrowful", "Sorrowful", &["sad, sorrowful, mournful, melancholic, somber, wistful, grieving, nostalgic, reflective music"]),
                mood("wondrous", "Wondrous", &["hopeful, inspiring"]),
                mood("joyful", "Joyful", &["happy, upbeat"]),
                mood("quirky", "Quirky", &["quirky, weird, offbeat, groovy, funky, eccentric, strange, campy, oddball, mischievous music"]),
                mood("mysterious", "Mysterious", &["mysterious, unsettling, eerie, strange, uncanny, otherworldly, suspenseful, looming, dread music"]),
                mood("grim", "Grim", &["horror, dark, heavy"]),
                mood("heroic", "Heroic", &["heroic, triumphant, bold, rousing, victorious, adventurous, noble, uplifting, epic orchestral music"]),
            ],
            energy: crate::models::EnergyPrompts {
                positive: vec!["energetic, upbeat, powerful".into()],
                negative: vec!["slow, relaxing, calm".into()],
            },
        }
    }

    #[test]
    fn v13_default_json_is_what_v13_wrote() {
        assert_eq!(
            settings::mood_vocabulary_json(&v13_default_vocabulary()).unwrap(),
            V13_DEFAULT_MOOD_VOCABULARY_JSON
        );
    }

    #[test]
    fn v14_swaps_the_untouched_v13_defaults_for_the_settled_set() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lutedrop.db");
        create_v12_db(&path, V13_DEFAULT_MOOD_VOCABULARY_JSON);
        Connection::open(&path)
            .unwrap()
            .execute("UPDATE meta SET value = '13' WHERE key = 'schema_version'", [])
            .unwrap();

        let conn = open_db(&path).unwrap();
        assert_eq!(read_schema_version(&conn), SCHEMA_VERSION);
        assert_eq!(
            meta_value(&conn, "mood_vocabulary").unwrap(),
            settings::mood_vocabulary_json(&MoodVocabulary::defaults()).unwrap()
        );
        assert!(meta_value(&conn, "mood_vocabulary").unwrap().contains("\"battle\""));
        assert_eq!(meta_value(&conn, "mood_calibration"), None);
        assert_eq!(mood_rows(&conn), vec![("L".into(), "peaceful".into(), None, Some(0.25))]);
        assert_eq!(pre_migration_backups(dir.path(), 13).len(), 1);
    }

    type MoodRow = (String, String, Option<f64>, Option<f64>);

    fn mood_rows(conn: &Connection) -> Vec<MoodRow> {
        conn.prepare(
            "SELECT library_entry_id, mood, auto_score, user_level FROM library_entry_moods
             ORDER BY library_entry_id, mood",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    }

    fn meta_value(conn: &Connection, key: &str) -> Option<String> {
        use rusqlite::OptionalExtension;
        conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()
            .unwrap()
    }

    fn stored_score_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM entry_analysis WHERE scores_json IS NOT NULL", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn v13_replaces_the_untouched_v12_defaults_and_clears_suggestions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lutedrop.db");
        create_v12_db(&path, V12_DEFAULT_MOOD_VOCABULARY_JSON);

        let conn = open_db_to_v13(&path);
        assert_eq!(read_schema_version(&conn), 13);
        // The new defaults are stored, byte for byte as a fresh database's.
        assert_eq!(
            meta_value(&conn, "mood_vocabulary").unwrap(),
            settings::mood_vocabulary_json(&MoodVocabulary::defaults()).unwrap()
        );
        // The calibration and every suggestion are gone; the user's level
        // stays (its suggestion cleared), and rows with no level go.
        assert_eq!(meta_value(&conn, "mood_calibration"), None);
        assert_eq!(mood_rows(&conn), vec![("L".into(), "peaceful".into(), None, Some(0.25))]);
        assert_eq!(stored_score_count(&conn), 0);
        let status: String = conn
            .query_row("SELECT status FROM entry_analysis WHERE library_entry_id = 'S'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "embedded");
        let reviewed: Option<String> = conn
            .query_row("SELECT mood_reviewed_at FROM library_entries WHERE id = 'L'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(reviewed.as_deref(), Some("1700000000000"));

        // The pre-migration copy keeps the v12 vocabulary and suggestions.
        let backups = pre_migration_backups(dir.path(), 12);
        assert_eq!(backups.len(), 1);
        let copy = Connection::open(&backups[0]).unwrap();
        assert_eq!(read_schema_version(&copy), 12);
        assert_eq!(meta_value(&copy, "mood_vocabulary").as_deref(), Some(V12_DEFAULT_MOOD_VOCABULARY_JSON));
        assert!(meta_value(&copy, "mood_calibration").is_some());
        assert_eq!(mood_rows(&copy).len(), 4);
    }

    #[test]
    fn v13_treats_a_missing_vocabulary_as_the_v12_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lutedrop.db");
        create_v12_db(&path, V12_DEFAULT_MOOD_VOCABULARY_JSON);
        Connection::open(&path)
            .unwrap()
            .execute("DELETE FROM meta WHERE key = 'mood_vocabulary'", [])
            .unwrap();

        let conn = open_db_to_v13(&path);
        assert_eq!(read_schema_version(&conn), 13);
        assert_eq!(
            meta_value(&conn, "mood_vocabulary").unwrap(),
            settings::mood_vocabulary_json(&MoodVocabulary::defaults()).unwrap()
        );
        assert_eq!(meta_value(&conn, "mood_calibration"), None);
        assert_eq!(mood_rows(&conn), vec![("L".into(), "peaceful".into(), None, Some(0.25))]);
        assert_eq!(stored_score_count(&conn), 0);
        assert_eq!(pre_migration_backups(dir.path(), 12).len(), 1);
    }

    #[test]
    fn v13_leaves_an_edited_vocabulary_alone() {
        let mut edited = v12_default_vocabulary();
        edited.moods[0].name = "Calm".into();
        let edited_json = settings::mood_vocabulary_json(&edited).unwrap();
        // Unreadable and merely reformatted rows are not the shipped bytes either.
        let reformatted = serde_json::to_string_pretty(&v12_default_vocabulary()).unwrap();
        for stored in [edited_json.as_str(), "{oops", reformatted.as_str()] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("lutedrop.db");
            create_v12_db(&path, stored);

            let conn = open_db_to_v13(&path);
            assert_eq!(read_schema_version(&conn), 13);
            assert_eq!(meta_value(&conn, "mood_vocabulary").as_deref(), Some(stored));
            assert!(meta_value(&conn, "mood_calibration").is_some());
            assert_eq!(mood_rows(&conn).len(), 4);
            assert_eq!(stored_score_count(&conn), 1);
            assert_eq!(pre_migration_backups(dir.path(), 12).len(), 1);
        }
    }

    #[test]
    fn migration_v14_to_v15_adds_mood_flags_and_keeps_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lutedrop.db");
        let id = {
            let conn = open_db(&path).unwrap();
            let id = library::insert_library_entry(&conn, "/a.mp3", None, "A", None, None, None, "music").unwrap();
            conn.execute(
                "INSERT INTO library_entry_moods (library_entry_id, mood, auto_score, user_level) VALUES (?1, 'grim', 0.4, 0.75)",
                [&id],
            )
            .unwrap();
            // Put it back to v14: no flag table, version 14.
            conn.execute_batch("DROP TABLE mood_flags; UPDATE meta SET value = '14' WHERE key = 'schema_version';")
                .unwrap();
            id
        };

        let conn = open_db(&path).unwrap();
        assert_eq!(read_schema_version(&conn), 15);
        let flags: i64 = conn.query_row("SELECT COUNT(*) FROM mood_flags", [], |r| r.get(0)).unwrap();
        assert_eq!(flags, 0);
        assert_eq!(mood_rows(&conn), vec![(id, "grim".into(), Some(0.4), Some(0.75))]);
        assert_eq!(pre_migration_backups(dir.path(), 14).len(), 1);
    }

    #[test]
    fn entries_load_their_flag_per_mood() {
        use crate::models::{FlagBand, FlagBound, FlagKind};
        let conn = open_memory_db();
        let a = library::insert_library_entry(&conn, "/a.mp3", None, "A", None, None, None, "music").unwrap();
        let vocab = MoodVocabulary::defaults();
        let mood = vocab.moods[0].id.clone();
        let sig = mood_flags::lines_sig(&vocab.prompt_signature(&mood).unwrap());
        mood_flags::set_flag(
            &conn, &a, &mood, FlagKind::Band, Some(FlagBand::Understandable), FlagBound::In, Some(0.4), None, &sig,
        )
        .unwrap();

        let entries = load_library_entries(&conn, &vocab).unwrap();
        let e = entries.iter().find(|e| e.id == a).unwrap();
        let flag = e.moods.iter().find(|m| m.mood == mood).unwrap().flag.clone().unwrap();
        assert_eq!(flag.band, Some(FlagBand::Understandable));
        assert!(!flag.stale);
        assert!(e.moods.iter().filter(|m| m.mood != mood).all(|m| m.flag.is_none()));
    }

    #[test]
    fn entries_load_moods_in_vocabulary_order() {
        let conn = open_memory_db();
        let set = library::insert_library_entry(&conn, "/a.mp3", None, "A", None, None, None, "music").unwrap();
        let unset = library::insert_library_entry(&conn, "/b.mp3", None, "B", None, None, None, "music").unwrap();
        conn.execute_batch(&format!(
            "INSERT INTO library_entry_moods (library_entry_id, mood, auto_score, user_level) VALUES
               ('{set}', 'energy', 0.3, NULL),
               ('{set}', 'grim', 0.8, 0.25),
               ('{set}', 'peaceful', NULL, 1.0),
               ('{set}', 'somber', 0.9, NULL);
             UPDATE library_entries SET mood_reviewed_at = '1700000000000' WHERE id = '{set}';"
        ))
        .unwrap();

        let entries = load_library_entries(&conn, &MoodVocabulary::defaults()).unwrap();
        let a = entries.iter().find(|e| e.id == set).unwrap();
        let v = |mood: &str, auto, user| MoodValue { mood: mood.into(), auto, user, flag: None };
        // Every id, in order; missing rows (and a retired id) give neither layer.
        assert_eq!(
            a.moods,
            vec![
                v("peaceful", None, Some(1.0)),
                v("sorrowful", None, None),
                v("wondrous", None, None),
                v("joyful", None, None),
                v("quirky", None, None),
                v("mysterious", None, None),
                v("grim", Some(0.8), Some(0.25)),
                v("heroic", None, None),
                v("battle", None, None),
                v("energy", Some(0.3), None),
            ]
        );
        assert_eq!(a.mood_reviewed_at.as_deref(), Some("1700000000000"));

        let b = entries.iter().find(|e| e.id == unset).unwrap();
        assert_eq!(b.moods.len(), 10);
        assert!(b.moods.iter().all(|m| m.auto.is_none() && m.user.is_none()));
        assert_eq!(b.mood_reviewed_at, None);

        // Another vocabulary: its own ids and order.
        let mut custom = MoodVocabulary::defaults();
        custom.moods.retain(|m| m.id == "grim" || m.id == "peaceful");
        custom.moods.reverse();
        let entries = load_library_entries(&conn, &custom).unwrap();
        let a = entries.iter().find(|e| e.id == set).unwrap();
        assert_eq!(
            a.moods,
            vec![v("grim", Some(0.8), Some(0.25)), v("peaceful", None, Some(1.0)), v("energy", Some(0.3), None)]
        );
    }

    #[test]
    fn state_reports_the_calibration_and_whether_moods_need_recalibration() {
        use crate::autotag::plan::{calibrate, MoodRaw};
        let conn = open_memory_db();
        ensure_test_config(&conn);
        let state = load_full_state(&conn).unwrap();
        assert_eq!((state.mood_calibration, state.moods_need_recalibration), (None, false));

        // A calibration of the defaults, taken from 50 tracks.
        let vocab = MoodVocabulary::defaults();
        let mood: HashMap<String, MoodRaw> = (0..50)
            .map(|k| {
                let raw = 0.1 + (k % 2) as f64 * 0.02;
                let moods = vocab.moods.iter().map(|m| (m.id.clone(), raw)).collect();
                (format!("h{k}"), MoodRaw { moods, energy: raw })
            })
            .collect();
        let cal = calibrate(&mood, &vocab, "1700000000000".into()).unwrap();
        settings::save_mood_calibration(&conn, &cal).unwrap();
        let state = load_full_state(&conn).unwrap();
        assert_eq!(
            state.mood_calibration,
            Some(MoodCalibrationInfo {
                calibrated_at: "1700000000000".into(),
                tracks: 50,
                source: "library".into(),
                reference_set: None,
            })
        );
        assert!(!state.moods_need_recalibration);

        // Changed prompts: needs recalibration.
        let mut edited = vocab.clone();
        edited.moods[2].prompts = vec!["awe".into()];
        library::save_mood_vocabulary(&conn, &edited).unwrap();
        assert!(load_full_state(&conn).unwrap().moods_need_recalibration);
    }

}
