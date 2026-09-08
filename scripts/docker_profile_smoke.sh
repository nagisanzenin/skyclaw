#!/usr/bin/env bash
# Runs only against an explicitly supplied local image. No model credentials.
set -euo pipefail
image=${1:?usage: docker_profile_smoke.sh IMAGE}
case_id="temm1e-profile-smoke-${GITHUB_RUN_ID:-$$}-${RANDOM}"
volume="${case_id}-data"
cleanup() {
  docker rm -f "$case_id" >/dev/null 2>&1 || true
  docker volume rm "$volume" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker volume create "$volume" >/dev/null
start_and_wait() {
  docker run -d --name "$case_id" --network none \
    --mount "type=volume,src=$volume,dst=/var/lib/temm1e" \
    "$image" start >/dev/null
  for attempt in $(seq 1 30); do
    if docker exec "$case_id" curl -fsS http://localhost:8080/health >/dev/null 2>&1; then
      return
    fi
    if [ "$(docker inspect --format '{{.State.Running}}' "$case_id")" != true ]; then
      docker logs "$case_id"
      return 1
    fi
    sleep 1
  done
  docker logs "$case_id"
  return 1
}
start_and_wait
# The actual application must have created its memory database in the mount.
docker exec "$case_id" sh -ec '
  test "$TEMM1E_DATA_DIR" = /var/lib/temm1e
  test -s "$TEMM1E_DATA_DIR/memory.db"
  test ! -e /root/.temm1e/memory.db
  umask 077
  printf original > "$TEMM1E_DATA_DIR/mount-probe"
  printf replacement > "$TEMM1E_DATA_DIR/mount-probe.tmp"
  mv "$TEMM1E_DATA_DIR/mount-probe.tmp" "$TEMM1E_DATA_DIR/mount-probe"
'
docker stop --time 15 "$case_id" >/dev/null
test "$(docker inspect --format '{{.State.ExitCode}}' "$case_id")" = 0
docker rm "$case_id" >/dev/null
start_and_wait
docker exec "$case_id" sh -ec '
  test -s "$TEMM1E_DATA_DIR/memory.db"
  test "$(cat "$TEMM1E_DATA_DIR/mount-probe")" = replacement
  test "$(stat -c %a "$TEMM1E_DATA_DIR/mount-probe")" = 600
'
docker stop --time 15 "$case_id" >/dev/null
test "$(docker inspect --format '{{.State.ExitCode}}' "$case_id")" = 0
printf 'PASS: application profile survives container replacement; directory permits private atomic replacement.\n'
