#!/usr/bin/env python3
"""Actual CLI tool loop: remember/recall/forget target authenticated user facts."""
import argparse
import http.server
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import threading
from memory_policy_fixture import seed

OWN = 'POLICY_SEED_OWN_7421'
FOREIGN = 'POLICY_SEED_FOREIGN_9526'
NEW = 'NEW_PRIVATE_FACT_3128'


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        size = int(self.headers.get('Content-Length', 0))
        if size > 4 * 1024 * 1024:
            self.send_error(413)
            return
        request = json.loads(self.rfile.read(size))
        self.server.requests.append(request)
        self.server.authorizations.append(self.headers.get('Authorization'))
        index = len(self.server.requests) - 1
        actions = [
            {'action': 'recall', 'query': 'POLICY_SEED'},
            {'action': 'remember', 'scope': 'user', 'content': NEW, 'subject_key': 'fixture:new'},
            {'action': 'forget', 'query': 'POLICY_SEED'},
        ]
        if index < len(actions):
            message = {'role': 'assistant', 'content': None, 'tool_calls': [
                {'id': f'engram-call-{index}', 'type': 'function',
                 'function': {'name': 'engram', 'arguments': json.dumps(actions[index])}}]}
            stop = 'tool_calls'
        else:
            message, stop = {'role': 'assistant', 'content': 'ENGRAM_FIXTURE_RETURNED'}, 'stop'
        body = json.dumps({'id': f'fixture-{index}', 'choices': [{'index': 0, 'message': message, 'finish_reason': stop}],
                           'usage': {'prompt_tokens': 100, 'completion_tokens': 20, 'total_tokens': 120}}).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('binary', type=Path)
    args = parser.parse_args()
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    server.requests, server.authorizations = [], []
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix='temm1e-engram-identity-') as directory:
            root = Path(directory)
            profile = root / 'profile'
            profile.mkdir(mode=0o700)
            seed(profile)
            with sqlite3.connect(profile / 'memory.db') as db:
                db.execute("UPDATE engram_facts SET scope='user:local', content=?, summary=?, essence=?", (OWN, OWN, OWN))
                db.execute("INSERT INTO engram_facts SELECT 'foreign', ?, ?, ?, fact_type, 'user:other', pinned_by, NULL, importance, created_at, last_accessed, tags, links FROM engram_facts", (FOREIGN, FOREIGN, FOREIGN))
            (profile / 'config.toml').write_text(f'''[provider]
name = "openai"
model = "identity-fixture"
api_key = "local-fixture-only"
base_url = "http://127.0.0.1:{server.server_port}/v1"
[agent]
v2_optimizations = false
self_audit_enabled = false
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
            env = {key: value for key, value in os.environ.items()
                   if not key.endswith(('_API_KEY', '_TOKEN')) and not key.startswith('TEMM1E_')}
            env['TEMM1E_DATA_DIR'] = str(profile)
            result = subprocess.run([str(args.binary.resolve()), 'chat'], input='Please perform the local memory fixture.\n/quit\n',
                                    text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                    cwd=root, env=env, timeout=40)
            assert result.returncode == 0, result.stdout[-2500:]
            assert 'ENGRAM_FIXTURE_RETURNED' in result.stdout, result.stdout[-2500:]
            foreground = [r for r in server.requests if 'You are a technical writer.' not in str(r.get('messages', [{}])[0].get('content', ''))]
            auxiliary = len(server.requests) - len(foreground)
            assert len(foreground) == 4, len(foreground)
            assert auxiliary == 1, f'expected one optional blueprint request, got {auxiliary}'
            if OWN not in json.dumps(foreground[1]):
                print(json.dumps([m for r in foreground for m in r.get('messages', []) if m.get('role') == 'tool'], indent=2))
            assert OWN in json.dumps(foreground[1]), 'explicit recall failed to return the current user fact'
            assert all(FOREIGN not in json.dumps(r) for r in server.requests), 'another user fact reached the provider'
            assert all(a == 'Bearer local-fixture-only' for a in server.authorizations)
            with sqlite3.connect(profile / 'memory.db') as db:
                rows = db.execute('SELECT content, scope FROM engram_facts ORDER BY content').fetchall()
            assert rows == sorted([(FOREIGN, 'user:other'), (NEW, 'user:local')]), rows
            print(json.dumps({'passed': True, 'provider_requests': len(server.requests), 'foreground_requests': 4, 'blueprint_requests': auxiliary, 'current_user_recalled': True,
                              'current_user_fact_deleted': True, 'new_fact_user_scoped': True,
                              'foreign_fact_preserved_and_not_exposed': True}, indent=2))
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


if __name__ == '__main__':
    main()
