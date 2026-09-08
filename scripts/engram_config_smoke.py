#!/usr/bin/env python3
"""Real CLI rejects invalid numeric memory policy before invoking a provider."""
import argparse
import http.server
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
from tui_pty_smoke import Provider
from memory_policy_fixture import seed


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('binary', type=Path)
    args = parser.parse_args()
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    server.requests, server.authorizations = [], []
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    results = []
    try:
        for policy, field in [
            ('p_max_frac = nan', 'p_max_frac'),
            ('eta = inf', 'eta'),
            ('tau_days = 0.0', 'tau_days'),
            ('theta_down = 4.0\ntheta_up = 2.0', 'theta_down'),
            ('p_max_frac = 0.0\neta = 0.0\ntau_days = 60.0', None),
        ]:
            with tempfile.TemporaryDirectory(prefix='temm1e-engram-config-') as directory:
                root = Path(directory)
                profile = root / 'profile'
                profile.mkdir(mode=0o700)
                seed(profile)
                original_memory = (profile / 'memory.db').read_bytes()
                (profile / 'config.toml').write_text(f'''[provider]
name = "openai"
model = "pty-fixture"
api_key = "local-fixture-only"
base_url = "http://127.0.0.1:{server.server_port}/v1"
[agent]
v2_optimizations = false
[memory.engram]
enabled = false
{policy}
[perpetuum]
enabled = false
[hive]
enabled = false
[social]
enabled = false
[consciousness]
enabled = false
''')
                env = {key: value for key, value in os.environ.items()
                       if not key.endswith(('_API_KEY', '_TOKEN')) and not key.startswith('TEMM1E_')}
                env['TEMM1E_DATA_DIR'] = str(profile)
                start = len(server.requests)
                result = subprocess.run([str(args.binary.resolve()), 'chat'], input='Hi\n/quit\n',
                                        text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                        cwd=root, env=env, timeout=30)
                requests = len(server.requests) - start
                if field:
                    assert result.returncode != 0, result.stdout[-2000:]
                    assert f'memory.engram.{field}' in result.stdout, result.stdout[-2000:]
                    assert requests == 0, 'invalid configuration invoked a provider'
                    assert (profile / 'memory.db').read_bytes() == original_memory
                else:
                    assert result.returncode == 0, result.stdout[-2000:]
                    assert 'PTY_REPLY_42' in result.stdout, result.stdout[-2000:]
                    assert requests == 1, requests
                results.append({'invalid_field': field, 'provider_requests': requests, 'passed': True})
        print(json.dumps(results, indent=2))
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


if __name__ == '__main__':
    main()
