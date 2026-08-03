#!/usr/bin/env bash
set -euo pipefail

client_root="${1:?generated Python client root is required}"
evidence_path="${FERROBOX_OPENAPI_SDK_EVIDENCE:?evidence path is required}"
lock_path="${FERROBOX_OPENAPI_PYTHON_LOCK:?lock evidence path is required}"
api_url="${FERROBOX_API_URL:?API URL is required}"
audit_path="${FERROBOX_AUDIT_LOG:?audit log path is required}"
repo_root="${GITHUB_WORKSPACE:?GITHUB_WORKSPACE is required}"
work_dir="$(mktemp -d)"
client_under_test="${work_dir}/client"

cleanup() {
    status="$?"
    set +e
    rm -rf -- "${work_dir}"
    return "${status}"
}
trap cleanup EXIT

test -x target/debug/ferrobox-api
test -f "${client_root}/pyproject.toml"
cp -a -- "${client_root}" "${client_under_test}"
export UV_PROJECT_ENVIRONMENT="${work_dir}/venv"
export UV_EXCLUDE_NEWER='2026-08-02T23:59:59Z'
uv --directory "${client_under_test}" lock --python 3.12
install -D -m 0644 "${client_under_test}/uv.lock" "${lock_path}"

FERROBOX_API_URL="${api_url}" \
FERROBOX_AUDIT_LOG="${audit_path}" \
FERROBOX_OPENAPI_SDK_EVIDENCE="${evidence_path}" \
PYTHONDONTWRITEBYTECODE=1 \
uv --directory "${client_under_test}" run --locked --python 3.12 \
    python "${repo_root}/scripts/e2e-openapi-python.py"
