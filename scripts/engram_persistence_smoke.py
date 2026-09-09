#!/usr/bin/env python3
"""Real CLI: precise memory deletion and truthful unsupported-backend behavior."""
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

CASES = ['ambiguous', 'exact', 'foreign', 'unsupported', 'remember']


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers.get('Content-Length', 0))))
        self.server.requests.append(request)
        index = len(self.server.requests) - 1
        if index == 0:
            message = {'role': 'assistant', 'content': None, 'tool_calls': [
                {'id': 'memory-operation', 'type': 'function', 'function': {'name': 'engram', 'arguments': json.dumps(self.server.action)}}]}
            stop = 'tool_calls'
        else:
            message, stop = {'role': 'assistant', 'content': 'PERSISTENCE_FIXTURE_RETURNED'}, 'stop'
        body = json.dumps({'id': f'fixture-{index}', 'choices': [{'index': 0, 'message': message, 'finish_reason': stop}],
                           'usage': {'prompt_tokens': 100, 'completion_tokens': 20, 'total_tokens': 120}}).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def run(binary, server, case):
    with tempfile.TemporaryDirectory(prefix='temm1e-engram-persist-') as directory:
        root = Path(directory)
        profile = root / 'profile'
        profile.mkdir(mode=0o700)
        if case != 'unsupported':
            seed(profile)
            with sqlite3.connect(profile / 'memory.db') as db:
                db.execute("UPDATE engram_facts SET id='own-a', scope='user:local', content='MATCHMARKER alpha', summary='MATCHMARKER alpha', essence='alpha'")
                db.execute("INSERT INTO engram_facts SELECT 'own-b','MATCHMARKER beta','MATCHMARKER beta','beta',fact_type,scope,pinned_by,NULL,importance,created_at,last_accessed,tags,links FROM engram_facts WHERE id='own-a'")
                db.execute("INSERT INTO engram_facts SELECT 'foreign-a','FOREIGN_PRIVATE_5279','FOREIGN_PRIVATE_5279','foreign',fact_type,'user:other',pinned_by,NULL,importance,created_at,last_accessed,tags,links FROM engram_facts WHERE id='own-a'")
        actions = {
            'ambiguous': {'action': 'forget', 'query': 'MATCHMARKER'},
            'exact': {'action': 'forget', 'id': 'own-a'},
            'foreign': {'action': 'forget', 'id': 'foreign-a'},
            'unsupported': {'action': 'remember', 'scope': 'user', 'content': 'NEW_CAPTURE'},
            'remember': {'action': 'remember', 'scope': 'user', 'content': 'NEW_CAPTURE'},
        }
        server.action, server.requests = actions[case], []
        memory_config = f'backend = "markdown"\npath = "{(profile / "markdown").as_posix()}"' if case == 'unsupported' else 'backend = "sqlite"'
        (profile / 'config.toml').write_text(f'''[provider]
name = "openai"
model = "persistence-fixture"
api_key = "local-fixture-only"
base_url = "http://127.0.0.1:{server.server_port}/v1"
[agent]
v2_optimizations = false
self_audit_enabled = false
[memory]
{memory_config}
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
        result = subprocess.run([str(binary), 'chat'], input='Please perform the requested memory operation.\n/quit\n',
                                text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, cwd=root, env=env, timeout=40)
        assert result.returncode == 0, result.stdout[-2500:]
        assert 'PERSISTENCE_FIXTURE_RETURNED' in result.stdout, result.stdout[-2500:]
        foreground = [r for r in server.requests if 'You are a technical writer.' not in str(r.get('messages', [{}])[0].get('content', ''))]
        auxiliary = len(server.requests) - len(foreground)
        assert len(foreground) == 2 and auxiliary <= 1, (len(foreground), auxiliary)
        tool_text = '\n'.join(str(m.get('content', '')) for m in foreground[-1].get('messages', []) if m.get('role') == 'tool')
        if case == 'unsupported':
            assert 'not support' in tool_text.lower() or 'unsupported' in tool_text.lower(), tool_text
            assert 'Remembered' not in tool_text, tool_text
            permanent = profile / 'markdown' / 'MEMORY.md'
            assert not permanent.exists() or 'NEW_CAPTURE' not in permanent.read_text(errors='replace')
        else:
            with sqlite3.connect(profile / 'memory.db') as db:
                rows = db.execute('SELECT id, content, scope FROM engram_facts').fetchall()
            ids = {r[0] for r in rows}
            if case == 'ambiguous':
                assert ids == {'own-a', 'own-b', 'foreign-a'}, 'ambiguous request deleted a candidate'
                assert 'multiple' in tool_text.lower() or 'ambiguous' in tool_text.lower(), tool_text
            elif case == 'exact':
                assert ids == {'own-b', 'foreign-a'}, rows
                assert 'Forgotten' in tool_text, tool_text
            elif case == 'foreign':
                assert ids == {'own-a', 'own-b', 'foreign-a'}, rows
                assert 'FOREIGN_PRIVATE_5279' not in tool_text, tool_text
            else:
                assert any(content == 'NEW_CAPTURE' and scope == 'user:local' for _, content, scope in rows), rows
        return {'case': case, 'passed': True, 'provider_requests': len(server.requests), 'foreground_requests': 2, 'blueprint_requests': auxiliary}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('binary', type=Path)
    parser.add_argument('--case', choices=CASES)
    args = parser.parse_args()
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        print(json.dumps([run(args.binary.resolve(), server, case) for case in ([args.case] if args.case else CASES)], indent=2))
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


if __name__ == '__main__':
    main()
