#!/usr/bin/env bash
set -euo pipefail

if [[ "${EUID}" -ne 0 ]]; then
    echo "build-oci-rootfs.sh must run as root" >&2
    exit 2
fi

crane="${1:?usage: build-oci-rootfs.sh CRANE IMAGE PLATFORM GUEST OUTPUT_EXT4 EVIDENCE_DIR}"
image_reference="${2:?usage: build-oci-rootfs.sh CRANE IMAGE PLATFORM GUEST OUTPUT_EXT4 EVIDENCE_DIR}"
platform="${3:?usage: build-oci-rootfs.sh CRANE IMAGE PLATFORM GUEST OUTPUT_EXT4 EVIDENCE_DIR}"
guest_binary="${4:?usage: build-oci-rootfs.sh CRANE IMAGE PLATFORM GUEST OUTPUT_EXT4 EVIDENCE_DIR}"
output_image="${5:?usage: build-oci-rootfs.sh CRANE IMAGE PLATFORM GUEST OUTPUT_EXT4 EVIDENCE_DIR}"
evidence_dir="${6:?usage: build-oci-rootfs.sh CRANE IMAGE PLATFORM GUEST OUTPUT_EXT4 EVIDENCE_DIR}"
expected_platform_digest="${FERROBOX_OCI_EXPECTED_MANIFEST_DIGEST:-}"
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

crane="$(realpath "${crane}")"
guest_binary="$(realpath "${guest_binary}")"
output_image="$(realpath -m "${output_image}")"
evidence_dir="$(realpath -m "${evidence_dir}")"
[[ -x "${crane}" ]] || { echo "crane is missing or not executable" >&2; exit 3; }
[[ -x "${guest_binary}" ]] || { echo "guest binary is missing or not executable" >&2; exit 3; }
[[ "${platform}" == "linux/amd64" ]] || { echo "only linux/amd64 is supported" >&2; exit 3; }
[[ "${image_reference}" != *://* && "${image_reference}" != *[[:space:]]* ]] || {
    echo "image reference must be a registry reference without a URL scheme" >&2
    exit 3
}
repository="${image_reference%@*}"
source_digest="${image_reference##*@}"
[[ "${repository}" != "${image_reference}" && "${repository}" != *"@"* ]] || {
    echo "image reference must contain exactly one digest separator" >&2
    exit 3
}
[[ "${source_digest}" =~ ^sha256:[0-9a-f]{64}$ ]] || {
    echo "image reference must use a full lowercase SHA-256 digest" >&2
    exit 3
}
if [[ -n "${expected_platform_digest}" && ! "${expected_platform_digest}" =~ ^sha256:[0-9a-f]{64}$ ]]; then
    echo "expected platform manifest digest is malformed" >&2
    exit 3
fi
if [[ -e "${output_image}" || -e "${evidence_dir}" ]]; then
    echo "output image and evidence directory must not already exist" >&2
    exit 3
fi

for executable in jq python3 sha256sum stat mkfs.ext4 e2fsck realpath; do
    command -v "${executable}" >/dev/null
done

staging="$(mktemp -d)"
partial_image="${output_image}.partial-$$"
cleanup() {
    rm -rf -- "${staging}"
    rm -f -- "${partial_image}"
}
trap cleanup EXIT
install -d -m 0755 "${staging}/evidence" "${staging}/blobs"

actual_source_digest="$("${crane}" digest "${image_reference}")"
[[ "${actual_source_digest}" == "${source_digest}" ]]
"${crane}" manifest "${image_reference}" >"${staging}/evidence/source-descriptor.json"
source_descriptor_digest="sha256:$(sha256sum "${staging}/evidence/source-descriptor.json" | awk '{print $1}')"
source_descriptor_size="$(stat --format '%s' "${staging}/evidence/source-descriptor.json")"
[[ "${source_descriptor_digest}" == "${source_digest}" ]]

resolved_manifest_digest="$(
    "${crane}" digest --platform "${platform}" "${image_reference}"
)"
[[ "${resolved_manifest_digest}" =~ ^sha256:[0-9a-f]{64}$ ]]
if [[ -n "${expected_platform_digest}" ]]; then
    [[ "${resolved_manifest_digest}" == "${expected_platform_digest}" ]]
fi
"${crane}" manifest --platform "${platform}" "${image_reference}" \
    >"${staging}/evidence/platform-manifest.json"
manifest_digest="sha256:$(sha256sum "${staging}/evidence/platform-manifest.json" | awk '{print $1}')"
manifest_size="$(stat --format '%s' "${staging}/evidence/platform-manifest.json")"
[[ "${manifest_digest}" == "${resolved_manifest_digest}" ]]

source_media_type="$(jq -er '.mediaType' "${staging}/evidence/source-descriptor.json")"
case "${source_media_type}" in
    application/vnd.oci.image.index.v1+json | application/vnd.docker.distribution.manifest.list.v2+json)
        jq --exit-status \
            --arg digest "${resolved_manifest_digest}" \
            --argjson size "${manifest_size}" '
                .schemaVersion == 2 and
                any(
                    .manifests[];
                    .digest == $digest and
                    .size == $size and
                    .platform.os == "linux" and
                    .platform.architecture == "amd64"
                )
            ' "${staging}/evidence/source-descriptor.json" >/dev/null
        ;;
    application/vnd.oci.image.manifest.v1+json | application/vnd.docker.distribution.manifest.v2+json)
        [[ "${source_digest}" == "${resolved_manifest_digest}" && "${source_descriptor_size}" == "${manifest_size}" ]]
        ;;
    *) echo "unsupported source descriptor media type: ${source_media_type}" >&2; exit 4 ;;
esac

manifest_media_type="$(jq -er '.mediaType' "${staging}/evidence/platform-manifest.json")"
case "${manifest_media_type}" in
    application/vnd.oci.image.manifest.v1+json | application/vnd.docker.distribution.manifest.v2+json) ;;
    *) echo "unsupported image manifest media type: ${manifest_media_type}" >&2; exit 4 ;;
esac
jq --exit-status '.schemaVersion == 2 and (.layers | type == "array") and (.layers | length > 0)' \
    "${staging}/evidence/platform-manifest.json" >/dev/null

config_digest="$(jq -er '.config.digest' "${staging}/evidence/platform-manifest.json")"
config_size="$(jq -er '.config.size' "${staging}/evidence/platform-manifest.json")"
config_media_type="$(jq -er '.config.mediaType' "${staging}/evidence/platform-manifest.json")"
[[ "${config_digest}" =~ ^sha256:[0-9a-f]{64}$ && "${config_size}" =~ ^[0-9]+$ ]]
case "${config_media_type}" in
    application/vnd.oci.image.config.v1+json | application/vnd.docker.container.image.v1+json) ;;
    *) echo "unsupported image config media type: ${config_media_type}" >&2; exit 4 ;;
esac
"${crane}" blob "${repository}@${config_digest}" >"${staging}/evidence/image-config.json"
actual_config_digest="sha256:$(sha256sum "${staging}/evidence/image-config.json" | awk '{print $1}')"
actual_config_size="$(stat --format '%s' "${staging}/evidence/image-config.json")"
[[ "${actual_config_digest}" == "${config_digest}" && "${actual_config_size}" == "${config_size}" ]]
jq --exit-status '
    .os == "linux" and
    .architecture == "amd64" and
    (.rootfs.diff_ids | type == "array")
' "${staging}/evidence/image-config.json" >/dev/null

layer_count="$(jq -er '.layers | length' "${staging}/evidence/platform-manifest.json")"
diff_id_count="$(jq -er '.rootfs.diff_ids | length' "${staging}/evidence/image-config.json")"
[[ "${layer_count}" == "${diff_id_count}" ]]
for ((index = 0; index < layer_count; index++)); do
    layer_digest="$(jq -er ".layers[${index}].digest" "${staging}/evidence/platform-manifest.json")"
    layer_size="$(jq -er ".layers[${index}].size" "${staging}/evidence/platform-manifest.json")"
    layer_media_type="$(jq -er ".layers[${index}].mediaType" "${staging}/evidence/platform-manifest.json")"
    [[ "${layer_digest}" =~ ^sha256:[0-9a-f]{64}$ && "${layer_size}" =~ ^[0-9]+$ ]]
    case "${layer_media_type}" in
        application/vnd.oci.image.layer.v1.tar | \
        application/vnd.oci.image.layer.v1.tar+gzip | \
        application/vnd.oci.image.layer.v1.tar+zstd | \
        application/vnd.docker.image.rootfs.diff.tar.gzip) ;;
        *) echo "unsupported layer media type: ${layer_media_type}" >&2; exit 4 ;;
    esac
    layer_path="${staging}/blobs/layer-${index}"
    "${crane}" blob "${repository}@${layer_digest}" >"${layer_path}"
    actual_layer_digest="sha256:$(sha256sum "${layer_path}" | awk '{print $1}')"
    actual_layer_size="$(stat --format '%s' "${layer_path}")"
    [[ "${actual_layer_digest}" == "${layer_digest}" && "${actual_layer_size}" == "${layer_size}" ]]
done
jq '[.layers[] | {media_type: .mediaType, digest, size}]' \
    "${staging}/evidence/platform-manifest.json" \
    >"${staging}/evidence/layers.json"

"${crane}" export --platform "${platform}" "${image_reference}" "${staging}/rootfs.tar"
rootfs_tar_sha256="$(sha256sum "${staging}/rootfs.tar" | awk '{print $1}')"
rootfs_tar_size="$(stat --format '%s' "${staging}/rootfs.tar")"
python3 "${script_dir}/safe-extract-tar.py" \
    "${staging}/rootfs.tar" \
    "${staging}/rootfs" \
    --max-members 200000 \
    --max-total-bytes 8589934592 \
    --max-file-bytes 4294967296 \
    --evidence "${staging}/evidence/extraction.json" \
    >/dev/null
rootfs="${staging}/rootfs"

ensure_inside_rootfs() {
    local path="$1"
    local resolved
    resolved="$(realpath -m "${path}")"
    case "${resolved}" in
        "${rootfs}" | "${rootfs}"/*) ;;
        *) echo "rootfs path resolves outside staging: ${path}" >&2; exit 5 ;;
    esac
}

for path in \
    "${rootfs}/usr/local/bin" \
    "${rootfs}/etc" \
    "${rootfs}/home/sandbox" \
    "${rootfs}/proc" \
    "${rootfs}/sys/fs/cgroup" \
    "${rootfs}/dev/pts" \
    "${rootfs}/run" \
    "${rootfs}/tmp" \
    "${rootfs}/sbin/init"; do
    ensure_inside_rootfs "${path}"
done
for account_file in "${rootfs}/etc/passwd" "${rootfs}/etc/group"; do
    if [[ -L "${account_file}" ]]; then
        echo "OCI account database must not be a symbolic link" >&2
        exit 5
    fi
done

install -d -m 0755 \
    "${rootfs}/usr/local/bin" \
    "${rootfs}/etc" \
    "${rootfs}/proc" \
    "${rootfs}/sys/fs/cgroup" \
    "${rootfs}/dev/pts" \
    "${rootfs}/run"
install -d -m 1777 "${rootfs}/tmp"
install -d -o 1000 -g 1000 -m 0750 "${rootfs}/home/sandbox"
install -m 0755 "${guest_binary}" "${rootfs}/usr/local/bin/ferrobox-guest"
cat >"${rootfs}/usr/local/bin/ferrobox-init" <<'INIT'
#!/bin/sh
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
mount -t proc proc /proc 2>/dev/null || true
mount -t sysfs sysfs /sys 2>/dev/null || true
mount -t devtmpfs devtmpfs /dev 2>/dev/null || true
mkdir -p /dev/pts /run /sys/fs/cgroup
mount -t devpts devpts /dev/pts 2>/dev/null || true
mount -t tmpfs tmpfs /run 2>/dev/null || true
mount -t cgroup2 cgroup2 /sys/fs/cgroup 2>/dev/null || true
exec /usr/local/bin/ferrobox-guest
INIT
chmod 0755 "${rootfs}/usr/local/bin/ferrobox-init"
ln -sfn /usr/local/bin/ferrobox-init "${rootfs}/sbin/init"

touch "${rootfs}/etc/passwd" "${rootfs}/etc/group"
if awk -F: '$1 == "sandbox" && $3 != 1000 { conflict = 1 } END { exit !conflict }' "${rootfs}/etc/passwd"; then
    echo "OCI image already defines sandbox with another UID" >&2
    exit 5
fi
if ! awk -F: '$3 == 1000 { found = 1 } END { exit !found }' "${rootfs}/etc/passwd"; then
    printf 'sandbox:x:1000:1000:Ferrobox workload:/home/sandbox:/bin/sh\n' >>"${rootfs}/etc/passwd"
fi
if awk -F: '$1 == "sandbox" && $3 != 1000 { conflict = 1 } END { exit !conflict }' "${rootfs}/etc/group"; then
    echo "OCI image already defines sandbox with another GID" >&2
    exit 5
fi
if ! awk -F: '$3 == 1000 { found = 1 } END { exit !found }' "${rootfs}/etc/group"; then
    printf 'sandbox:x:1000:\n' >>"${rootfs}/etc/group"
fi

logical_bytes="$(jq -er '.logical_file_bytes' "${staging}/evidence/extraction.json")"
image_bytes=$((logical_bytes * 2 + 268435456))
if ((image_bytes < 1073741824)); then
    image_bytes=1073741824
fi
alignment=67108864
image_bytes=$((((image_bytes + alignment - 1) / alignment) * alignment))
if ((image_bytes > 8589934592)); then
    echo "materialized rootfs exceeds the 8 GiB image limit" >&2
    exit 5
fi
install -d -m 0755 "$(dirname -- "${output_image}")" "$(dirname -- "${evidence_dir}")"
truncate --size "${image_bytes}" "${partial_image}"
mkfs.ext4 -q -F -L ferrobox-oci -d "${rootfs}" "${partial_image}"
e2fsck -fn "${partial_image}" >"${staging}/evidence/e2fsck.txt"

guest_sha256="$(sha256sum "${guest_binary}" | awk '{print $1}')"
init_sha256="$(sha256sum "${rootfs}/usr/local/bin/ferrobox-init" | awk '{print $1}')"
rootfs_sha256="$(sha256sum "${partial_image}" | awk '{print $1}')"
crane_version="$("${crane}" version)"
jq -n \
    --arg image_reference "${image_reference}" \
    --arg platform "${platform}" \
    --arg source_digest "${source_digest}" \
    --arg source_media_type "${source_media_type}" \
    --arg source_descriptor_digest "${source_descriptor_digest}" \
    --argjson source_descriptor_size "${source_descriptor_size}" \
    --arg resolved_manifest_digest "${resolved_manifest_digest}" \
    --arg manifest_media_type "${manifest_media_type}" \
    --argjson manifest_size "${manifest_size}" \
    --arg config_digest "${config_digest}" \
    --arg config_media_type "${config_media_type}" \
    --argjson config_size "${config_size}" \
    --arg crane_version "${crane_version}" \
    --arg rootfs_tar_sha256 "${rootfs_tar_sha256}" \
    --argjson rootfs_tar_size "${rootfs_tar_size}" \
    --arg guest_sha256 "${guest_sha256}" \
    --arg init_sha256 "${init_sha256}" \
    --arg rootfs_sha256 "${rootfs_sha256}" \
    --argjson rootfs_size "${image_bytes}" \
    --slurpfile config "${staging}/evidence/image-config.json" \
    --slurpfile layers "${staging}/evidence/layers.json" \
    --slurpfile extraction "${staging}/evidence/extraction.json" \
    '{
        schema_version: 1,
        image_reference: $image_reference,
        platform: $platform,
        source: {
            media_type: $source_media_type,
            digest: $source_digest,
            verified_digest: $source_descriptor_digest,
            size: $source_descriptor_size
        },
        manifest: {
            media_type: $manifest_media_type,
            digest: $resolved_manifest_digest,
            size: $manifest_size
        },
        config: {
            media_type: $config_media_type,
            digest: $config_digest,
            size: $config_size,
            os: $config[0].os,
            architecture: $config[0].architecture,
            user: ($config[0].config.User // ""),
            entrypoint: ($config[0].config.Entrypoint // []),
            command: ($config[0].config.Cmd // []),
            working_directory: ($config[0].config.WorkingDir // "")
        },
        layers: $layers[0],
        descriptor_bytes_verified: true,
        crane_version: $crane_version,
        flattened_rootfs: {
            sha256: $rootfs_tar_sha256,
            size: $rootfs_tar_size,
            extraction: $extraction[0]
        },
        ferrobox_injection: {
            command_uid: 1000,
            command_gid: 1000,
            guest_sha256: $guest_sha256,
            init_sha256: $init_sha256
        },
        ext4: {
            sha256: $rootfs_sha256,
            size: $rootfs_size,
            e2fsck_read_only: true
        }
    }' >"${staging}/evidence/oci-rootfs-evidence.json"

mv -- "${partial_image}" "${output_image}"
mv -- "${staging}/evidence" "${evidence_dir}"
printf 'Built OCI-derived rootfs %s from %s\n' "${output_image}" "${image_reference}"
