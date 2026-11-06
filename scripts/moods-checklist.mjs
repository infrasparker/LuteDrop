// Moods checklist (a development aid, not shipped with the app).
//
// A mood is a lens: it ranks every music track by that mood's score. This
// script reads the LuteDrop database read-only and prints, for named tracks,
// their rank in a given mood, so a change to the analyzer or the calibration
// can be checked against tracks whose place is known.
//
// Two ways to run it:
//
//   node scripts/moods-checklist.mjs [--db <lutedrop.db>] [--list <checklist.json>]
//     Checks every line of the checklist, a JSON object of
//     { "name substring": { "mood": "<id>", "top": <n> } }, and prints each
//     matching track's rank in that mood. Exit code 1 when a track ranks
//     below its `top`, has no score, or no track matches.
//
//   node scripts/moods-checklist.mjs [--db <lutedrop.db>] --mood <id> "<name>" ["<name>" ...]
//     Prints the rank in <id> of every music track whose name contains one of
//     the given substrings.
//
// The score ranked is the value the app shows: the level set by the user,
// else the analyzer's suggestion. Rank 1 is the highest score; tracks with
// equal scores share a rank. Tracks with no score for the mood are not ranked.
//
// Defaults: the app's database at %APPDATA%/com.lutedrop/lutedrop.db, and
// scripts/moods-checklist.json next to this script. Mood ids: peaceful,
// sorrowful, wondrous, cheerful, quirky, mysterious, grim, heroic, menacing,
// energy.
//
// Needs Node 22.13 or newer for the built-in `node:sqlite` module (it prints
// an "experimental" warning; that is expected). On an older Node, run the
// same query from Python's sqlite3 module, opening the database read-only with
// sqlite3.connect('file:<path>?mode=ro', uri=True):
//   SELECT e.name, COALESCE(m.user_level, m.auto_score) AS score
//   FROM library_entries e JOIN library_entry_moods m ON m.library_entry_id = e.id
//   WHERE e.entry_type = 'music' AND m.mood = '<id>' ORDER BY score DESC
import { readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const MOOD_IDS = [
  'peaceful', 'sorrowful', 'wondrous', 'cheerful', 'quirky',
  'mysterious', 'grim', 'heroic', 'menacing', 'energy',
];

/// The value after a flag, and the remaining arguments that are not flags.
const argv = process.argv.slice(2);
const flags = new Map();
const names = [];
for (let i = 0; i < argv.length; i++) {
  if (argv[i].startsWith('--')) {
    flags.set(argv[i], argv[i + 1]);
    i++;
  } else {
    names.push(argv[i]);
  }
}

const here = dirname(fileURLToPath(import.meta.url));
const dbPath = flags.get('--db') ?? join(process.env.APPDATA ?? '', 'com.lutedrop', 'lutedrop.db');
const adHocMood = flags.get('--mood');

if (adHocMood !== undefined && !MOOD_IDS.includes(adHocMood)) {
  console.error(`Unknown mood "${adHocMood}". Use one of: ${MOOD_IDS.join(', ')}`);
  process.exit(2);
}
if (adHocMood !== undefined && names.length === 0) {
  console.error('Give at least one track name after --mood <id>.');
  process.exit(2);
}

let DatabaseSync;
try {
  ({ DatabaseSync } = await import('node:sqlite'));
} catch {
  console.error('node:sqlite is not available (needs Node 22.13+). See the Python query at the top of this file.');
  process.exit(2);
}

const db = new DatabaseSync(dbPath, { readOnly: true });
const music = db.prepare("SELECT id, name FROM library_entries WHERE entry_type = 'music'").all();
const moodRows = db
  .prepare(
    `SELECT m.library_entry_id AS id, m.mood, COALESCE(m.user_level, m.auto_score) AS score,
            m.user_level IS NOT NULL AS set_by_user
     FROM library_entry_moods m JOIN library_entries e ON e.id = m.library_entry_id
     WHERE e.entry_type = 'music'`,
  )
  .all();
db.close();

/// For each mood: entry id -> { score, setByUser }, and the sorted scores.
const byMood = new Map(MOOD_IDS.map((id) => [id, { scores: new Map(), sorted: [] }]));
for (const r of moodRows) {
  const slot = byMood.get(r.mood);
  if (!slot || r.score === null) continue;
  slot.scores.set(r.id, { score: r.score, setByUser: Boolean(r.set_by_user) });
}
for (const slot of byMood.values()) {
  slot.sorted = [...slot.scores.values()].map((v) => v.score).sort((a, b) => b - a);
}

/// Rank of `score` in `mood`: 1 + the number of tracks scoring strictly higher.
function rankOf(mood, score) {
  const sorted = byMood.get(mood).sorted;
  let higher = 0;
  while (higher < sorted.length && sorted[higher] > score) higher++;
  return higher + 1;
}

/// One line per matching track: its rank (or why it has none).
function describe(mood, substring) {
  const needle = substring.toLowerCase();
  const slot = byMood.get(mood);
  const matches = music.filter((e) => e.name.toLowerCase().includes(needle));
  return matches.map((e) => {
    const v = slot.scores.get(e.id);
    if (!v) return { name: e.name, rank: null, text: `${e.name}: no ${mood} score` };
    const source = v.setByUser ? 'set by you' : 'suggested';
    const rank = rankOf(mood, v.score);
    return {
      name: e.name,
      rank,
      text: `${e.name}: #${rank} of ${slot.sorted.length} in ${mood} (${v.score.toFixed(2)}, ${source})`,
    };
  });
}

if (adHocMood !== undefined) {
  for (const substring of names) {
    const lines = describe(adHocMood, substring);
    if (lines.length === 0) console.log(`"${substring}": no music entry matches`);
    for (const line of lines) console.log(line.text);
  }
  process.exit(0);
}

const listPath = flags.get('--list') ?? join(here, 'moods-checklist.json');
const checklist = JSON.parse(readFileSync(listPath, 'utf8'));

let pass = 0;
let fail = 0;
let missing = 0;
for (const [substring, { mood, top }] of Object.entries(checklist)) {
  if (!MOOD_IDS.includes(mood)) {
    console.log(`BAD      "${substring}": unknown mood "${mood}"`);
    fail++;
    continue;
  }
  const lines = describe(mood, substring);
  if (lines.length === 0) {
    missing++;
    console.log(`MISSING  "${substring}": no music entry matches (want top ${top} in ${mood})`);
    continue;
  }
  for (const line of lines) {
    const ok = line.rank !== null && line.rank <= top;
    if (ok) pass++;
    else fail++;
    console.log(`${ok ? 'OK      ' : 'LOW     '} ${line.text}; want top ${top}`);
  }
}

console.log(`\n${pass} in place, ${fail} too low or unscored, ${missing} not found`);
process.exit(fail > 0 || missing > 0 ? 1 : 0);
