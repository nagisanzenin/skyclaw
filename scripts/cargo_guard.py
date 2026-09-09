#!/usr/bin/env python3
"""Run local Cargo validation with disk limits and disposable build output."""
import argparse
import contextlib
import math
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time

GIB = 1024**3
ROOT = Path(__file__).resolve().parent.parent
TARGET = ROOT / 'target' / 'guarded'


def tree_size(path):
    total = 0
    for directory, _, files in os.walk(path, followlinks=False):
        for name in files:
            try:
                total += (Path(directory) / name).lstat().st_size
            except FileNotFoundError:
                pass
    return total


def limit_reason(root, reserve, maximum):
    free = shutil.disk_usage(root).free
    used = tree_size(root / 'target')
    if free < reserve:
        return f'free space {free / GIB:.2f} GiB is below reserve {reserve / GIB:.2f} GiB'
    if used > maximum:
        return f'target uses {used / GIB:.2f} GiB, above limit {maximum / GIB:.2f} GiB'
    return None


def stop(process):
    if process.poll() is not None:
        return
    with contextlib.suppress(ProcessLookupError):
        if os.name == 'posix':
            os.killpg(process.pid, signal.SIGTERM)
        else:
            process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        with contextlib.suppress(ProcessLookupError):
            if os.name == 'posix':
                os.killpg(process.pid, signal.SIGKILL)
            else:
                process.kill()
        process.wait()


def run_guarded(command, root, env, reserve, maximum, check=limit_reason):
    reason = check(root, reserve, maximum)
    if reason:
        print(f'Build not started: {reason}', file=sys.stderr)
        return 75
    process = subprocess.Popen(command, cwd=root, env=env, start_new_session=os.name == 'posix')
    try:
        while process.poll() is None:
            reason = check(root, reserve, maximum)
            if reason:
                print(f'Build stopped without a validation result: {reason}', file=sys.stderr)
                stop(process)
                return 75
            time.sleep(1)
        return process.returncode
    finally:
        stop(process)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reserve-gib', type=float, default=8)
    parser.add_argument('--max-target-gib', type=float, default=8)
    parser.add_argument('--keep-cache', action='store_true', help='retain guarded outputs (e.g. to copy a release binary)')
    parser.add_argument('cargo_args', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    arguments = args.cargo_args
    if arguments[:1] == ['--']:
        arguments = arguments[1:]
    if not arguments or arguments[0] not in {'build', 'check', 'test', 'clippy'}:
        parser.error('provide -- followed by build, check, test or clippy and its arguments')
    if any(not math.isfinite(x) or x <= 0 for x in (args.reserve_gib, args.max_target_gib)):
        parser.error('disk limits must be finite and positive')
    if any(x == '--target-dir' or x.startswith('--target-dir=') for x in arguments):
        parser.error('the guard owns its target directory; do not override --target-dir')
    if any(x == '--config' or x.startswith('--config=') for x in arguments):
        parser.error('inline Cargo configuration is not supported by the disk guard')
    cargo = shutil.which('cargo')
    if not cargo:
        parser.error('cargo is not installed')
    env = os.environ.copy()
    env.update(CARGO_TARGET_DIR=str(TARGET), CARGO_INCREMENTAL='0',
               CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_TEST_DEBUG='0')
    # Serialize guarded runs. Raw Cargo runs are not controlled by this lock.
    with (ROOT / '.cargo-build.lock').open('a') as lock:
        if os.name == 'posix':
            import fcntl
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                print('Another guarded build is running', file=sys.stderr)
                return 75
        try:
            result = run_guarded([cargo, arguments[0], '--target-dir', str(TARGET), *arguments[1:]], ROOT, env,
                                 args.reserve_gib * GIB, args.max_target_gib * GIB)
        except KeyboardInterrupt:
            result = 130
        finally:
            if not args.keep_cache and TARGET.exists():
                cleanup = subprocess.run([cargo, 'clean', '--target-dir', str(TARGET)], cwd=ROOT, env=env)
                if cleanup.returncode:
                    print('Cleanup failed; guarded build artifacts remain', file=sys.stderr)
                    result = result or cleanup.returncode
        return result


if __name__ == '__main__':
    sys.exit(main())
