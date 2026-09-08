#!/usr/bin/env python3
"""Real CLI tool effect, unverified goal persistence, restart and workspace isolation."""
import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import threading

OBJECTIVE = 'Write the requested artifact and prove its behavior. Keep this original objective unchanged.'


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
        assert self.headers.get('Authorization') == 'Bearer local-goal-fixture'
        if len(self.server.requests) == 1:
            message = {'role': 'assistant', 'content': None, 'tool_calls': [{
                'id': 'write-fixture-artifact', 'type': 'function', 'function': {'name': 'file_write',
                'arguments': json.dumps({'path': 'goal-artifact.txt', 'content': 'ACTUAL_ARTIFACT_CREATED'})}}]}
        else:
            message = {'role': 'assistant', 'content': 'DONE, all requested tests passed. GOAL_REPLY_RETURNED'}
        body = json.dumps({'id': 'goal-fixture', 'choices': [{'index': 0, 'message': message,
                           'finish_reason': 'tool_calls' if message.get('tool_calls') else 'stop'}],
                           'usage': {'prompt_tokens': 100, 'completion_tokens': 20, 'total_tokens': 120}}).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def run(binary, server):
    with tempfile.TemporaryDirectory(prefix='temm1e-goal-ledger-') as directory:
        root = Path(directory)
        profile = root / 'profile'
        profile.mkdir(mode=0o700)
        (profile / 'config.toml').write_text(f'''[provider]
name = "openai"
model = "goal-fixture"
api_key = "local-goal-fixture"
base_url = "http://127.0.0.1:{server.server_port}/v1"
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

        def chat(workspace, text, data_dir=profile):
            run_env = dict(env, TEMM1E_DATA_DIR=str(data_dir))
            result = subprocess.run([str(binary), 'chat'], input=text, text=True, stdout=subprocess.PIPE,
                                    stderr=subprocess.STDOUT, cwd=workspace, env=run_env, timeout=40)
            assert result.returncode == 0, result.stdout[-4000:]
            return result.stdout

        first = chat(root, OBJECTIVE + '\n/goal-status\n/quit\n')
        assert 'GOAL_REPLY_RETURNED' in first, first[-4000:]
        assert 'awaiting_evidence' in first, first[-4000:]
        assert (profile / 'workspace' / 'goal-artifact.txt').read_text() == 'ACTUAL_ARTIFACT_CREATED'
        with sqlite3.connect(profile / 'executions.db') as db:
            goals = db.execute('SELECT id,state,revision,objective FROM goal_records').fetchall()
            assert len(goals) == 1, goals
            goal_id, state, revision, objective = goals[0]
            assert (state, revision, objective) == ('awaiting_evidence', 1, OBJECTIVE)
            snapshots = db.execute('SELECT hash,document FROM goal_evidence WHERE goal_id=?', (goal_id,)).fetchall()
            assert len(snapshots) == 1
            digest, document = snapshots[0]
            assert hashlib.sha256(document.encode()).hexdigest() == digest
            evidence = json.loads(document)
            assert evidence['tool'] == 'file_write' and evidence['is_error'] is False
            assert db.execute('SELECT state FROM executions WHERE id=?', (goal_id,)).fetchone()[0] == 'reply_returned'
            assert db.execute('SELECT kind FROM goal_events WHERE goal_id=? ORDER BY sequence', (goal_id,)).fetchall() == [
                ('admitted',), ('tool_intent',), ('tool_result_recorded',), ('reply_returned',)]
        before = len(server.requests)
        restarted = chat(root, '/goal-status\n/quit\n')
        assert goal_id in restarted and 'awaiting_evidence' in restarted
        # CLI's documented workspace is profile/workspace, not process cwd.
        # Copy only this synthetic journal to another profile: its embedded old
        # workspace scopes must not authorize access from the new workspace.
        foreign = root / 'foreign-profile'
        foreign.mkdir(mode=0o700)
        (foreign / 'config.toml').write_text((profile / 'config.toml').read_text())
        with sqlite3.connect(profile / 'executions.db') as source, sqlite3.connect(foreign / 'executions.db') as destination:
            source.backup(destination)
        other = chat(root, '/goal-status\n/quit\n', foreign)
        assert 'No typed goal records' in other and goal_id not in other and OBJECTIVE not in other
        assert len(server.requests) == before, 'read-only inspection/restart called provider'
        return {'passed': True, 'provider_requests': before, 'restart_inspection_requests': 0,
                'actual_artifact': True, 'state_after_done_prose': state, 'verified_evidence_hashes': len(snapshots),
                'foreign_workspace_records': 0}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('binary', type=Path)
    args = parser.parse_args()
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    server.requests = []
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        print(json.dumps(run(args.binary.resolve(), server), indent=2))
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


if __name__ == '__main__':
    main()
