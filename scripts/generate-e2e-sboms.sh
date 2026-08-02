#!/usr/bin/env bash
set -euo pipefail

if [[ "${EUID}" -ne 0 ]]; then
    echo "generate-e2e-sboms.sh must run as root" >&2
    exit 2
fi

syft_binary="${1:?usage: generate-e2e-sboms.sh SYFT ROOTFS IMAGE OUTPUT_DIR}"
rootfs_image="${2:?usage: generate-e2e-sboms.sh SYFT ROOTFS IMAGE OUTPUT_DIR}"
container_image="${3:?usage: generate-e2e-sboms.sh SYFT ROOTFS IMAGE OUTPUT_DIR}"
output_dir="${4:?usage: generate-e2e-sboms.sh SYFT ROOTFS IMAGE OUTPUT_DIR}"

syft_binary="$(realpath "${syft_binary}")"
rootfs_image="$(realpath "${rootfs_image}")"
output_dir="$(realpath -m "${output_dir}")"
[[ -x "${syft_binary}" ]]
[[ -f "${rootfs_image}" ]]
mkdir -p -- "${output_dir}"

mount_dir="$(mktemp -d)"
cache_dir="$(mktemp -d)"
cleanup() {
    if mountpoint --quiet "${mount_dir}"; then
        umount "${mount_dir}"
    fi
    rm -rf -- "${mount_dir}" "${cache_dir}"
}
trap cleanup EXIT

export SYFT_CACHE_DIR="${cache_dir}"
export SYFT_CHECK_FOR_APP_UPDATE=false
export SYFT_FORMAT_PRETTY=true

"${syft_binary}" scan dir:. \
    --exclude './.ci-artifacts/**' \
    --exclude './.git/**' \
    --exclude './target/**' \
    --source-name ferrobox-source \
    --source-version "${GITHUB_SHA:?GITHUB_SHA is required}" \
    --output "spdx-json=${output_dir}/ferrobox-source.spdx.json"

mount -o loop,ro,noload "${rootfs_image}" "${mount_dir}"
"${syft_binary}" scan "dir:${mount_dir}" \
    --source-name ferrobox-python-rootfs \
    --source-version "${GITHUB_SHA}" \
    --output "spdx-json=${output_dir}/ferrobox-rootfs.spdx.json"
umount "${mount_dir}"

"${syft_binary}" scan "docker:${container_image}" \
    --source-name ferrobox-python-comparator \
    --output "spdx-json=${output_dir}/python-image.spdx.json"

for sbom in \
    "${output_dir}/ferrobox-source.spdx.json" \
    "${output_dir}/ferrobox-rootfs.spdx.json" \
    "${output_dir}/python-image.spdx.json"; do
    jq --exit-status '
        .spdxVersion == "SPDX-2.3" and
        (.documentNamespace | type == "string" and length > 0) and
        (.packages | type == "array" and length > 0)
    ' "${sbom}" >/dev/null
    chmod 0644 "${sbom}"
done
