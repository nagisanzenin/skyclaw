#!/usr/bin/env python3
"""Actual Unix CLI model registration/removal, corruption and writer contention."""
import argparse
import fcntl
import http.server
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import tomllib

class Trap(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass
    def do_POST(self):
        self.server.calls += 1
        self.send_error(500, 'configuration commands must not call a provider')

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('binary', type=Path)
    parser.add_argument('--config-only', action='store_true')
    parser.add_argument('--foreign-saved', action='store_true')
    parser.add_argument('--offline', action='store_true')
    args = parser.parse_args()
    if (args.foreign_saved or args.offline) and not args.config_only:
        parser.error('--foreign-saved and --offline require --config-only')
    binary = args.binary.resolve()
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Trap)
    server.calls = 0
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix='temm1e-model-storage-') as directory:
            root = Path(directory)
            profile = root / 'profile'
            profile.mkdir(mode=0o700)
            (profile / 'config.toml').write_text(f'''[provider]
name = "openai"
model = "fixture"
api_key = "{'YOUR_API_KEY' if args.offline else 'local-model-storage-fixture'}"
base_url = "http://127.0.0.1:{server.server_port}/v1"
[witness]
enabled = false
[perpetuum]
enabled = false
[hive]
enabled = false
[memory.engram]
enabled = false
curator = "off"
''')
            models = profile / 'custom_models.toml'
            models.write_text('''[[models]]
provider = "anthropic"
name = "kept-model"
context_window = 8192
max_output_tokens = 2048
[[models]]
provider = "openai"
name = "fixture"
context_window = 32768
max_output_tokens = 4096
''')
            credentials = profile / 'credentials.toml'
            if args.foreign_saved:
                credentials.write_text(f'''active = "anthropic"
[[providers]]
name = "anthropic"
keys = ["local-foreign-fixture"]
model = "kept-model"
base_url = "http://127.0.0.1:{server.server_port}/foreign"
''')
            saved_credentials = credentials.read_bytes() if credentials.exists() else None
            env = {key: value for key, value in os.environ.items() if not key.endswith(('_API_KEY', '_TOKEN')) and not key.startswith('TEMM1E_')}
            env['TEMM1E_DATA_DIR'] = str(profile)
            def run(commands):
                result = subprocess.run([str(binary), 'chat'], input=commands + '\n/quit\n', text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=env, cwd=root, timeout=30)
                assert result.returncode == 0, result.stdout[-3000:]
                assert server.calls == 0, 'configuration command called provider'
                if args.foreign_saved:
                    assert credentials.read_bytes() == saved_credentials, 'model command changed foreign saved route'
                return result.stdout
            if not args.config_only:
                configured = run(f'proxy openai http://127.0.0.1:{server.server_port}/v1 local-model-storage-fixture model:fixture')
                assert 'Configured openai with model fixture' in configured, configured[-3000:]
            add = '/addmodel kept-model context:8192 output:1024 input_price:1 output_price:2 vision:false'
            output = run(add)
            assert 'Added custom model' in output, output[-4000:]
            saved = tomllib.loads(models.read_text())['models']
            assert len(saved) == 3, saved
            scoped = {model['provider']: model for model in saved if model['name'] == 'kept-model'}
            assert scoped['openai']['max_output_tokens'] == 1024
            assert scoped['openai']['image_input'] is False
            assert scoped['anthropic']['max_output_tokens'] == 2048
            assert models.stat().st_mode & 0o777 == 0o600
            listing = run('/listmodels')
            assert ('Provider: openai (configured)' if args.offline else 'Provider: openai (active)') in listing, listing
            assert 'Provider: anthropic (active)' not in listing, listing
            assert 'fixture' in listing and ('← saved' if args.offline else '← current') in listing, listing
            if args.offline:
                assert '← current' not in listing, listing
            if not args.offline:
                saved_before_choice = credentials.read_bytes() if credentials.exists() else None
                registry_before_choice = models.read_bytes()
                choice = run('/model kept-model\n/model')
                assert 'Model selected: kept-model on openai' in choice, choice
                assert 'Current: kept-model on openai' in choice, choice
                assert (credentials.read_bytes() if credentials.exists() else None) == saved_before_choice
                assert models.read_bytes() == registry_before_choice
                assert 'Current: fixture on openai' in run('/model'), 'session choice silently persisted'
            before = models.read_bytes()
            with (profile / 'custom_models.lock').open('r+') as lock:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                output = run(add + '\n/removemodel kept-model')
                assert 'Failed to save custom model' in output
                assert 'Failed to remove custom model' in output
                assert 'update is in progress' in output
                assert models.read_bytes() == before
            assert 'Removed 1 custom model' in run('/removemodel kept-model')
            assert [model['provider'] for model in tomllib.loads(models.read_text())['models'] if model['name'] == 'kept-model'] == ['anthropic']
            corrupt = b"[[models]\nname = 'unfinished'\n"
            models.write_bytes(corrupt)
            output = run(add + '\n/removemodel kept-model')
            assert 'Failed to save custom model' in output and 'Failed to remove custom model' in output
            assert models.read_bytes() == corrupt
            assert server.calls == 0
            print(json.dumps({'passed': True, 'provider_calls': server.calls, 'scoped_add_remove': True, 'contention_preserved_bytes': True, 'malformed_file_preserved': True, 'private_mode': '0600', 'config_only': args.config_only, 'foreign_saved_unchanged': args.foreign_saved, 'offline_configured': args.offline}))
    finally:
        server.shutdown()
        server.server_close()
        thread.join()

if __name__ == '__main__':
    main()
