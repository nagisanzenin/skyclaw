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
import memory_policy_fixture


def run(binary, server, trap, limited, engram_enabled=None):
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
enabled = {str(engram_enabled if engram_enabled is not None else True).lower()}
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
        (profile / 'credentials.toml').write_text(f'''active = "anthropic"
[[providers]]
name = "anthropic"
model = "unrelated-saved-model"
keys = ["synthetic-unrelated-key"]
base_url = "http://127.0.0.1:{trap.server_port}/v1"
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
        if engram_enabled is not None:
            memory_policy_fixture.seed(profile)
        env = {key: value for key, value in os.environ.items()
               if not key.endswith(('_API_KEY', '_TOKEN')) and not key.startswith('TEMM1E_')}
        env['TEMM1E_DATA_DIR'] = str(profile)
        count = len(server.requests)
        first = 'Chào Tem — CLI_INPUT_42'
        second = 'CLI_NEXT_43'
        if engram_enabled is not None:
            first += ' — This ordinary conversation checks the configured memory behavior.'
            second += ' — This is another ordinary conversation after changing the model.'
        input_text = f'{first}\nproxy openai {endpoint} local-fixture-only model:pty-fixture-next\n{second}\n/quit\n'
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
        if engram_enabled is not None:
            memory_policy_fixture.assert_policy(requests, engram_enabled)
        assert not trap.requests, 'unrelated saved endpoint received a request'
        assert all(auth == 'Bearer local-fixture-only' for auth in server.authorizations)
        return {'engram_enabled': engram_enabled, 'connection_isolation': True, 'passed': True, 'limited': limited, 'provider_requests': len(requests),
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
    trap = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    trap.requests, trap.authorizations = [], []
    trap_thread = threading.Thread(target=trap.serve_forever, daemon=True)
    trap_thread.start()
    try:
        print(json.dumps([run(args.binary.resolve(), server, trap, limited) for limited in [False, True]], indent=2))
    finally:
        trap.shutdown()
        trap.server_close()
        trap_thread.join()
        server.shutdown()
        server.server_close()
        thread.join()


if __name__ == '__main__':
    main()
