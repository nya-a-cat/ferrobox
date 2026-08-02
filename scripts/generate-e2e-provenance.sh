#!/usr/bin/env bash
set -euo pipefail

output="${1:?usage: generate-e2e-provenance.sh OUTPUT SBOM_DIR}"
sbom_dir="${2:?usage: generate-e2e-provenance.sh OUTPUT SBOM_DIR}"
expected_image_digest="${FERROBOX_DOCKER_IMAGE_DIGEST:?FERROBOX_DOCKER_IMAGE_DIGEST is required}"
container_image="${FERROBOX_DOCKER_IMAGE:?FERROBOX_DOCKER_IMAGE is required}"

output="$(realpath -m "${output}")"
sbom_dir="$(realpath "${sbom_dir}")"
staging="$(mktemp -d)"
cleanup() {
    rm -rf -- "${staging}"
}
trap cleanup EXIT

subjects="${staging}/subjects.jsonl"
executed_files="${staging}/executed-files.jsonl"
guest_assets="${staging}/guest-assets.jsonl"
sboms="${staging}/sboms.jsonl"
: >"${subjects}"
: >"${executed_files}"
: >"${guest_assets}"
: >"${sboms}"

file_record() {
    local name="$1"
    local category="$2"
    local path="$3"
    local destination="$4"
    local canonical sha256 size_bytes

    canonical="$(realpath "${path}")"
    [[ -f "${canonical}" ]]
    sha256="$(sha256sum "${canonical}" | awk '{print $1}')"
    size_bytes="$(stat --format='%s' "${canonical}")"
    jq --compact-output --null-input \
        --arg name "${name}" \
        --arg category "${category}" \
        --arg path "${canonical}" \
        --arg sha256 "${sha256}" \
        --argjson size_bytes "${size_bytes}" \
        '{
            name: $name,
            category: $category,
            path: $path,
            size_bytes: $size_bytes,
            digest: {sha256: $sha256}
        }' >>"${destination}"
}

add_subject() {
    local name="$1"
    local path="$2"
    local canonical sha256
    canonical="$(realpath "${path}")"
    [[ -f "${canonical}" ]]
    sha256="$(sha256sum "${canonical}" | awk '{print $1}')"
    jq --compact-output --null-input \
        --arg name "${name}" \
        --arg sha256 "${sha256}" \
        '{name: $name, digest: {sha256: $sha256}}' >>"${subjects}"
}

add_executable() {
    local name="$1"
    local category="$2"
    local path="$3"
    [[ -x "${path}" ]]
    file_record "${name}" "${category}" "${path}" "${executed_files}"
}

add_sbom() {
    local name="$1"
    local path="$2"
    local canonical sha256 size_bytes package_count namespace
    canonical="$(realpath "${path}")"
    jq --exit-status '.spdxVersion == "SPDX-2.3" and (.packages | length > 0)' \
        "${canonical}" >/dev/null
    sha256="$(sha256sum "${canonical}" | awk '{print $1}')"
    size_bytes="$(stat --format='%s' "${canonical}")"
    package_count="$(jq '.packages | length' "${canonical}")"
    namespace="$(jq --raw-output '.documentNamespace' "${canonical}")"
    jq --compact-output --null-input \
        --arg name "${name}" \
        --arg path "${canonical}" \
        --arg sha256 "${sha256}" \
        --arg namespace "${namespace}" \
        --argjson size_bytes "${size_bytes}" \
        --argjson package_count "${package_count}" \
        '{
            name: $name,
            format: "SPDX-2.3 JSON",
            path: $path,
            size_bytes: $size_bytes,
            package_count: $package_count,
            document_namespace: $namespace,
            digest: {sha256: $sha256}
        }' >>"${sboms}"
}

add_subject ferrobox-node target/debug/ferrobox-node
add_subject ferrobox-api target/debug/ferrobox-api
add_subject ferrobox-guest target/x86_64-unknown-linux-musl/release/ferrobox-guest
add_subject microvm-probe target/debug/microvm-probe
add_subject python.ext4 /opt/ferrobox/images/python.ext4

add_executable ferrobox-node ferrobox target/debug/ferrobox-node
add_executable ferrobox-api ferrobox target/debug/ferrobox-api
add_executable ferrobox-guest ferrobox target/x86_64-unknown-linux-musl/release/ferrobox-guest
add_executable microvm-probe ferrobox target/debug/microvm-probe
add_executable firecracker isolation /opt/ferrobox/bin/firecracker
add_executable jailer isolation /opt/ferrobox/bin/jailer
add_executable cloud-hypervisor comparator /opt/ferrobox/bin/cloud-hypervisor
add_executable runsc comparator /usr/local/bin/runsc
add_executable containerd-shim-runsc-v1 comparator /usr/local/bin/containerd-shim-runsc-v1
add_executable kata-runtime comparator /opt/kata/bin/kata-runtime
add_executable containerd-shim-kata-v2 comparator /opt/kata/bin/containerd-shim-kata-v2
add_executable docker comparator "$(command -v docker)"
add_executable dockerd comparator "$(command -v dockerd)"
add_executable runc comparator "$(command -v runc)"
add_executable containerd comparator "$(command -v containerd)"
add_executable ctr comparator "$(command -v ctr)"
add_executable syft evidence "${FERROBOX_SYFT:?FERROBOX_SYFT is required}"

gvisor_sidecar_count=0
while IFS= read -r -d '' sidecar; do
    sidecar_name="gvisor-$(basename "${sidecar}")"
    add_executable "${sidecar_name}" comparator "${sidecar}"
    gvisor_sidecar_count=$((gvisor_sidecar_count + 1))
done < <(find /usr/local/bin/gvisor-bin -type f -perm /111 -print0 | sort -z)
((gvisor_sidecar_count > 0))

kata_config=/mnt/ferrobox/runtime/kata/configuration-qemu.toml
[[ -f "${kata_config}" ]]
kata_asset_count=0
while IFS= read -r kata_asset; do
    [[ -n "${kata_asset}" && -f "${kata_asset}" ]] || continue
    file_record "kata-$(basename "${kata_asset}")" kata "${kata_asset}" "${guest_assets}"
    kata_asset_count=$((kata_asset_count + 1))
done < <(
    sed -nE \
        's/^[[:space:]]*(path|kernel|image|initrd|virtio_fs_daemon)[[:space:]]*=[[:space:]]*"([^"]+)".*/\2/p' \
        "${kata_config}" | sort -u
)
((kata_asset_count >= 3))

file_record ferrobox-kernel guest /opt/ferrobox/images/vmlinux "${guest_assets}"
file_record ferrobox-rootfs guest /opt/ferrobox/images/python.ext4 "${guest_assets}"

add_sbom ferrobox-source "${sbom_dir}/ferrobox-source.spdx.json"
add_sbom ferrobox-rootfs "${sbom_dir}/ferrobox-rootfs.spdx.json"
add_sbom python-image "${sbom_dir}/python-image.spdx.json"

resolved_image_digest="$(
    docker image inspect --format '{{json .RepoDigests}}' "${container_image}" |
        jq --raw-output --arg digest "${expected_image_digest}" \
            '.[] | select(endswith("@" + $digest))' | head -n 1
)"
[[ -n "${resolved_image_digest}" ]]

workflow_sha256="$(sha256sum .github/workflows/kvm.yml | awk '{print $1}')"
cargo_lock_sha256="$(sha256sum Cargo.lock | awk '{print $1}')"
rootfs_recipe_sha256="$(sha256sum scripts/build-python-rootfs.sh | awk '{print $1}')"
generated_at="$(date --utc +'%Y-%m-%dT%H:%M:%SZ')"
kernel_release="$(uname -r)"

jq --null-input \
    --argjson subjects "$(jq --slurp . "${subjects}")" \
    --argjson executed_files "$(jq --slurp . "${executed_files}")" \
    --argjson guest_assets "$(jq --slurp . "${guest_assets}")" \
    --argjson sboms "$(jq --slurp . "${sboms}")" \
    --arg generated_at "${generated_at}" \
    --arg repository "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}" \
    --arg github_sha "${GITHUB_SHA:?GITHUB_SHA is required}" \
    --arg workflow_ref "${GITHUB_WORKFLOW_REF:?GITHUB_WORKFLOW_REF is required}" \
    --arg run_id "${GITHUB_RUN_ID:?GITHUB_RUN_ID is required}" \
    --arg run_attempt "${GITHUB_RUN_ATTEMPT:?GITHUB_RUN_ATTEMPT is required}" \
    --arg runner_os "${RUNNER_OS:?RUNNER_OS is required}" \
    --arg runner_arch "${RUNNER_ARCH:?RUNNER_ARCH is required}" \
    --arg runner_image_os "${ImageOS:-unknown}" \
    --arg runner_image_version "${ImageVersion:-unknown}" \
    --arg kernel_release "${kernel_release}" \
    --arg workflow_sha256 "${workflow_sha256}" \
    --arg cargo_lock_sha256 "${cargo_lock_sha256}" \
    --arg rootfs_recipe_sha256 "${rootfs_recipe_sha256}" \
    --arg image "${container_image}" \
    --arg repo_digest "${resolved_image_digest}" \
    --arg firecracker_url "$(cat "${RUNNER_TEMP}/firecracker/SOURCE.url")" \
    --arg firecracker_archive_sha256 "$(awk '{print $1}' "${RUNNER_TEMP}/firecracker/SOURCE.sha256")" \
    --arg syft_url "$(cat "$(dirname "${FERROBOX_SYFT}")/SOURCE.url")" \
    --arg syft_archive_sha256 "$(awk '{print $1}' "$(dirname "${FERROBOX_SYFT}")/SOURCE.sha256")" \
    --arg kernel_url "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.15/x86_64/vmlinux-6.1.155" \
    --arg kernel_sha256 "$(sha256sum /opt/ferrobox/images/vmlinux | awk '{print $1}')" \
    --arg gvisor_url "https://storage.googleapis.com/gvisor/releases/release/20260721/x86_64/gvisor.tar.bz2" \
    --arg gvisor_sha512 "$(awk '{print $1}' "${RUNNER_TEMP}/gvisor-tarball.sha512")" \
    --arg cloud_hypervisor_url "https://github.com/cloud-hypervisor/cloud-hypervisor/releases/download/v53.0/cloud-hypervisor-static" \
    --arg cloud_hypervisor_sha256 "$(awk '{print $1}' "${RUNNER_TEMP}/cloud-hypervisor.sha256")" \
    --arg kata_url "https://github.com/kata-containers/kata-containers/releases/download/3.31.0/kata-static-3.31.0-amd64.tar.zst" \
    --arg kata_sha256 "$(awk '{print $1}' "${RUNNER_TEMP}/kata-static.sha256")" \
    '{
        _type: "https://in-toto.io/Statement/v1",
        subject: $subjects,
        predicateType: "https://github.com/nya-a-cat/ferrobox/e2e-provenance/v1",
        predicate: {
            schema_version: 1,
            generated_at: $generated_at,
            source: {
                repository: $repository,
                git_commit: $github_sha,
                workflow_ref: $workflow_ref,
                workflow_sha256: $workflow_sha256,
                cargo_lock_sha256: $cargo_lock_sha256,
                rootfs_recipe_sha256: $rootfs_recipe_sha256
            },
            invocation: {
                run_id: $run_id,
                run_attempt: ($run_attempt | tonumber),
                url: ("https://github.com/" + $repository + "/actions/runs/" + $run_id)
            },
            environment: {
                runner_os: $runner_os,
                runner_arch: $runner_arch,
                runner_image_os: $runner_image_os,
                runner_image_version: $runner_image_version,
                kernel_release: $kernel_release
            },
            boundary: {
                included: "project outputs, downloaded isolation inputs, comparator runtimes, guest assets, and workload images executed by KVM E2E",
                excluded: "GitHub runner base-image utilities and build-only operating-system packages",
                signature: "unsigned-evidence; release attestation is tracked separately"
            },
            executed_files: $executed_files,
            guest_assets: $guest_assets,
            workload_images: [{reference: $image, repo_digest: $repo_digest}],
            upstream_inputs: [
                {name: "firecracker", uri: $firecracker_url, digest: {sha256: $firecracker_archive_sha256}},
                {name: "syft", uri: $syft_url, digest: {sha256: $syft_archive_sha256}},
                {name: "guest-kernel", uri: $kernel_url, digest: {sha256: $kernel_sha256}},
                {name: "gvisor", uri: $gvisor_url, digest: {sha512: $gvisor_sha512}},
                {name: "cloud-hypervisor", uri: $cloud_hypervisor_url, digest: {sha256: $cloud_hypervisor_sha256}},
                {name: "kata-containers", uri: $kata_url, digest: {sha256: $kata_sha256}}
            ],
            sboms: $sboms
        }
    }' >"${output}"

jq --exit-status --arg expected_image_digest "${expected_image_digest}" '
    ._type == "https://in-toto.io/Statement/v1" and
    .predicateType == "https://github.com/nya-a-cat/ferrobox/e2e-provenance/v1" and
    (.subject | length == 5) and
    ([.subject[].name] | sort == [
        "ferrobox-api",
        "ferrobox-guest",
        "ferrobox-node",
        "microvm-probe",
        "python.ext4"
    ]) and
    (.predicate.executed_files | length >= 17) and
    (all(.predicate.executed_files[]; .digest.sha256 | test("^[0-9a-f]{64}$"))) and
    ([.predicate.executed_files[].name] | length == (unique | length)) and
    (.predicate.guest_assets | length >= 5) and
    (all(.predicate.guest_assets[]; .digest.sha256 | test("^[0-9a-f]{64}$"))) and
    (.predicate.workload_images | length == 1) and
    (.predicate.workload_images[0].repo_digest | endswith("@" + $expected_image_digest)) and
    (.predicate.upstream_inputs | length == 6) and
    (.predicate.sboms | length == 3) and
    (all(.predicate.sboms[];
        .format == "SPDX-2.3 JSON" and
        .package_count > 0 and
        (.digest.sha256 | test("^[0-9a-f]{64}$"))
    ))
' "${output}" >/dev/null

while IFS=$'\t' read -r path expected; do
    actual="$(sha256sum "${path}" | awk '{print $1}')"
    [[ "${actual}" == "${expected}" ]]
done < <(
    jq --raw-output '
        .predicate.executed_files[],
        .predicate.guest_assets[],
        .predicate.sboms[] |
        [.path, .digest.sha256] | @tsv
    ' "${output}"
)
