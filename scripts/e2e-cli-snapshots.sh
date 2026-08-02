#!/usr/bin/env bash
set -euo pipefail
umask 077

api_url="${FERROBOX_API_URL:?FERROBOX_API_URL is required}"
runtime_root="${FERROBOX_RUNTIME_ROOT:?FERROBOX_RUNTIME_ROOT is required}"
audit_log="${FERROBOX_AUDIT_LOG:?FERROBOX_AUDIT_LOG is required}"
evidence_path="${FERROBOX_SNAPSHOT_CLI_EVIDENCE:?FERROBOX_SNAPSHOT_CLI_EVIDENCE is required}"
work_dir="$(mktemp -d)"
snapshot_id=""
snapshot_token=""
sandbox_snapshot_ids=()
sandbox_snapshot_tokens=()
sandbox_ids=()
sandbox_tokens=()
all_tokens=()

delete_sandbox() {
    local sandbox_id="$1"
    local token="$2"
    FERROBOX_TOKEN="${token}" target/debug/ferrobox --api-url "${api_url}" \
        delete "${sandbox_id}" >/dev/null
}

cleanup() {
    status="$?"
    set +e
    for index in "${!sandbox_ids[@]}"; do
        delete_sandbox "${sandbox_ids[$index]}" "${sandbox_tokens[$index]}" \
            >/dev/null 2>&1
    done
    for index in "${!sandbox_snapshot_ids[@]}"; do
        if [[ -n "${sandbox_snapshot_ids[$index]}" ]]; then
            FERROBOX_SNAPSHOT_TOKEN="${sandbox_snapshot_tokens[$index]}" \
                target/debug/ferrobox --api-url "${api_url}" snapshot delete \
                "${sandbox_snapshot_ids[$index]}" >/dev/null 2>&1
        fi
    done
    rm -rf -- "${work_dir}"
    return "${status}"
}
trap cleanup EXIT

test -x target/debug/ferrobox
curl --fail --silent "${api_url}/healthz" >/dev/null

set +x
source_response="$(
    target/debug/ferrobox --api-url "${api_url}" create \
        --template python --cpu 1 --memory-mb 512 --ttl 600
)"
source_id="$(jq -er '.sandbox_id' <<<"${source_response}")"
source_token="$(jq -er '.token' <<<"${source_response}")"
sandbox_ids+=("${source_id}")
sandbox_tokens+=("${source_token}")
all_tokens+=("${source_token}")
unset source_response

printf 'captured\n' >"${work_dir}/captured.txt"
FERROBOX_TOKEN="${source_token}" target/debug/ferrobox --api-url "${api_url}" \
    write "${source_id}" /home/sandbox/state.txt "${work_dir}/captured.txt" \
    >/dev/null

snapshot_response="$(
    FERROBOX_TOKEN="${source_token}" target/debug/ferrobox --api-url "${api_url}" \
        snapshot create "${source_id}" --name cli-checkpoint
)"
snapshot_id="$(jq -er '.snapshot_id' <<<"${snapshot_response}")"
snapshot_token="$(jq -er '.token' <<<"${snapshot_response}")"
sandbox_snapshot_ids+=("${snapshot_id}")
sandbox_snapshot_tokens+=("${snapshot_token}")
all_tokens+=("${snapshot_token}")
[[ "$(jq -r '.source_state' <<<"${snapshot_response}")" == "running" ]]
[[ "${snapshot_token}" != "${source_token}" ]]
unset snapshot_response

secondary_response="$(
    FERROBOX_TOKEN="${source_token}" target/debug/ferrobox --api-url "${api_url}" \
        snapshot create "${source_id}" --name cli-pagination
)"
secondary_id="$(jq -er '.snapshot_id' <<<"${secondary_response}")"
secondary_token="$(jq -er '.token' <<<"${secondary_response}")"
sandbox_snapshot_ids+=("${secondary_id}")
sandbox_snapshot_tokens+=("${secondary_token}")
all_tokens+=("${secondary_token}")
[[ "${secondary_id}" != "${snapshot_id}" ]]
[[ "${secondary_token}" != "${snapshot_token}" ]]
unset secondary_response

first_page="$(
    FERROBOX_TOKEN="${source_token}" target/debug/ferrobox --api-url "${api_url}" \
        snapshot list "${source_id}" --limit 1
)"
[[ "$(jq -r '.snapshots[0].snapshot_id' <<<"${first_page}")" == "${snapshot_id}" ]]
[[ "$(jq -r '.next_cursor' <<<"${first_page}")" == "${snapshot_id}" ]]
second_page="$(
    FERROBOX_TOKEN="${source_token}" target/debug/ferrobox --api-url "${api_url}" \
        snapshot list "${source_id}" --limit 1 --cursor "${snapshot_id}"
)"
[[ "$(jq -r '.snapshots[0].snapshot_id' <<<"${second_page}")" == "${secondary_id}" ]]
[[ "$(jq -r '.next_cursor' <<<"${second_page}")" == "null" ]]
FERROBOX_SNAPSHOT_TOKEN="${secondary_token}" target/debug/ferrobox \
    --api-url "${api_url}" snapshot delete "${secondary_id}" >/dev/null
sandbox_snapshot_ids[1]=""
sandbox_snapshot_tokens[1]=""

inspect_response="$(
    FERROBOX_SNAPSHOT_TOKEN="${snapshot_token}" target/debug/ferrobox \
        --api-url "${api_url}" snapshot inspect "${snapshot_id}"
)"
[[ "$(jq -r '.snapshot_id' <<<"${inspect_response}")" == "${snapshot_id}" ]]
if FERROBOX_SNAPSHOT_TOKEN="${source_token}" target/debug/ferrobox \
    --api-url "${api_url}" snapshot inspect "${snapshot_id}" \
    >"${work_dir}/wrong-snapshot-token.log" 2>&1; then
    echo "sandbox token unexpectedly authorized snapshot inspection" >&2
    exit 1
fi
grep --fixed-strings --quiet '401 Unauthorized' "${work_dir}/wrong-snapshot-token.log"
if FERROBOX_TOKEN="${snapshot_token}" target/debug/ferrobox \
    --api-url "${api_url}" inspect "${source_id}" \
    >"${work_dir}/wrong-sandbox-token.log" 2>&1; then
    echo "snapshot token unexpectedly authorized sandbox inspection" >&2
    exit 1
fi
grep --fixed-strings --quiet '401 Unauthorized' "${work_dir}/wrong-sandbox-token.log"

verify_response="$(
    FERROBOX_SNAPSHOT_TOKEN="${snapshot_token}" target/debug/ferrobox \
        --api-url "${api_url}" snapshot verify "${snapshot_id}"
)"
[[ "$(jq -r '.valid' <<<"${verify_response}")" == "true" ]]
[[ "$(jq -r '.checked_artifacts' <<<"${verify_response}")" == "3" ]]

printf 'mutated\n' >"${work_dir}/mutated.txt"
FERROBOX_TOKEN="${source_token}" target/debug/ferrobox --api-url "${api_url}" \
    write "${source_id}" /home/sandbox/state.txt "${work_dir}/mutated.txt" \
    --overwrite >/dev/null
rollback_response="$(
    FERROBOX_TOKEN="${source_token}" target/debug/ferrobox --api-url "${api_url}" \
        snapshot rollback "${source_id}" "${snapshot_id}"
)"
[[ "$(jq -r '.sandbox_id' <<<"${rollback_response}")" == "${source_id}" ]]
[[ "$(jq -r '.state' <<<"${rollback_response}")" == "running" ]]
FERROBOX_TOKEN="${source_token}" target/debug/ferrobox --api-url "${api_url}" \
    read "${source_id}" /home/sandbox/state.txt --output "${work_dir}/rolled-back.txt"
cmp "${work_dir}/captured.txt" "${work_dir}/rolled-back.txt"

restore_response="$(
    FERROBOX_SNAPSHOT_TOKEN="${snapshot_token}" target/debug/ferrobox \
        --api-url "${api_url}" snapshot restore "${snapshot_id}" --ttl 600
)"
restored_id="$(jq -er '.sandbox_id' <<<"${restore_response}")"
restored_token="$(jq -er '.token' <<<"${restore_response}")"
[[ "${restored_id}" != "${source_id}" ]]
[[ "${restored_token}" != "${source_token}" ]]
[[ "${restored_token}" != "${snapshot_token}" ]]
sandbox_ids+=("${restored_id}")
sandbox_tokens+=("${restored_token}")
all_tokens+=("${restored_token}")
unset restore_response
FERROBOX_TOKEN="${restored_token}" target/debug/ferrobox --api-url "${api_url}" \
    read "${restored_id}" /home/sandbox/state.txt --output "${work_dir}/restored.txt"
cmp "${work_dir}/captured.txt" "${work_dir}/restored.txt"

clone_response="$(
    FERROBOX_SNAPSHOT_TOKEN="${snapshot_token}" target/debug/ferrobox \
        --api-url "${api_url}" snapshot clone "${snapshot_id}" --count 2 --ttl 600
)"
[[ "$(jq -r '.sandboxes | length' <<<"${clone_response}")" == "2" ]]
while IFS=$'\t' read -r clone_id clone_token; do
    sandbox_ids+=("${clone_id}")
    sandbox_tokens+=("${clone_token}")
    all_tokens+=("${clone_token}")
    FERROBOX_TOKEN="${clone_token}" target/debug/ferrobox --api-url "${api_url}" \
        read "${clone_id}" /home/sandbox/state.txt \
        --output "${work_dir}/clone-${clone_id}.txt"
    cmp "${work_dir}/captured.txt" "${work_dir}/clone-${clone_id}.txt"
done < <(jq -r '.sandboxes[] | [.sandbox_id, .token] | @tsv' <<<"${clone_response}")
unset clone_response
[[ "${#sandbox_ids[@]}" == "4" ]]
[[ "$(printf '%s\n' "${sandbox_ids[@]}" | sort --unique | wc -l)" == "4" ]]
[[ "$(printf '%s\n' "${all_tokens[@]}" | sort --unique | wc -l)" == "6" ]]

for index in "${!sandbox_ids[@]}"; do
    delete_sandbox "${sandbox_ids[$index]}" "${sandbox_tokens[$index]}"
done
FERROBOX_SNAPSHOT_TOKEN="${snapshot_token}" target/debug/ferrobox \
    --api-url "${api_url}" snapshot delete "${snapshot_id}" >/dev/null
sandbox_snapshot_ids[0]=""
sandbox_snapshot_tokens[0]=""

for token in "${all_tokens[@]}"; do
    ! grep --fixed-strings --quiet "${token}" "${audit_log}"
done
[[ -z "$(find "${runtime_root}/snapshots" -mindepth 1 -maxdepth 1 -print -quit)" ]]

jq --null-input \
    --arg source_sandbox_id "${source_id}" \
    --arg snapshot_id "${snapshot_id}" \
    --arg restored_sandbox_id "${restored_id}" \
    '{
      schema_version: 1,
      source_sandbox_id: $source_sandbox_id,
      snapshot_id: $snapshot_id,
      restored_sandbox_id: $restored_sandbox_id,
      clone_count: 2,
      checks: [
        "create-paginated-list-inspect-verify",
        "sandbox-snapshot-token-separation",
        "same-id-rollback",
        "independent-restore",
        "two-clone-file-state",
        "credential-redaction",
        "artifact-cleanup"
      ]
    }' >"${evidence_path}"

sandbox_ids=()
sandbox_tokens=()
sandbox_snapshot_ids=()
sandbox_snapshot_tokens=()
snapshot_id=""
snapshot_token=""
cat "${evidence_path}"
