"""Synthetic disc images for xiso.py; contains no game data.

    python3 -m unittest discover -s tools/web-port -p 'test_*.py'
"""
import contextlib
import io
import shutil
import struct
import tempfile
import unittest
from collections import namedtuple
from pathlib import Path
from unittest import mock

import xiso
from xiso import SECTOR, DiscError, DiscImage

Usage = namedtuple('Usage', 'total used free')


def sectors(size):
    return -(-size // SECTOR)


def table(entries, pad_offsets=False):
    """A directory table: [(name, sector, size, attributes)] as a balanced tree, root first, no
    entry crossing a sector, padded with 0xFF. With `pad_offsets`, a child that starts a new
    sector is pointed at through the padding before it, as some writers do."""
    order = sorted(entries, key=lambda e: e[0].upper())
    placed = []  # (entry, left index, right index), pre-order

    def place(items):
        if not items:
            return None
        middle = len(items) // 2
        index = len(placed)
        placed.append([items[middle], None, None])
        placed[index][1] = place(items[:middle])
        placed[index][2] = place(items[middle + 1:])
        return index

    place(order)
    data, offsets, padded = bytearray(), [], {}
    for (name, _, _, _), _, _ in placed:
        length = (xiso.ENTRY.size + len(name) + 3) & ~3
        if len(data) % SECTOR + length > SECTOR:
            pad = len(data)
            data += b'\xff' * (SECTOR - len(data) % SECTOR)
            padded[len(offsets)] = pad
        offsets.append(len(data))
        data += bytes(length)
    for index, ((name, first, size, attributes), left, right) in enumerate(placed):
        def link(child):
            if child is None:
                return 0
            return (padded[child] if pad_offsets and child in padded else offsets[child]) // 4
        raw = name.encode('latin-1')
        xiso.ENTRY.pack_into(data, offsets[index], link(left), link(right), first, size, attributes, len(raw))
        data[offsets[index] + xiso.ENTRY.size:offsets[index] + xiso.ENTRY.size + len(raw)] = raw
    data += b'\xff' * (-len(data) % SECTOR)
    return bytes(data)


def image(path, tree, partition=0, pad_offsets=False, volume_tail=xiso.MAGIC):
    """Writes `tree` ({name: bytes | dict}) as an image at `partition` (sparse before it)."""
    # Table sizes depend only on names: lay out tables first, then files.
    folders, files = [], []

    def survey(node, key):
        folders.append(key)
        for name, value in node.items():
            if isinstance(value, dict):
                survey(value, key + (name,))
            else:
                files.append(key + (name,))

    survey(tree, ())

    def node_at(key):
        node = tree
        for part in key:
            node = node[part]
        return node

    sizes = {key: len(table([(n, 0, 0, 0) for n in node_at(key)])) if node_at(key) else 0 for key in folders}
    first, next_sector = {}, 33  # the volume descriptor is sector 32
    for key in folders:
        first[key] = next_sector if sizes[key] else 0
        next_sector += sectors(sizes[key])
    for key in files:
        first[key] = next_sector
        next_sector += max(1, sectors(len(node_at(key))))
    with open(path, 'wb') as out:
        def put(sector, data):
            out.seek(partition + sector * SECTOR)
            out.write(data)

        descriptor = bytearray(SECTOR)
        descriptor[:len(xiso.MAGIC)] = xiso.MAGIC
        struct.pack_into('<II', descriptor, len(xiso.MAGIC), first[()], sizes[()])
        descriptor[-len(xiso.MAGIC):] = volume_tail
        put(32, descriptor)
        for key in folders:
            node = node_at(key)
            if node:
                entries = [(name, first[key + (name,)],
                            sizes[key + (name,)] if isinstance(value, dict) else len(value),
                            xiso.DIRECTORY if isinstance(value, dict) else 0x20)
                           for name, value in node.items()]
                put(first[key], table(entries, pad_offsets))
        for key in files:
            put(first[key], node_at(key))
        out.truncate(partition + next_sector * SECTOR)


def flatten(tree, prefix=''):
    for name, value in tree.items():
        if isinstance(value, dict):
            yield prefix + name + '/', None
            yield from flatten(value, prefix + name + '/')
        else:
            yield prefix + name, value


def read_back(root):
    root = Path(root)
    return {p.relative_to(root).as_posix() + ('/' if p.is_dir() else ''): None if p.is_dir() else p.read_bytes()
            for p in root.rglob('*')}


# A disc's shape: nested folders, an empty folder and file, a file over several sectors, and a
# folder whose table spans sectors (long names force padding before entries).
DISC = {
    'default.xex': b'XEX2' + bytes(5000),
    'data': {
        'big': {'miscload.big': b'BIGF' * 1000, 'db.big': b'db'},
        'content': {f'worldDIST_District{i:03}_{"x" * 60}.big': bytes([i]) * (i * 37) for i in range(120)},
        'empty': {},
        'zero.bin': b'',
    },
    '$SystemUpdate': {'su20076000_00000000': b'update'},
}


class DiscImageTests(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix='xiso-test-'))
        self.addCleanup(shutil.rmtree, self.dir)

    def check_round_trip(self, **layout):
        path = self.dir / 'disc.iso'
        image(path, DISC, **layout)
        with DiscImage(path) as disc:
            listed = [e.path + ('/' if e.directory else '') for e in disc.entries()]
            disc.extract(self.dir / 'out')
        self.assertEqual(sorted(listed), sorted(name for name, _ in flatten(DISC)))
        self.assertEqual(read_back(self.dir / 'out'), dict(flatten(DISC)))
        return disc

    def test_extracts_every_file_and_folder(self):
        disc = self.check_round_trip()
        self.assertEqual(disc.partition, 0)

    def test_finds_the_game_partition_of_full_disc_images(self):
        for partition in xiso.PARTITIONS[1:]:
            with self.subTest(partition=hex(partition)):
                disc = self.check_round_trip(partition=partition)
                self.assertEqual(disc.partition, partition)
                shutil.rmtree(self.dir / 'out')

    def test_offsets_into_padding_mean_the_next_sector(self):
        self.check_round_trip(pad_offsets=True)

    def test_lists_folders_before_their_contents_in_name_order(self):
        path = self.dir / 'disc.iso'
        image(path, {'b': {'z': b'1', 'a': b'2'}, 'A': b'3'})
        with DiscImage(path) as disc:
            self.assertEqual([e.path for e in disc.entries()], ['A', 'b', 'b/a', 'b/z'])

    def test_an_empty_disc_extracts_nothing(self):
        path = self.dir / 'disc.iso'
        image(path, {})
        with DiscImage(path) as disc:
            self.assertEqual(list(disc.entries()), [])
            self.assertEqual(disc.extract(self.dir / 'out'), 0)

    def test_refuses_what_is_not_a_disc(self):
        path = self.dir / 'other.iso'
        path.write_bytes(bytes(0x20000))
        with self.assertRaisesRegex(DiscError, 'not an Xbox 360 disc image'):
            DiscImage(path)
        image(path, DISC, volume_tail=b'X' * len(xiso.MAGIC))
        with self.assertRaisesRegex(DiscError, 'header is damaged'):
            DiscImage(path)

    def test_refuses_an_incomplete_image_before_writing(self):
        path = self.dir / 'disc.iso'
        image(path, DISC)
        with path.open('r+b') as f:
            f.truncate(path.stat().st_size - SECTOR)
        with DiscImage(path) as disc, self.assertRaisesRegex(DiscError, 'incomplete'):
            disc.extract(self.dir / 'out')
        self.assertEqual(list((self.dir / 'out').iterdir()), [])

    def test_refuses_without_free_space(self):
        path = self.dir / 'disc.iso'
        image(path, DISC)
        with DiscImage(path) as disc, mock.patch.object(xiso.shutil, 'disk_usage', return_value=Usage(10, 10, 100)):
            with self.assertRaisesRegex(DiscError, 'needs .* free'):
                disc.extract(self.dir / 'out')
        self.assertEqual(list((self.dir / 'out').iterdir()), [])

    def test_refuses_a_destination_with_files(self):
        path = self.dir / 'disc.iso'
        image(path, DISC)
        (self.dir / 'out').mkdir()
        (self.dir / 'out/kept').write_bytes(b'')
        with DiscImage(path) as disc, self.assertRaisesRegex(DiscError, 'not empty'):
            disc.extract(self.dir / 'out')

    def test_refuses_unsafe_names(self):
        for name in ('..', '.', 'a/b', 'a\\b', 'a\0'):
            with self.subTest(name=name):
                path = self.dir / 'disc.iso'
                image(path, {'ok': b'1', name: b'2'})
                with DiscImage(path) as disc, self.assertRaisesRegex(DiscError, 'unsafe name'):
                    list(disc.entries())

    def test_refuses_names_that_collide_without_case(self):
        path = self.dir / 'disc.iso'
        image(path, {'Data': {'a': b'1'}, 'DATA': {'b': b'2'}})
        with DiscImage(path) as disc, self.assertRaisesRegex(DiscError, 'listed twice'):
            list(disc.entries())

    def test_refuses_tables_that_loop(self):
        path = self.dir / 'disc.iso'
        image(path, {'a': b'1', 'b': b'2', 'c': b'3'})
        data = bytearray(path.read_bytes())
        root = 33 * SECTOR
        # The root entry's left subtree points back at the root entry.
        struct.pack_into('<H', data, root, 0)
        right = struct.unpack_from('<H', data, root + 2)[0]
        struct.pack_into('<H', data, root + right * 4, 0)
        struct.pack_into('<H', data, root + right * 4 + 2, right)
        path.write_bytes(bytes(data))
        with DiscImage(path) as disc, self.assertRaisesRegex(DiscError, 'loops'):
            list(disc.entries())

    def test_refuses_folders_that_contain_themselves(self):
        path = self.dir / 'disc.iso'
        image(path, {'a': {'b': b'1'}})
        data = bytearray(path.read_bytes())
        # Folder a's table is sector 34; make its entry b a folder whose table is the root (33).
        struct.pack_into('<IIB', data, 34 * SECTOR + 4, 33, SECTOR, xiso.DIRECTORY)
        path.write_bytes(bytes(data))
        with DiscImage(path) as disc, self.assertRaisesRegex(DiscError, 'loops'):
            list(disc.entries())

    def test_command_line(self):
        path = self.dir / 'disc.iso'
        image(path, {'default.xex': b'x'})
        output = io.StringIO()
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
            self.assertEqual(xiso.main(['extract', str(path), str(self.dir / 'out')]), 0)
            self.assertEqual(xiso.main(['list', str(path)]), 0)
            self.assertEqual(xiso.main(['nonsense']), 2)
        self.assertEqual((self.dir / 'out/default.xex').read_bytes(), b'x')
        self.assertIn('default.xex  1', output.getvalue())
        self.assertIn('xiso.py extract IMAGE DESTINATION', output.getvalue())


if __name__ == '__main__':
    unittest.main()
