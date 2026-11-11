//! Pure planning for an analysis run: which entries the analyzer scores, and
//! which mood values its raw scores earn. Nothing here touches the database
//! or the network.

use crate::models::{EnergyPrompts, MoodVocabulary, LearnedMoodProfile, ENERGY_ID};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

/// What the planner needs to know about one library entry.
#[derive(Debug, Clone)]
pub struct EntryFacts {
    pub id: String,
    /// SHA-256 of the file; `None` until the entry has been hashed.
    pub hash: Option<String>,
    pub is_music: bool,
}

/// Body of the analyzer's `POST /score`: the hashes to score moods for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreRequest {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, LearnedMoodProfile>,
    pub targets: Vec<String>,
    /// The app's mood vocabulary: prompts per mood id. The analyzer uses its
    /// own defaults when this is left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mood_prompts: Option<BTreeMap<String, Vec<String>>>,
    /// Energy's prompts, both sides; defaults when left out, as above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_prompts: Option<EnergyPrompts>,
}

impl ScoreRequest {
    /// The request with the vocabulary's prompts, so the analyzer scores the
    /// app's moods rather than its defaults.
    pub fn with_moods(mut self, vocabulary: &MoodVocabulary) -> Self {
        self.profiles = vocabulary.moods.iter().filter_map(|m| Some((m.id.clone(), m.profile.clone()?))).collect();
        self.mood_prompts = Some(vocabulary.moods.iter().map(|m| (m.id.clone(), m.prompts.clone())).collect());
        self.energy_prompts = Some(vocabulary.energy.clone());
        self
    }
}

/// Response body of the analyzer's `POST /score`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreResponse {
    pub model: String,
    /// Raw mood scores per entry hash. Empty when the analyzer sent none
    /// (its mood model is not loaded, or it predates mood). An entry without
    /// the `{moods, energy}` shape (an analyzer that predates the mood
    /// profile) is left out, as if the analyzer sent no mood for it.
    #[serde(default, deserialize_with = "complete_moods")]
    pub mood: HashMap<String, MoodRaw>,
    /// Mood statistics of the analyzer's fixed reference set, when it sends
    /// them. `None` when absent (an older analyzer) or malformed.
    #[serde(default, deserialize_with = "lenient_reference")]
    pub mood_reference: Option<MoodReference>,
}

/// The analyzer's fixed reference set: how many tracks it holds and, per mood
/// id (Energy included), the mean and population sd of their raw scores.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoodReference {
    /// The set's id, e.g. `incompetech-300-v1`.
    pub set: String,
    pub tracks: usize,
    pub stats: HashMap<String, MoodStat>,
}

/// Reads `mood_reference`, turning anything that is not the expected shape
/// into `None`. A plain `Option<MoodReference>` would fail the whole response
/// on a bad value; here we first read it as untyped JSON (which always
/// works), then try to convert, and `.ok()` swallows a failed conversion.
fn lenient_reference<'de, D: Deserializer<'de>>(d: D) -> Result<Option<MoodReference>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).ok())
}

/// The analyzer's raw mood scores for one entry: each mood's absolute prompt
/// similarity keyed by mood id (the ids the request sent), and Energy (energetic minus calm prompt
/// similarity). Any other field the analyzer sends (`pools`, or the scales
/// `bright`, `grand`, `sad`, `scary` and `light`, from older analyzers) is
/// ignored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MoodRaw {
    #[serde(default)]
    pub learned: HashMap<String, f64>,
    pub moods: HashMap<String, f64>,
    pub energy: f64,
}

/// Reads the `mood` map, keeping only the entries in the current shape.
fn complete_moods<'de, D: Deserializer<'de>>(d: D) -> Result<HashMap<String, MoodRaw>, D::Error> {
    let raw: HashMap<String, serde_json::Value> = HashMap::deserialize(d)?;
    Ok(raw
        .into_iter()
        .filter_map(|(hash, value)| Some((hash, serde_json::from_value(value).ok()?)))
        .collect())
}

/// A calibration needs raw mood scores from at least this many music tracks.
pub const CALIBRATION_MIN_TRACKS: usize = 50;

/// The slope of the logistic that turns a z-score into 0..1:
/// `score = 1 / (1 + exp(-MOOD_SLOPE * (z - midpoint)))`.
pub const MOOD_SLOPE: f64 = 1.5;

/// Where a mood's bar reads 0.5, in standard deviations above the library
/// average. A mood is rare in most libraries, so "above average" is cheap;
/// with the midpoint half an sd up, about one track in ten per mood reaches
/// the top band (0.75 and above) in a library of a few hundred, which is the
/// head count the user settled on (one sd up left only the top 5%). Energy
/// keeps its midpoint at the average (0): it is two-sided, and average energy
/// is a meaningful middle.
pub const MOOD_MIDPOINT_Z: f64 = 0.5;

/// Where a mood's bar reads 0.5 against the reference set: the 75th
/// percentile of a normal distribution (z = 0.674).
pub const REFERENCE_MIDPOINT_Z: f64 = 0.674;

/// Slope against the reference set: chosen so the bar reads 0.75 at the 90th
/// percentile (z = 1.2816), i.e. `ln(3)` of logit over that span.
pub fn reference_slope() -> f64 {
    3f64.ln() / (1.2816 - REFERENCE_MIDPOINT_Z)
}

/// Which statistics a calibration holds: the user's own library, or the
/// analyzer's fixed reference set.
// `rename_all = "lowercase"` makes the JSON values "library" / "reference".
// `#[default]` marks the variant `Default::default()` returns, which serde
// uses for old stored calibrations that have no `source` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CalibrationSource {
    #[default]
    Library,
    Reference,
}

/// The z-score at which `id`'s bar reads 0.5 for a calibration of `source`.
pub fn midpoint_z(id: &str, source: CalibrationSource) -> f64 {
    match (id == ENERGY_ID, source) {
        (true, _) => 0.0,
        (false, CalibrationSource::Library) => MOOD_MIDPOINT_Z,
        (false, CalibrationSource::Reference) => REFERENCE_MIDPOINT_Z,
    }
}

/// The logistic slope for a calibration of `source`. Energy uses the library
/// slope either way: it has no reference percentile lines of its own.
pub(crate) fn slope(id: &str, source: CalibrationSource) -> f64 {
    if id != ENERGY_ID && source == CalibrationSource::Reference {
        reference_slope()
    } else {
        MOOD_SLOPE
    }
}

/// Mean and standard deviation of one mood's raw score.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MoodStat {
    pub mean: f64,
    pub sd: f64,
}

/// The frozen mood calibration: per id (Energy included), the mean and
/// population standard deviation of its raw score across the library's music
/// when it was taken, plus the prompts each id was scored with then. Taken
/// once, from the first `/score` response that covers at least
/// `CALIBRATION_MIN_TRACKS` music tracks, and never recomputed automatically
/// (only Recalibrate clears it), so values do not drift as the library grows.
/// Stored as JSON `{calibratedAt, tracks, prompts: {id: [..]}, stats: {id:
/// {mean, sd}}}` in the `mood_calibration` setting, plus the optional keys
/// `source` ("library" or "reference"; absent in older saves, read as
/// library) and `referenceSet` (the reference set's id; only written for a
/// reference calibration).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoodCalibration {
    /// Unix milliseconds as a string.
    pub calibrated_at: String,
    /// How many music tracks the statistics come from.
    pub tracks: usize,
    /// Each id's prompt signature (`MoodVocabulary::prompt_signature`) at
    /// the time. An id whose prompts changed since is not scored until
    /// Recalibrate.
    pub prompts: BTreeMap<String, Vec<String>>,
    pub stats: BTreeMap<String, MoodStat>,
    /// Where the statistics come from. Calibrations saved before this field
    /// existed have none, and read as `Library`.
    #[serde(default)]
    pub source: CalibrationSource,
    /// For a `Reference` calibration, the reference set's id.
    // `skip_serializing_if` leaves the key out of the JSON when it is `None`,
    // so library calibrations are stored exactly as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_set: Option<String>,
}

/// One raw score of an entry: a mood's, or Energy's.
fn raw_score(mood: &MoodRaw, id: &str) -> Option<f64> {
    if id == ENERGY_ID {
        Some(mood.energy)
    } else {
        mood.moods.get(id).copied()
    }
}

/// Computes a mood calibration for every id of `vocabulary` from one
/// `/score` response's raw mood scores (population standard deviation).
/// `None` when fewer than `CALIBRATION_MIN_TRACKS` entries carry every id's
/// score, or when an id has no spread (its z-scores would be meaningless).
/// The response only holds music targets, so every row is a music track.
pub fn calibrate(
    mood: &HashMap<String, MoodRaw>,
    vocabulary: &MoodVocabulary,
    calibrated_at: String,
) -> Option<MoodCalibration> {
    let ids = vocabulary.ids();
    let rows: Vec<Vec<f64>> = mood
        .values()
        .filter_map(|m| ids.iter().map(|id| raw_score(m, id)).collect::<Option<Vec<f64>>>())
        .collect();
    if rows.len() < CALIBRATION_MIN_TRACKS {
        return None;
    }
    let n = rows.len() as f64;
    let mut stats = BTreeMap::new();
    let mut prompts = BTreeMap::new();
    for (i, id) in ids.iter().enumerate() {
        let mean = rows.iter().map(|r| r[i]).sum::<f64>() / n;
        let sd = (rows.iter().map(|r| (r[i] - mean).powi(2)).sum::<f64>() / n).sqrt();
        if !sd.is_finite() || sd <= 1e-12 {
            return None;
        }
        stats.insert(id.clone(), MoodStat { mean, sd });
        prompts.insert(id.clone(), vocabulary.prompt_signature(id)?);
    }
    Some(MoodCalibration {
        calibrated_at,
        tracks: rows.len(),
        prompts,
        stats,
        source: CalibrationSource::Library,
        reference_set: None,
    })
}

/// A calibration built from the analyzer's reference-set statistics.
/// `None` unless every id of `vocabulary` has a stat with a usable spread
/// (finite sd above 1e-12), same rule as `calibrate`.
pub fn reference_calibration(
    reference: &MoodReference,
    vocabulary: &MoodVocabulary,
    calibrated_at: String,
) -> Option<MoodCalibration> {
    let mut stats = BTreeMap::new();
    let mut prompts = BTreeMap::new();
    for id in vocabulary.ids() {
        let stat = *reference.stats.get(&id)?;
        if !stat.sd.is_finite() || stat.sd <= 1e-12 {
            return None;
        }
        prompts.insert(id.clone(), vocabulary.prompt_signature(&id)?);
        stats.insert(id, stat);
    }
    Some(MoodCalibration {
        calibrated_at,
        tracks: reference.tracks,
        prompts,
        stats,
        source: CalibrationSource::Reference,
        reference_set: Some(reference.set.clone()),
    })
}

/// A library calibration with statistics added for the ids it cannot score
/// (a mood added, or its prompts changed, since it was taken), computed from
/// this response's raw scores for those ids alone; every other id keeps its
/// statistics. So after a mood's lines change, the next run rescores that
/// mood without touching the others. `None` when there is nothing to add,
/// the calibration is a reference one (the reference set is offered whole),
/// or an id lacks `CALIBRATION_MIN_TRACKS` scores or spread.
pub fn extend_library_calibration(
    stored: &MoodCalibration,
    mood: &HashMap<String, MoodRaw>,
    vocabulary: &MoodVocabulary,
) -> Option<MoodCalibration> {
    if stored.source != CalibrationSource::Library {
        return None;
    }
    let scorable: HashSet<String> = scorable_ids(stored, vocabulary).into_iter().collect();
    let missing: Vec<String> = vocabulary.ids().into_iter().filter(|id| !scorable.contains(id)).collect();
    if missing.is_empty() {
        return None;
    }
    let mut out = stored.clone();
    for id in missing {
        let values: Vec<f64> = mood.values().filter_map(|m| raw_score(m, &id)).collect();
        if values.len() < CALIBRATION_MIN_TRACKS {
            return None;
        }
        let n = values.len() as f64;
        let mean = values.iter().sum::<f64>() / n;
        let sd = (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n).sqrt();
        if !sd.is_finite() || sd <= 1e-12 {
            return None;
        }
        out.prompts.insert(id.clone(), vocabulary.prompt_signature(&id)?);
        out.stats.insert(id, MoodStat { mean, sd });
    }
    Some(out)
}

/// True when two floats are equal up to rounding noise: within a relative
/// 1e-12 (or an absolute 1e-12 for values near zero).
fn approx_eq(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-12 * 1.0_f64.max(a.abs()).max(b.abs())
}

/// True when two calibrations are the same apart from when they were taken.
/// The statistics are compared with a tiny tolerance: reading a float back
/// from JSON can differ from the original in its last bit, and an exact
/// comparison would then see a "change" and re-save the calibration every run.
pub fn same_calibration(a: &MoodCalibration, b: &MoodCalibration) -> bool {
    let stats_match = a.stats.len() == b.stats.len()
        && a.stats.iter().all(|(id, sa)| {
            b.stats
                .get(id)
                .is_some_and(|sb| approx_eq(sa.mean, sb.mean) && approx_eq(sa.sd, sb.sd))
        });
    a.tracks == b.tracks
        && a.prompts == b.prompts
        && stats_match
        && a.source == b.source
        && a.reference_set == b.reference_set
}

/// The ids of `vocabulary` the calibration can score: those it has
/// statistics for, taken with the same prompts the vocabulary has now. In
/// profile order.
pub fn scorable_ids(calibration: &MoodCalibration, vocabulary: &MoodVocabulary) -> Vec<String> {
    vocabulary
        .ids()
        .into_iter()
        .filter(|id| {
            calibration.stats.contains_key(id)
                && calibration.prompts.get(id) == vocabulary.prompt_signature(id).as_ref()
        })
        .collect()
}

/// True when there is a calibration but it cannot score some id of the
/// vocabulary (a mood added, or prompts changed, since it was taken). Without
/// a calibration the next run takes one, so nothing needs recalibrating.
pub fn needs_recalibration(calibration: Option<&MoodCalibration>, vocabulary: &MoodVocabulary) -> bool {
    calibration.is_some_and(|cal| scorable_ids(cal, vocabulary).len() < vocabulary.moods.len() + 1)
}

/// Turns one raw score into 0..1: `z = (raw - mean) / sd`, then
/// `1 / (1 + exp(-slope * (z - midpoint)))`, clamped to [0, 1]. Midpoint and
/// slope depend on the calibration's `source` (see `midpoint_z`).
/// The ranking never depends on this; only the number shown does.
pub fn mood_score(id: &str, raw: f64, stat: MoodStat, source: CalibrationSource) -> f64 {
    let z = (raw - stat.mean) / stat.sd - midpoint_z(id, source);
    (1.0 / (1.0 + (-slope(id, source) * z).exp())).clamp(0.0, 1.0)
}

/// The analyzer's 0..1 value for each of `ids` (see `scorable_ids`) on one
/// entry, in that order. An id the entry has no raw score for, or the
/// calibration has no statistics for, is left out; `None` when that leaves
/// nothing.
///
/// Rule: moods never depend on Role. Only the entry's raw scores and the
/// frozen calibration go in; a combat track can be Peaceful.
///
/// Learned profiles return final personal-fit bars; these never receive the
/// reference logistic a second time. Other moods retain word calibration.
pub fn auto_moods(mood: &MoodRaw, calibration: &MoodCalibration, ids: &[String]) -> Option<Vec<(String, f64)>> {
    let out: Vec<(String, f64)> = ids
        .iter()
        .filter_map(|id| {
            if let Some(value) = mood.learned.get(id).filter(|v| v.is_finite() && **v >= 0.0 && **v <= 1.0) {
                return Some((id.clone(), *value));
            }
            let raw = raw_score(mood, id)?;
            let stat = *calibration.stats.get(id)?;
            Some((id.clone(), mood_score(id, raw, stat, calibration.source)))
        })
        .collect();
    (!out.is_empty()).then_some(out)
}

/// The analyzer layer one scored entry should carry after a run. It is
/// decided from the scores alone, whatever the user's layer holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    /// The analyzer's value for each scorable mood id (see `auto_moods`),
    /// when there is a mood calibration and the analyzer sent the entry's
    /// raw mood scores.
    pub moods: Option<Vec<(String, f64)>>,
}

/// Turns a local library path into the path the analyzer host sees.
///
/// The prefix match ignores ASCII case and treats `\` and `/` alike. Returns
/// `None` when `local_prefix` is empty, the path is not inside it, nothing
/// follows the prefix, or the rest of the path has a `..` segment.
pub fn map_path(local: &str, local_prefix: &str, remote_prefix: &str) -> Option<String> {
    let local = local.replace('\\', "/");
    let prefix = local_prefix.replace('\\', "/");
    let prefix = prefix.trim_end_matches('/');
    if prefix.is_empty() || local.len() < prefix.len() || !local.is_char_boundary(prefix.len()) {
        return None;
    }
    let (head, rest) = local.split_at(prefix.len());
    if !head.eq_ignore_ascii_case(prefix) {
        return None;
    }
    // "Z:/media/audio2" must not match the prefix "Z:/media/audio".
    if !rest.is_empty() && !rest.starts_with('/') {
        return None;
    }
    let rest = rest.trim_start_matches('/');
    if rest.is_empty() || rest.split('/').any(|part| part == "..") {
        return None;
    }
    let remote = remote_prefix.trim_end_matches('/');
    Some(format!("{remote}/{rest}"))
}

/// Builds the `/score` request: every music entry whose hash is in
/// `embedded` is a target.
pub fn build_score_request(entries: &[EntryFacts], embedded: &HashSet<String>) -> ScoreRequest {
    let targets = entries
        .iter()
        .filter(|e| e.is_music)
        .filter_map(|e| e.hash.as_ref())
        .filter(|hash| embedded.contains(*hash))
        .cloned()
        .collect();
    ScoreRequest {
        profiles: BTreeMap::new(),
        targets,
        mood_prompts: None,
        energy_prompts: None,
    }
}

/// Decides the analyzer layer (moods) of each scored entry.
///
/// Every music entry whose hash was a `/score` target appears in the result,
/// so callers can tell scored entries from the rest; an entry the analyzer
/// sent no mood for gets `moods: None` (its stored values stay). Moods are
/// scored only with a `calibration`, and only the `mood_ids` it can score
/// (see `scorable_ids` and `auto_moods`).
pub fn decide(
    entries: &[EntryFacts],
    targets: &[String],
    resp: &ScoreResponse,
    calibration: Option<&MoodCalibration>,
    mood_ids: &[String],
) -> HashMap<String, Decision> {
    let targets: HashSet<&str> = targets.iter().map(String::as_str).collect();
    entries
        .iter()
        .filter(|e| e.is_music)
        .filter_map(|e| {
            let hash = e.hash.as_deref().filter(|h| targets.contains(h))?;
            let moods = calibration.and_then(|cal| auto_moods(resp.mood.get(hash)?, cal, mood_ids));
            Some((e.id.clone(), Decision { moods }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MoodDef;

    fn entry(id: &str) -> EntryFacts {
        EntryFacts {
            id: id.into(),
            hash: Some(format!("h-{id}")),
            is_music: true,
        }
    }

    fn response() -> ScoreResponse {
        ScoreResponse {
            mood: HashMap::new(),
            mood_reference: None,
            model: "test".into(),
        }
    }

    fn hashes(list: &[&str]) -> Vec<String> {
        list.iter().map(|h| h.to_string()).collect()
    }

    #[test]
    fn map_path_handles_case_and_slashes() {
        assert_eq!(
            map_path(
                "z:/Media/Audio/music/A/b.opus",
                "Z:\\media\\audio",
                "/media/audio"
            ),
            Some("/media/audio/music/A/b.opus".to_string())
        );
    }

    #[test]
    fn map_path_outside_prefix_is_none() {
        assert_eq!(
            map_path("D:/other/a.opus", "Z:/media/audio", "/media/audio"),
            None
        );
        // Same leading letters, different folder.
        assert_eq!(
            map_path("Z:/media/audio2/a.opus", "Z:/media/audio", "/media/audio"),
            None
        );
    }

    #[test]
    fn map_path_rejects_empty_remainder_and_parent_segments() {
        assert_eq!(
            map_path("Z:/media/audio", "Z:/media/audio", "/media/audio"),
            None
        );
        assert_eq!(
            map_path("Z:/media/audio/", "Z:/media/audio", "/media/audio"),
            None
        );
        assert_eq!(
            map_path(
                "Z:/media/audio/../secret.opus",
                "Z:/media/audio",
                "/media/audio"
            ),
            None
        );
    }

    #[test]
    fn map_path_empty_prefix_is_none() {
        assert_eq!(map_path("Z:/media/audio/a.opus", "", "/media/audio"), None);
    }

    #[test]
    fn request_targets_only_embedded_music() {
        let mut not_music = entry("n");
        not_music.is_music = false;
        let mut no_hash = entry("x");
        no_hash.hash = None;
        let entries = vec![entry("a"), entry("b"), not_music, no_hash];
        let embedded: HashSet<String> = ["h-a", "h-n"].iter().map(|s| s.to_string()).collect();
        let req = build_score_request(&entries, &embedded);
        assert_eq!(req.targets, hashes(&["h-a"]));
    }

    #[test]
    fn decide_covers_every_music_target_whatever_the_response_holds() {
        // The analyzer sends no gate or tag scores any more: an entry is
        // decided because it was a target, with moods from `resp.mood`.
        let mut no_hash = entry("n");
        no_hash.hash = None;
        let mut not_music = entry("m");
        not_music.is_music = false;
        let entries = vec![entry("scored"), entry("no-mood"), entry("untargeted"), no_hash, not_music];
        let mut resp = response();
        resp.mood = [("h-scored".to_string(), with_moods([0.2; N]))].into();
        let targets = hashes(&["h-scored", "h-no-mood", "h-m"]);
        let cal = flat_calibration(0.2, 0.01);
        let out = decide(&entries, &targets, &resp, Some(&cal), &ids());
        assert_eq!(out.keys().count(), 2);
        assert_eq!(out["scored"].moods.as_ref().map(Vec::len), Some(N));
        assert_eq!(out["no-mood"].moods, None);
        assert!(!out.contains_key("untargeted") && !out.contains_key("n") && !out.contains_key("m"));
    }

    #[test]
    fn request_sends_no_pace() {
        let req = build_score_request(&[entry("a")], &HashSet::new());
        let json = serde_json::to_value(&req).unwrap();
        for gone in ["pace", "gate", "tags", "scene"] {
            assert!(json.get(gone).is_none(), "{gone}");
        }
    }

    /// The default vocabulary's ids: nine moods, then energy.
    fn ids() -> Vec<String> {
        MoodVocabulary::defaults().ids()
    }

    /// Raw scores in the default vocabulary's order (nine moods, then energy).
    const N: usize = 10;
    fn with_moods(raws: [f64; N]) -> MoodRaw {
        let ids = ids();
        MoodRaw { learned: Default::default(),
            moods: ids[..N - 1].iter().zip(raws).map(|(m, v)| (m.clone(), v)).collect(),
            energy: raws[N - 1],
        }
    }

    #[test]
    fn learned_bars_bypass_reference_calibration() {
        let cal = flat_calibration(0.2, 0.05);
        let mut raw = with_moods([0.9; N]);
        raw.learned.insert("grim".into(), 0.31);
        let scored = auto_moods(&raw, &cal, &ids()).unwrap();
        assert_eq!(scored.iter().find(|(id, _)| id == "grim").unwrap().1, 0.31);
        assert!(scored.iter().find(|(id, _)| id == "heroic").unwrap().1 > 0.99);
    }

    /// A calibration of the default vocabulary with the same mean and sd for
    /// every id.
    fn flat_calibration(mean: f64, sd: f64) -> MoodCalibration {
        let v = MoodVocabulary::defaults();
        MoodCalibration {
            calibrated_at: "1".into(),
            tracks: 50,
            prompts: ids().into_iter().map(|id| (id.clone(), v.prompt_signature(&id).unwrap())).collect(),
            stats: ids().into_iter().map(|id| (id, MoodStat { mean, sd })).collect(),
            source: CalibrationSource::Library,
            reference_set: None,
        }
    }

    #[test]
    fn request_carries_the_vocabulary_only_when_given() {
        let req = build_score_request(&[entry("a")], &HashSet::new());
        let json = serde_json::to_value(&req).unwrap();
        assert!(json.get("mood_prompts").is_none() && json.get("energy_prompts").is_none());

        let mut v = MoodVocabulary::defaults();
        v.moods.truncate(2);
        let json = serde_json::to_value(req.with_moods(&v)).unwrap();
        assert_eq!(
            json["mood_prompts"],
            serde_json::json!({
                "peaceful": v.moods[0].prompts,
                "sorrowful": v.moods[1].prompts,
            })
        );
        assert_eq!(json["energy_prompts"]["positive"][0], "energetic, exciting, intense, upbeat, powerful");
        assert_eq!(json["energy_prompts"]["negative"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn extend_library_calibration_adds_only_the_changed_mood() {
        let stored = flat_calibration(0.2, 0.05);
        let mut v = MoodVocabulary::defaults();
        assert_eq!(extend_library_calibration(&stored, &HashMap::new(), &v), None, "nothing to add");
        v.moods[7].prompts = vec!["fanfare".into()];
        let id = v.moods[7].id.clone();
        let mood: HashMap<String, MoodRaw> = (0..CALIBRATION_MIN_TRACKS)
            .map(|k| (format!("h{k}"), with_moods([0.5 + (k % 2) as f64 * 0.1; N])))
            .collect();
        let out = extend_library_calibration(&stored, &mood, &v).unwrap();
        assert_eq!(out.prompts[&id], vec!["fanfare"]);
        assert!((out.stats[&id].mean - 0.55).abs() < 1e-9 && (out.stats[&id].sd - 0.05).abs() < 1e-9);
        for other in ids().into_iter().filter(|x| *x != id) {
            assert_eq!(out.stats[&other], stored.stats[&other], "{other} keeps its stats");
        }
        assert!(!needs_recalibration(Some(&out), &v));
        // Too few tracks, or a reference calibration: nothing.
        let few: HashMap<String, MoodRaw> = mood.into_iter().take(CALIBRATION_MIN_TRACKS - 1).collect();
        assert_eq!(extend_library_calibration(&stored, &few, &v), None);
        let reference = MoodCalibration { source: CalibrationSource::Reference, ..stored };
        assert_eq!(extend_library_calibration(&reference, &HashMap::new(), &v), None);
    }

    #[test]
    fn library_mood_score_is_unchanged() {
        let stat = MoodStat { mean: 0.2, sd: 0.04 };
        // A mood reads 0.5 half an sd above the mean (z = 0.5); z = 1 reads
        // what z = 0.5 used to, and the mean reads below half.
        assert!((mood_score("heroic", 0.22, stat, CalibrationSource::Library) - 0.5).abs() < 1e-12);
        assert!((mood_score("heroic", 0.20, stat, CalibrationSource::Library) - 0.320821300824607025).abs() < 1e-12);
        assert!((mood_score("heroic", 0.24, stat, CalibrationSource::Library) - 0.679178699175392975).abs() < 1e-12);
        // Energy keeps its midpoint at the mean.
        assert_eq!(mood_score(ENERGY_ID, 0.2, stat, CalibrationSource::Library), 0.5);
        assert!((mood_score(ENERGY_ID, 0.24, stat, CalibrationSource::Library) - 0.817_574_476_193_643_7).abs() < 1e-12);
        // Extremes stay inside [0, 1].
        assert_eq!(mood_score("heroic", 10.0, stat, CalibrationSource::Library), 1.0);
        assert!((0.0..1e-20).contains(&mood_score("heroic", -10.0, stat, CalibrationSource::Library)));
        assert!(mood_score("heroic", f64::MAX, stat, CalibrationSource::Library) <= 1.0 && mood_score("heroic", f64::MIN, stat, CalibrationSource::Library) >= 0.0);
    }

    fn reference_with(stats: &[(String, f64)]) -> MoodReference {
        MoodReference {
            set: "incompetech-300-v1".into(),
            tracks: 300,
            stats: stats.iter().map(|(id, sd)| (id.clone(), MoodStat { mean: 0.1, sd: *sd })).collect(),
        }
    }

    #[test]
    fn reference_mood_score_puts_the_lines_at_the_reference_percentiles() {
        let stat = MoodStat { mean: 0.0, sd: 1.0 };
        let r = CalibrationSource::Reference;
        assert!((mood_score("grim", 0.674, stat, r) - 0.5).abs() < 1e-9);
        assert!((mood_score("grim", 1.2816, stat, r) - 0.75).abs() < 1e-3);
        assert!((mood_score("grim", 1.889, stat, r) - 0.9).abs() < 0.005);
        // Energy is two-sided: its midpoint is the mean.
        assert!((mood_score(ENERGY_ID, 0.0, stat, r) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn reference_calibration_needs_every_id_with_spread() {
        let v = MoodVocabulary::defaults();
        let full: Vec<(String, f64)> = ids().into_iter().map(|id| (id, 0.05)).collect();
        let cal = reference_calibration(&reference_with(&full), &v, "9".into()).unwrap();
        assert_eq!(cal.source, CalibrationSource::Reference);
        assert_eq!(cal.reference_set.as_deref(), Some("incompetech-300-v1"));
        assert_eq!(cal.tracks, 300);
        assert_eq!(cal.calibrated_at, "9");
        for id in ids() {
            assert_eq!(cal.prompts[&id], v.prompt_signature(&id).unwrap());
            assert_eq!(cal.stats[&id], MoodStat { mean: 0.1, sd: 0.05 });
        }
        assert!(reference_calibration(&reference_with(&full[1..]), &v, "9".into()).is_none());
        let mut flat = full.clone();
        flat[3].1 = 0.0;
        assert!(reference_calibration(&reference_with(&flat), &v, "9".into()).is_none());
    }

    #[test]
    fn old_calibration_json_parses_as_library() {
        let old = r#"{"calibratedAt":"1","tracks":50,"prompts":{"grim":["x"]},"stats":{"grim":{"mean":0.1,"sd":0.2}}}"#;
        let cal: MoodCalibration = serde_json::from_str(old).unwrap();
        assert_eq!(cal.source, CalibrationSource::Library);
        assert_eq!(cal.reference_set, None);
        // A library calibration does not write the optional field back out.
        let json = serde_json::to_value(&cal).unwrap();
        assert!(json.get("referenceSet").is_none());
        assert_eq!(json["source"], "library");

        let full: Vec<(String, f64)> = ids().into_iter().map(|id| (id, 0.05)).collect();
        let r = reference_calibration(&reference_with(&full), &MoodVocabulary::defaults(), "2".into()).unwrap();
        let back: MoodCalibration = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back, r);
        assert_eq!(serde_json::to_value(&r).unwrap()["source"], "reference");
    }

    #[test]
    fn same_calibration_ignores_the_timestamp() {
        let a = flat_calibration(0.1, 0.05);
        let mut b = a.clone();
        b.calibrated_at = "2".into();
        assert!(same_calibration(&a, &b));
        b.tracks += 1;
        assert!(!same_calibration(&a, &b));
        let mut c = a.clone();
        c.source = CalibrationSource::Reference;
        assert!(!same_calibration(&a, &c));
    }

    #[test]
    fn same_calibration_tolerates_float_noise_but_not_real_change() {
        let a = flat_calibration(0.1, 0.05);

        // One-ulp-scale noise (as a JSON round trip can cause) is still the same.
        let mut noisy = a.clone();
        for stat in noisy.stats.values_mut() {
            stat.mean *= 1.0 + 1e-15;
            stat.sd *= 1.0 - 1e-15;
        }
        assert!(same_calibration(&a, &noisy));

        // A real difference is not.
        let mut moved = a.clone();
        for stat in moved.stats.values_mut() {
            stat.mean += 1e-6;
        }
        assert!(!same_calibration(&a, &moved));

        // A different reference set is not.
        let mut other_set = a.clone();
        other_set.reference_set = Some("other-set".into());
        assert!(!same_calibration(&a, &other_set));
    }

    #[test]
    fn score_response_reads_mood_reference_and_tolerates_garbage() {
        let base = r#""model":"m""#;
        let good = format!(
            r#"{{{base},"mood_reference":{{"set":"s","tracks":3,"stats":{{"grim":{{"mean":0.1,"sd":0.2}}}}}}}}"#
        );
        let resp: ScoreResponse = serde_json::from_str(&good).unwrap();
        let r = resp.mood_reference.unwrap();
        assert_eq!((r.set.as_str(), r.tracks), ("s", 3));
        assert_eq!(r.stats["grim"], MoodStat { mean: 0.1, sd: 0.2 });

        let bad = format!(r#"{{{base},"mood_reference":5}}"#);
        assert_eq!(serde_json::from_str::<ScoreResponse>(&bad).unwrap().mood_reference, None);
        let absent = format!("{{{base}}}");
        assert_eq!(serde_json::from_str::<ScoreResponse>(&absent).unwrap().mood_reference, None);
    }

    #[test]
    fn auto_moods_scores_every_id_against_its_own_calibration() {
        let mut cal = flat_calibration(0.1, 0.05);
        // Peaceful has the highest raw score but also the highest mean.
        cal.stats.insert("peaceful".into(), MoodStat { mean: 0.3, sd: 0.05 });
        // Energy is a difference, so it has its own centre and spread.
        cal.stats.insert("energy".into(), MoodStat { mean: -0.02, sd: 0.01 });
        let m = with_moods([0.3, 0.15, 0.1, 0.1, 0.1, 0.1, 0.1, 0.0, 0.1, -0.04]);
        let out = auto_moods(&m, &cal, &ids()).unwrap();
        assert_eq!(out.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>(), ids());
        // A mood's bar is centred half an sd up.
        assert!((out[0].1 - 0.320821300824607025).abs() < 1e-12); // peaceful: z 0
        assert!((out[1].1 - 0.679178699175392975).abs() < 1e-12); // sorrowful: z 1
        assert!((out[7].1 - 0.022977369910025615).abs() < 1e-12); // heroic: z -2, centred -2.5
        // Energy keeps its centre at the mean.
        assert!((out[9].1 - 0.047_425_873_177_566_78).abs() < 1e-12); // energy: z -2
    }

    #[test]
    fn auto_moods_leaves_out_ids_without_a_score_or_statistics() {
        let cal = flat_calibration(0.0, 1.0);
        let mut m = with_moods([0.1; N]);
        m.moods.remove("wondrous");
        let out = auto_moods(&m, &cal, &ids()).unwrap();
        assert_eq!(out.len(), N - 1);
        assert!(out.iter().all(|(id, _)| id != "wondrous"));
        // An id the calibration lacks (a mood added since) is left out too.
        let extra = ["energy".to_string(), "spooky".to_string()];
        let out = auto_moods(&with_moods([0.1; N]), &cal, &extra).unwrap();
        assert_eq!(out.len(), 1);
        // Nothing left: none.
        assert_eq!(auto_moods(&m, &cal, &["wondrous".to_string()]), None);
    }

    #[test]
    fn scorable_ids_skip_changed_and_new_moods() {
        let cal = flat_calibration(0.0, 1.0);
        let mut v = MoodVocabulary::defaults();
        assert_eq!(scorable_ids(&cal, &v), ids());
        assert!(!needs_recalibration(Some(&cal), &v));
        assert!(!needs_recalibration(None, &v));

        // Removing a mood keeps the rest scorable.
        v.moods.retain(|m| m.id != "quirky");
        assert_eq!(scorable_ids(&cal, &v).len(), N - 1);
        assert!(!needs_recalibration(Some(&cal), &v));

        // Changed prompts, a new mood and changed Energy prompts are not.
        v.moods[0].prompts.push("gentle".into());
        v.moods.push(MoodDef { profile: None, id: "spooky".into(), name: "Spooky".into(), prompts: vec!["ghosts".into()] });
        v.energy.negative.pop();
        let ok = scorable_ids(&cal, &v);
        assert!(!ok.contains(&"peaceful".to_string()));
        assert!(!ok.contains(&"spooky".to_string()));
        assert!(!ok.contains(&"energy".to_string()));
        assert_eq!(ok.len(), N - 3);
        assert!(needs_recalibration(Some(&cal), &v));

        // Renaming changes nothing.
        let mut renamed = MoodVocabulary::defaults();
        renamed.moods[0].name = "Safe".into();
        assert!(!needs_recalibration(Some(&cal), &renamed));
    }

    /// `n` entries whose id `i` scores `0.1 + i * 0.01 + (k % 2) * step`.
    fn library(n: usize, step: f64) -> HashMap<String, MoodRaw> {
        (0..n)
            .map(|k| {
                let bump = if k % 2 == 0 { 0.0 } else { step };
                let raws: [f64; N] = std::array::from_fn(|i| 0.1 + i as f64 * 0.01 + bump);
                (format!("h-{k}"), with_moods(raws))
            })
            .collect()
    }

    fn calibrate_defaults(mood: &HashMap<String, MoodRaw>) -> Option<MoodCalibration> {
        calibrate(mood, &MoodVocabulary::defaults(), "1700000000000".into())
    }

    #[test]
    fn calibrate_needs_fifty_tracks_with_every_score() {
        assert_eq!(calibrate_defaults(&library(49, 0.02)), None);
        // Entries missing a mood do not count.
        let mut short = library(49, 0.02);
        let mut partial = with_moods([0.1; N]);
        partial.moods.remove("grim");
        short.insert("h-old".into(), partial);
        assert_eq!(calibrate_defaults(&short), None);

        let cal = calibrate_defaults(&library(50, 0.02)).unwrap();
        assert_eq!(cal.tracks, 50);
        assert_eq!(cal.calibrated_at, "1700000000000");
        assert_eq!(cal.stats.len(), N);
        // Half at base, half at base + 0.02: mean base + 0.01, population sd 0.01.
        for (i, id) in ids().iter().enumerate() {
            let stat = cal.stats[id];
            assert!((stat.mean - (0.11 + i as f64 * 0.01)).abs() < 1e-12);
            assert!((stat.sd - 0.01).abs() < 1e-12);
        }
        // It records the prompts it was taken with.
        let v = MoodVocabulary::defaults();
        assert_eq!(cal.prompts["joyful"], v.moods[3].prompts);
        assert_eq!(scorable_ids(&cal, &v), ids());
    }

    #[test]
    fn calibrate_covers_the_vocabulary_it_is_given() {
        // A vocabulary with fewer moods calibrates from the same response.
        let mut v = MoodVocabulary::defaults();
        v.moods.truncate(3);
        let cal = calibrate(&library(50, 0.02), &v, "1".into()).unwrap();
        assert_eq!(cal.stats.keys().collect::<Vec<_>>(), ["energy", "peaceful", "sorrowful", "wondrous"]);
        // A mood the analyzer did not score blocks calibration.
        v.moods.push(MoodDef { profile: None, id: "spooky".into(), name: "Spooky".into(), prompts: vec!["ghosts".into()] });
        assert_eq!(calibrate(&library(50, 0.02), &v, "1".into()), None);
    }

    #[test]
    fn calibrate_refuses_an_id_without_spread() {
        assert_eq!(calibrate_defaults(&library(60, 0.0)), None);
    }

    #[test]
    fn calibration_round_trips_as_json() {
        let cal = calibrate_defaults(&library(50, 0.02)).unwrap();
        let text = serde_json::to_string(&cal).unwrap();
        assert!(text.starts_with(r#"{"calibratedAt":"1700000000000","tracks":50,"prompts":{"#));
        assert!(text.contains(r#""stats":{"battle":{"mean":"#));
        let back: MoodCalibration = serde_json::from_str(&text).unwrap();
        assert_eq!(back.prompts, cal.prompts);
        for (id, stat) in &cal.stats {
            assert!((back.stats[id].mean - stat.mean).abs() < 1e-15 && (back.stats[id].sd - stat.sd).abs() < 1e-15);
        }
    }

    #[test]
    fn decide_scores_moods_only_with_a_calibration() {
        let mut resp = response();
        resp.mood = [("h-a".to_string(), with_moods([0.2; N]))].into();
        let entries = [entry("a"), entry("b")];
        let targets = hashes(&["h-a", "h-b"]);
        let out = decide(&entries, &targets, &resp, None, &ids());
        assert_eq!((out["a"].moods.clone(), out["b"].moods.clone()), (None, None));

        let cal = flat_calibration(0.2, 0.01);
        let out = decide(&entries, &targets, &resp, Some(&cal), &ids());
        assert_eq!(out["a"].moods, auto_moods(&resp.mood["h-a"], &cal, &ids()));
        // At the mean, Energy reads 0.5 and every mood reads the centred-half-sd-up value.
        for (id, v) in out["a"].moods.as_ref().unwrap() {
            let want = if id == ENERGY_ID { 0.5 } else { 0.320821300824607025 };
            assert!((v - want).abs() < 1e-12, "{id}: {v}");
        }
        // No mood for the entry: no moods.
        assert_eq!(out["b"].moods, None);
        // Only the ids asked for are scored.
        let out = decide(&entries, &targets, &resp, Some(&cal), &["grim".to_string()]);
        let grim = out["a"].moods.as_ref().unwrap();
        assert_eq!(grim.len(), 1);
        assert_eq!(grim[0].0, "grim");
        assert!((grim[0].1 - 0.320821300824607025).abs() < 1e-12);
    }

    #[test]
    fn old_analyzer_response_parses_without_pace_or_mood() {
        let json = r#"{"model":"m"}"#;
        let resp: ScoreResponse = serde_json::from_str(json).unwrap();
        assert!(resp.mood.is_empty());
        // An analyzer that still sends gate and tag scores parses too; they
        // are ignored.
        let json = r#"{"model":"m","gate":{"quality":{"auc":0.9,"pos":12,"neg":9},
            "trusted":true,"threshold":0.61,"scores":{"h":0.9}},"tags":{}}"#;
        let resp: ScoreResponse = serde_json::from_str(json).unwrap();
        assert!(resp.mood.is_empty());
    }

    #[test]
    fn mood_response_drops_entries_without_the_moods_shape() {
        let json = r#"{"model":"m","gate":{"quality":{"auc":0.9,"pos":12,"neg":9},
            "trusted":true,"threshold":0.61,"scores":{}},"tags":{},
            "pace":{"quality":{"auc":null,"pos":0,"neg":0},"trusted":false,"threshold":null,"scores":{}},
            "mood":{"h-new":{"moods":{"peaceful":0.1,"sorrowful":0.2,"wondrous":0.3,"joyful":0.4,
                                      "quirky":0.5,"mysterious":0.6,"grim":0.7,"heroic":0.8,"battle":0.9},
                             "energy":-0.05},
                    "h-pools":{"pools":{"peaceful":0.1,"cheerful":0.2,"quirky":0.3,"somber":0.4,"awe":0.5,
                                        "mysterious":0.6,"heroic":0.7,"menacing":0.8}},
                    "h-v14":{"bright":0.1,"light":0.2,"grand":0.3,"energy":0.4,"sad":0.25,"scary":0.15},
                    "h-old":{"bright":0.1,"light":0.2}}}"#;
        let resp: ScoreResponse = serde_json::from_str(json).unwrap();
        // Entries from analyzers that predate the mood profile (pools, the
        // scales) count as no mood.
        assert_eq!(resp.mood.len(), 1);
        assert_eq!(resp.mood["h-new"], with_moods([0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, -0.05]));
    }
}
