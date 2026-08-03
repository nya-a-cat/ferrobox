#!/usr/bin/env bash
set -euo pipefail

client_root="${1:?generated Python client root is required}"
evidence_path="${FERROBOX_OPENAPI_SDK_EVIDENCE:?evidence path is required}"
lock_path="${FERROBOX_OPENAPI_PYTHON_LOCK:?lock evidence path is required}"
freeze_path="${FERROBOX_OPENAPI_PYTHON_FREEZE:?consumer freeze evidence path is required}"
package_dir="${FERROBOX_OPENAPI_SDK_PACKAGE_DIR:?Python package directory is required}"
api_url="${FERROBOX_API_URL:?API URL is required}"
audit_path="${FERROBOX_AUDIT_LOG:?audit log path is required}"
repo_root="${GITHUB_WORKSPACE:?GITHUB_WORKSPACE is required}"
work_dir="$(mktemp -d)"
client_under_test="${work_dir}/client"
consumer_venv="${work_dir}/consumer-venv"

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
mkdir -p "${package_dir}"
export UV_PROJECT_ENVIRONMENT="${work_dir}/venv"
export UV_EXCLUDE_NEWER='2026-08-02T23:59:59Z'
uv --directory "${client_under_test}" lock --python 3.12
install -D -m 0644 "${client_under_test}/uv.lock" "${lock_path}"
uv --directory "${client_under_test}" build --wheel --out-dir "${package_dir}"
wheel="${package_dir}/ferrobox_client-0.1.0-py3-none-any.whl"
test -f "${wheel}"
uv venv --python 3.12 "${consumer_venv}"
uv pip install --python "${consumer_venv}/bin/python" "${wheel}"
uv pip freeze --python "${consumer_venv}/bin/python" >"${freeze_path}"

FERROBOX_API_URL="${api_url}" \
FERROBOX_AUDIT_LOG="${audit_path}" \
FERROBOX_OPENAPI_SDK_EVIDENCE="${evidence_path}" \
PYTHONDONTWRITEBYTECODE=1 \
    "${consumer_venv}/bin/python" "${repo_root}/scripts/e2e-openapi-python.py"
