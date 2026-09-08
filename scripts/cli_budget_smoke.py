#!/usr/bin/env python3
"""Real CLI connection reconstruction and budget admission, local fixtures only."""
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
    with tempfile.TemporaryDirectory(prefix='temm1e-cli-budget-') as directory:
        root = Path(directory)
        profile = root / 'profile'
        profile.mkdir(mode=0o700)
        endpoint = f'http://127.0.0.1:{server.server_port}/v1'
        (profile / 'config.toml').write_text(f'''[provider]
name = "openai"
model = "pty-fixture"
api_key = "local-fixture-only"
base_url = "{endpoint}"
[agent]
max_spend_usd = {0.0002 if limited else 0.0}
self_audit_enabled = false
[memory.engram]
curator = "off"
[perpetuum]
enabled = false
[hive]
enabled = false
[social]
enabled = false
[consciousness]
enabled = false
''')
        (profile / 'credentials.toml').write_text(f'''active = "openai"
[[providers]]
name = "openai"
model = "pty-fixture"
keys = ["local-fixture-only"]
base_url = "{endpoint}"
''')
        (profile / 'credentials.toml').chmod(0o600)
        (profile / 'custom_models.toml').write_text(''.join(f'''[[models]]
provider = "openai"
name = "{model}"
context_window = 32768
max_output_tokens = 4096
input_price_per_1m = 1.0
output_price_per_1m = 1.0
pricing_verified = true
''' for model in ['pty-fixture', 'pty-fixture-next']))
        env = {key: value for key, value in os.environ.items()
               if not key.endswith(('_API_KEY', '_TOKEN')) and not key.startswith('TEMM1E_')}
        env['TEMM1E_DATA_DIR'] = str(profile)
        count = len(server.requests)
        input_text = f'Chào Tem — CLI_INPUT_42\nproxy openai {endpoint} local-fixture-only model:pty-fixture-next\nCLI_NEXT_43\n/quit\n'
        result = subprocess.run([str(binary), 'chat'], input=input_text, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                cwd=root, env=env, timeout=40)
        assert result.returncode == 0, result.stdout[-3000:]
        assert 'PTY_REPLY_42' in result.stdout, result.stdout[-3000:]
        assert 'Configured openai with model pty-fixture-next' in result.stdout, result.stdout[-3000:]
        requests = server.requests[count:]
        if limited:
            assert 'Budget exceeded' in result.stdout, result.stdout[-3000:]
            assert len(requests) == 2, f'exhausted reconstruction invoked provider: {len(requests)}'
            assert all(request.get('model') == 'pty-fixture' for request in requests)
            assert 'PTY_REPLY_43' not in result.stdout
        else:
            assert 'PTY_REPLY_43' in result.stdout, result.stdout[-3000:]
            assert len(requests) == 4, f'unexpected request count: {len(requests)}'
            assert requests[-1].get('model') == 'pty-fixture-next'
        assert all(auth == 'Bearer local-fixture-only' for auth in server.authorizations)
        return {'passed': True, 'limited': limited, 'provider_requests': len(requests),
                'replacement_requests': sum(r.get('model') == 'pty-fixture-next' for r in requests),
                'unicode_input': any('CLI_INPUT_42' in json.dumps(r) for r in requests)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('binary', type=Path)
    args = parser.parse_args()
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    server.requests, server.authorizations = [], []
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        print(json.dumps([run(args.binary.resolve(), server, limited) for limited in [False, True]], indent=2))
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


if __name__ == '__main__':
    main()
