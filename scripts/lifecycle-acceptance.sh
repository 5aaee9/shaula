#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
# The parent must itself live inside the delegation: moving children between
# unrelated cgroups would require write access to their common ancestor.
SHAULA_TEST_CGROUP="/sys/fs/cgroup$(sed -n 's/^0:://p' /proc/self/cgroup)"
export SHAULA_TEST_CGROUP
test "$SHAULA_TEST_CGROUP" != /sys/fs/cgroup/
test -w "$SHAULA_TEST_CGROUP/cgroup.procs"
if [[ ${SHAULA_ACCEPTANCE_SUITE:-all} == all || ${SHAULA_ACCEPTANCE_SUITE:-all} == baseline ]]; then
  bash scripts/lifecycle-worker-tests.sh
fi
node --test scripts/forgejo-lifecycle/*.test.mjs
node scripts/forgejo-lifecycle/run.mjs
