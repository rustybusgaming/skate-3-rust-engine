"""Set up Skate 3 from your own Xbox 360 ISO on macOS or Linux (docs/web-port/setup.md).

The same conversion as the Windows setup (tools/asset_pipeline), with its Windows-only parts
replaced: the ISO is extracted by xiso.py instead of extract-xiso, the audio decoder is
Homebrew's vgmstream, and the engine checks run on a native build for this machine. Run it
through convert.sh, which prepares Python and builds that engine:

    tools/web-port/convert.sh [SOURCE] [--out FOLDER] [--without-audio]

SOURCE is your Skate 3 ISO, or default.xex in an extracted copy; without it a file dialog asks
for the ISO. The result is a package folder like the Windows build's (FOLDER/data), which the
browser build plays:

    cargo run --release -p skate-web-server -- --installation FOLDER
"""
from __future__ import annotations

import argparse
import fnmatch
import os
import shutil
import subprocess
import sys
import tempfile
import traceback
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from xiso import DiscError, DiscImage, gigabytes  # noqa: E402

DEFAULT_OUT = Path.home() / 'Skate 3 Rust Engine'
# What setup reads from the disc by these exact names (tools/asset_pipeline/install.py,
# setup_state.py). Windows finds a file whatever its case; macOS and Linux matching is exact.
REQUIRED = ('default.xex', 'data/big/miscload.big', 'data/big/miscboot.big', 'data/big/db.big',
            'data/content/createacharacter.big')
DISTRICTS = 'data/content/worldDIST_*.big'
# SKATE_SETUP_PRIORITY (tools/asset_pipeline/setup_budget.py) as a nice value.
NICENESS = {'normal': 0, 'below_normal': 10, 'idle': 19}


class SetupError(RuntimeError):
    """Stops setup before any conversion, with a message for the player."""


def check_names(paths, refresh=False):
    """`paths` ('/'-separated) must hold what setup reads, by the exact names it uses. An
    update (`refresh`) may need only default.xex, as on Windows."""
    present = set(paths)
    folded = {}
    for path in present:
        folded.setdefault(path.casefold(), path)
    for wanted in REQUIRED[:1] if refresh else REQUIRED:
        if wanted in present:
            continue
        if wanted.casefold() in folded:
            raise SetupError(f'This disc names {wanted} as {folded[wanted.casefold()]}, which setup cannot '
                             'read outside Windows. Please report it with that name.')
        raise SetupError(f'This is not a Skate 3 Xbox 360 disc: it has no {wanted}.')
    if not refresh and not any(fnmatch.fnmatchcase(path, DISTRICTS) for path in present):
        near = sorted(path for path in present if fnmatch.fnmatch(path.casefold(), DISTRICTS.casefold()))
        if near:
            raise SetupError(f'This disc names its maps like {near[0]}, which setup cannot find outside Windows. '
                             'Please report it with that name.')
        raise SetupError('This Skate 3 disc has no maps (data/content/worldDIST_*.big).')


def folder_paths(root: Path):
    return [path.relative_to(root).as_posix() for path in root.rglob('*') if path.is_file()]


def choose_iso() -> Path:
    """Asks for the ISO in a file dialog."""
    prompt = 'Select your Skate 3 Xbox 360 ISO'
    if sys.platform == 'darwin':
        print('Choose your ISO in the window that opened (it may be behind this one).', flush=True)
        # No type filter: one the system misreads would grey out the ISO; DiscImage checks it.
        command = ['osascript', '-e', f'POSIX path of (choose file with prompt "{prompt}")']
    elif shutil.which('zenity'):
        command = ['zenity', '--file-selection', f'--title={prompt}', '--file-filter=Xbox 360 ISO | *.iso *.ISO']
    elif shutil.which('kdialog'):
        command = ['kdialog', '--title', prompt, '--getopenfilename', str(Path.home()), '*.iso *.ISO']
    else:
        raise SetupError('No file dialog here: give the ISO, e.g. tools/web-port/convert.sh ~/Downloads/Skate3.iso')
    chosen = subprocess.run(command, capture_output=True, text=True)
    path = chosen.stdout.strip()
    if chosen.returncode or not path:
        raise SetupError('No ISO chosen.')
    return Path(path)


def lower_priority():
    """Below normal priority, as on Windows, so the machine stays usable; children inherit it."""
    value = os.environ.get('SKATE_SETUP_PRIORITY', '').strip().lower()
    try:
        os.nice(NICENESS.get(value, NICENESS['below_normal']))
    except OSError:
        pass


def extraction_progress(report):
    shown = [-1]

    def progress(done, total):
        tenth = done * 10 // max(total, 1)
        if tenth != shown[0]:
            shown[0] = tenth
            report(f'Extracting your ISO: {tenth * 10} % ({gigabytes(done)} of {gigabytes(total)})')
    return progress


def convert(source: Path, out: Path, game_exe: Path, report=print) -> Path:
    """Converts `source` (an ISO, default.xex or an extracted game folder) into `out`/data and
    returns the installation folder."""
    from tools.asset_pipeline.customiser_setup import install
    base = out / 'data'
    # As the game asks for on Windows: an existing installation is updated, not replaced.
    refresh = (base / 'installation.json').is_file()
    if source.is_file() and source.suffix.lower() == '.iso':
        with DiscImage(source) as disc:
            # A disc is always whole, unlike a folder kept for an update.
            check_names([entry.path for entry in disc.entries() if not entry.directory])
            out.mkdir(parents=True, exist_ok=True)
            # Beside the installation, as the Windows setup extracts it; removed afterwards.
            with tempfile.TemporaryDirectory(prefix='disc-', dir=out) as temporary:
                game = Path(temporary) / 'disc'
                report(f'Extracting {source.name}')
                disc.extract(game, extraction_progress(report))
                return install(game, base, game_exe, report, refresh=refresh)
    game = source.parent if source.is_file() and source.name.lower() == 'default.xex' else source
    if not game.is_dir():
        raise SetupError(f'{source}: choose your Skate 3 ISO, or default.xex in an extracted copy.')
    check_names(folder_paths(game), refresh)
    return install(game, base, game_exe, report, refresh=refresh)


def main(argv=None):
    parser = argparse.ArgumentParser(description='Set up Skate 3 from your own Xbox 360 ISO (docs/web-port/setup.md).')
    parser.add_argument('source', nargs='?', type=Path, help='your Skate 3 ISO, or default.xex in an extracted copy (default: ask)')
    parser.add_argument('--out', type=Path, default=DEFAULT_OUT, help=f'the package folder to set up (default: {DEFAULT_OUT})')
    parser.add_argument('--game-exe', type=Path, required=True, help='a native engine build, for setup\'s checks (convert.sh builds it)')
    parser.add_argument('--without-audio', action='store_true', help='set up without vgmstream: the game has no audio')
    args = parser.parse_args(argv)
    out = args.out.expanduser().resolve()
    try:
        if not args.game_exe.is_file():
            raise SetupError(f'{args.game_exe}: no engine build. Run setup through tools/web-port/convert.sh.')
        if shutil.which('vgmstream-cli') is None and not args.without_audio:
            raise SetupError('Game audio needs vgmstream: brew install vgmstream (or pass --without-audio).')
        source = (args.source or choose_iso()).expanduser().resolve()
        if not source.exists():
            raise SetupError(f'{source} does not exist.')
        print(f'Setting up {source.name} into {out}', flush=True)
        lower_priority()
        installation = convert(source, out, args.game_exe.resolve(), lambda text: print(text, flush=True))
        from tools.asset_pipeline.validation_report import summary
        warnings = summary(installation)
    except (SetupError, DiscError) as error:
        print(error, file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        # The extracted disc is already removed; an existing installation is untouched.
        print('\nStopped.', file=sys.stderr)
        return 130
    except Exception as error:
        log = out / 'data/setup-error.log'
        try:
            log.parent.mkdir(parents=True, exist_ok=True)
            log.write_text(traceback.format_exc(), encoding='utf-8')
        except OSError:
            traceback.print_exc()
            log = None
        print(f'Setup could not finish: {error}' + (f'\nDetails: {log}' if log else ''), file=sys.stderr)
        return 1
    if warnings:
        print(f'Ready, with {len(warnings)} warnings or unavailable components. Details: {installation / "setup-report.json"}')
    else:
        print('Ready.')
    print('\nThis folder holds your game\'s data: keep it to yourself, never upload it.')
    print(f'Play it in the browser (docs/web-port/setup.md):\n\n'
          '    tools/web-port/build.sh    # once: the browser package\n'
          f'    cargo run --release -p skate-web-server -- --installation "{out}"\n\n'
          'then open the http://localhost:8080/#mount=... address it prints, in Chrome or Edge.')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
