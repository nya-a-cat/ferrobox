#!/usr/bin/env bash
set -euo pipefail

if [[ "${EUID}" -ne 0 ]]; then
    echo "e2e-oci-kvm.sh must run as root" >&2
    exit 2
fi

firecracker="${FERROBOX_FIRECRACKER:?FERROBOX_FIRECRACKER is required}"
jailer="${FERROBOX_JAILER:?FERROBOX_JAILER is required}"
kernel="${FERROBOX_KERNEL:?FERROBOX_KERNEL is required}"
rootfs="${FERROBOX_ROOTFS:?FERROBOX_ROOTFS is required}"
chroot_base="${FERROBOX_CHROOT_BASE:?FERROBOX_CHROOT_BASE is required}"
runtime_root="${FERROBOX_RUNTIME_ROOT:?FERROBOX_RUNTIME_ROOT is required}"
rootfs_evidence="${FERROBOX_OCI_ROOTFS_EVIDENCE:?FERROBOX_OCI_ROOTFS_EVIDENCE is required}"
output="${FERROBOX_OCI_E2E_OUTPUT:?FERROBOX_OCI_E2E_OUTPUT is required}"
api_binary="${FERROBOX_API_BINARY:-target/debug/ferrobox-api}"
api_url="${FERROBOX_API_URL:-http://127.0.0.1:18084}"
work_dir="$(mktemp -d)"
api_pid=""
sandbox_id=""
token=""

cleanup() {
    status="$?"
    set +e
    if [[ -n "${sandbox_id}" && -n "${token}" ]]; then
        curl --silent \
            --request DELETE \
            --header "authorization: Bearer ${token}" \
            "${api_url}/v1/sandboxes/${sandbox_id}" >/dev/null 2>&1
    fi
    if [[ "${status}" -ne 0 && -f "${work_dir}/api.log" ]]; then
        echo "::group::OCI KVM API log"
        cat "${work_dir}/api.log"
        echo "::endgroup::"
    fi
    if [[ -n "${api_pid}" ]]; then
        kill "${api_pid}" 2>/dev/null || true
        wait "${api_pid}" 2>/dev/null || true
    fi
    rm -rf -- "${work_dir}"
    return "${status}"
}
trap cleanup EXIT

record_api_failure() {
    local stage="$1"
    local http_status="$2"
    local response_path="$3"
    local failure_output
    failure_output="$(dirname -- "${output}")/oci-kvm-api-failure.json"
    if [[ -s "${response_path}" ]] && jq -e . "${response_path}" >/dev/null 2>&1; then
        jq -c \
            --arg stage "${stage}" \
            --arg http_status "${http_status}" '
                {
                    stage: $stage,
                    http_status: $http_status,
                    error: {
                        code: (.error.code // "invalid_response"),
                        message: (.error.message // "invalid response body")
                    }
                }
            ' "${response_path}" >"${failure_output}"
    else
        jq -n -c \
            --arg stage "${stage}" \
            --arg http_status "${http_status}" '
                {
                    stage: $stage,
                    http_status: $http_status,
                    error: {
                        code: "invalid_response",
                        message: "response body was not valid JSON"
                    }
                }
            ' >"${failure_output}"
    fi
    echo "::group::Sanitized OCI API failure" >&2
    cat "${failure_output}" >&2
    echo "::endgroup::" >&2
}

api_call() {
    local stage="$1"
    local expected_status="$2"
    shift 2
    local response_path="${work_dir}/${stage}.response"
    local http_status
    if ! http_status="$(
        curl --silent --show-error \
            --output "${response_path}" \
            --write-out '%{http_code}' \
            "$@"
    )"; then
        record_api_failure "${stage}" "000" "${response_path}"
        return 1
    fi
    if [[ "${http_status}" != "${expected_status}" ]]; then
        record_api_failure "${stage}" "${http_status}" "${response_path}"
        return 1
    fi
    cat "${response_path}"
}

for path in "${firecracker}" "${jailer}" "${api_binary}"; do
    test -x "${path}"
done
for path in "${kernel}" "${rootfs}" "${rootfs_evidence}"; do
    test -f "${path}"
done
test -c /dev/kvm
test -r /dev/kvm
test -w /dev/kvm
mkdir -p -- "${chroot_base}" "${runtime_root}" "$(dirname -- "${output}")"

source_reference="$(jq -er '.image_reference' "${rootfs_evidence}")"
platform="$(jq -er '.platform' "${rootfs_evidence}")"
source_digest="$(jq -er '.source.digest' "${rootfs_evidence}")"
manifest_digest="$(jq -er '.manifest.digest' "${rootfs_evidence}")"
[[ "${source_reference}" == *"@${source_digest}" ]]
[[ "${platform}" == "linux/amd64" ]]

before_pids="$(pgrep -x firecracker || true)"
"${api_binary}" \
    --backend firecracker \
    --listen 127.0.0.1:18084 \
    --audit-log "${work_dir}/audit/events.jsonl" \
    --firecracker "${firecracker}" \
    --jailer "${jailer}" \
    --kernel "${kernel}" \
    --rootfs "${rootfs}" \
    --chroot-base "${chroot_base}" \
    --runtime-root "${runtime_root}" \
    >"${work_dir}/api.log" 2>&1 &
api_pid="$!"

for _ in $(seq 1 200); do
    if curl --fail --silent "${api_url}/healthz" >/dev/null; then
        break
    fi
    if ! kill -0 "${api_pid}" 2>/dev/null; then
        wait "${api_pid}"
    fi
    sleep 0.05
done
curl --fail --silent "${api_url}/healthz" >/dev/null

create_response="$(
    api_call create 201 \
        --header 'content-type: application/json' \
        --data '{"template":"oci-python","cpu_count":1,"memory_mb":512,"timeout_seconds":120,"network":{"internet_access":false}}' \
        "${api_url}/v1/sandboxes"
)"
sandbox_id="$(jq -er '.sandbox_id' <<<"${create_response}")"
token="$(jq -er '.token' <<<"${create_response}")"
[[ "$(jq -r '.state' <<<"${create_response}")" == "running" ]]
unset create_response

exec_payload="$(jq -n '{
    argv: [
        "python3",
        "-c",
        "import os, platform; assert os.getuid() == 1000; print(\"oci-python=\" + platform.python_version())"
    ],
    cwd: "/home/sandbox",
    environment: {},
    timeout_seconds: 30,
    max_output_bytes: 1048576
}')"
exec_response="$(
    api_call python-exec 200 \
        --header "authorization: Bearer ${token}" \
        --header 'content-type: application/json' \
        --data "${exec_payload}" \
        "${api_url}/v1/sandboxes/${sandbox_id}/commands"
)"
python_version="$(jq -er '.stdout | select(startswith("oci-python="))' <<<"${exec_response}")"
[[ "$(jq -r '.termination.kind' <<<"${exec_response}")" == "exited" ]]

true_response="$(
    api_call true-exec 200 \
        --header "authorization: Bearer ${token}" \
        --header 'content-type: application/json' \
        --data '{"argv":["/bin/true"],"cwd":"/home/sandbox","environment":{},"timeout_seconds":30,"max_output_bytes":1024}' \
        "${api_url}/v1/sandboxes/${sandbox_id}/commands"
)"
[[ "$(jq -r '.termination.kind' <<<"${true_response}")" == "exited" ]]

write_response="$(
    api_call file-write 200 \
        --request PUT \
        --header "authorization: Bearer ${token}" \
        --header 'content-type: application/json' \
        --data '{"path":"/home/sandbox/oci.txt","content_base64":"ZmVycm9ib3gtb2NpCg==","overwrite":false}' \
        "${api_url}/v1/sandboxes/${sandbox_id}/files"
)"
[[ "$(jq -r '.bytes_written' <<<"${write_response}")" == "13" ]]
read_response="$(
    api_call file-read 200 \
        --header "authorization: Bearer ${token}" \
        "${api_url}/v1/sandboxes/${sandbox_id}/files?path=%2Fhome%2Fsandbox%2Foci.txt"
)"
[[ "$(jq -r '.content_base64' <<<"${read_response}")" == "ZmVycm9ib3gtb2NpCg==" ]]
list_response="$(
    api_call directory-list 200 \
        --header "authorization: Bearer ${token}" \
        "${api_url}/v1/sandboxes/${sandbox_id}/directories?path=%2Fhome%2Fsandbox"
)"
jq --exit-status 'any(.entries[]; .name == "oci.txt" and .kind == "file")' \
    <<<"${list_response}" >/dev/null

api_call pause 204 \
    --request POST \
    --header "authorization: Bearer ${token}" \
    "${api_url}/v1/sandboxes/${sandbox_id}/pause" >/dev/null
paused_status="$(
    curl --silent --output /dev/null --write-out '%{http_code}' \
        --header "authorization: Bearer ${token}" \
        --header 'content-type: application/json' \
        --data "${exec_payload}" \
        "${api_url}/v1/sandboxes/${sandbox_id}/commands"
)"
[[ "${paused_status}" == "409" ]]
api_call resume 204 \
    --request POST \
    --header "authorization: Bearer ${token}" \
    "${api_url}/v1/sandboxes/${sandbox_id}/resume" >/dev/null
resumed_response="$(
    api_call resumed-exec 200 \
        --request POST \
        --header "authorization: Bearer ${token}" \
        --header 'content-type: application/json' \
        --data '{"argv":["/bin/true"],"cwd":"/home/sandbox","environment":{},"timeout_seconds":30,"max_output_bytes":1024}' \
        "${api_url}/v1/sandboxes/${sandbox_id}/commands"
)"
[[ "$(jq -r '.termination.kind' <<<"${resumed_response}")" == "exited" ]]

api_call delete 204 \
    --request DELETE \
    --header "authorization: Bearer ${token}" \
    "${api_url}/v1/sandboxes/${sandbox_id}" >/dev/null
deleted_status="$(
    curl --silent --output /dev/null --write-out '%{http_code}' \
        --header "authorization: Bearer ${token}" \
        "${api_url}/v1/sandboxes/${sandbox_id}"
)"
[[ "${deleted_status}" == "404" ]]
! grep --fixed-strings --quiet "${token}" "${work_dir}/audit/events.jsonl"
grep --fixed-strings --quiet '"operation":"delete"' "${work_dir}/audit/events.jsonl"

completed_id="${sandbox_id}"
sandbox_id=""
token=""
kill "${api_pid}"
wait "${api_pid}" || true
api_pid=""
sleep 1
after_pids="$(pgrep -x firecracker || true)"
[[ "${after_pids}" == "${before_pids}" ]]
if ip netns list | grep --quiet '^fb-'; then
    echo "Ferrobox network namespace leaked after OCI E2E" >&2
    exit 6
fi

jq -n \
    --arg github_sha "${GITHUB_SHA:-unknown}" \
    --arg image_reference "${source_reference}" \
    --arg platform "${platform}" \
    --arg source_digest "${source_digest}" \
    --arg manifest_digest "${manifest_digest}" \
    --arg sandbox_id "${completed_id}" \
    --arg python_version "${python_version}" \
    '{
        schema_version: 1,
        github_sha: $github_sha,
        image_reference: $image_reference,
        platform: $platform,
        source_digest: $source_digest,
        manifest_digest: $manifest_digest,
        sandbox_id: $sandbox_id,
        python_version: $python_version,
        checks: [
            "digest-bound-rootfs",
            "microvm-ready",
            "uid-1000-python",
            "argv-execution",
            "file-write-read-list",
            "pause-reject-resume",
            "delete-stale-handle",
            "credential-redaction",
            "process-cleanup",
            "network-resource-cleanup"
        ]
    }' >"${output}"

printf 'OCI KVM E2E passed for sandbox %s (%s)\n' "${completed_id}" "${python_version}"
