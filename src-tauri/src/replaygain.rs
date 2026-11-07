//! ReplayGain: reading the loudness tags from audio files, and turning them
//! into a per-entry volume factor that evens out music loudness.
//!
//! ## What ReplayGain is
//! A ReplayGain tagger measures how loud a track sounds and writes
//! `REPLAYGAIN_TRACK_GAIN` (in dB): how much to turn the track up or down so
//! it plays at a standard loudness. A loud track gets a negative gain (turn it
//! down), a quiet one a positive gain. `REPLAYGAIN_TRACK_PEAK` is the track's
//! highest sample as a linear value (1.0 = full scale).
//!
//! ## How LuteDrop uses it
//! LuteDrop never turns anything *up*: boosting a quiet track could clip, and
//! the engine clamps volumes to 1 anyway. Instead it picks the library's
//! *typical* loudness, the median gain over music entries, and only turns down
//! tracks louder than that. So for a music entry with gain `g` and the median
//! `reference`:
//!
//! ```text
//! factor = min(1, 10^((g − reference) / 20))
//! ```
//!
//! `g < reference` means the track is louder than typical, so the factor is
//! below 1; typical-or-quieter tracks get exactly 1. Ambiance (quiet beds,
//! meant to sit under the music) and untagged entries always get 1, and so
//! does everything when the setting is off.

use std::collections::HashMap;
use std::path::Path;

use lofty::config::ParseOptions;
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::Tag;
use rusqlite::Connection;

use crate::database::settings;
use crate::models::EntryType;

/// The ReplayGain values read from one file. Either can be missing.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ReplayGain {
    /// `REPLAYGAIN_TRACK_GAIN` in dB (or one converted from `R128_TRACK_GAIN`).
    pub track_gain_db: Option<f64>,
    /// `REPLAYGAIN_TRACK_PEAK`, linear (1.0 = full scale).
    pub track_peak: Option<f64>,
}

/// Gains outside ±60 dB are not real measurements; such a tag is ignored.
const MAX_PLAUSIBLE_GAIN_DB: f64 = 60.0;

/// `R128_TRACK_GAIN` (Opus) is relative to −23 LUFS, ReplayGain 2 to
/// −18 LUFS, so an R128 gain is 5 dB lower than the ReplayGain gain for the
/// same track. Adding this puts both on one scale, so a library that mixes
/// the two still gets one sensible median.
const R128_TO_REPLAYGAIN_DB: f64 = 5.0;

/// Parses a ReplayGain gain such as `"-3.82 dB"`, `"+2.1 dB"` or `"1.5"`.
pub fn parse_gain_db(text: &str) -> Option<f64> {
    let t = text.trim();
    let t = t
        .strip_suffix("dB")
        .or_else(|| t.strip_suffix("db"))
        .or_else(|| t.strip_suffix("DB"))
        .unwrap_or(t)
        .trim();
    let t = t.strip_prefix('+').unwrap_or(t);
    let value: f64 = t.parse().ok()?;
    (value.is_finite() && value.abs() <= MAX_PLAUSIBLE_GAIN_DB).then_some(value)
}

/// Parses a ReplayGain peak such as `"0.988251"`: linear, above zero. Values
/// up to 10 are accepted (decoded lossy audio can overshoot 1.0 a little).
pub fn parse_peak(text: &str) -> Option<f64> {
    let value: f64 = text.trim().parse().ok()?;
    (value.is_finite() && value > 0.0 && value <= 10.0).then_some(value)
}

/// Converts an Opus `R128_TRACK_GAIN` value (a whole number in Q7.8 fixed
/// point, i.e. 1/256 dB steps, relative to −23 LUFS) to a ReplayGain-scale dB
/// gain.
pub fn parse_r128_gain(text: &str) -> Option<f64> {
    let q78: i32 = text.trim().parse().ok()?;
    if !(i16::MIN as i32..=i16::MAX as i32).contains(&q78) {
        return None;
    }
    Some(q78 as f64 / 256.0 + R128_TO_REPLAYGAIN_DB)
}

/// Which ReplayGain field a tag item holds, if any. lofty maps the standard
/// keys (ID3v2 `TXXX:REPLAYGAIN_TRACK_GAIN`, Vorbis `REPLAYGAIN_TRACK_GAIN`)
/// to `ItemKey::ReplayGainTrack*`; anything it does not know (lower-case
/// descriptions some taggers write, `R128_TRACK_GAIN`) arrives as
/// `ItemKey::Unknown` and is matched here by name, ignoring case.
enum Field {
    Gain,
    Peak,
    R128Gain,
}

fn field_of(key: &ItemKey) -> Option<Field> {
    match key {
        ItemKey::ReplayGainTrackGain => Some(Field::Gain),
        ItemKey::ReplayGainTrackPeak => Some(Field::Peak),
        ItemKey::Unknown(name) => {
            if name.eq_ignore_ascii_case("REPLAYGAIN_TRACK_GAIN") {
                Some(Field::Gain)
            } else if name.eq_ignore_ascii_case("REPLAYGAIN_TRACK_PEAK") {
                Some(Field::Peak)
            } else if name.eq_ignore_ascii_case("R128_TRACK_GAIN") {
                Some(Field::R128Gain)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Reads the ReplayGain values from a file's tags (all of them: an MP3 can
/// carry ID3v2 and APE). The first readable value of each field wins. A
/// `REPLAYGAIN_TRACK_GAIN` is preferred over an `R128_TRACK_GAIN`.
pub fn from_tags(tags: &[Tag]) -> ReplayGain {
    let mut gain = None;
    let mut r128 = None;
    let mut peak = None;
    for item in tags.iter().flat_map(|t| t.items()) {
        let Some(text) = item.value().text() else { continue };
        match field_of(item.key()) {
            Some(Field::Gain) if gain.is_none() => gain = parse_gain_db(text),
            Some(Field::Peak) if peak.is_none() => peak = parse_peak(text),
            Some(Field::R128Gain) if r128.is_none() => r128 = parse_r128_gain(text),
            _ => {}
        }
    }
    ReplayGain { track_gain_db: gain.or(r128), track_peak: peak }
}

/// Reads the ReplayGain tags of the file at `path`. Only the tags are read,
/// not the audio properties, so this stays quick on a network share.
pub fn read_file(path: &Path) -> Result<ReplayGain, String> {
    let tagged = Probe::open(path)
        .map_err(|e| format!("open: {e}"))?
        .options(ParseOptions::new().read_properties(false))
        .guess_file_type()
        .map_err(|e| format!("probe: {e}"))?
        .read()
        .map_err(|e| format!("read tags: {e}"))?;
    Ok(from_tags(tagged.tags()))
}

// ─── Normalisation factor ──────────────────────────────────────────────────

/// Median of `values` (mean of the middle two for an even count); `None` for
/// an empty slice.
pub fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let mid = values.len() / 2;
    Some(if values.len() % 2 == 0 {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    })
}

/// The most a track is ever turned down, in dB. Real tags in the library stop
/// around −6 dB; the floor only guards against a corrupt tag (say −50 dB)
/// muting a track.
pub const MAX_REDUCTION_DB: f64 = 12.0;

/// Attenuation-only factor for a track with gain `gain_db` against the
/// library's `reference_db`: below 1 for a louder-than-typical track, else 1,
/// and never below −`MAX_REDUCTION_DB`.
pub fn attenuation_factor(gain_db: f64, reference_db: f64) -> f64 {
    let floor = 10f64.powf(-MAX_REDUCTION_DB / 20.0);
    10f64.powf((gain_db - reference_db) / 20.0).clamp(floor, 1.0)
}

/// Everything needed to work out an entry's loudness factor: whether the
/// setting is on, and the library's typical (median) music gain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Loudness {
    pub enabled: bool,
    /// Median `rg_track_gain_db` over music entries that have one; `None`
    /// when no music entry is tagged.
    pub reference_db: Option<f64>,
}

impl Loudness {
    /// Builds the context from the setting and every music entry's gain.
    pub fn new(enabled: bool, music_gains: impl IntoIterator<Item = f64>) -> Self {
        let mut gains: Vec<f64> = music_gains.into_iter().collect();
        Self { enabled, reference_db: median(&mut gains) }
    }

    /// Reads the setting and the music gains from the database. Cheap: one
    /// pass over `library_entries`.
    pub fn load(conn: &Connection) -> Result<Self, String> {
        let enabled = settings::load_normalize_music_loudness(conn)?;
        let mut stmt = conn
            .prepare(
                "SELECT rg_track_gain_db FROM library_entries
                 WHERE entry_type = 'music' AND rg_track_gain_db IS NOT NULL",
            )
            .map_err(|e| format!("Failed to prepare music gains query: {e}"))?;
        let gains = stmt
            .query_map([], |row| row.get::<_, f64>(0))
            .map_err(|e| format!("Failed to query music gains: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to read music gains: {e}"))?;
        Ok(Self::new(enabled, gains))
    }

    /// The factor the entry's volume is multiplied by on playback (0 < f ≤ 1).
    pub fn factor(&self, entry_type: &EntryType, gain_db: Option<f64>) -> f64 {
        match (self.enabled, entry_type, gain_db, self.reference_db) {
            (true, EntryType::Music, Some(g), Some(reference)) => attenuation_factor(g, reference),
            _ => 1.0,
        }
    }
}

/// The factor of every library entry, by id, for pushing to the engine when
/// the setting or the reference changes while tracks play.
pub fn all_factors(conn: &Connection) -> Result<HashMap<String, f64>, String> {
    let loudness = Loudness::load(conn)?;
    let mut stmt = conn
        .prepare("SELECT id, entry_type, rg_track_gain_db FROM library_entries")
        .map_err(|e| format!("Failed to prepare loudness query: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<f64>>(2)?))
        })
        .map_err(|e| format!("Failed to query loudness: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read loudness: {e}"))?;
    Ok(rows
        .into_iter()
        .map(|(id, ty, gain)| {
            let factor = loudness.factor(&EntryType::from_str(&ty), gain);
            (id, factor)
        })
        .collect())
}

/// The factor of one entry (1 if it does not exist).
pub fn factor_for_entry(conn: &Connection, entry_id: &str) -> Result<f64, String> {
    use rusqlite::OptionalExtension;
    let row: Option<(String, Option<f64>)> = conn
        .query_row(
            "SELECT entry_type, rg_track_gain_db FROM library_entries WHERE id = ?1",
            [entry_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| format!("Failed to read entry loudness: {e}"))?;
    match row {
        Some((ty, gain)) => Ok(Loudness::load(conn)?.factor(&EntryType::from_str(&ty), gain)),
        None => Ok(1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lofty::id3::v2::Id3v2Tag;
    use lofty::ogg::VorbisComments;

    #[test]
    fn corrupt_gain_never_mutes_a_track() {
        // A −50 dB tag against a −4 dB reference would be −46 dB: floored at −12 dB.
        let f = attenuation_factor(-50.0, -4.0);
        assert!((20.0 * f.log10() + MAX_REDUCTION_DB).abs() < 1e-9, "factor {f}");
        // Real-world reductions pass through untouched.
        let real = attenuation_factor(-10.1, -3.8);
        assert!((20.0 * real.log10() + 6.3).abs() < 1e-9, "factor {real}");
    }

    #[test]
    fn parses_gain_strings() {
        assert_eq!(parse_gain_db("-3.82 dB"), Some(-3.82));
        assert_eq!(parse_gain_db("+2.10 dB"), Some(2.1));
        assert_eq!(parse_gain_db(" 1.5 "), Some(1.5));
        assert_eq!(parse_gain_db("-7.00db"), Some(-7.0));
        assert_eq!(parse_gain_db("loud"), None);
        assert_eq!(parse_gain_db("NaN dB"), None);
        assert_eq!(parse_gain_db("-99 dB"), None);
    }

    #[test]
    fn parses_peaks() {
        assert_eq!(parse_peak("0.988251"), Some(0.988251));
        assert_eq!(parse_peak("1.05"), Some(1.05));
        assert_eq!(parse_peak("0"), None);
        assert_eq!(parse_peak("-0.5"), None);
        assert_eq!(parse_peak("x"), None);
    }

    #[test]
    fn r128_gain_is_q78_shifted_to_the_replaygain_scale() {
        // −2048/256 = −8 dB at −23 LUFS → −3 dB at −18 LUFS.
        assert_eq!(parse_r128_gain("-2048"), Some(-3.0));
        assert_eq!(parse_r128_gain("0"), Some(5.0));
        assert_eq!(parse_r128_gain("40000"), None);
        assert_eq!(parse_r128_gain("-1.5"), None);
    }

    #[test]
    fn reads_id3v2_txxx_frames() {
        let mut id3 = Id3v2Tag::new();
        id3.insert_user_text("REPLAYGAIN_TRACK_GAIN".into(), "-6.48 dB".into());
        id3.insert_user_text("REPLAYGAIN_TRACK_PEAK".into(), "0.977000".into());
        let rg = from_tags(&[Tag::from(id3)]);
        assert_eq!(rg, ReplayGain { track_gain_db: Some(-6.48), track_peak: Some(0.977) });
    }

    #[test]
    fn reads_lower_case_txxx_descriptions() {
        let mut id3 = Id3v2Tag::new();
        id3.insert_user_text("replaygain_track_gain".into(), "+1.25 dB".into());
        let rg = from_tags(&[Tag::from(id3)]);
        assert_eq!(rg.track_gain_db, Some(1.25));
        assert_eq!(rg.track_peak, None);
    }

    #[test]
    fn reads_vorbis_comments() {
        let mut vc = VorbisComments::new();
        vc.insert("REPLAYGAIN_TRACK_GAIN".into(), "-3.10 dB".into());
        vc.insert("REPLAYGAIN_TRACK_PEAK".into(), "0.5".into());
        let rg = from_tags(&[Tag::from(vc)]);
        assert_eq!(rg, ReplayGain { track_gain_db: Some(-3.1), track_peak: Some(0.5) });
    }

    #[test]
    fn falls_back_to_r128_and_prefers_replaygain() {
        let mut vc = VorbisComments::new();
        vc.insert("R128_TRACK_GAIN".into(), "-512".into());
        assert_eq!(from_tags(&[Tag::from(vc.clone())]).track_gain_db, Some(3.0));
        vc.insert("REPLAYGAIN_TRACK_GAIN".into(), "-4 dB".into());
        assert_eq!(from_tags(&[Tag::from(vc)]).track_gain_db, Some(-4.0));
    }

    #[test]
    fn untagged_reads_as_none() {
        assert_eq!(from_tags(&[]), ReplayGain::default());
        let mut vc = VorbisComments::new();
        vc.insert("TITLE".into(), "x".into());
        assert_eq!(from_tags(&[Tag::from(vc)]), ReplayGain::default());
    }

    /// Writes Vorbis comments into a copy of the Opus fixture and reads them
    /// back from disk, end to end through lofty's file probing.
    #[test]
    fn reads_tags_from_an_opus_file() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.opus");
        let dir = tempfile::tempdir().unwrap();
        let copy = dir.path().join("tagged.opus");
        std::fs::copy(&fixture, &copy).unwrap();

        let mut vc = VorbisComments::new();
        vc.insert("REPLAYGAIN_TRACK_GAIN".into(), "-2.50 dB".into());
        vc.insert("REPLAYGAIN_TRACK_PEAK".into(), "0.912".into());
        vc.save_to_path(&copy, lofty::config::WriteOptions::default()).unwrap();

        let rg = read_file(&copy).unwrap();
        assert_eq!(rg, ReplayGain { track_gain_db: Some(-2.5), track_peak: Some(0.912) });
    }

    #[test]
    fn missing_file_is_an_error() {
        assert!(read_file(Path::new("Z:/definitely/missing.opus")).is_err());
    }

    #[test]
    fn median_of_odd_even_and_empty() {
        assert_eq!(median(&mut [3.0, -1.0, 2.0]), Some(2.0));
        assert_eq!(median(&mut [4.0, 1.0, 2.0, 3.0]), Some(2.5));
        assert_eq!(median(&mut []), None);
    }

    #[test]
    fn only_louder_than_typical_music_is_turned_down() {
        let l = Loudness::new(true, [-6.0, -4.0, -2.0]);
        assert_eq!(l.reference_db, Some(-4.0));
        // 6 dB louder than typical → about half the amplitude.
        let f = l.factor(&EntryType::Music, Some(-10.0));
        assert!((f - 10f64.powf(-6.0 / 20.0)).abs() < 1e-12);
        assert!((f - 0.501).abs() < 0.001);
        // Typical and quieter tracks are left alone (never boosted).
        assert_eq!(l.factor(&EntryType::Music, Some(-4.0)), 1.0);
        assert_eq!(l.factor(&EntryType::Music, Some(8.0)), 1.0);
    }

    #[test]
    fn ambiance_untagged_and_off_get_one() {
        let l = Loudness::new(true, [-4.0]);
        assert_eq!(l.factor(&EntryType::Ambiance, Some(-20.0)), 1.0);
        assert_eq!(l.factor(&EntryType::Music, None), 1.0);
        let off = Loudness::new(false, [-4.0]);
        assert_eq!(off.factor(&EntryType::Music, Some(-20.0)), 1.0);
        let no_reference = Loudness::new(true, []);
        assert_eq!(no_reference.factor(&EntryType::Music, Some(-20.0)), 1.0);
    }

    #[test]
    fn db_reference_counts_only_tagged_music() {
        let conn = crate::database::open_memory_db();
        let lib = crate::database::library::insert_library_entry;
        let a = lib(&conn, "/a.mp3", None, "A", None, None, None, "music").unwrap();
        let b = lib(&conn, "/b.mp3", None, "B", None, None, None, "music").unwrap();
        let c = lib(&conn, "/c.mp3", None, "C", None, None, None, "music").unwrap();
        let amb = lib(&conn, "/d.ogg", None, "D", None, None, None, "ambiance").unwrap();
        let untagged = lib(&conn, "/e.mp3", None, "E", None, None, None, "music").unwrap();
        let set = crate::database::library::set_replaygain;
        set(&conn, &a, &ReplayGain { track_gain_db: Some(-8.0), track_peak: Some(1.0) }).unwrap();
        set(&conn, &b, &ReplayGain { track_gain_db: Some(-4.0), track_peak: None }).unwrap();
        set(&conn, &c, &ReplayGain { track_gain_db: Some(0.0), track_peak: None }).unwrap();
        set(&conn, &amb, &ReplayGain { track_gain_db: Some(-30.0), track_peak: None }).unwrap();

        let l = Loudness::load(&conn).unwrap();
        assert!(l.enabled, "the setting defaults to on");
        assert_eq!(l.reference_db, Some(-4.0));

        let factors = all_factors(&conn).unwrap();
        assert!((factors[&a] - 10f64.powf(-4.0 / 20.0)).abs() < 1e-12);
        assert_eq!(factors[&b], 1.0);
        assert_eq!(factors[&c], 1.0);
        assert_eq!(factors[&amb], 1.0);
        assert_eq!(factors[&untagged], 1.0);
        assert_eq!(factor_for_entry(&conn, &a).unwrap(), factors[&a]);
        assert_eq!(factor_for_entry(&conn, "nope").unwrap(), 1.0);

        settings::save_normalize_music_loudness(&conn, false).unwrap();
        assert!(all_factors(&conn).unwrap().values().all(|f| *f == 1.0));
    }
}
