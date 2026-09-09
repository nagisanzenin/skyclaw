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
        reviewer = any(m.get('role') == 'system' and str(m.get('content', '')).startswith('You are a predicate verifier') for m in request.get('messages', []))
        if planner:
            content = json.dumps({'goal': 'weaker model-authored goal', 'postconditions': [
                {'kind': 'file_exists', 'path': 'fixture.txt'},
                {'kind': 'grep_count_at_least', 'pattern': 'token', 'path_glob': '*.txt', 'n': 2},
                {'kind': 'grep_absent', 'pattern': 'TODO', 'path_glob': '*.txt'}]})
            if self.server.unknown_composite:
                draft = json.loads(content)
                draft['postconditions'].append({'kind': 'not_of', 'predicate': {
                    'kind': 'all_of', 'predicates': [{'kind': 'elapsed_under', 'start_marker': 'unset', 'max_secs': 1}]}})
                content = json.dumps(draft)
            if self.server.evidence_ref:
                draft = json.loads(content)
                draft['evidence_required'] = [{'id': 'artifact', 'kind': {'kind': 'file', 'path': 'fixture.txt'}, 'description': 'actual fixture file'}]
                draft['postconditions'].append({'kind': 'aspect_verifier', 'rubric': 'Review supplied file', 'evidence_refs': ['artifact'], 'advisory': False})
                content = json.dumps(draft)
            if self.server.failing_check:
                draft = json.loads(content)
                draft['postconditions'].append({'kind': 'file_exists', 'path': 'never-created-required.txt'})
                content = json.dumps(draft)
        elif reviewer:
            assert request['model'] in ('witness-fixture', 'witness-fixture-two')
            expected_output = (512 if request['model'] == 'witness-fixture-two' else 1024) if self.server.small_output_limits else 4096
            assert request.get('max_tokens') == expected_output, request
            assert 'token token' in json.dumps(request)
            content = json.dumps({'verdict': 'pass', 'reason': 'reviewed fixture bytes'})
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
tier1_enabled = {str(server.model_calls is not None).lower()}
tier2_enabled = false
{f'model_verification_max_calls = {server.model_calls}' if server.model_calls is not None else ''}
show_readout = true
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
        if server.switch_model:
            with (profile / 'custom_models.toml').open('a') as models:
                models.write('\n[[models]]\nprovider = "openai"\nname = "witness-fixture-two"\ncontext_window = 32768\nmax_output_tokens = 4096\ninput_price_per_1m = 1.0\noutput_price_per_1m = 1.0\npricing_verified = true\n')
        if server.small_output_limits:
            models = profile / 'custom_models.toml'
            content = models.read_text().replace('max_output_tokens = 4096', 'max_output_tokens = 1024', 1)
            content = content.replace('max_output_tokens = 4096', 'max_output_tokens = 512')
            models.write_text(content)
        env = {k: v for k, v in os.environ.items() if not k.endswith(('_API_KEY', '_TOKEN')) and not k.startswith('TEMM1E_')}
        env['TEMM1E_DATA_DIR'] = str(profile)
        objectives = OBJECTIVES[:1] if limited else OBJECTIVES
        server.profile = profile
        before, planners_before = len(server.requests), server.planners
        model_command = '/model witness-fixture-two' if server.slash_model else f'proxy openai http://127.0.0.1:{server.server_port}/v1 local-witness-fixture model:witness-fixture-two'
        commands = [objectives[0], model_command, objectives[1]] if server.switch_model and not limited else objectives
        result = subprocess.run([str(binary), 'chat'], input='\n'.join(commands + ['/quit', '']), text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, cwd=root, env=env, timeout=45)
        assert result.returncode == 0, result.stdout[-3000:]
        requests, planners = len(server.requests) - before, server.planners - planners_before
        if limited:
            assert requests == 1 and planners == 1, (requests, planners, result.stdout[-3000:])
            assert 'WITNESS_FOREGROUND_RETURNED' not in result.stdout
            assert 'Budget exceeded' in result.stdout, result.stdout[-3000:]
        else:
            assert (requests, planners) == (6 if server.evidence_ref and server.model_calls else 4, 2), (requests, planners)
            assert 'WITNESS_FOREGROUND_RETURNED' in result.stdout
            if server.switch_model:
                per_turn = 3 if server.evidence_ref and server.model_calls else 2
                assert [request['model'] for request in server.requests[before:]] == ['witness-fixture'] * per_turn + ['witness-fixture-two'] * per_turn
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
                if server.evidence_ref:
                    assert len(saved['oath']['evidence_required']) == 1
                assert saved['workspace'] == str((profile / 'workspace').resolve())
            assert not db.execute("SELECT id FROM goal_records WHERE state='succeeded'").fetchall()
            assessments = db.execute('SELECT goal_id,hash,document FROM goal_assessments').fetchall()
            assert len(assessments) == (0 if limited else 2), assessments
            expected = 'failed' if server.failing_check else 'inconclusive' if server.unknown_composite or (server.evidence_ref and not server.model_calls) else 'passed'
            for goal_id, digest, document in assessments:
                assert hashlib.sha256(document.encode()).hexdigest() == digest
                saved = json.loads(document)
                assert saved['coverage'] == 'unverified' and saved['declared_outcome'] == expected, saved
                if server.evidence_ref and server.model_calls:
                    assert len(saved['model_evidence']) == 1
                    assert saved['model_evidence'][0]['snapshots'][0]['content'] == 'token token'
                    assert 'Model verification cost: unavailable' in result.stdout
                for entry in saved['observations']:
                    raw = json.dumps(entry['observation'], ensure_ascii=False, separators=(',', ':')).encode()
                    assert hashlib.sha256(raw).hexdigest() == entry['hash']
                    assert entry['observation']['kind'] == 'evaluator_report'
        inspect = '/goal-status\n' + ''.join('/goal-assessment ' + goal_id + '\n' for goal_id in goals) + '/quit\n'
        restart = subprocess.run([str(binary), 'chat'], input=inspect, text=True,
                                 stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=env, cwd=root, timeout=30)
        assert restart.returncode == 0, restart.stdout[-3000:]
        assert 'model criteria saved; coverage unverified' in restart.stdout
        if limited:
            assert 'No saved assessment' in restart.stdout
        else:
            assert f'Recorded declared checks: {expected.capitalize()}.' in restart.stdout
            assert 'Full request coverage: unverified' in restart.stdout
        assert len(server.requests) - before == requests, 'inspection must not call provider'
        with sqlite3.connect(profile / 'witness.db') as db:
            rows = db.execute("SELECT root_goal_id,subtask_id,payload_json FROM witness_ledger WHERE entry_type='oath_sealed'").fetchall()
            assert len(rows) == len(objectives), rows
            assert {row[0] for row in rows} == set(goals)
            assert len({row[1] for row in rows}) == len(rows)
            for goal_id, _, payload in rows:
                oath = json.loads(payload)
                assert oath['goal'] == goals[goal_id], oath
            if server.unknown_composite or server.failing_check:
                verdicts = [json.loads(row[0]) for row in db.execute("SELECT payload_json FROM witness_ledger WHERE entry_type='verdict_rendered'")]
                assert len(verdicts) == (0 if limited else 2), verdicts
                for verdict in verdicts:
                    assert verdict['outcome'] == ('fail' if server.failing_check else 'inconclusive'), verdict
                    assert verdict['per_predicate'][-1]['outcome'] == ('fail' if server.failing_check else 'inconclusive'), verdict
        return {'passed': True, 'limited': limited, 'requests': requests, 'planner_requests': planners,
                'unique_execution_bound_oaths': len(rows), 'original_objectives_preserved': True, 'criteria_frozen_before_foreground': True, 'restart_inspection_provider_calls': 0, 'unknown_composite_checked': server.unknown_composite, 'declared_assessment': None if limited else expected, 'disabled_evidence_verifier_checked': server.evidence_ref and not server.model_calls, 'configured_model_call_limit': server.model_calls}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('binary', type=Path)
    parser.add_argument('--unknown-composite', action='store_true')
    parser.add_argument('--failing-check', action='store_true')
    parser.add_argument('--evidence-ref', action='store_true')
    parser.add_argument('--model-calls', type=int)
    parser.add_argument('--switch-model', action='store_true')
    parser.add_argument('--slash-model', action='store_true')
    parser.add_argument('--small-output-limits', action='store_true')
    args = parser.parse_args()
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    server.requests, server.planners = [], 0
    server.unknown_composite = args.unknown_composite
    server.failing_check = args.failing_check
    server.evidence_ref = args.evidence_ref
    server.model_calls = args.model_calls
    server.switch_model = args.switch_model or args.slash_model
    server.slash_model = args.slash_model
    server.small_output_limits = args.small_output_limits
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
