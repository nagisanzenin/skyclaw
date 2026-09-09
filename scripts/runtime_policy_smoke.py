#!/usr/bin/env python3
"""Positive/negative control: explicit Engram policy through CLI/TUI replacement."""
import argparse
import http.server
import json
from pathlib import Path
import tempfile
import threading
import cli_budget_smoke
import tui_pty_smoke


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('binary', type=Path)
    args = parser.parse_args()
    servers, threads = [], []
    for _ in range(2):
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), tui_pty_smoke.Provider)
        server.requests, server.authorizations = [], []
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        servers.append(server)
        threads.append(thread)
    server, trap = servers
    try:
        results = []
        for enabled in [False, True]:
            results.append(cli_budget_smoke.run(args.binary.resolve(), server, trap, False, engram_enabled=enabled))
            with tempfile.TemporaryDirectory(prefix='temm1e-tui-policy-') as directory:
                results.append(tui_pty_smoke.run(args.binary.resolve(), Path(directory), server, trap, engram_enabled=enabled))
        print(json.dumps(results, indent=2))
    finally:
        for server, thread in zip(servers, threads):
            server.shutdown()
            server.server_close()
            thread.join()


if __name__ == '__main__':
    main()
