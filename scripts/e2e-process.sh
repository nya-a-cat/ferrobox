#!/usr/bin/env bash
set -euo pipefail

api_url="${FERROBOX_API_URL:-http://127.0.0.1:18080}"
runtime_root="$(mktemp -d)"
api_pid=""

cleanup() {
    status="$?"
    if [[ "${status}" -ne 0 && -f "${runtime_root}/api.log" ]]; then
        echo "::group::Ferrobox API log"
        cat "${runtime_root}/api.log"
        echo "::endgroup::"
    fi
    if [[ -n "${api_pid}" ]]; then
        kill "${api_pid}" 2>/dev/null || true
        wait "${api_pid}" 2>/dev/null || true
    fi
    rm -rf -- "${runtime_root}"
    return "${status}"
}
trap cleanup EXIT

target/debug/ferrobox-api \
    --backend process \
    --unsafe-process-runtime \
    --listen 127.0.0.1:18080 \
    --process-root "${runtime_root}/sandboxes" \
    --audit-log "${runtime_root}/audit/events.jsonl" \
    >"${runtime_root}/api.log" 2>&1 &
api_pid="$!"

for _ in $(seq 1 120); do
    if curl --fail --silent "${api_url}/healthz" >/dev/null; then
        break
    fi
    if ! kill -0 "${api_pid}" 2>/dev/null; then
        wait "${api_pid}"
    fi
    sleep 0.25
done
curl --fail --silent "${api_url}/healthz" >/dev/null

create_response="$(
    curl --fail --silent \
        --header 'content-type: application/json' \
        --data '{"template":"python","cpu_count":1,"memory_mb":512,"timeout_seconds":120,"network":{"internet_access":false}}' \
        "${api_url}/v1/sandboxes"
)"
sandbox_id="$(jq -er '.sandbox_id' <<<"${create_response}")"
token="$(jq -er '.token' <<<"${create_response}")"

exec_response="$(
    curl --fail --silent \
        --header "authorization: Bearer ${token}" \
        --header 'content-type: application/json' \
        --data '{"argv":["python3","-c","print(42)"],"cwd":"/home/sandbox","environment":{},"timeout_seconds":30,"max_output_bytes":1048576}' \
        "${api_url}/v1/sandboxes/${sandbox_id}/commands"
)"
[[ "$(jq -r '.stdout' <<<"${exec_response}")" == "42" ]]
[[ "$(jq -r '.termination.kind' <<<"${exec_response}")" == "exited" ]]

literal_response="$(
    curl --fail --silent \
        --header "authorization: Bearer ${token}" \
        --header 'content-type: application/json' \
        --data '{"argv":["python3","-c","import sys; print(sys.argv[1])","$(touch /tmp/ferrobox-injected);"],"cwd":"/home/sandbox","environment":{},"timeout_seconds":30,"max_output_bytes":1048576}' \
        "${api_url}/v1/sandboxes/${sandbox_id}/commands"
)"
[[ "$(jq -r '.stdout' <<<"${literal_response}")" == '$(touch /tmp/ferrobox-injected);' ]]
[[ ! -e /tmp/ferrobox-injected ]]

curl --fail --silent \
    --request PUT \
    --header "authorization: Bearer ${token}" \
    --header 'content-type: application/json' \
    --data '{"path":"/home/sandbox/hello.txt","content_base64":"aGVsbG8K","overwrite":false}' \
    "${api_url}/v1/sandboxes/${sandbox_id}/files" >/dev/null

read_response="$(
    curl --fail --silent --get \
        --header "authorization: Bearer ${token}" \
        --data-urlencode 'path=/home/sandbox/hello.txt' \
        "${api_url}/v1/sandboxes/${sandbox_id}/files"
)"
[[ "$(jq -r '.content_base64' <<<"${read_response}")" == "aGVsbG8K" ]]

traversal_status="$(
    curl --silent --output /dev/null --write-out '%{http_code}' --get \
        --header "authorization: Bearer ${token}" \
        --data-urlencode 'path=../../etc/passwd' \
        "${api_url}/v1/sandboxes/${sandbox_id}/files"
)"
[[ "${traversal_status}" == "400" ]]

snapshot_status="$(
    curl --silent --output /dev/null --write-out '%{http_code}' \
        --header "authorization: Bearer ${token}" \
        --header 'content-type: application/json' \
        --data '{"name":"unsupported-on-process-backend"}' \
        "${api_url}/v1/sandboxes/${sandbox_id}/snapshots"
)"
[[ "${snapshot_status}" == "501" ]]

curl --fail --silent \
    --request DELETE \
    --header "authorization: Bearer ${token}" \
    "${api_url}/v1/sandboxes/${sandbox_id}" >/dev/null

deleted_status="$(
    curl --silent --output /dev/null --write-out '%{http_code}' \
        --header "authorization: Bearer ${token}" \
        "${api_url}/v1/sandboxes/${sandbox_id}"
)"
[[ "${deleted_status}" == "404" ]]
! grep --fixed-strings --quiet "${token}" "${runtime_root}/audit/events.jsonl"
grep --fixed-strings --quiet '"operation":"delete"' "${runtime_root}/audit/events.jsonl"

printf 'Process/API E2E passed for sandbox %s\n' "${sandbox_id}"
