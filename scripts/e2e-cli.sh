#!/usr/bin/env bash
set -euo pipefail

api_url="${FERROBOX_API_URL:-http://127.0.0.1:18081}"
test_python="${FERROBOX_TEST_PYTHON:-python3}"
runtime_root="$(mktemp -d)"
api_pid=""
sandbox_id=""
token=""

cleanup() {
    status="$?"
    set +e
    if [[ -n "${sandbox_id}" && -n "${token}" ]]; then
        FERROBOX_TOKEN="${token}" target/debug/ferrobox \
            --api-url "${api_url}" delete "${sandbox_id}" >/dev/null 2>&1
    fi
    if [[ "${status}" -ne 0 && -f "${runtime_root}/api.log" ]]; then
        echo "::group::Ferrobox CLI E2E API log"
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

test -x target/debug/ferrobox-api
test -x target/debug/ferrobox

target/debug/ferrobox-api \
    --backend process \
    --unsafe-process-runtime \
    --listen 127.0.0.1:18081 \
    --process-root "${runtime_root}/sandboxes" \
    --audit-log "${runtime_root}/audit/events.jsonl" \
    >"${runtime_root}/api.log" 2>&1 &
api_pid="$!"

for ((attempt = 0; attempt < 120; attempt++)); do
    if curl --fail --silent "${api_url}/healthz" >/dev/null; then
        break
    fi
    if ! kill -0 "${api_pid}" 2>/dev/null; then
        wait "${api_pid}"
    fi
    sleep 0.25
done
curl --fail --silent "${api_url}/healthz" >/dev/null

set +x
create_response="$(
    target/debug/ferrobox --api-url "${api_url}" create \
        --template python --cpu 1 --memory-mb 512 --ttl 120
)"
sandbox_id="$(jq -er '.sandbox_id' <<<"${create_response}")"
token="$(jq -er '.token' <<<"${create_response}")"
[[ "$(jq -r '.state' <<<"${create_response}")" == "running" ]]
unset create_response

inspect_response="$(
    FERROBOX_TOKEN="${token}" target/debug/ferrobox --api-url "${api_url}" \
        inspect "${sandbox_id}"
)"
[[ "$(jq -r '.sandbox_id' <<<"${inspect_response}")" == "${sandbox_id}" ]]
[[ "$(jq -r '.state' <<<"${inspect_response}")" == "running" ]]

exec_response="$(
    FERROBOX_TOKEN="${token}" target/debug/ferrobox --api-url "${api_url}" \
        exec "${sandbox_id}" -- "${test_python}" -c 'print(42)'
)"
exec_stdout="$(jq -r '.stdout' <<<"${exec_response}")"
[[ "${exec_stdout%$'\r'}" == "42" ]]
[[ "$(jq -r '.termination.kind' <<<"${exec_response}")" == "exited" ]]

literal='$(touch /tmp/ferrobox-cli-injected);'
literal_response="$(
    FERROBOX_TOKEN="${token}" target/debug/ferrobox --api-url "${api_url}" \
        exec "${sandbox_id}" -- "${test_python}" -c \
        'import sys; print(sys.argv[1])' "${literal}"
)"
literal_stdout="$(jq -r '.stdout' <<<"${literal_response}")"
[[ "${literal_stdout%$'\r'}" == "${literal}" ]]
[[ ! -e /tmp/ferrobox-cli-injected ]]

printf 'ferrobox-cli\n' >"${runtime_root}/input.txt"
FERROBOX_TOKEN="${token}" target/debug/ferrobox --api-url "${api_url}" \
    write "${sandbox_id}" /home/sandbox/input.txt "${runtime_root}/input.txt" >/dev/null
FERROBOX_TOKEN="${token}" target/debug/ferrobox --api-url "${api_url}" \
    read "${sandbox_id}" /home/sandbox/input.txt \
    --output "${runtime_root}/output.txt"
cmp "${runtime_root}/input.txt" "${runtime_root}/output.txt"

list_response="$(
    FERROBOX_TOKEN="${token}" target/debug/ferrobox --api-url "${api_url}" \
        list "${sandbox_id}" /home/sandbox
)"
[[ "$(jq -r '.entries[] | select(.name == "input.txt") | .kind' <<<"${list_response}")" == "file" ]]

pause_output="$(
    FERROBOX_TOKEN="${token}" target/debug/ferrobox --api-url "${api_url}" \
        pause "${sandbox_id}"
)"
[[ "${pause_output}" == "paused ${sandbox_id}" ]]
paused_response="$(
    FERROBOX_TOKEN="${token}" target/debug/ferrobox --api-url "${api_url}" \
        inspect "${sandbox_id}"
)"
[[ "$(jq -r '.state' <<<"${paused_response}")" == "paused" ]]
if FERROBOX_TOKEN="${token}" target/debug/ferrobox --api-url "${api_url}" \
    exec "${sandbox_id}" -- "${test_python}" -c 'print(42)' \
    >"${runtime_root}/paused-exec.log" 2>&1; then
    echo "command unexpectedly ran while the sandbox was paused" >&2
    exit 1
fi
grep -Fq '409 Conflict' "${runtime_root}/paused-exec.log"

resume_output="$(
    FERROBOX_TOKEN="${token}" target/debug/ferrobox --api-url "${api_url}" \
        resume "${sandbox_id}"
)"
[[ "${resume_output}" == "resumed ${sandbox_id}" ]]
resumed_response="$(
    FERROBOX_TOKEN="${token}" target/debug/ferrobox --api-url "${api_url}" \
        inspect "${sandbox_id}"
)"
[[ "$(jq -r '.state' <<<"${resumed_response}")" == "running" ]]

delete_output="$(
    FERROBOX_TOKEN="${token}" target/debug/ferrobox --api-url "${api_url}" \
        delete "${sandbox_id}"
)"
[[ "${delete_output}" == "deleted ${sandbox_id}" ]]

deleted_status="$(
    curl --silent --output /dev/null --write-out '%{http_code}' \
        --header "authorization: Bearer ${token}" \
        "${api_url}/v1/sandboxes/${sandbox_id}"
)"
[[ "${deleted_status}" == "404" ]]
! grep -Fq "${token}" "${runtime_root}/audit/events.jsonl"
grep -Fq '"operation":"delete"' "${runtime_root}/audit/events.jsonl"

completed_id="${sandbox_id}"
sandbox_id=""
token=""
printf 'CLI/Agent Skill E2E passed for sandbox %s\n' "${completed_id}"
