"""Xbox 360 disc images (XDVDFS), read without extract-xiso.

The Windows setup extracts the player's ISO with extract-xiso, which it downloads as a Windows
program. Outside Windows, convert.py extracts it with this reader instead
(docs/web-port/setup.md). It reads the player's own image and contains no game data.

A disc's game partition starts with a volume descriptor (sector 32) naming the root directory
table. Each directory table is a binary tree of entries: an entry gives the offsets of its left
and right subtrees (in 4-byte units from the table's start, 0 for none), its first sector, its
size, its attributes and its name.

    python3 tools/web-port/xiso.py list IMAGE
    python3 tools/web-port/xiso.py extract IMAGE DESTINATION
"""
from __future__ import annotations

import shutil
import struct
import sys
from dataclasses import dataclass
from pathlib import Path

SECTOR = 2048
MAGIC = b'MICROSOFT*XBOX*MEDIA'
# The volume descriptor, from the game partition's start.
VOLUME = 0x10000
# Where the game partition starts, tried in extract-xiso's order: a rebuilt image, a full Xbox 360
# disc (XGD2), a later Xbox 360 disc (XGD3), an original Xbox disc (XGD1).
PARTITIONS = (0, 0x0FD90000, 0x02080000, 0x18300000)
ENTRY = struct.Struct('<HHIIBB')  # left, right, first sector, size, attributes, name length
DIRECTORY = 0x10
PADDING = 0xFFFF
# Deeper than any disc; a deeper image is damaged or made to loop.
DEPTH = 64
# Directory tables are a few sectors; a larger one is damaged.
TABLE_LIMIT = 64 << 20
CHUNK = 8 << 20


class DiscError(RuntimeError):
    """Not an Xbox disc image, or a damaged or incomplete one."""


@dataclass(frozen=True)
class Entry:
    path: str  # '/'-separated, from the disc's root
    sector: int
    size: int
    directory: bool


def gigabytes(size: int) -> str:
    return f'{size / 1e9:.1f} GB'


class DiscImage:
    """An ISO opened for reading. Use as a context manager."""

    def __init__(self, path):
        self.path = Path(path)
        self.stream = self.path.open('rb')
        try:
            self.length = self.path.stat().st_size
            self.partition = self._partition()
            descriptor = self._read(self.partition + VOLUME, SECTOR)
            self.root_sector, self.root_size = struct.unpack_from('<II', descriptor, len(MAGIC))
            if descriptor[-len(MAGIC):] != MAGIC:
                raise DiscError(f'{self.path.name}: the disc header is damaged. Copy the image again.')
        except BaseException:
            self.stream.close()
            raise

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.stream.close()

    def _read(self, offset: int, size: int) -> bytes:
        self.stream.seek(offset)
        data = self.stream.read(size)
        if len(data) != size:
            raise DiscError(f'{self.path.name} ends early: the image is incomplete. Copy it again.')
        return data

    def _partition(self) -> int:
        for start in PARTITIONS:
            if start + VOLUME + SECTOR <= self.length and self._read(start + VOLUME, len(MAGIC)) == MAGIC:
                return start
        raise DiscError(f'{self.path.name} is not an Xbox 360 disc image. Select the ISO of your Skate 3 disc.')

    def _damaged(self, why: str) -> DiscError:
        return DiscError(f'{self.path.name}: {why}; the image is damaged. Copy it again.')

    def entries(self):
        """Every file and folder, each folder before its contents, in disc order (by name)."""
        yield from self._directory(self.root_sector, self.root_size, '', 0, set())

    def _directory(self, sector: int, size: int, prefix: str, depth: int, tables: set):
        if size == 0:
            return
        if depth > DEPTH or sector in tables or size > TABLE_LIMIT:
            raise self._damaged(f'the folder table of /{prefix} loops or is too large')
        tables.add(sector)
        table = self._read(self.partition + sector * SECTOR, size)
        names = set()
        visited = set()
        # In order (left subtree, entry, right subtree) without recursion: a damaged table could
        # be a list thousands of entries deep.
        stack, offset = [], 0
        while stack or offset is not None:
            while offset is not None:
                node = self._node(table, offset, prefix, visited)
                if node is None:
                    break
                stack.append(node)
                offset = node[0] or None
            if not stack:
                break
            _, right, first, length, attributes, name = stack.pop()
            folded = name.casefold()
            if folded in names:
                raise self._damaged(f'/{prefix}{name} is listed twice')
            names.add(folded)
            entry = Entry(prefix + name, first, length, bool(attributes & DIRECTORY))
            yield entry
            if entry.directory:
                yield from self._directory(first, length, entry.path + '/', depth + 1, tables)
            offset = right or None

    def _node(self, table: bytes, offset: int, prefix: str, visited: set):
        """The entry `offset` (4-byte units) names, or None for an empty table."""
        position = offset * 4
        if position + 2 > len(table):
            raise self._damaged(f'an entry of /{prefix} lies outside its table')
        if struct.unpack_from('<H', table, position)[0] == PADDING:
            if offset == 0:
                return None
            # As extract-xiso reads it: an offset into a sector's padding means the next sector.
            position = (position // SECTOR + 1) * SECTOR
            if position + 2 > len(table) or struct.unpack_from('<H', table, position)[0] == PADDING:
                raise self._damaged(f'an entry of /{prefix} points into padding')
        if position in visited:
            raise self._damaged(f'the folder table of /{prefix} loops')
        visited.add(position)
        if position + ENTRY.size > len(table):
            raise self._damaged(f'an entry of /{prefix} lies outside its table')
        left, right, first, length, attributes, name_length = ENTRY.unpack_from(table, position)
        start = position + ENTRY.size
        raw = table[start:start + name_length]
        if len(raw) != name_length:
            raise self._damaged(f'an entry name of /{prefix} lies outside its table')
        name = raw.decode('latin-1')
        if name in ('', '.', '..') or any(c in name for c in '/\\\0'):
            raise self._damaged(f'/{prefix} holds an unsafe name {name!r}')
        return left, right, first, length, attributes, name

    def extract(self, destination, progress=None):
        """Extract every file into `destination`, which must be empty. `progress(done, total)`
        is called as bytes are copied. Checks the whole image, and the free space, before
        writing anything."""
        destination = Path(destination)
        destination.mkdir(parents=True, exist_ok=True)
        if any(destination.iterdir()):
            raise DiscError(f'{destination} is not empty')
        entries = list(self.entries())
        for entry in entries:
            if not entry.directory and entry.size and self.partition + entry.sector * SECTOR + entry.size > self.length:
                raise DiscError(f'{self.path.name} is incomplete: {entry.path} ends past the end of the image. Copy it again.')
        total = sum(entry.size for entry in entries if not entry.directory)
        free = shutil.disk_usage(destination).free
        if total > free:
            raise DiscError(f'Extracting {self.path.name} needs {gigabytes(total)} free beside the installation; {gigabytes(free)} is free.')
        done = 0
        for entry in entries:
            target = destination.joinpath(*entry.path.split('/'))
            if entry.directory:
                target.mkdir()
                continue
            self.stream.seek(self.partition + entry.sector * SECTOR)
            left = entry.size
            with target.open('xb') as output:
                while left:
                    data = self.stream.read(min(CHUNK, left))
                    if not data:
                        raise DiscError(f'{self.path.name} ends early: the image is incomplete. Copy it again.')
                    output.write(data)
                    left -= len(data)
                    done += len(data)
                    if progress:
                        progress(done, total)
        return total


def main(argv):
    if len(argv) == 2 and argv[0] == 'list':
        with DiscImage(argv[1]) as disc:
            for entry in disc.entries():
                print(entry.path + '/' if entry.directory else f'{entry.path}  {entry.size}')
        return 0
    if len(argv) == 3 and argv[0] == 'extract':
        with DiscImage(argv[1]) as disc:
            total = disc.extract(argv[2])
        print(f'Extracted {gigabytes(total)} into {argv[2]}')
        return 0
    print(__doc__.strip().rsplit('\n\n', 1)[-1], file=sys.stderr)
    return 2


if __name__ == '__main__':
    try:
        raise SystemExit(main(sys.argv[1:]))
    except DiscError as error:
        print(error, file=sys.stderr)
        raise SystemExit(1)
