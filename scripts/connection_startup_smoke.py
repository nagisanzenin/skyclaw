#!/usr/bin/env python3
"""Actual server factory acceptance for config-only key pools; no account use."""
import argparse
import http.server
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import threading
import time
import urllib.request
from tui_pty_smoke import Provider


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('binary', type=Path)
    args = parser.parse_args()
    fixture = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    fixture.requests, fixture.authorizations = [], []
    thread = threading.Thread(target=fixture.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix='temm1e-server-connection-') as directory:
            root = Path(directory)
            profile = root / 'profile'
            profile.mkdir(mode=0o700)
            with socket.socket() as reservation:
                reservation.bind(('127.0.0.1', 0))
                port = reservation.getsockname()[1]
            (profile / 'config.toml').write_text(f'''[gateway]
host = "127.0.0.1"
port = {port}
[provider]
name = "fixture-connection"
model = "fixture-selected-model"
keys = ["synthetic-selected-key", "synthetic-selected-backup"]
base_url = "http://127.0.0.1:{fixture.server_port}/v1"
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
            saved = '''active = "anthropic"
[[providers]]
name = "anthropic"
model = "unrelated-saved-model"
keys = ["synthetic-unrelated-key"]
base_url = "not an HTTP endpoint"
'''
            (profile / 'credentials.toml').write_text(saved)
            (profile / 'credentials.toml').chmod(0o600)
            env = {key: value for key, value in os.environ.items()
                   if not key.endswith(('_API_KEY', '_TOKEN')) and not key.startswith('TEMM1E_')}
            env['TEMM1E_DATA_DIR'] = str(profile)
            with tempfile.TemporaryFile() as log:
                process = subprocess.Popen([str(args.binary.resolve()), 'start'], cwd=root, env=env,
                                           stdin=subprocess.DEVNULL, stdout=log, stderr=log)
                try:
                    deadline = time.monotonic() + 15
                    response = None
                    while time.monotonic() < deadline and process.poll() is None:
                        try:
                            with urllib.request.urlopen(f'http://127.0.0.1:{port}/dashboard/api/config', timeout=0.5) as body:
                                response = json.load(body)
                            break
                        except OSError:
                            time.sleep(0.05)
                    if response is None:
                        log.seek(0)
                        raise AssertionError(log.read(16000).decode(errors='replace'))
                    assert response['provider'] == 'fixture-connection', response
                    assert response['model'] == 'fixture-selected-model', response
                    assert (profile / 'credentials.toml').read_text() == saved
                    assert not fixture.requests, 'startup unexpectedly invoked the provider'
                    process.send_signal(signal.SIGTERM)
                    assert process.wait(timeout=10) == 0
                    print(json.dumps({'passed': True, 'config_key_pool_selected': True,
                                      'unrelated_saved_endpoint_ignored': True, 'saved_file_unchanged': True,
                                      'provider_requests': 0, 'sigterm_exit': 0}, indent=2))
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.wait()
    finally:
        fixture.shutdown()
        fixture.server_close()
        thread.join()


if __name__ == '__main__':
    main()
