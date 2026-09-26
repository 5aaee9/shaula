#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
: "${SHAULA_TEST_CGROUP:?set a delegated cgroup v2 test directory}"
SHAULA_TEST_TERRAFORM="$(command -v terraform)"
export SHAULA_TEST_TERRAFORM
cargo test --locked -p shaula --test worker_process --test serve_lifecycle --test personal_token_restart --test http_state_backend -- --ignored --nocapture
