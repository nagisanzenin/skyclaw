#!/usr/bin/env python3
"""Exercise the real TUI on a Unix PTY with a local HTTP provider fixture.
No actual provider key or account is used. Preserve user terminal settings.
"""
import argparse
import fcntl
import http.server
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import tempfile
import termios
import threading
import time


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        size = int(self.headers.get('Content-Length', '0'))
        if size > 4 * 1024 * 1024:
            self.send_error(413)
            return
        request = json.loads(self.rfile.read(size))
        self.server.requests.append(request)
        self.server.authorizations.append(self.headers.get('Authorization'))
        reply = 'PTY_REPLY_43' if request.get('model') == 'pty-fixture-next' else 'PTY_REPLY_42'
        usage = {'prompt_tokens': 100, 'completion_tokens': 5, 'total_tokens': 105}
        if request.get('stream'):
            events = [
                {'choices': [{'index': 0, 'delta': {'content': reply}, 'finish_reason': None}]},
                {'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'stop'}]},
                {'choices': [], 'usage': usage},
            ]
            body = ''.join('data: ' + json.dumps(event) + '\n\n' for event in events).encode() + b'data: [DONE]\n\n'
            kind = 'text/event-stream'
        else:
            body = json.dumps({'id': 'pty-fixture', 'choices': [{'index': 0, 'message': {'role': 'assistant', 'content': reply}, 'finish_reason': 'stop'}], 'usage': usage}).encode()
            kind = 'application/json'
        self.send_response(200)
        self.send_header('Content-Type', kind)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def run(binary, root, server, trap, onboarding=False, restore_only=False):
    profile = root / 'profile'
    profile.mkdir(mode=0o700, exist_ok=restore_only)
    config = profile / 'config.toml'
    config.write_text(f'''[provider]
name = "openai"
model = "pty-fixture"
api_key = "local-fixture-only"
base_url = "http://127.0.0.1:{server.server_port}/v1"
[memory.engram]
curator = "off"
[perpetuum]
enabled = false
[hive]
enabled = false
''')
    saved_path = profile / 'credentials.toml'
    saved_text = f'''active = "anthropic"
[[providers]]
name = "anthropic"
model = "unrelated-saved-model"
keys = ["synthetic-unrelated-key"]
base_url = "http://127.0.0.1:{trap.server_port}/v1"
'''
    if onboarding:
        config.write_text('')
    else:
        saved_path.write_text(saved_text)
        saved_path.chmod(0o600)
    requests_before = len(server.requests)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 100, 0, 0))
    # Intentionally non-default: stty sane must not clobber this on exit.
    original = termios.tcgetattr(slave)
    original[3] &= ~termios.ECHOE
    termios.tcsetattr(slave, termios.TCSANOW, original)
    env = {key: value for key, value in os.environ.items()
           if not key.endswith(('_API_KEY', '_TOKEN')) and not key.startswith('TEMM1E_')}
    env.update(TEMM1E_DATA_DIR=str(profile), TERM='xterm-256color')
    process = subprocess.Popen([str(binary), 'tui'], cwd=root, env=env,
                               stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
    output = bytearray()
    def until(marker, seconds=20, from_start=False):
        deadline = time.monotonic() + seconds
        start = 0 if from_start else len(output)
        while time.monotonic() < deadline:
            if marker in output[start:]:
                return
            if process.poll() is not None:
                raise AssertionError(f'TUI exited early: {process.returncode}')
            if select.select([master], [], [], 0.1)[0]:
                output.extend(os.read(master, 65536))
                if len(output) > 4 * 1024 * 1024:
                    raise AssertionError('terminal output exceeded smoke limit')
        raise AssertionError(f'timed out waiting for {marker!r}; terminal tail={bytes(output[-1500:])!r}')
    try:
        input_latency = None
        if onboarding:
            until(b'Setup')
        elif restore_only:
            until(b'PTY_REPLY_42', from_start=True)
            assert b'PTY_INPUT_42' in output, 'saved user transcript was not restored'
            os.write(master, b'/history invalid\r')
            until(b'Usage:')
            os.write(master, b'/history-more\r')
            until(b'page.')  # terminal cursor positioning separates words in the rendered sentence
        else:
            until(b'pty-fixture')
            start = time.monotonic()
            os.write(master, 'Chào Tem — PTY_INPUT_42'.encode())
            until(b'PTY_INPUT_42')
            input_latency = time.monotonic() - start
            os.write(master, b'\r')
            until(b'PTY_REPLY_42')
            time.sleep(0.3)  # allow final completion event to reach the TUI
            os.write(master, b'/model pty-fixture-next\r')
            until(b'Switched')
            os.write(master, b'PTY_SWITCH_43\r')
            until(b'PTY_REPLY_43')
        # Force a resize while the event stream and renderer are active.
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 18, 60, 0, 0))
        os.kill(process.pid, signal.SIGWINCH)
        time.sleep(0.2)
        os.write(master, b'\x04')  # Ctrl+D on empty input
        deadline = time.monotonic() + 12
        while process.poll() is None and time.monotonic() < deadline:
            if select.select([master], [], [], 0.1)[0]:
                output.extend(os.read(master, 65536))
        assert process.poll() == 0, 'TUI did not exit cleanly within 12 seconds'
        assert termios.tcgetattr(slave) == original, 'TUI changed caller terminal attributes'
        assert b'\x1b[?1049l' in output, 'alternate screen was not left'
        assert b'\x1b[?25h' in output, 'cursor was not restored'
        if onboarding or restore_only:
            assert len(server.requests) == requests_before, 'read-only startup unexpectedly called provider'
        else:
            assert len(server.requests) > requests_before, 'no actual provider HTTP request occurred'
            assert any('PTY_INPUT_42' in json.dumps(request, ensure_ascii=False) for request in server.requests)
        assert not trap.requests, 'unrelated saved endpoint received a provider request'
        assert all(auth == 'Bearer local-fixture-only' for auth in server.authorizations), 'selected endpoint received a wrong credential'
        if not onboarding:
            assert saved_path.read_text() == saved_text, 'config-owned switch overwrote saved credentials'
        if not onboarding and not restore_only:
            assert any(request.get('model') == 'pty-fixture-next' for request in server.requests[requests_before:]), 'replacement runtime did not use the selected custom model'
        return {'connection_isolation': True, 'model_switch': not onboarding and not restore_only, 'passed': True, 'input_echo_seconds': round(input_latency, 4) if input_latency is not None else None,
                'provider_requests': len(server.requests) - requests_before, 'onboarding': onboarding, 'restore_only': restore_only, 'terminal_bytes': len(output),
                'terminal_attributes_restored': True, 'resize_and_exit': True}
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
        os.close(master)
        os.close(slave)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('binary', type=Path)
    args = parser.parse_args()
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    server.requests = []
    server.authorizations = []
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    trap = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    trap.requests, trap.authorizations = [], []
    trap_thread = threading.Thread(target=trap.serve_forever, daemon=True)
    trap_thread.start()
    try:
        results = []
        for onboarding in [False, True]:
            with tempfile.TemporaryDirectory(prefix='temm1e-pty-') as directory:
                results.append(run(args.binary.resolve(), Path(directory), server, trap, onboarding))
                if not onboarding:
                    results.append(run(args.binary.resolve(), Path(directory), server, trap, restore_only=True))
        print(json.dumps(results, indent=2))
    finally:
        trap.shutdown()
        trap.server_close()
        trap_thread.join()
        server.shutdown()
        server.server_close()
        thread.join()


if __name__ == '__main__':
    main()
