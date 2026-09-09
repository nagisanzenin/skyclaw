#!/usr/bin/env python3
"""Actual CLI factory: observer trajectory, disabled calls and owner-budget gate."""
import argparse
import http.server
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        size = int(self.headers.get('Content-Length', '0'))
        if not 0 < size <= 4 * 1024 * 1024:
            self.send_error(413)
            return
        request = json.loads(self.rfile.read(size))
        assert self.headers.get('Authorization') == 'Bearer local-consciousness-fixture'
        assert request['model'] == 'consciousness-fixture'
        systems = '\n'.join(str(m.get('content', '')) for m in request['messages'] if m['role'] == 'system')
        kind = 'pre' if systems.startswith('You are the consciousness layer') and 'You observe' in systems else 'post' if systems.startswith('You are the consciousness layer') else 'foreground'
        if kind in ("pre", "post"):
            assert request.get("max_tokens") == min(1024, self.server.output_limit), request.get("max_tokens")
        self.server.requests.append((kind, request))
        content = {'pre': 'Keep the requested function signature.', 'post': 'POST_INSIGHT_SENTINEL keep the signature.', 'foreground': 'CONSCIOUS_FOREGROUND_RETURNED'}[kind]
        body = json.dumps({'id': 'fixture', 'choices': [{'index': 0, 'message': {'role': 'assistant', 'content': content}, 'finish_reason': 'stop'}], 'usage': {'prompt_tokens': 100, 'completion_tokens': 20, 'total_tokens': 120}}).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def run(binary, enabled, limited, output_limit=4096, new_conversation=False):
    with tempfile.TemporaryDirectory(prefix='temm1e-consciousness-') as temporary:
        root = Path(temporary)
        profile = root / 'profile'
        profile.mkdir(mode=0o700)
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
        server.requests = []
        server.output_limit = output_limit
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            (profile / 'config.toml').write_text(f'''[provider]
name = "openai"
model = "consciousness-fixture"
api_key = "local-consciousness-fixture"
base_url = "http://127.0.0.1:{server.server_port}/v1"
[agent]
v2_optimizations = false
self_audit_enabled = false
max_spend_usd = {0.0001 if limited else 0.0}
[memory.engram]
enabled = false
curator = "off"
[witness]
enabled = false
[perpetuum]
enabled = false
[hive]
enabled = false
[social]
enabled = false
[consciousness]
enabled = {str(enabled).lower()}
''')
            (profile / 'custom_models.toml').write_text(f'''[[models]]
provider = "openai"
name = "consciousness-fixture"
context_window = 32768
max_output_tokens = {output_limit}
input_price_per_1m = 1.0
output_price_per_1m = 1.0
pricing_verified = true
''')
            env = {k: v for k, v in os.environ.items() if not k.endswith(('_API_KEY', '_TOKEN')) and not k.startswith('TEMM1E_')}
            env['TEMM1E_DATA_DIR'] = str(profile)
            objective = 'In the workspace, write `demo.rs` with pub fn greet(name: &str) -> String.'
            commands = ([objective, '/session-new', objective] if new_conversation else [objective] * (1 if limited else 2)) + ['/quit', '']
            result = subprocess.run([str(binary), 'chat'], input='\n'.join(commands), text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, cwd=root, env=env, timeout=45)
            assert result.returncode == 0, result.stdout[-4000:]
            kinds = [kind for kind, _ in server.requests]
            expected = ['pre'] if limited else ['pre', 'foreground', 'post'] * 2 if enabled else ['foreground'] * 2
            assert kinds == expected, (kinds, result.stdout[-4000:])
            if limited:
                assert 'Budget exceeded' in result.stdout, result.stdout[-4000:]
                assert 'CONSCIOUS_FOREGROUND_RETURNED' not in result.stdout
            elif enabled:
                second_pre = json.dumps(server.requests[3][1])
                assert ('POST_INSIGHT_SENTINEL' in second_pre) == (not new_conversation), second_pre
                assert ('Consciousness-T1' in second_pre) == (not new_conversation), second_pre
            print(f'PASS enabled={enabled} limited={limited} output_limit={output_limit} new_conversation={new_conversation} actual HTTP calls={kinds}')
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('binary', type=Path)
    args = parser.parse_args()
    for enabled, limited in [(True, False), (False, False), (True, True)]:
        run(args.binary.resolve(), enabled, limited)
    run(args.binary.resolve(), True, False, output_limit=256)

    run(args.binary.resolve(), True, False, new_conversation=True)
