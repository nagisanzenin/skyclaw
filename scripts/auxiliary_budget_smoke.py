#!/usr/bin/env python3
"""Actual CLI: optional Engram calls obey the foreground command budget."""
import argparse
import http.server
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
from tui_pty_smoke import Provider


def run(binary, server, limited):
    with tempfile.TemporaryDirectory(prefix='temm1e-aux-budget-') as directory:
        root = Path(directory)
        profile = root / 'profile'
        profile.mkdir(mode=0o700)
        (profile / 'config.toml').write_text(f'''[provider]
name = "openai"
model = "pty-fixture"
api_key = "local-fixture-only"
base_url = "http://127.0.0.1:{server.server_port}/v1"
[agent]
max_spend_usd = {0.0002 if limited else 0.0}
self_audit_enabled = false
[memory.engram]
enabled = true
curator = "substantive"
[perpetuum]
enabled = false
[hive]
enabled = false
[social]
enabled = false
[consciousness]
enabled = false
''')
        (profile / 'custom_models.toml').write_text('''[[models]]
provider = "openai"
name = "pty-fixture"
context_window = 32768
max_output_tokens = 4096
input_price_per_1m = 1.0
output_price_per_1m = 1.0
pricing_verified = true
''')
        env = {key: value for key, value in os.environ.items()
               if not key.endswith(('_API_KEY', '_TOKEN')) and not key.startswith('TEMM1E_')}
        env['TEMM1E_DATA_DIR'] = str(profile)
        start = len(server.requests)
        result = subprocess.run([str(binary), 'chat'],
                                input='Chào Tem — I prefer Vietnamese for our ordinary conversations about my projects.\n/quit\n',
                                text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                cwd=root, env=env, timeout=40)
        assert result.returncode == 0, result.stdout[-3000:]
        assert 'PTY_REPLY_42' in result.stdout, result.stdout[-3000:]
        requests = server.requests[start:]
        curator = [r for r in requests if 'precise long-term-memory curator' in json.dumps(r)]
        assert len(curator) == (0 if limited else 1), f'limited={limited}: curator requests={len(curator)}, total={len(requests)}'
        assert len(requests) == (2 if limited else 3), f'limited={limited}: total requests={len(requests)}'
        assert all(a == 'Bearer local-fixture-only' for a in server.authorizations)
        return {'limited': limited, 'passed': True, 'provider_requests': len(requests), 'curator_requests': len(curator)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('binary', type=Path)
    args = parser.parse_args()
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    server.requests, server.authorizations = [], []
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        print(json.dumps([run(args.binary.resolve(), server, limited) for limited in [True, False]], indent=2))
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


if __name__ == '__main__':
    main()
