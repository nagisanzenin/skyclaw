#!/usr/bin/env python3
"""Actual CLI proxy/model replacement followed by TemDOS delegation, local HTTP only."""
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
        length = int(self.headers.get('Content-Length', 0))
        if not 0 < length <= 4 * 1024 * 1024:
            self.send_error(413)
            return
        request = json.loads(self.rfile.read(length))
        self.server.requests.append(request)
        self.server.authorizations.append(self.headers.get('Authorization'))
        messages = request.get('messages', [])
        core = any(m.get('role') == 'system' and str(m.get('content', '')).startswith('CORE_BINDING_SENTINEL') for m in messages)
        has_tool = any(m.get('role') == 'tool' for m in messages)
        user = '\n'.join(str(m.get('content', '')) for m in messages if m.get('role') == 'user')
        if core:
            self.server.core_requests.append(request)
            message = {'role': 'assistant', 'content': f'CORE_ENDPOINT_{self.server.label}'}
        elif self.server.label == 'B' and 'INVOKE_AFTER_SWITCH' in user and not has_tool:
            message = {'role': 'assistant', 'content': None, 'tool_calls': [{
                'id': 'core-after-switch', 'type': 'function', 'function': {
                    'name': 'invoke_core', 'arguments': json.dumps({'core': 'binding', 'task': 'Inspect current connection'})}}]}
        elif has_tool:
            self.server.tool_returns.extend(m for m in messages if m.get('role') == 'tool')
            message = {'role': 'assistant', 'content': 'BINDING_PARENT_RETURNED'}
        else:
            message = {'role': 'assistant', 'content': 'BINDING_READY' if self.server.label == 'A' else 'SKIP'}
        body = json.dumps({'id': 'local-fixture', 'choices': [{'index': 0, 'message': message,
                           'finish_reason': 'tool_calls' if message.get('tool_calls') else 'stop'}],
                           'usage': {'prompt_tokens': 100, 'completion_tokens': 20, 'total_tokens': 120}}).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def run(binary, old, current):
    with tempfile.TemporaryDirectory(prefix='temm1e-runtime-binding-') as directory:
        root = Path(directory)
        profile = root / 'profile'
        (profile / 'cores').mkdir(parents=True, mode=0o700)
        (profile / 'cores' / 'binding.md').write_text('''---
name: binding
description: Local connection fixture
version: 1.0.0
---
CORE_BINDING_SENTINEL <task>
''')
        (profile / 'config.toml').write_text(f'''[provider]
name = "openai"
model = "binding-old"
api_key = "local-key-A"
base_url = "http://127.0.0.1:{old.server_port}/v1"
[agent]
v2_optimizations = false
self_audit_enabled = false
max_task_duration_secs = 20
[memory.engram]
enabled = false
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
        env = {k: v for k, v in os.environ.items() if not k.endswith(('_API_KEY', '_TOKEN')) and not k.startswith('TEMM1E_')}
        env['TEMM1E_DATA_DIR'] = str(profile)
        input_text = f'Hello\nproxy openai http://127.0.0.1:{current.server_port}/v1 local-key-B model:binding-current\nINVOKE_AFTER_SWITCH\n/quit\n'
        result = subprocess.run([str(binary), 'chat'], input=input_text, text=True, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, cwd=root, env=env, timeout=45)
        assert result.returncode == 0, result.stdout[-3000:]
        assert 'BINDING_READY' in result.stdout, result.stdout[-3000:]
        assert 'BINDING_PARENT_RETURNED' in result.stdout, result.stdout[-3000:]
        assert not old.core_requests, 'delegation used obsolete endpoint'
        assert len(old.requests) == 1, f'obsolete endpoint received extra calls: {len(old.requests)}'
        assert len(current.core_requests) == 1, f'expected one actual core request, got {len(current.core_requests)}'
        assert all(r.get('model') == 'binding-current' for r in current.requests)
        assert old.authorizations == ['Bearer local-key-A']
        assert all(a == 'Bearer local-key-B' for a in current.authorizations)
        assert any('CORE_ENDPOINT_B' in str(m.get('content')) for m in current.tool_returns)
        assert not any('CORE_ENDPOINT_A' in str(m.get('content')) for m in current.tool_returns)
        return {'passed': True, 'old_endpoint_requests': len(old.requests), 'current_endpoint_requests': len(current.requests),
                'current_core_requests': len(current.core_requests), 'credentials_isolated': True}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('binary', type=Path)
    args = parser.parse_args()
    servers = []
    try:
        for label in ['A', 'B']:
            server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
            server.label = label
            server.requests, server.authorizations, server.core_requests, server.tool_returns = [], [], [], []
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            servers.append((server, thread))
        print(json.dumps(run(args.binary.resolve(), servers[0][0], servers[1][0]), indent=2))
    finally:
        for server, thread in servers:
            server.shutdown()
            server.server_close()
            thread.join()


if __name__ == '__main__':
    main()
