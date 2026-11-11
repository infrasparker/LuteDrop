"""Tag this library's verified game albums, preserving existing tags and files.

Dry-run by default. --apply first makes a SQLite backup, then adds memberships
in one transaction. Requires mutagen to read embedded album metadata. Close
LuteDrop before applying so the next launch loads the new tags immediately.
No fuzzy matching: unknown albums, fan albums, and tabletop music are skipped.
"""
from contextlib import closing
import argparse
from collections import Counter
from datetime import datetime
import json
import os
from pathlib import Path
import sqlite3
import uuid
import mutagen


def game_tag(album):
    text = album.casefold().replace('\u2019', "'")
    # Reviewed embedded album names, not guesses from track titles. Exact matches
    # deliberately leave future cover/fan albums for a separate review.
    albums = {
        'pillars of eternity': 'pillars of eternity',
        'pillars of eternity ii: deadfire': 'pillars of eternity 2',
        'the witcher 3: wild hunt (original game soundtrack)': 'witcher',
        'the witcher 3: wild hunt - blood and wine (official soundtrack)': 'witcher',
        'the witcher 3: wild hunt (unreleased tracks)': 'witcher',
        'the elder scrolls iv: oblivion: original game soundtrack': 'elder scrolls',
        'the elder scrolls v: skyrim: original game soundtrack': 'elder scrolls',
        "baldur's gate 3 (original game soundtrack)": "baldur's gate",
        'clair obscur: expedition 33: original soundtrack': 'clair obscur',
        'divinity: original sin 2 ost': 'divinity',
        'valheim soundtrack': 'valheim',
        'sword coast legends (original game soundtrack)': 'sword coast legends',
        'soma (original video game soundtrack)': 'soma',
        'trine original soundtrack': 'trine',
        'trine soundtrack': 'trine',
        'hollow knight: original soundtrack': 'hollow knight',
        'darkest dungeon (original video game soundtrack)': 'darkest dungeon',
        'darkest dungeon: color of madness dlc (original soundtrack)': 'darkest dungeon',
        'monster hunter world: iceborne original soundtrack': 'monster hunter',
        'monster hunter: world original soundtrack': 'monster hunter',
        "assassin's creed iii: original game soundtrack": "assassin's creed",
        'hearthstone: music from the tavern (volume 1)': 'hearthstone',
        'hearthstone: music from the tavern (volume 2)': 'hearthstone',
        'hearthstone: curse of naxxramas': 'hearthstone',
        'esoteric ebb original soundtrack': 'esoteric ebb',
        'genshin impact - jade moon upon a sea of clouds (original game soundtrack)': 'genshin impact',
        'dragon age inquisition': 'dragon age',
    }
    tag = albums.get(text.strip())
    return 'ost: ' + tag if tag else None



def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--db', type=Path, default=Path(os.environ['APPDATA']) / 'com.lutedrop/lutedrop.db')
    parser.add_argument('--apply', action='store_true')
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    with closing(sqlite3.connect(args.db.resolve().as_uri() + '?mode=ro', uri=True)) as source:
        rows = source.execute("SELECT id,name,file_path FROM library_entries WHERE entry_type='music'").fetchall()
    planned, skipped = [], []
    for entry_id, name, path in rows:
        try:
            metadata = mutagen.File(path, easy=True)
            albums = metadata.get('album', []) if metadata else []
            tag = game_tag(albums[0]) if len(albums) == 1 else None
        except Exception as error:
            skipped.append({'name': name, 'reason': type(error).__name__})
            continue
        if tag:
            planned.append({'entryId': entry_id, 'name': name, 'album': albums[0], 'tags': ['game ost', tag]})
        else:
            skipped.append({'name': name, 'album': albums, 'reason': 'No confirmed game-album match'})
    report = {'applied': False, 'entries': planned, 'skipped': skipped}
    if args.apply:
        backup = args.db.parent / 'backups' / ('before-ost-tags-' + datetime.now().strftime('%Y%m%d-%H%M%S-%f') + '.db')
        backup.parent.mkdir(parents=True, exist_ok=True)
        with closing(sqlite3.connect(args.db)) as conn, conn:
            with closing(sqlite3.connect(backup)) as saved:
                conn.backup(saved)
            conn.execute('PRAGMA foreign_keys=ON')
            conn.execute('BEGIN IMMEDIATE')
            for item in planned:
                for tag in item['tags']:
                    conn.execute('INSERT OR IGNORE INTO tags(id,name) VALUES (?,?)', (str(uuid.uuid4()), tag))
                    conn.execute('INSERT OR IGNORE INTO library_entry_tags(library_entry_id,tag_id) SELECT ?,id FROM tags WHERE name=?', (item['entryId'], tag))
            # Verify each intended membership before the transaction can commit.
            for item in planned:
                actual = {r[0] for r in conn.execute('SELECT t.name FROM tags t JOIN library_entry_tags et ON et.tag_id=t.id WHERE et.library_entry_id=?', (item['entryId'],))}
                if not set(item['tags']).issubset(actual):
                    raise RuntimeError('Tag verification failed; changes rolled back')
        report.update(applied=True, backup=str(backup))
    if args.report:
        args.report.write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
    print(('Applied' if args.apply else 'Planned') + f' game OST tags for {len(planned)} tracks; skipped {len(skipped)}.')
    for tag, count in sorted(Counter(item['tags'][1] for item in planned).items()):
        print(f'  {tag}: {count}')
    for item in skipped:
        print(f"  Skipped: {item['name']} ({item.get('album', item['reason'])})")
    if args.apply:
        print('Backup: ' + report['backup'])


if __name__ == '__main__':
    main()
