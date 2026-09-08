#!/usr/bin/env python3
"""Actual CLI Witness: original objectives, execution IDs and planner budget."""
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

OBJECTIVES = [f'In the workspace, write `demo.rs` with pub fn greet(name: &str) -> String. Preserve REQUIREMENT_{i}.' for i in range(2)]


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
        assert self.headers.get('Authorization') == 'Bearer local-witness-fixture'
        planner = any(m.get('role') == 'system' and str(m.get('content', '')).startswith('You are the Oath Planner') for m in request.get('messages', []))
        self.server.planners += int(planner)
        if planner:
            content = json.dumps({'goal': 'weaker model-authored goal', 'postconditions': [
                {'kind': 'file_exists', 'path': 'fixture.txt'},
                {'kind': 'grep_count_at_least', 'pattern': 'token', 'path_glob': '*.txt', 'n': 2},
                {'kind': 'grep_absent', 'pattern': 'TODO', 'path_glob': '*.txt'}]})
        else:
            with sqlite3.connect(self.server.profile / 'executions.db') as db:
                active = db.execute("SELECT COUNT(*) FROM goal_criteria c JOIN goal_records g ON g.id=c.goal_id WHERE g.state='running'").fetchone()[0]
                assert active == 1, 'criteria must be frozen before foreground model work'
            content = 'WITNESS_FOREGROUND_RETURNED'
        body = json.dumps({'id': 'local-witness', 'choices': [{'index': 0, 'message': {'role': 'assistant', 'content': content}, 'finish_reason': 'stop'}],
                           'usage': {'prompt_tokens': 100, 'completion_tokens': 20, 'total_tokens': 120}}).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def run(binary, server, limited):
    with tempfile.TemporaryDirectory(prefix='temm1e-witness-binding-') as directory:
        root = Path(directory)
        profile = root / 'profile'
        (profile / 'workspace').mkdir(parents=True, mode=0o700)
        (profile / 'workspace' / 'fixture.txt').write_text('token token')
        (profile / 'config.toml').write_text(f'''[provider]
name = "openai"
model = "witness-fixture"
api_key = "local-witness-fixture"
base_url = "http://127.0.0.1:{server.server_port}/v1"
[agent]
v2_optimizations = false
self_audit_enabled = false
max_spend_usd = {0.0001 if limited else 0.0}
[memory.engram]
enabled = false
curator = "off"
[witness]
enabled = true
auto_planner_oath = true
strictness = "observe"
[perpetuum]
enabled = false
[hive]
enabled = false
[social]
enabled = false
[consciousness]
enabled = false
''')
        (profile / 'custom_models.toml').write_text('''[[models]]
provider = "openai"
name = "witness-fixture"
context_window = 32768
max_output_tokens = 4096
input_price_per_1m = 1.0
output_price_per_1m = 1.0
pricing_verified = true
''')
        env = {k: v for k, v in os.environ.items() if not k.endswith(('_API_KEY', '_TOKEN')) and not k.startswith('TEMM1E_')}
        env['TEMM1E_DATA_DIR'] = str(profile)
        objectives = OBJECTIVES[:1] if limited else OBJECTIVES
        server.profile = profile
        before, planners_before = len(server.requests), server.planners
        result = subprocess.run([str(binary), 'chat'], input='\n'.join(objectives + ['/quit', '']), text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, cwd=root, env=env, timeout=45)
        assert result.returncode == 0, result.stdout[-3000:]
        requests, planners = len(server.requests) - before, server.planners - planners_before
        if limited:
            assert requests == 1 and planners == 1, (requests, planners, result.stdout[-3000:])
            assert 'WITNESS_FOREGROUND_RETURNED' not in result.stdout
            assert 'Budget exceeded' in result.stdout, result.stdout[-3000:]
        else:
            assert (requests, planners) == (4, 2), (requests, planners)
            assert 'WITNESS_FOREGROUND_RETURNED' in result.stdout
        with sqlite3.connect(profile / 'executions.db') as db:
            goals = dict(db.execute('SELECT id,objective FROM goal_records').fetchall())
            assert len(goals) == len(objectives)
            assert sorted(goals.values()) == sorted(objectives)
            criteria = db.execute('SELECT goal_id,hash,document FROM goal_criteria').fetchall()
            assert len(criteria) == len(objectives)
            for goal_id, digest, document in criteria:
                assert hashlib.sha256(document.encode()).hexdigest() == digest
                saved = json.loads(document)
                assert saved['origin'] == 'model_proposed' and saved['coverage'] == 'unverified'
                assert saved['oath']['root_goal_id'] == goal_id
                assert saved['oath']['goal'] == goals[goal_id]
                assert saved['workspace'] == str((profile / 'workspace').resolve())
            assert not db.execute("SELECT id FROM goal_records WHERE state='succeeded'").fetchall()
        restart = subprocess.run([str(binary), 'chat'], input='/goal-status\n/quit\n', text=True,
                                 stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=env, cwd=root, timeout=30)
        assert restart.returncode == 0, restart.stdout[-3000:]
        assert 'model criteria saved; coverage unverified' in restart.stdout
        assert len(server.requests) - before == requests, 'inspection must not call provider'
        with sqlite3.connect(profile / 'witness.db') as db:
            rows = db.execute("SELECT root_goal_id,subtask_id,payload_json FROM witness_ledger WHERE entry_type='oath_sealed'").fetchall()
            assert len(rows) == len(objectives), rows
            assert {row[0] for row in rows} == set(goals)
            assert len({row[1] for row in rows}) == len(rows)
            for goal_id, _, payload in rows:
                oath = json.loads(payload)
                assert oath['goal'] == goals[goal_id], oath
        return {'passed': True, 'limited': limited, 'requests': requests, 'planner_requests': planners,
                'unique_execution_bound_oaths': len(rows), 'original_objectives_preserved': True, 'criteria_frozen_before_foreground': True, 'restart_inspection_provider_calls': 0}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('binary', type=Path)
    args = parser.parse_args()
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    server.requests, server.planners = [], 0
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        print(json.dumps([run(args.binary.resolve(), server, limited) for limited in [False, True]], indent=2))
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


if __name__ == '__main__':
    main()
