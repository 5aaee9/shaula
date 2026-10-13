#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
# Same delegated-cgroup contract as lifecycle-acceptance.sh.
SHAULA_TEST_CGROUP="/sys/fs/cgroup$(sed -n 's/^0:://p' /proc/self/cgroup)"
export SHAULA_TEST_CGROUP
test "$SHAULA_TEST_CGROUP" != /sys/fs/cgroup/
test -w "$SHAULA_TEST_CGROUP/cgroup.procs"
node --test scripts/gitlab-lifecycle/*.test.mjs
# baseline = G1/G2/G7/G8 (run.mjs); demand|busy|token|uncertain = G3–G6 (suite.mjs).
if [[ ${SHAULA_GITLAB_SUITE:-baseline} == baseline ]]; then
  node scripts/gitlab-lifecycle/run.mjs
else
  node scripts/gitlab-lifecycle/suite.mjs
fi
