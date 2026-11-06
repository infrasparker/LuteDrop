// Pools checklist (a development aid, not shipped with the app).
//
// Reads the LuteDrop database read-only and a JSON file of
// { "name substring": "expected pool" }, and prints how each matching music
// entry's pool compares: the shown pool (confirmed, else suggested) against
// the expected one. Exit code 1 when anything is wrong or missing.
//
// Usage:
//   node scripts/pools-checklist.mjs [--db <path to lutedrop.db>] [--list <checklist.json>]
//
// Defaults: the app's database at %APPDATA%/com.lutedrop/lutedrop.db, and
// scripts/pools-checklist.json next to this script.
//
// Needs Node 22.13 or newer for the built-in `node:sqlite` module (it prints
// an "experimental" warning; that is expected). On an older Node, run the
// same query from Python's sqlite3 module, opening the database read-only with
// sqlite3.connect('file:<path>?mode=ro', uri=True):
//   SELECT name, pool_auto, pool_auto_p, pool_user FROM library_entries WHERE entry_type = 'music'
// and compare each name containing a checklist key with pool_user ?? pool_auto.
import { readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

function arg(name) {
  const i = process.argv.indexOf(name);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

const here = dirname(fileURLToPath(import.meta.url));
const dbPath = arg('--db') ?? join(process.env.APPDATA ?? '', 'com.lutedrop', 'lutedrop.db');
const listPath = arg('--list') ?? join(here, 'pools-checklist.json');

let DatabaseSync;
try {
  ({ DatabaseSync } = await import('node:sqlite'));
} catch {
  console.error('node:sqlite is not available (needs Node 22.13+). See the Python one-liner at the top of this file.');
  process.exit(2);
}

const checklist = JSON.parse(readFileSync(listPath, 'utf8'));
const db = new DatabaseSync(dbPath, { readOnly: true });
const rows = db
  .prepare("SELECT name, pool_auto, pool_auto_p, pool_user FROM library_entries WHERE entry_type = 'music'")
  .all();
db.close();

let wrong = 0;
let missing = 0;
let right = 0;
for (const [substring, expected] of Object.entries(checklist)) {
  const needle = substring.toLowerCase();
  const matches = rows.filter((r) => r.name.toLowerCase().includes(needle));
  if (matches.length === 0) {
    missing++;
    console.log(`MISSING  "${substring}": no music entry matches (expected ${expected})`);
    continue;
  }
  for (const r of matches) {
    const shown = r.pool_user ?? r.pool_auto;
    const source = r.pool_user !== null ? 'confirmed' : r.pool_auto !== null ? 'suggested' : 'none';
    const p = r.pool_user === null && r.pool_auto_p !== null ? ` p=${r.pool_auto_p.toFixed(2)}` : '';
    if (shown === expected) {
      right++;
    } else {
      wrong++;
      console.log(`WRONG    ${r.name}: ${shown ?? '-'} (${source}${p}), expected ${expected}`);
    }
  }
}

console.log(`\n${right} right, ${wrong} wrong, ${missing} not found`);
process.exit(wrong > 0 || missing > 0 ? 1 : 0);
