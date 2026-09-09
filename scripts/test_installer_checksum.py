"""Exercise the exact standalone installer verifier without installing binaries."""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SOURCE = (Path(__file__).resolve().parents[1] / 'install.sh').read_text()
VERIFIER = SOURCE.split('# BEGIN CHECKSUM VERIFIER', 1)[1].split('\n', 1)[1].split('# END CHECKSUM VERIFIER', 1)[0]

class ChecksumTest(unittest.TestCase):
    def verify(self, manifest, missing_tools=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'artifact').write_bytes(b'fixture binary')
            (root / 'manifest').write_text(manifest)
            script = 'error() { printf "%s\\n" "$1" >&2; exit 1; }\ninfo() { printf "%s\\n" "$1"; }\n' + VERIFIER + '\nverify_checksum "$1" "$2" artifact\n'
            env = dict(os.environ)
            if missing_tools:
                env['PATH'] = directory
            return subprocess.run(['/bin/sh', '-c', script, 'checksum-test', str(root / 'artifact'), str(root / 'manifest')], env=env, capture_output=True, text=True)

    def test_exact_name(self):
        digest = hashlib.sha256(b'fixture binary').hexdigest()
        result = self.verify(f'{digest}  artifact\n' + '0' * 64 + '  artifact-desktop\n')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('Checksum verified', result.stdout)

    def test_fail_closed(self):
        digest = hashlib.sha256(b'fixture binary').hexdigest()
        for manifest in ['', '0' * 64 + '  artifact\n', f'{digest}  another\n', f'{digest}  artifact\n' * 2, 'invalid  artifact\n']:
            with self.subTest(manifest=manifest):
                result = self.verify(manifest)
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn('Checksum verified', result.stdout)

    def test_missing_hash_program(self):
        result = self.verify(hashlib.sha256(b'fixture binary').hexdigest() + '  artifact\n', True)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn('Checksum verified', result.stdout)

if __name__ == '__main__':
    unittest.main()
