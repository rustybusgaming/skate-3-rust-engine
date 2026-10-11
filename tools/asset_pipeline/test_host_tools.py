"""Setup outside Windows: system tools instead of the pinned Windows downloads, and the memory
query that bounds the map workers (docs/web-port/setup.md). No game data."""
import os
import unittest
from pathlib import Path
from unittest import mock

from . import install

VM_STAT = """Mach Virtual Memory Statistics: (page size of 16384 bytes)
Pages free:                               10000.
Pages active:                            300000.
Pages inactive:                           20000.
Pages speculative:                         3000.
Pages throttled:                              0.
Pages wired down:                        100000.
Pages purgeable:                           4000.
"""


@unittest.skipIf(os.name == 'nt', 'Windows downloads the pinned tools')
class HostToolTests(unittest.TestCase):
    def test_tools_come_from_the_system(self):
        with mock.patch.object(install.shutil, 'which', return_value='/opt/homebrew/bin/vgmstream-cli'):
            found = install.dependency(Path('unused'), 'vgmstream-cli', 'https://example.invalid/x.zip', '0' * 64, print)
        self.assertEqual(found, Path('/opt/homebrew/bin/vgmstream-cli'))

    def test_a_missing_tool_names_its_package(self):
        with mock.patch.object(install.shutil, 'which', return_value=None):
            with self.assertRaisesRegex(OSError, r'vgmstream-cli is not installed \(brew install vgmstream\)'):
                install.dependency(Path('unused'), 'vgmstream-cli', 'https://example.invalid/x.zip', '0' * 64, print)
            with self.assertRaisesRegex(OSError, 'extract-xiso is not installed$'):
                install.dependency(Path('unused'), 'extract-xiso', 'https://example.invalid/x.zip', '0' * 64, print)


class MemoryTests(unittest.TestCase):
    def test_macos_counts_free_inactive_and_speculative_pages(self):
        result = mock.Mock(stdout=VM_STAT)
        with mock.patch.object(install.sys, 'platform', 'darwin'), \
                mock.patch.object(install.subprocess, 'run', return_value=result) as run:
            self.assertEqual(install.available_memory(), 16384 * (10000 + 20000 + 3000))
        self.assertEqual(run.call_args.args[0], ['vm_stat'])

    def test_macos_without_vm_stat_is_unknown(self):
        with mock.patch.object(install.sys, 'platform', 'darwin'), \
                mock.patch.object(install.subprocess, 'run', side_effect=FileNotFoundError):
            self.assertIsNone(install.available_memory())

    def test_linux_reads_mem_available(self):
        meminfo = 'MemTotal:       32000000 kB\nMemFree:         1000000 kB\nMemAvailable:   12000000 kB\n'
        with mock.patch.object(install.sys, 'platform', 'linux'), \
                mock.patch('builtins.open', mock.mock_open(read_data=meminfo)):
            self.assertEqual(install.available_memory(), 12000000 * 1024)

    def test_map_workers_leave_room_on_a_small_machine(self):
        with mock.patch.dict(install.os.environ), mock.patch.object(install.os, 'cpu_count', return_value=10):
            install.os.environ.pop('SKATE_SETUP_MAP_WORKERS', None)
            for gigabytes, workers in ((16, 3), (8, 2), (4, 1)):
                with mock.patch.object(install, 'available_memory', return_value=gigabytes * install.GIB):
                    self.assertEqual(install.map_workers(), workers, gigabytes)


if __name__ == '__main__':
    unittest.main()
