"""Actual CLI restart/import fixture with a localhost provider; no live credentials."""
import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import threading

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
root = args.output.resolve()
root.mkdir(parents=True, exist_ok=False)
binary = args.binary.resolve()
requests = []


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_POST(self):
        length = int(self.headers.get('Content-Length', '0'))
        if length > 2 * 1024 * 1024:
            self.send_error(413)
            return
        body = json.loads(self.rfile.read(length))
        requests.append(body)
        # Fixture content is constant: assertions below inspect actual requests
        # and native storage, not whether this fake model can recall anything.
        response = json.dumps({
            'id': 'fixture', 'object': 'chat.completion', 'model': 'fixture-model',
            'choices': [{'index': 0, 'message': {'role': 'assistant', 'content': 'Fixture reply.'}, 'finish_reason': 'stop'}],
            'usage': {'prompt_tokens': 20, 'completion_tokens': 4, 'total_tokens': 24},
        }).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(response)))
        self.end_headers()
        self.wfile.write(response)


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
threading.Thread(target=server.serve_forever, daemon=True).start()
profile = root / 'data'
env = {k: v for k, v in os.environ.items()
       if not any(x in k.upper() for x in ('TOKEN', 'API_KEY', 'SECRET', 'TEMM1E_'))}
env.update(TEMM1E_DATA_DIR=str(profile), RUST_LOG='error')
config = root / 'config.toml'
config.write_text(f'''[provider]
name = "openai-compatible"
api_key = "offline-fixture"
model = "fixture-model"
base_url = "http://127.0.0.1:{server.server_port}/v1"
[agent]
v2_optimizations = false
streaming_enabled = false
self_audit_enabled = false
max_context_tokens = 30000
[heartbeat]
enabled = false
[perpetuum]
enabled = false
[consciousness]
enabled = false
[witness]
enabled = false
[hive]
enabled = false
''')


def run(label, text):
    result = subprocess.run([str(binary), '--config', str(config), 'chat'],
                            input=text + '\n/quit\n', capture_output=True, text=True,
                            cwd=root, env=env, timeout=45)
    (root / f'{label}.stdout.log').write_text(result.stdout)
    (root / f'{label}.stderr.log').write_text(result.stderr)
    assert result.returncode == 0, (label, result.returncode)
    return result.stdout


try:
    run('bootstrap', '')
    legacy = [{'role': 'user', 'content': 'Original constraint: maple17, never publish.'}]
    legacy += [{'role': 'assistant' if i % 2 else 'user', 'content': f'Old turn {i}.'}
               for i in range(239)]
    raw = json.dumps(legacy, separators=(',', ':'))
    with sqlite3.connect(profile / 'memory.db') as db:
        db.execute('INSERT INTO memory_entries(id,content,metadata,timestamp,session_id,entry_type) VALUES(?,?,?,?,?,?)',
                   ('chat_history:cli', raw, '{}', '2026-09-08T00:00:00Z', 'cli', 'conversation'))
    before = len(requests)
    preview = run('preview', '/history-import')
    assert '240 messages' in preview
    confirm = '/history-import confirm ' + hashlib.sha256(raw.encode()).hexdigest()
    assert 'Imported 240 messages' in run('import', confirm)
    assert 'already imported' in run('repeat-import', confirm)
    assert len(requests) == before, 'management command reached provider'
    run('correction', 'Latest correction: allowed color amber; retain the original constraint.')
    run('restart', 'Continue under all earlier constraints.')
    foreground = [r for r in requests if r.get('messages', [{}])[-1].get('content') == 'Continue under all earlier constraints.']
    assert len(foreground) == 1
    captured = json.dumps(foreground[0])
    assert 'maple17' in captured and 'allowed color amber' in captured
    with sqlite3.connect(profile / 'executions.db') as db:
        epoch, checkpoint, busy = db.execute('SELECT epoch,checkpoint,busy_owner FROM conversation_heads').fetchone()
        count = len(json.loads(checkpoint)['message_ids'])
        assert count == 244 and busy is None, (count, busy)
    assert 'Started conversation' in run('new', '/session-new')
    run('new-turn', 'A new conversation starts here.')
    new_foreground = [r for r in requests if r.get('messages', [{}])[-1].get('content') == 'A new conversation starts here.']
    assert len(new_foreground) == 1
    captured_new = json.dumps(new_foreground[0])
    assert 'maple17' not in captured_new and 'allowed color amber' not in captured_new
    with sqlite3.connect(profile / 'memory.db') as db:
        assert db.execute("SELECT content FROM memory_entries WHERE id='chat_history:cli'").fetchone()[0] == raw
    report = {'native_messages_before_reset': count, 'legacy_preserved': True,
              'commands_bypass_provider': True, 'restart_context_preserved': True,
              'new_epoch_excludes_old_context': True, 'provider_requests': len(requests)}
    (root / 'requests.json').write_text(json.dumps(requests, indent=2))
    (root / 'evidence.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))
finally:
    server.shutdown()
    server.server_close()
