"""Synthetic permanent memory for actual-entrypoint policy regression tests."""
import json
import sqlite3
import time

SENTINEL = 'POLICY_MEMORY_SENTINEL_8E2F'


def seed(profile):
    path = profile / 'memory.db'
    with sqlite3.connect(path) as db:
        db.execute('''CREATE TABLE engram_facts (
            id TEXT PRIMARY KEY, content TEXT NOT NULL, summary TEXT NOT NULL,
            essence TEXT NOT NULL, fact_type TEXT NOT NULL DEFAULT 'reference',
            scope TEXT NOT NULL DEFAULT 'global', pinned_by TEXT NOT NULL DEFAULT 'none',
            subject_key TEXT, importance REAL NOT NULL DEFAULT 1.0,
            created_at INTEGER NOT NULL, last_accessed INTEGER NOT NULL,
            tags TEXT NOT NULL DEFAULT '[]', links TEXT NOT NULL DEFAULT '[]')''')
        now = int(time.time())
        db.execute('''INSERT INTO engram_facts
            (id, content, summary, essence, scope, pinned_by, importance, created_at, last_accessed)
            VALUES (?, ?, ?, ?, 'global', 'user', 5.0, ?, ?)''',
            ('policy-fixture', SENTINEL, SENTINEL, SENTINEL, now, now))
    path.chmod(0o600)


def assert_policy(requests, enabled):
    for model in ['pty-fixture', 'pty-fixture-next']:
        selected = [request for request in requests if request.get('model') == model]
        assert selected, f'no real request for {model}'
        injected = any(SENTINEL in json.dumps(request) for request in selected)
        assert injected == enabled, f'Engram enabled={enabled}, model={model}, permanent sentinel injected={injected}'
    assert len(requests) == 4, f'curator=off unexpectedly invoked extra calls: {len(requests)}'
