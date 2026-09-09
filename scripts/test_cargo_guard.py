import importlib.util
import os
from pathlib import Path
import sys
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('cargo_guard', Path(__file__).with_name('cargo_guard.py'))
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


class DiskGuardTests(unittest.TestCase):
    def test_refuses_before_start(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            marker = root / 'started'
            result = guard.run_guarded([sys.executable, '-c', f'open({str(marker)!r}, "w").close()'], root,
                                       os.environ.copy(), 1, 1, check=lambda *_: 'insufficient space')
            self.assertEqual(result, 75)
            self.assertFalse(marker.exists())

    def test_stops_active_process_on_limit(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            marker = root / 'finished'
            calls = 0
            def check(*_):
                nonlocal calls
                calls += 1
                return None if calls == 1 else 'target limit exceeded'
            result = guard.run_guarded([sys.executable, '-c', f'import time; time.sleep(20); open({str(marker)!r}, "w").close()'],
                                       root, os.environ.copy(), 1, 1, check=check)
            self.assertEqual(result, 75)
            self.assertFalse(marker.exists())

    def test_preserves_failure_exit_and_counts_target_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / 'target'
            target.mkdir()
            (target / 'artifact').write_bytes(b'x' * 100)
            self.assertEqual(guard.tree_size(target), 100)
            self.assertIn('above limit', guard.limit_reason(root, 1, 99))
            result = guard.run_guarded([sys.executable, '-c', 'raise SystemExit(7)'], root,
                                       os.environ.copy(), 1, 1000)
            self.assertEqual(result, 7)


if __name__ == '__main__':
    unittest.main()
