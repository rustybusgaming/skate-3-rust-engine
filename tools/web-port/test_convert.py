"""convert.py with synthetic discs; contains no game data. The conversion itself is the Windows
setup's (tools/asset_pipeline, with its own tests): here it is replaced by a recorder.

    python3 -m unittest discover -s tools/web-port -p 'test_*.py'
"""
import contextlib
import io
import shutil
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import convert
from convert import SetupError, check_names
from test_xiso import image

GAME = {
    'default.xex': b'XEX2',
    'data': {
        'big': {'miscload.big': b'1', 'miscboot.big': b'2', 'db.big': b'3'},
        'content': {'createacharacter.big': b'4', 'worldDIST_University.big': b'5', 'worldDIST_SkateSchool.big': b'6'},
    },
}
PATHS = ['default.xex', 'data/big/miscload.big', 'data/big/miscboot.big', 'data/big/db.big',
         'data/content/createacharacter.big', 'data/content/worldDIST_University.big']


class NameTests(unittest.TestCase):
    def test_a_whole_disc_passes(self):
        check_names(PATHS)

    def test_refuses_another_disc(self):
        with self.assertRaisesRegex(SetupError, 'not a Skate 3 Xbox 360 disc: it has no default.xex'):
            check_names(PATHS[1:])
        with self.assertRaisesRegex(SetupError, 'no data/big/db.big'):
            check_names([p for p in PATHS if p != 'data/big/db.big'])

    def test_names_setup_cannot_find_outside_windows(self):
        renamed = [p.replace('miscload', 'MiscLoad') for p in PATHS]
        with self.assertRaisesRegex(SetupError, 'names data/big/miscload.big as data/big/MiscLoad.big'):
            check_names(renamed)
        with self.assertRaisesRegex(SetupError, 'maps like data/content/WORLDDIST_University.big'):
            check_names([p.replace('worldDIST', 'WORLDDIST') for p in PATHS])
        with self.assertRaisesRegex(SetupError, 'has no maps'):
            check_names(PATHS[:-1])

    def test_an_update_needs_only_default_xex(self):
        check_names(['default.xex'], refresh=True)
        with self.assertRaisesRegex(SetupError, 'no default.xex'):
            check_names([], refresh=True)


class ConvertTests(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix='convert-test-'))
        self.addCleanup(shutil.rmtree, self.dir)
        self.calls = []

        def install(game, base, game_exe, report, refresh=False):
            # The disc is extracted when setup runs, beside the installation.
            self.calls.append({'game': game, 'base': base, 'game_exe': game_exe, 'refresh': refresh,
                               'files': sorted(p.relative_to(game).as_posix() for p in game.rglob('*') if p.is_file())})
            return base / 'installations/0123'
        patcher = mock.patch('tools.asset_pipeline.customiser_setup.install', install)
        patcher.start()
        self.addCleanup(patcher.stop)

    def files(self, tree, prefix=''):
        for name, value in tree.items():
            if isinstance(value, dict):
                yield from self.files(value, prefix + name + '/')
            else:
                yield prefix + name

    def test_extracts_the_iso_beside_the_installation_and_removes_it(self):
        iso = self.dir / 'Skate 3.iso'
        image(iso, GAME, partition=0x0FD90000)
        out = self.dir / 'Skate 3 Rust Engine'
        with contextlib.redirect_stdout(io.StringIO()) as printed:
            result = convert.convert(iso, out, self.dir / 'skate3rust', report=print)
        self.assertEqual(result, out / 'data/installations/0123')
        [call] = self.calls
        self.assertEqual(call['files'], sorted(self.files(GAME)))
        self.assertEqual(call['game'].parent.parent, out)
        self.assertEqual(call['base'], out / 'data')
        self.assertFalse(call['refresh'])
        self.assertFalse(call['game'].exists())
        self.assertEqual(list(out.iterdir()), [])
        self.assertIn('Extracting your ISO: 100 %', printed.getvalue())

    def test_updates_an_existing_installation(self):
        iso = self.dir / 'disc.iso'
        image(iso, GAME)
        out = self.dir / 'out'
        (out / 'data').mkdir(parents=True)
        (out / 'data/installation.json').write_text('{}')
        convert.convert(iso, out, self.dir / 'skate3rust', report=lambda _: None)
        self.assertTrue(self.calls[0]['refresh'])

    def test_takes_an_extracted_copy_by_default_xex_or_folder(self):
        game = self.dir / 'game'
        for path in self.files(GAME):
            (game / path).parent.mkdir(parents=True, exist_ok=True)
            (game / path).write_bytes(b'x')
        for source in (game / 'default.xex', game):
            convert.convert(source, self.dir / 'out', self.dir / 'skate3rust', report=lambda _: None)
            self.assertEqual(self.calls.pop()['game'], game)

    def test_checks_the_disc_before_extracting(self):
        iso = self.dir / 'other.iso'
        image(iso, {'default.xex': b'x', 'data': {'other.big': b'y'}})
        with self.assertRaisesRegex(SetupError, 'not a Skate 3'):
            convert.convert(iso, self.dir / 'out', self.dir / 'skate3rust', report=lambda _: None)
        self.assertEqual(self.calls, [])
        self.assertFalse((self.dir / 'out').exists())

    def test_refuses_other_files(self):
        other = self.dir / 'notes.txt'
        other.write_text('x')
        with self.assertRaisesRegex(SetupError, 'choose your Skate 3 ISO'):
            convert.convert(other, self.dir / 'out', self.dir / 'skate3rust', report=lambda _: None)


class CommandLineTests(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix='convert-cli-'))
        self.addCleanup(shutil.rmtree, self.dir)
        self.exe = self.dir / 'skate3rust'
        self.exe.write_bytes(b'')

    def run_main(self, *argv):
        output = io.StringIO()
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output), \
                mock.patch.object(convert, 'lower_priority'):
            code = convert.main([*argv])
        return code, output.getvalue()

    def test_needs_the_engine_build_and_vgmstream(self):
        code, said = self.run_main('--game-exe', str(self.dir / 'missing'), str(self.dir / 'x.iso'))
        self.assertEqual(code, 1)
        self.assertIn('convert.sh', said)
        with mock.patch.object(convert.shutil, 'which', return_value=None):
            code, said = self.run_main('--game-exe', str(self.exe), str(self.dir / 'x.iso'))
        self.assertEqual(code, 1)
        self.assertIn('brew install vgmstream', said)

    def test_reports_a_missing_source_and_a_bad_image(self):
        code, said = self.run_main('--without-audio', '--game-exe', str(self.exe), '--out', str(self.dir / 'out'), str(self.dir / 'x.iso'))
        self.assertEqual((code, 'does not exist' in said), (1, True))
        (self.dir / 'x.iso').write_bytes(bytes(0x20000))
        code, said = self.run_main('--without-audio', '--game-exe', str(self.exe), '--out', str(self.dir / 'out'), str(self.dir / 'x.iso'))
        self.assertEqual((code, 'not an Xbox 360 disc image' in said), (1, True))

    def test_logs_a_failed_conversion(self):
        (self.dir / 'x.iso').write_bytes(b'')
        with mock.patch.object(convert, 'convert', side_effect=ValueError('broken map')):
            code, said = self.run_main('--without-audio', '--game-exe', str(self.exe), '--out', str(self.dir / 'out'), str(self.dir / 'x.iso'))
        self.assertEqual(code, 1)
        self.assertIn('Setup could not finish: broken map', said)
        self.assertIn('ValueError: broken map', (self.dir / 'out/data/setup-error.log').read_text())

    def test_prints_how_to_play(self):
        (self.dir / 'x.iso').write_bytes(b'')
        with mock.patch.object(convert, 'convert', return_value=self.dir / 'out/data/installations/0123'), \
                mock.patch('tools.asset_pipeline.validation_report.summary', return_value=[]):
            code, said = self.run_main('--without-audio', '--game-exe', str(self.exe), '--out', str(self.dir / 'out'), str(self.dir / 'x.iso'))
        self.assertEqual(code, 0)
        self.assertIn(f'skate-web-server -- --installation "{self.dir / "out"}"', said)
        self.assertIn('never upload it', said)


if __name__ == '__main__':
    unittest.main()
