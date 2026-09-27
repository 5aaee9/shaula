#!/usr/bin/env bash
# LW-14 under systemd: launch a real job in a Delegate=yes transient unit,
# kill the unit as an ungraceful daemon exit (systemd then removes its whole
# cgroup subtree), and require the next invocation of the same unit to fence
# the recorded identity from its replaced delegated root.
# Usage: scripts/lifecycle-systemd-restart.sh [--user]
set -euo pipefail
cd "$(dirname "$0")/.."
mode=("${1:-}")
[ "${mode[0]}" = "--user" ] || mode=()
bin="$(cargo test --locked -p shaula --test worker_process --no-run --message-format=json 2>/dev/null \
  | jq -r 'select(.executable != null and .target.name == "worker_process") | .executable' | tail -1)"
test -x "$bin"
unit="shaula-restart-$(od -An -N4 -tx4 /dev/urandom | tr -d ' ')"
state="$(mktemp -d)"
trap 'systemctl "${mode[@]}" stop "$unit" 2>/dev/null || true; systemctl "${mode[@]}" reset-failed "$unit" 2>/dev/null || true; rm -rf "$state"' EXIT
identity="$state/identity.json"
run() {
  systemd-run "${mode[@]}" --quiet --unit="$unit" -p Delegate=yes -p KillMode=control-group \
    --setenv=SHAULA_RESTART_ROLE="$1" --setenv=SHAULA_RESTART_IDENTITY="$identity" \
    "${@:2}" /bin/sh -c 'export SHAULA_TEST_CGROUP="/sys/fs/cgroup$(sed -n "s/^0:://p" /proc/self/cgroup)"; exec "$0" restart::systemd_restart_role --exact --ignored --nocapture' "$bin"
}
run launch
for _ in $(seq 1 300); do [ -s "$identity" ] && break; sleep 0.1; done
if [ ! -s "$identity" ]; then
  journalctl "${mode[@]}" -u "$unit" --no-pager -n 30 >&2 || true
  exit 1
fi
group="$(jq -r .containment "$identity")"
test -d "$group"
systemctl "${mode[@]}" kill -s SIGKILL "$unit"
for _ in $(seq 1 100); do systemctl "${mode[@]}" is-active --quiet "$unit" || break; sleep 0.1; done
test ! -e "$group"
systemctl "${mode[@]}" reset-failed "$unit" 2>/dev/null || true
run fence --wait --pipe
echo "PASS: $unit"
