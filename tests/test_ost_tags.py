"""Exercise the one-off tagger against a temporary database, never live data."""
from contextlib import closing
import importlib.util
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('ost_tags', Path(__file__).parents[1] / 'scripts/tag-game-osts.py')
tagger = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tagger)


class OstTagsTest(unittest.TestCase):
    def test_exact_albums_separate_pillars_and_skip_fan_recordings(self):
        self.assertEqual(tagger.game_tag('Pillars of Eternity II: Deadfire'), 'ost: pillars of eternity 2')
        self.assertEqual(tagger.game_tag('Pillars of Eternity'), 'ost: pillars of eternity')
        self.assertEqual(tagger.game_tag('The Witcher 3: Wild Hunt - Blood and Wine (Official Soundtrack)'), 'ost: witcher')
        for album in ['Journey to Velen', 'The Witcher 3: Fan Covers', 'The Lost Mine (Unofficial LMoP Soundtrack)']:
            self.assertIsNone(tagger.game_tag(album))

    def test_apply_is_additive_idempotent_music_only_and_backed_up(self):
        with tempfile.TemporaryDirectory() as directory:
            db = Path(directory) / 'test.db'
            with closing(sqlite3.connect(db)) as conn, conn:
                conn.executescript('''
                    CREATE TABLE library_entries(id TEXT PRIMARY KEY,name TEXT,file_path TEXT,entry_type TEXT);
                    CREATE TABLE tags(id TEXT PRIMARY KEY,name TEXT UNIQUE);
                    CREATE TABLE library_entry_tags(library_entry_id TEXT REFERENCES library_entries(id),tag_id TEXT REFERENCES tags(id),PRIMARY KEY(library_entry_id,tag_id));
                    INSERT INTO library_entries VALUES ('music','song','music','music'),('fan','fan song','fan','music'),('ambient','ambience','ambient','ambiance');
                    INSERT INTO tags VALUES ('travel','travel');
                    INSERT INTO library_entry_tags VALUES ('music','travel');
                ''')
            def metadata(path, **_kwargs):
                return {'album': ['Journey to Velen' if path == 'fan' else 'Pillars of Eternity II: Deadfire']}
            with patch.object(tagger.mutagen, 'File', side_effect=metadata), patch('sys.argv', ['tag', '--db', str(db)]), patch('builtins.print'):
                tagger.main()
            with closing(sqlite3.connect(db)) as conn, conn:
                self.assertEqual(conn.execute('SELECT count(*) FROM library_entry_tags').fetchone()[0], 1, 'dry-run cannot write')
            with patch.object(tagger.mutagen, 'File', side_effect=metadata), patch('sys.argv', ['tag', '--db', str(db), '--apply']), patch('builtins.print'):
                tagger.main()
                tagger.main()
            with closing(sqlite3.connect(db)) as conn, conn:
                tags = conn.execute('SELECT e.library_entry_id,t.name FROM library_entry_tags e JOIN tags t ON t.id=e.tag_id').fetchall()
                self.assertEqual(set(tags), {('music','travel'), ('music','game ost'), ('music','ost: pillars of eternity 2')})
                self.assertEqual(conn.execute('PRAGMA integrity_check').fetchone()[0], 'ok')
            backups = sorted((Path(directory) / 'backups').glob('*.db'))
            self.assertEqual(len(backups), 2)
            with closing(sqlite3.connect(backups[0])) as before:
                self.assertEqual(before.execute('SELECT count(*) FROM library_entry_tags').fetchone()[0], 1)


if __name__ == '__main__':
    unittest.main()
