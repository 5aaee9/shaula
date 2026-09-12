#!/usr/bin/env bash
set -euo pipefail

base_url="${FORGEJO_URL:-http://127.0.0.1:3000}"
forgejo_user="shaula-e2e-admin"
admin_password="Admin123456!"
repo="shaula-forgejo-e2e"
runner_name="shaula-e2e-${GITHUB_RUN_ID:-local}"
runner_label="shaula-e2e"

wait_http() {
  for _ in $(seq 1 60); do
    curl --fail --silent "$base_url/api/v1/version" >/dev/null && return 0
    sleep 2
  done
  echo "Forgejo did not become ready" >&2
  return 1
}

json_api() { curl --fail --silent --show-error -H "Authorization: Bearer $api_token" -H 'Content-Type: application/json' "$@"; }

wait_http
docker exec --user git forgejo forgejo admin user create --username "$forgejo_user" --password "$admin_password" --email admin@example.invalid --admin --must-change-password=false >/dev/null 2>&1 || true
api_token="$(docker exec --user git forgejo forgejo admin user generate-access-token --username "$forgejo_user" --token-name shaula-e2e --scopes all --raw)"
json_api -X POST "$base_url/api/v1/user/repos" --data "{\"name\":\"$repo\",\"private\":true,\"auto_init\":false}" >/dev/null

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT
mkdir -p "$tmp_dir/.forgejo/workflows"
cat >"$tmp_dir/.forgejo/workflows/match.yaml" <<'YAML'
name: shaula-forgejo-match
on: [push]
jobs:
  smoke:
    runs-on: [shaula-e2e]
    steps:
      - run: test "$FORGEJO_REPOSITORY" = "shaula-e2e-admin/shaula-forgejo-e2e"
      - run: echo forgejo-e2e-ok > forgejo-e2e-result.txt
YAML
cat >"$tmp_dir/.forgejo/workflows/mismatch.yaml" <<'YAML'
name: shaula-forgejo-mismatch
on: [push]
jobs:
  mismatch:
    runs-on: [shaula-e2e-mismatch]
    steps:
      - run: echo this-job-must-remain-queued
YAML
git -C "$tmp_dir" init -q
git -C "$tmp_dir" config user.email e2e@example.invalid
git -C "$tmp_dir" config user.name shaula-e2e
git -C "$tmp_dir" add .
git -C "$tmp_dir" commit -qm 'test: forgejo e2e workflow'
git -C "$tmp_dir" branch -M main
git -C "$tmp_dir" remote add origin "http://$forgejo_user:$admin_password@127.0.0.1:3000/$forgejo_user/$repo.git"
registration="$(json_api -X POST "$base_url/api/v1/admin/actions/runners" --data "{\"name\":\"$runner_name\",\"ephemeral\":true}")"
runner_uuid="$(jq -r .uuid <<<"$registration")"
runner_token="$(jq -r .token <<<"$registration")"
test -n "$runner_uuid" -a "$runner_uuid" != null
test -n "$runner_token" -a "$runner_token" != null
printf '%s' "$runner_token" >"$tmp_dir/token"

runner_version="${FORGEJO_RUNNER_VERSION:-13.1.0}"
curl --fail --silent --show-error -L -o "$tmp_dir/forgejo-runner" "https://code.forgejo.org/forgejo/runner/releases/download/v${runner_version}/forgejo-runner-${runner_version}-linux-amd64"
chmod +x "$tmp_dir/forgejo-runner"
"$tmp_dir/forgejo-runner" one-job --url "$base_url" --uuid "$runner_uuid" --token-url "file://$tmp_dir/token" --label "$runner_label:docker://node:20-bookworm" --wait >"$tmp_dir/runner.log" 2>&1 &
runner_pid=$!

inventory_status=""
for _ in $(seq 1 60); do
  inventory="$(json_api "$base_url/api/v1/admin/actions/runners?limit=50" || true)"
  inventory_status="$(jq -r --arg name "$runner_name" '[.[] | select(.name == $name)][0].status // empty' <<<"$inventory")"
  if [[ "$inventory_status" == idle ]]; then break; fi
  sleep 1
done
test "$inventory_status" = idle

# Dispatch only after the ephemeral runner has been observed idle. The second
# workflow uses a label for which no runner exists and must remain queued.
git -C "$tmp_dir" push -q origin main

status=""
for _ in $(seq 1 90); do
  runs="$(json_api "$base_url/api/v1/repos/$forgejo_user/$repo/actions/runs?limit=1" || true)"
  status="$(jq -r '[.workflow_runs[]? | select(.name == "shaula-forgejo-match")][0].status // empty' <<<"$runs")"
  case "$status" in
  success) break ;;
  failure | cancelled)
    cat "$tmp_dir/runner.log"
    echo "Forgejo workflow failed: $status" >&2
    exit 1
    ;;
  esac
  sleep 2
done
test "$status" = success
wait "$runner_pid"

mismatch_status=""
for _ in $(seq 1 30); do
  runs="$(json_api "$base_url/api/v1/repos/$forgejo_user/$repo/actions/runs?limit=50" || true)"
  mismatch_status="$(jq -r '[.workflow_runs[]? | select(.name == "shaula-forgejo-mismatch")][0].status // empty' <<<"$runs")"
  [[ -n "$mismatch_status" ]] && break
  sleep 1
done
test "$mismatch_status" != success
test "$mismatch_status" != cancelled

for _ in $(seq 1 30); do
  remaining="$(json_api "$base_url/api/v1/admin/actions/runners?limit=50" || true)"
  if jq -e --arg name "$runner_name" '[.[] | select(.name == $name)] | length == 0' <<<"$remaining"; then exit 0; fi
  sleep 2
done
echo "ephemeral runner still present after successful job" >&2
cat "$tmp_dir/runner.log" >&2
exit 1
