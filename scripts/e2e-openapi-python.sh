#!/usr/bin/env bash
set -euo pipefail

client_root="${1:?generated Python client root is required}"
evidence_path="${FERROBOX_OPENAPI_PYTHON_EVIDENCE:?evidence path is required}"
api_url="${FERROBOX_API_URL:-http://127.0.0.1:18083}"
repo_root="${GITHUB_WORKSPACE:?GITHUB_WORKSPACE is required}"
work_dir="$(mktemp -d)"
api_pid=""

cleanup() {
    status="$?"
    set +e
    if [[ "${status}" -ne 0 && -f "${work_dir}/api.log" ]]; then
        echo "::group::Generated OpenAPI Python E2E API log"
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

test -x target/debug/ferrobox-api
test -f "${client_root}/pyproject.toml"
export UV_PROJECT_ENVIRONMENT="${work_dir}/venv"
export UV_EXCLUDE_NEWER='2026-08-02T23:59:59Z'
uv --directory "${client_root}" lock --python 3.12

target/debug/ferrobox-api \
    --backend process \
    --unsafe-process-runtime \
    --listen 127.0.0.1:18083 \
    --process-root "${work_dir}/sandboxes" \
    --audit-log "${work_dir}/audit/events.jsonl" \
    >"${work_dir}/api.log" 2>&1 &
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

FERROBOX_API_URL="${api_url}" \
FERROBOX_AUDIT_LOG="${work_dir}/audit/events.jsonl" \
FERROBOX_OPENAPI_PYTHON_EVIDENCE="${evidence_path}" \
uv --directory "${client_root}" run --locked --python 3.12 \
    python "${repo_root}/scripts/e2e-openapi-python.py"
