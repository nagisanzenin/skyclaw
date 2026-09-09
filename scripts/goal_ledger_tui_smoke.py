#!/usr/bin/env python3
"""Actual PTY goal inspection during use and after restart; no new model calls."""
import argparse
import http.server
import json
from pathlib import Path
import tempfile
import threading
from tui_pty_smoke import Provider, run


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('binary', type=Path)
    args = parser.parse_args()
    servers = []
    try:
        for _ in range(2):
            server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
            server.requests, server.authorizations = [], []
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            servers.append((server, thread))
        server, trap = servers[0][0], servers[1][0]
        with tempfile.TemporaryDirectory(prefix='temm1e-goal-pty-') as directory:
            root = Path(directory)
            print(json.dumps([run(args.binary.resolve(), root, server, trap, goal_check=True),
                              run(args.binary.resolve(), root, server, trap, restore_only=True, goal_check=True)], indent=2))
    finally:
        for server, thread in servers:
            server.shutdown()
            server.server_close()
            thread.join()


if __name__ == '__main__':
    main()
