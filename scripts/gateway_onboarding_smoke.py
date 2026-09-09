#!/usr/bin/env python3
"""Real no-key gateway lifecycle, with a private disposable profile and no model calls."""
import argparse
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    env = {key: value for key, value in os.environ.items()
           if not any(part in key.upper() for part in ('TOKEN', 'API_KEY', 'SECRET', 'TEMM1E_'))}
    env['RUST_LOG'] = 'warn'
    with tempfile.TemporaryDirectory(prefix='temm1e-onboarding-') as directory:
        root = Path(directory)
        env['TEMM1E_DATA_DIR'] = str(root / 'data')
        with socket.socket() as reservation:
            reservation.bind(('127.0.0.1', 0))
            port = reservation.getsockname()[1]
        config = root / 'config.toml'
        config.write_text(f'[gateway]\nhost = "127.0.0.1"\nport = {port}\n[heartbeat]\nenabled = false\n[perpetuum]\nenabled = false\n')
        config.chmod(0o600)
        base = f'http://127.0.0.1:{port}'
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

        def fetch(path):
            try:
                with opener.open(base + path, timeout=0.5) as response:
                    return response.status, json.load(response)
            except urllib.error.HTTPError as error:
                return error.code, json.load(error)

        for attempt in range(2):
            with (root / f'server-{attempt}.log').open('w+') as log:
                process = subprocess.Popen([str(binary), '--config', str(config), 'start', '--host', '127.0.0.1'],
                                           cwd=root, env=env, stdin=subprocess.DEVNULL,
                                           stdout=log, stderr=subprocess.STDOUT)
                try:
                    deadline = time.monotonic() + 20
                    while time.monotonic() < deadline:
                        assert process.poll() is None, f'early exit: {process.returncode}'
                        try:
                            if fetch('/health')[0] == 200:
                                break
                        except (OSError, urllib.error.URLError):
                            pass
                        time.sleep(0.1)
                    else:
                        raise AssertionError('onboarding did not start liveness endpoint')
                    assert fetch('/ready') == (503, {'status': 'onboarding'})
                    code, status = fetch('/status')
                    assert code == 200 and status['status'] == 'onboarding'
                    assert not status['provider'] and not status['tools']
                    assert (root / 'data' / 'memory.db').is_file()
                    process.send_signal(signal.SIGTERM)
                    assert process.wait(timeout=15) == 0
                    log.seek(0)
                    logs = log.read()
                    assert 'Failed to parse built-in blueprint' not in logs
                    assert 'Gateway error' not in logs
                except BaseException:
                    log.seek(0)
                    print(log.read())
                    raise
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.wait(timeout=5)
        print(json.dumps({'onboarding_starts': 2, 'liveness': 200, 'readiness': 503,
                          'sigterm_exit': 0, 'profile_reopened': True}))


if __name__ == '__main__':
    main()
