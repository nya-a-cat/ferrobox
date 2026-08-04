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
template_store="${FERROBOX_TEMPLATE_STORE:?FERROBOX_TEMPLATE_STORE is required}"
fsverity="${FERROBOX_FSVERITY:?FERROBOX_FSVERITY is required}"
chroot_base="${FERROBOX_CHROOT_BASE:?FERROBOX_CHROOT_BASE is required}"
runtime_root="${FERROBOX_RUNTIME_ROOT:?FERROBOX_RUNTIME_ROOT is required}"
rootfs_evidence="${FERROBOX_OCI_ROOTFS_EVIDENCE:?FERROBOX_OCI_ROOTFS_EVIDENCE is required}"
template_record="${FERROBOX_OCI_TEMPLATE_RECORD:?FERROBOX_OCI_TEMPLATE_RECORD is required}"
fsverity_evidence="${FERROBOX_FSVERITY_EVIDENCE:?FERROBOX_FSVERITY_EVIDENCE is required}"
output="${FERROBOX_OCI_E2E_OUTPUT:?FERROBOX_OCI_E2E_OUTPUT is required}"
api_binary="${FERROBOX_API_BINARY:-target/debug/ferrobox-api}"
api_url="${FERROBOX_API_URL:-http://127.0.0.1:18084}"
profile="${FERROBOX_OCI_PROFILE:-python}"
expected_template_alias="${FERROBOX_TEMPLATE_ALIAS:-oci-python}"
sandbox_cpu_count="${FERROBOX_SANDBOX_CPU_COUNT:-1}"
sandbox_memory_mb="${FERROBOX_SANDBOX_MEMORY_MB:-512}"
sandbox_timeout_seconds="${FERROBOX_SANDBOX_TIMEOUT_SECONDS:-120}"
browser_expected_version="${FERROBOX_BROWSER_EXPECTED_VERSION:-}"
browser_fixture="${FERROBOX_BROWSER_FIXTURE:-scripts/fixtures/browser-smoke.html}"
work_dir="$(mktemp -d)"
api_pid=""
sandbox_id=""
token=""

case "${profile}" in
    python) [[ -z "${browser_expected_version}" ]] ;;
    browser) [[ "${browser_expected_version}" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] ;;
    *) echo "unsupported OCI KVM profile: ${profile}" >&2; exit 2 ;;
esac
[[ "${sandbox_cpu_count}" =~ ^[1-9][0-9]*$ ]]
[[ "${sandbox_memory_mb}" =~ ^[1-9][0-9]*$ ]]
[[ "${sandbox_timeout_seconds}" =~ ^[1-9][0-9]*$ ]]

mark_stage() {
    local stage="$1"
    printf '%s\n' "${stage}" >"${work_dir}/current-stage"
    echo "::notice title=OCI KVM stage::${stage}" >&2
}

record_stage_failure() {
    local stage="unknown"
    local response_path
    local failure_output
    if [[ -s "${work_dir}/current-stage" ]]; then
        stage="$(<"${work_dir}/current-stage")"
    fi
    response_path="${work_dir}/${stage}.response"
    failure_output="$(dirname -- "${output}")/${profile}-kvm-stage-failure.json"
    if [[ -s "${response_path}" ]] && jq -e . "${response_path}" >/dev/null 2>&1; then
        jq -c \
            --arg stage "${stage}" '
                {
                    stage: $stage,
                    response_present: true,
                    response: {
                        state: (.state // null),
                        termination: (.termination // null),
                        stdout: (.stdout // ""),
                        stderr: (.stderr // ""),
                        error: (
                            if .error then {
                                code: (.error.code // "unknown"),
                                message: (.error.message // "unknown")
                            } else null end
                        )
                    }
                }
            ' "${response_path}" >"${failure_output}"
    else
        jq -n -c \
            --arg stage "${stage}" '
                {
                    stage: $stage,
                    response_present: false
                }
            ' >"${failure_output}"
    fi
    echo "::group::Sanitized OCI KVM stage failure" >&2
    cat "${failure_output}" >&2
    echo "::endgroup::" >&2
}

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
        record_stage_failure
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
    failure_output="$(dirname -- "${output}")/${profile}-kvm-api-failure.json"
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
    mark_stage "${stage}"
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

require_zero_exit() {
    local stage="$1"
    local response="$2"
    if jq --exit-status '.termination == {kind: "exited", exit_code: 0}' \
        <<<"${response}" >/dev/null; then
        return 0
    fi
    echo "::group::Sanitized guest command failure" >&2
    jq -c \
        --arg stage "${stage}" '
            {
                stage: $stage,
                termination,
                stdout: (.stdout // ""),
                stderr: (.stderr // "")
            }
        ' <<<"${response}" >&2
    echo "::endgroup::" >&2
    return 1
}

for path in "${firecracker}" "${jailer}" "${api_binary}" "${fsverity}"; do
    test -x "${path}"
done
for path in "${kernel}" "${rootfs}" "${rootfs_evidence}" "${template_record}" "${fsverity_evidence}"; do
    test -f "${path}"
done
test -d "${template_store}"
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

template_id="$(jq -er '.record.template_id' "${template_record}")"
template_alias="$(jq -er '.record.alias' "${template_record}")"
template_spec_digest="$(jq -er '.record.spec_digest' "${template_record}")"
template_source_reference="$(jq -er '.record.descriptor.source.reference' "${template_record}")"
template_source_digest="$(jq -er '.record.descriptor.source.digest' "${template_record}")"
template_kernel_digest="$(jq -er '.record.descriptor.artifacts.kernel.digest' "${template_record}")"
template_rootfs_digest="$(jq -er '.record.descriptor.artifacts.rootfs.digest' "${template_record}")"
template_kernel_location="$(jq -er '.record.locations.kernel' "${template_record}")"
template_rootfs_location="$(jq -er '.record.locations.rootfs' "${template_record}")"
jq --exit-status '.record.status == "ready" and .verification.valid == true' \
    "${template_record}" >/dev/null
[[ "${template_id}" =~ ^tpl-[0-9a-f]{60}$ ]]
[[ "${template_alias}" == "${expected_template_alias}" ]]
[[ "${template_spec_digest}" =~ ^sha256:[0-9a-f]{64}$ ]]
[[ "${template_source_reference}" == "${source_reference}" ]]
[[ "${template_source_digest}" == "${manifest_digest}" ]]
configured_kernel_location="$(realpath "${kernel}")"
configured_rootfs_location="$(realpath "${rootfs}")"
configured_rootfs_size="$(stat --format='%s' "${rootfs}")"
configured_rootfs_digest="sha256:$(sha256sum "${rootfs}" | awk '{print $1}')"
test -f "${template_kernel_location}"
test -f "${template_rootfs_location}"
[[ "${template_kernel_location}" != "${configured_kernel_location}" ]]
[[ "${template_rootfs_location}" != "${configured_rootfs_location}" ]]
[[ "${configured_rootfs_size}" -lt 4096 ]]
[[ "${template_kernel_digest}" == "sha256:$(sha256sum "${kernel}" | awk '{print $1}')" ]]
[[ "${template_kernel_digest}" == "sha256:$(sha256sum "${template_kernel_location}" | awk '{print $1}')" ]]
[[ "${template_rootfs_digest}" == "sha256:$(sha256sum "${template_rootfs_location}" | awk '{print $1}')" ]]
[[ "${template_rootfs_digest}" != "${configured_rootfs_digest}" ]]

fsverity_contract="$(jq -er '.contract_version' "${fsverity_evidence}")"
fsverity_kernel_digest="$(jq -er \
    --arg path "${template_kernel_location}" \
    '.artifacts[] | select(.name == "kernel" and .path == $path) | .fsverity_digest' \
    "${fsverity_evidence}")"
fsverity_rootfs_digest="$(jq -er \
    --arg path "${template_rootfs_location}" \
    '.artifacts[] | select(.name == "rootfs" and .path == $path) | .fsverity_digest' \
    "${fsverity_evidence}")"
fsverity_kernel_p95_us="$(jq -er \
    '.artifacts[] | select(.name == "kernel") | .measure_p95_us' \
    "${fsverity_evidence}")"
fsverity_rootfs_p95_us="$(jq -er \
    '.artifacts[] | select(.name == "rootfs") | .measure_p95_us' \
    "${fsverity_evidence}")"
jq --arg github_sha "${GITHUB_SHA:?GITHUB_SHA is required}" --exit-status '
    .schema_version == 1 and
    .contract_version == "ferrobox-fsverity-evidence-v1" and
    .github.commit == $github_sha and
    (.artifacts | length) == 2 and
    all(.artifacts[];
        .filesystem == "btrfs" and
        .traditional_sha256_verified == true and
        .measurements_match_offline_digest == true and
        .write_rejected_errno > 0
    )
' "${fsverity_evidence}" >/dev/null
[[ "${fsverity_kernel_digest}" =~ ^sha256:[0-9a-f]{64}$ ]]
[[ "${fsverity_rootfs_digest}" =~ ^sha256:[0-9a-f]{64}$ ]]
current_kernel_verity_digest="$(
    "${fsverity}" measure "${template_kernel_location}" |
        awk 'NR == 1 { print $1 }'
)"
current_rootfs_verity_digest="$(
    "${fsverity}" measure "${template_rootfs_location}" |
        awk 'NR == 1 { print $1 }'
)"
[[ "${current_kernel_verity_digest}" == "${fsverity_kernel_digest}" ]]
[[ "${current_rootfs_verity_digest}" == "${fsverity_rootfs_digest}" ]]

before_pids="$(pgrep -x firecracker || true)"
"${api_binary}" \
    --backend firecracker \
    --listen 127.0.0.1:18084 \
    --audit-log "${work_dir}/audit/events.jsonl" \
    --firecracker "${firecracker}" \
    --jailer "${jailer}" \
    --kernel "${kernel}" \
    --rootfs "${rootfs}" \
    --template-store "${template_store}" \
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

missing_template_id="tpl-$(printf '0%.0s' {1..60})"
[[ "${missing_template_id}" != "${template_id}" ]]
missing_payload="$(jq -n \
    --arg template "${missing_template_id}" \
    --argjson cpu_count "${sandbox_cpu_count}" \
    --argjson memory_mb "${sandbox_memory_mb}" \
    --argjson timeout_seconds "${sandbox_timeout_seconds}" '{
    template: $template,
    cpu_count: $cpu_count,
    memory_mb: $memory_mb,
    timeout_seconds: $timeout_seconds,
    network: {internet_access: false}
}')"
missing_response="$(
    api_call missing-template 404 \
        --header 'content-type: application/json' \
        --data "${missing_payload}" \
        "${api_url}/v1/sandboxes"
)"
jq --exit-status '.error.code == "not_found"' <<<"${missing_response}" >/dev/null

missing_alias="missing-catalog-alias"
missing_alias_payload="$(jq -n \
    --arg template "${missing_alias}" \
    --argjson cpu_count "${sandbox_cpu_count}" \
    --argjson memory_mb "${sandbox_memory_mb}" \
    --argjson timeout_seconds "${sandbox_timeout_seconds}" '{
    template: $template,
    cpu_count: $cpu_count,
    memory_mb: $memory_mb,
    timeout_seconds: $timeout_seconds,
    network: {internet_access: false}
}')"
missing_alias_response="$(
    api_call missing-template-alias 404 \
        --header 'content-type: application/json' \
        --data "${missing_alias_payload}" \
        "${api_url}/v1/sandboxes"
)"
jq --exit-status '.error.code == "not_found"' <<<"${missing_alias_response}" >/dev/null

create_payload="$(jq -n \
    --arg template "${template_alias}" \
    --argjson cpu_count "${sandbox_cpu_count}" \
    --argjson memory_mb "${sandbox_memory_mb}" \
    --argjson timeout_seconds "${sandbox_timeout_seconds}" '{
    template: $template,
    cpu_count: $cpu_count,
    memory_mb: $memory_mb,
    timeout_seconds: $timeout_seconds,
    network: {internet_access: false}
}')"

create_response="$(
    api_call create 201 \
        --header 'content-type: application/json' \
        --data "${create_payload}" \
        "${api_url}/v1/sandboxes"
)"
sandbox_id="$(jq -er '.sandbox_id' <<<"${create_response}")"
token="$(jq -er '.token' <<<"${create_response}")"
[[ "$(jq -r '.state' <<<"${create_response}")" == "running" ]]
unset create_response

python_version=""
browser_process_uid=""
chromium_path=""
chromium_version=""
headless_shell_path=""
headless_shell_version=""
dom_marker=""
browser_fixture_sha256=""
screenshot_sha256=""
screenshot_size_bytes=0
screenshot_width=0
screenshot_height=0
screenshot_byte_identical_twice=false
sandbox_bypass_flag_present=false

if [[ "${profile}" == "python" ]]; then
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
    require_zero_exit python-exec "${exec_response}"

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
else
    uid_payload='{"argv":["id","-u"],"cwd":"/home/sandbox","environment":{},"timeout_seconds":30,"max_output_bytes":1024}'
    uid_response="$(
        api_call browser-uid 200 \
            --header "authorization: Bearer ${token}" \
            --header 'content-type: application/json' \
            --data "${uid_payload}" \
            "${api_url}/v1/sandboxes/${sandbox_id}/commands"
    )"
    require_zero_exit browser-uid "${uid_response}"
    browser_process_uid="$(jq -er '.stdout | sub("\\n$"; "") | tonumber' \
        <<<"${uid_response}")"
    [[ "${browser_process_uid}" == "1000" ]]

    discover_payload="$(jq -n '{
        argv: [
            "/bin/sh",
            "-c",
            "full=\"\"; for browser in /ms-playwright/chromium-*/chrome-linux/chrome /ms-playwright/chromium-*/chrome-linux64/chrome; do if [ -x \"$browser\" ]; then full=\"$browser\"; break; fi; done; shell=\"\"; for browser in /ms-playwright/chromium_headless_shell-*/chrome-headless-shell-linux64/chrome-headless-shell; do if [ -x \"$browser\" ]; then shell=\"$browser\"; break; fi; done; [ -n \"$full\" ] && [ -n \"$shell\" ] || exit 1; printf \"%s\\n%s\\n\" \"$full\" \"$shell\""
        ],
        cwd: "/home/sandbox",
        environment: {},
        timeout_seconds: 30,
        max_output_bytes: 4096
    }')"
    discover_response="$(
        api_call browser-discover 200 \
            --header "authorization: Bearer ${token}" \
            --header 'content-type: application/json' \
            --data "${discover_payload}" \
            "${api_url}/v1/sandboxes/${sandbox_id}/commands"
    )"
    require_zero_exit browser-discover "${discover_response}"
    chromium_path="$(jq -er '.stdout | split("\n") | map(select(length > 0)) | .[0]' \
        <<<"${discover_response}")"
    headless_shell_path="$(jq -er \
        '.stdout | split("\n") | map(select(length > 0)) | .[1]' \
        <<<"${discover_response}")"
    [[ "${chromium_path}" == /ms-playwright/chromium-*/chrome-linux*/chrome ]]
    [[ "${headless_shell_path}" == \
        /ms-playwright/chromium_headless_shell-*/chrome-headless-shell-linux64/chrome-headless-shell ]]

    version_payload="$(jq -n --arg chromium "${chromium_path}" '{
        argv: [$chromium, "--version"],
        cwd: "/home/sandbox",
        environment: {},
        timeout_seconds: 30,
        max_output_bytes: 4096
    }')"
    version_response="$(
        api_call browser-version 200 \
            --header "authorization: Bearer ${token}" \
            --header 'content-type: application/json' \
            --data "${version_payload}" \
            "${api_url}/v1/sandboxes/${sandbox_id}/commands"
    )"
    require_zero_exit browser-version "${version_response}"
    chromium_version="$(jq -er \
        '.stdout | capture("(?<version>[0-9]+\\.[0-9]+\\.[0-9]+\\.[0-9]+)").version' \
        <<<"${version_response}")"
    [[ "${chromium_version}" == "${browser_expected_version}" ]]

    headless_version_payload="$(jq -n --arg chromium "${headless_shell_path}" '{
        argv: [$chromium, "--version"],
        cwd: "/home/sandbox",
        environment: {},
        timeout_seconds: 30,
        max_output_bytes: 4096
    }')"
    headless_version_response="$(
        api_call browser-headless-version 200 \
            --header "authorization: Bearer ${token}" \
            --header 'content-type: application/json' \
            --data "${headless_version_payload}" \
            "${api_url}/v1/sandboxes/${sandbox_id}/commands"
    )"
    require_zero_exit browser-headless-version "${headless_version_response}"
    headless_shell_version="$(jq -er \
        '.stdout | capture("(?<version>[0-9]+\\.[0-9]+\\.[0-9]+\\.[0-9]+)").version' \
        <<<"${headless_version_response}")"
    [[ "${headless_shell_version}" == "${browser_expected_version}" ]]
    exec_payload="${version_payload}"

    test -f "${browser_fixture}"
    browser_fixture_sha256="$(sha256sum "${browser_fixture}" | awk '{print $1}')"
    browser_html_base64="$(base64 --wrap=0 "${browser_fixture}")"
    browser_html_size="$(stat --format='%s' "${browser_fixture}")"
    html_write_payload="$(jq -n \
        --arg content "${browser_html_base64}" '{
            path: "/home/sandbox/browser-smoke.html",
            content_base64: $content,
            overwrite: false
        }
    ')"
    html_write_response="$(
        api_call browser-html-write 200 \
            --request PUT \
            --header "authorization: Bearer ${token}" \
            --header 'content-type: application/json' \
            --data "${html_write_payload}" \
            "${api_url}/v1/sandboxes/${sandbox_id}/files"
    )"
    [[ "$(jq -r '.bytes_written' <<<"${html_write_response}")" == "${browser_html_size}" ]]

    assert_browser_sandbox_args() {
        local payload="$1"
        local bypass_flag
        for bypass_flag in --no-sandbox --disable-setuid-sandbox; do
            if grep --fixed-strings --quiet -- "${bypass_flag}" <<<"${payload}"; then
                echo "browser smoke contains sandbox bypass flag: ${bypass_flag}" >&2
                return 7
            fi
        done
    }

    dom_payload="$(jq -n --arg chromium "${headless_shell_path}" '{
        argv: [
            $chromium,
            "--disable-gpu",
            "--disable-dev-shm-usage",
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-features=OptimizationHints",
            "--hide-scrollbars",
            "--no-first-run",
            "--no-default-browser-check",
            "--force-device-scale-factor=1",
            "--timeout=10000",
            "--user-data-dir=/home/sandbox/chromium-dom-profile",
            "--dump-dom",
            "file:///home/sandbox/browser-smoke.html"
        ],
        cwd: "/home/sandbox",
        environment: {HOME: "/home/sandbox"},
        timeout_seconds: 60,
        max_output_bytes: 1048576
    }')"
    assert_browser_sandbox_args "${dom_payload}"
    dom_response="$(
        api_call browser-dom 200 \
            --header "authorization: Bearer ${token}" \
            --header 'content-type: application/json' \
            --data "${dom_payload}" \
            "${api_url}/v1/sandboxes/${sandbox_id}/commands"
    )"
    require_zero_exit browser-dom "${dom_response}"
    dom_stdout="$(jq -er '.stdout' <<<"${dom_response}")"
    grep --fixed-strings --quiet 'data-ferrobox-js="ferrobox-js-ok"' <<<"${dom_stdout}"
    grep --fixed-strings --quiet 'ferrobox-browser-kvm:executed' <<<"${dom_stdout}"
    printf '%s\n' "${dom_stdout}" >"$(dirname -- "${output}")/browser-dom.html"
    dom_marker=ferrobox-js-ok

    run_browser_screenshot() {
        local stage="$1"
        local profile_dir="$2"
        local screenshot_path="$3"
        local payload
        local response
        payload="$(jq -n \
            --arg chromium "${headless_shell_path}" \
            --arg profile_dir "${profile_dir}" \
            --arg screenshot_path "${screenshot_path}" '{
                argv: [
                    $chromium,
                    "--disable-gpu",
                    "--disable-dev-shm-usage",
                    "--disable-background-networking",
                    "--disable-component-update",
                    "--disable-features=OptimizationHints",
                    "--hide-scrollbars",
                    "--no-first-run",
                    "--no-default-browser-check",
                    "--force-device-scale-factor=1",
                    "--timeout=10000",
                    "--window-size=800,600",
                    ("--user-data-dir=" + $profile_dir),
                    ("--screenshot=" + $screenshot_path),
                    "file:///home/sandbox/browser-smoke.html"
                ],
                cwd: "/home/sandbox",
                environment: {HOME: "/home/sandbox"},
                timeout_seconds: 60,
                max_output_bytes: 1048576
            }')"
        assert_browser_sandbox_args "${payload}"
        response="$(
            api_call "${stage}" 200 \
                --header "authorization: Bearer ${token}" \
                --header 'content-type: application/json' \
                --data "${payload}" \
                "${api_url}/v1/sandboxes/${sandbox_id}/commands"
        )"
        require_zero_exit "${stage}" "${response}"
    }

    run_browser_screenshot \
        browser-screenshot-a \
        /home/sandbox/chromium-screenshot-a-profile \
        /home/sandbox/browser-a.png
    run_browser_screenshot \
        browser-screenshot-b \
        /home/sandbox/chromium-screenshot-b-profile \
        /home/sandbox/browser-b.png

    screenshot_a_response="$(
        api_call browser-screenshot-read-a 200 \
            --header "authorization: Bearer ${token}" \
            "${api_url}/v1/sandboxes/${sandbox_id}/files?path=%2Fhome%2Fsandbox%2Fbrowser-a.png"
    )"
    screenshot_b_response="$(
        api_call browser-screenshot-read-b 200 \
            --header "authorization: Bearer ${token}" \
            "${api_url}/v1/sandboxes/${sandbox_id}/files?path=%2Fhome%2Fsandbox%2Fbrowser-b.png"
    )"
    jq -er '.content_base64' <<<"${screenshot_a_response}" \
        | base64 --decode >"${work_dir}/browser-a.png"
    jq -er '.content_base64' <<<"${screenshot_b_response}" \
        | base64 --decode >"${work_dir}/browser-b.png"
    screenshot_size_bytes="$(stat --format='%s' "${work_dir}/browser-a.png")"
    [[ "${screenshot_size_bytes}" -ge 1024 ]]
    [[ "${screenshot_size_bytes}" -le 1048576 ]]
    screenshot_sha256="$(sha256sum "${work_dir}/browser-a.png" | awk '{print $1}')"
    screenshot_b_sha256="$(sha256sum "${work_dir}/browser-b.png" | awk '{print $1}')"
    [[ "${screenshot_sha256}" == "${screenshot_b_sha256}" ]]
    cmp --silent "${work_dir}/browser-a.png" "${work_dir}/browser-b.png"
    screenshot_byte_identical_twice=true
    read -r screenshot_width screenshot_height < <(
        python3 - "${work_dir}/browser-a.png" <<'PY'
import struct
import sys

data = open(sys.argv[1], "rb").read(24)
if len(data) != 24 or data[:8] != b"\x89PNG\r\n\x1a\n" or data[12:16] != b"IHDR":
    raise SystemExit("invalid PNG signature or IHDR")
print(*struct.unpack(">II", data[16:24]))
PY
    )
    [[ "${screenshot_width}" == "800" ]]
    [[ "${screenshot_height}" == "600" ]]
    install -m 0444 \
        "${work_dir}/browser-a.png" \
        "$(dirname -- "${output}")/browser-screenshot.png"

    list_response="$(
        api_call browser-directory-list 200 \
            --header "authorization: Bearer ${token}" \
            "${api_url}/v1/sandboxes/${sandbox_id}/directories?path=%2Fhome%2Fsandbox"
    )"
    jq --exit-status '
        any(.entries[]; .name == "browser-smoke.html" and .kind == "file") and
        any(.entries[]; .name == "browser-a.png" and .kind == "file") and
        any(.entries[]; .name == "browser-b.png" and .kind == "file")
    ' <<<"${list_response}" >/dev/null
fi

true_response="$(
    api_call true-exec 200 \
        --header "authorization: Bearer ${token}" \
        --header 'content-type: application/json' \
        --data '{"argv":["/bin/true"],"cwd":"/home/sandbox","environment":{},"timeout_seconds":30,"max_output_bytes":1024}' \
        "${api_url}/v1/sandboxes/${sandbox_id}/commands"
)"
require_zero_exit true-exec "${true_response}"

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
grep --fixed-strings --quiet "${template_alias}" "${work_dir}/audit/events.jsonl"
grep --fixed-strings --quiet '"operation":"delete"' "${work_dir}/audit/events.jsonl"
resolution_log="$(
    grep --fixed-strings 'resolved immutable template alias' "${work_dir}/api.log"
)"
grep --fixed-strings --quiet "${template_alias}" <<<"${resolution_log}"
grep --fixed-strings --quiet "${template_id}" <<<"${resolution_log}"

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

if [[ "${profile}" == "python" ]]; then
    jq -n \
        --arg github_sha "${GITHUB_SHA:-unknown}" \
        --arg image_reference "${source_reference}" \
        --arg platform "${platform}" \
        --arg source_digest "${source_digest}" \
        --arg manifest_digest "${manifest_digest}" \
        --arg template_id "${template_id}" \
        --arg template_alias "${template_alias}" \
        --arg template_spec_digest "${template_spec_digest}" \
        --arg template_source_reference "${template_source_reference}" \
        --arg template_source_digest "${template_source_digest}" \
        --arg configured_kernel_location "${configured_kernel_location}" \
        --arg configured_rootfs_location "${configured_rootfs_location}" \
        --arg configured_rootfs_digest "${configured_rootfs_digest}" \
        --arg template_kernel_location "${template_kernel_location}" \
        --arg template_rootfs_location "${template_rootfs_location}" \
        --arg fsverity_contract "${fsverity_contract}" \
        --arg fsverity_kernel_digest "${fsverity_kernel_digest}" \
        --arg fsverity_rootfs_digest "${fsverity_rootfs_digest}" \
        --argjson fsverity_kernel_p95_us "${fsverity_kernel_p95_us}" \
        --argjson fsverity_rootfs_p95_us "${fsverity_rootfs_p95_us}" \
        --argjson configured_rootfs_size "${configured_rootfs_size}" \
        --arg sandbox_id "${completed_id}" \
        --arg python_version "${python_version}" \
        '{
            schema_version: 1,
            github_sha: $github_sha,
            image_reference: $image_reference,
            platform: $platform,
            source_digest: $source_digest,
            manifest_digest: $manifest_digest,
            template_id: $template_id,
            template_alias: $template_alias,
            template_spec_digest: $template_spec_digest,
            template_source_reference: $template_source_reference,
            template_source_digest: $template_source_digest,
            runtime_integrity: {
                contract_version: $fsverity_contract,
                source_assets_fsverity_enabled: true,
                kernel_digest: $fsverity_kernel_digest,
                rootfs_digest: $fsverity_rootfs_digest,
                kernel_measure_p95_us: $fsverity_kernel_p95_us,
                rootfs_measure_p95_us: $fsverity_rootfs_p95_us
            },
            runtime_selection: {
                requested_template_alias: $template_alias,
                resolved_template_id: $template_id,
                configured_kernel: $configured_kernel_location,
                configured_rootfs: $configured_rootfs_location,
                configured_rootfs_digest: $configured_rootfs_digest,
                configured_rootfs_size_bytes: $configured_rootfs_size,
                resolved_kernel: $template_kernel_location,
                resolved_rootfs: $template_rootfs_location,
                locations_distinct: (
                    $configured_kernel_location != $template_kernel_location and
                    $configured_rootfs_location != $template_rootfs_location
                )
            },
            sandbox_id: $sandbox_id,
            python_version: $python_version,
            checks: [
                "digest-bound-rootfs",
                "content-derived-template-identity",
                "template-runtime-artifact-match",
                "unknown-template-id-rejected",
                "unknown-template-alias-rejected",
                "api-template-alias-resolution",
                "alias-canonicalized-to-content-id",
                "catalog-assets-override-configured-fallback",
                "fs-verity-source-assets",
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
    printf 'OCI KVM E2E passed for sandbox %s (%s)\n' \
        "${completed_id}" "${python_version}"
else
    jq -n \
        --arg github_sha "${GITHUB_SHA:-unknown}" \
        --arg image_reference "${source_reference}" \
        --arg platform "${platform}" \
        --arg source_digest "${source_digest}" \
        --arg manifest_digest "${manifest_digest}" \
        --arg template_id "${template_id}" \
        --arg template_alias "${template_alias}" \
        --arg template_spec_digest "${template_spec_digest}" \
        --arg template_source_reference "${template_source_reference}" \
        --arg template_source_digest "${template_source_digest}" \
        --arg configured_kernel_location "${configured_kernel_location}" \
        --arg configured_rootfs_location "${configured_rootfs_location}" \
        --arg configured_rootfs_digest "${configured_rootfs_digest}" \
        --arg template_kernel_location "${template_kernel_location}" \
        --arg template_rootfs_location "${template_rootfs_location}" \
        --arg fsverity_contract "${fsverity_contract}" \
        --arg fsverity_kernel_digest "${fsverity_kernel_digest}" \
        --arg fsverity_rootfs_digest "${fsverity_rootfs_digest}" \
        --argjson fsverity_kernel_p95_us "${fsverity_kernel_p95_us}" \
        --argjson fsverity_rootfs_p95_us "${fsverity_rootfs_p95_us}" \
        --argjson configured_rootfs_size "${configured_rootfs_size}" \
        --arg sandbox_id "${completed_id}" \
        --arg chromium_path "${chromium_path}" \
        --arg chromium_version "${chromium_version}" \
        --arg headless_shell_path "${headless_shell_path}" \
        --arg headless_shell_version "${headless_shell_version}" \
        --arg dom_marker "${dom_marker}" \
        --arg browser_fixture_sha256 "${browser_fixture_sha256}" \
        --arg screenshot_sha256 "${screenshot_sha256}" \
        --argjson browser_process_uid "${browser_process_uid}" \
        --argjson sandbox_bypass_flag_present "${sandbox_bypass_flag_present}" \
        --argjson screenshot_size_bytes "${screenshot_size_bytes}" \
        --argjson screenshot_width "${screenshot_width}" \
        --argjson screenshot_height "${screenshot_height}" \
        --argjson screenshot_byte_identical_twice "${screenshot_byte_identical_twice}" \
        '{
            schema_version: 1,
            contract_version: "ferrobox-browser-kvm-evidence-v1",
            github_sha: $github_sha,
            image_reference: $image_reference,
            platform: $platform,
            source_digest: $source_digest,
            manifest_digest: $manifest_digest,
            template_id: $template_id,
            template_alias: $template_alias,
            template_spec_digest: $template_spec_digest,
            template_source_reference: $template_source_reference,
            template_source_digest: $template_source_digest,
            runtime_integrity: {
                contract_version: $fsverity_contract,
                source_assets_fsverity_enabled: true,
                kernel_digest: $fsverity_kernel_digest,
                rootfs_digest: $fsverity_rootfs_digest,
                kernel_measure_p95_us: $fsverity_kernel_p95_us,
                rootfs_measure_p95_us: $fsverity_rootfs_p95_us
            },
            runtime_selection: {
                requested_template_alias: $template_alias,
                resolved_template_id: $template_id,
                configured_kernel: $configured_kernel_location,
                configured_rootfs: $configured_rootfs_location,
                configured_rootfs_digest: $configured_rootfs_digest,
                configured_rootfs_size_bytes: $configured_rootfs_size,
                resolved_kernel: $template_kernel_location,
                resolved_rootfs: $template_rootfs_location,
                locations_distinct: (
                    $configured_kernel_location != $template_kernel_location and
                    $configured_rootfs_location != $template_rootfs_location
                )
            },
            sandbox_id: $sandbox_id,
            browser: {
                process_uid: $browser_process_uid,
                executable: $headless_shell_path,
                chromium_version: $headless_shell_version,
                full_chromium_executable: $chromium_path,
                full_chromium_version: $chromium_version,
                execution_engine: "chromium-headless-shell",
                headless_mode: "playwright-default-shell",
                sandbox_bypass_flag_present: $sandbox_bypass_flag_present,
                network_enabled: false,
                dom: {
                    url: "file:///home/sandbox/browser-smoke.html",
                    source_fixture_sha256: $browser_fixture_sha256,
                    javascript_marker: $dom_marker,
                    retained_artifact: "browser-dom.html"
                },
                screenshot: {
                    path: "/home/sandbox/browser-a.png",
                    retained_artifact: "browser-screenshot.png",
                    size_bytes: $screenshot_size_bytes,
                    sha256: $screenshot_sha256,
                    png_signature_verified: true,
                    width: $screenshot_width,
                    height: $screenshot_height,
                    byte_identical_twice: $screenshot_byte_identical_twice
                }
            },
            checks: [
                "digest-bound-rootfs",
                "content-derived-template-identity",
                "template-runtime-artifact-match",
                "unknown-template-id-rejected",
                "unknown-template-alias-rejected",
                "api-template-alias-resolution",
                "alias-canonicalized-to-content-id",
                "catalog-assets-override-configured-fallback",
                "fs-verity-source-assets",
                "microvm-ready",
                "uid-1000-browser",
                "chromium-version-pinned",
                "chromium-sandbox-required",
                "network-disabled",
                "offline-local-document",
                "fixture-source-bound",
                "javascript-dom-execution",
                "screenshot-file-api",
                "png-signature-and-dimensions",
                "screenshot-byte-identical-twice",
                "retained-browser-artifacts",
                "pause-reject-resume",
                "delete-stale-handle",
                "credential-redaction",
                "process-cleanup",
                "network-resource-cleanup"
            ]
        }' >"${output}"
    printf 'Browser KVM E2E passed for sandbox %s (Chromium %s, Playwright headless shell %s)\n' \
        "${completed_id}" "${chromium_version}" "${headless_shell_version}"
fi
