"""Run after building scripts/tests/tooltip_cache_harness.cpp with Win32 MSVC."""
import os
from pathlib import Path
import subprocess
import unittest
from client_tooltip_cache import build_stub, build_command_stub, build_melee_stub


class TooltipCacheTest(unittest.TestCase):
    def test_native_melee_sync_ordering_and_unchanged_attacks(self):
        harness = Path(__file__).resolve().parents[1] / 'logs/melee_sync_harness.exe'
        if not harness.is_file():
            self.skipTest('Build the Win32 melee sync harness first.')
        with subprocess.Popen([str(harness)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, text=True) as process:
            addresses = [int(x) for x in process.stdout.readline().split()]
            self.assertEqual(len(addresses), 5)
            output, error = process.communicate(build_melee_stub(*addresses).hex() + '\n', timeout=10)
            self.assertEqual(process.returncode, 0, error)
            self.assertIn('PASS:', output)

    def test_win32_calling_convention_and_cache_invalidation(self):
        harness = Path(os.environ.get('REKO_TOOLTIP_HARNESS',
                                     str(Path(__file__).resolve().parents[1] / 'logs/tooltip_cache_harness.exe')))
        if not harness.is_file():
            self.skipTest('Build the Win32 tooltip cache harness first.')
        with subprocess.Popen([str(harness)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, text=True) as process:
            addresses = [int(x) for x in process.stdout.readline().split()]
            self.assertEqual(len(addresses), 5)
            process.stdin.write(build_stub(*addresses).hex() + '\n')
            process.stdin.flush()
            commands = [int(x) for x in process.stdout.readline().split()]
            self.assertEqual(len(commands), 3)
            output, error = process.communicate(build_command_stub(*commands).hex() + '\n', timeout=10)
            self.assertEqual(process.returncode, 0, error)
            self.assertIn('PASS:', output)
            self.assertIn('other native slash commands preserved', output)


if __name__ == '__main__':
    unittest.main()
