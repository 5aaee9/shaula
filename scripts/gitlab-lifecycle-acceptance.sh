#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
# Same delegated-cgroup contract as lifecycle-acceptance.sh.
SHAULA_TEST_CGROUP="/sys/fs/cgroup$(sed -n 's/^0:://p' /proc/self/cgroup)"
export SHAULA_TEST_CGROUP
test "$SHAULA_TEST_CGROUP" != /sys/fs/cgroup/
test -w "$SHAULA_TEST_CGROUP/cgroup.procs"
node scripts/gitlab-lifecycle/run.mjs
