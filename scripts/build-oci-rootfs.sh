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
mkfs_ext4="${FERROBOX_MKFS_EXT4:-}"
e2fsck="${FERROBOX_E2FSCK:-}"
dumpe2fs="${FERROBOX_DUMPE2FS:-}"
mke2fs_config="${FERROBOX_MKE2FS_CONFIG:-}"
e2fsprogs_manifest="${FERROBOX_E2FSPROGS_MANIFEST:-}"

if [[ -z "${mkfs_ext4}" ]]; then
    mkfs_ext4="$(command -v mkfs.ext4)"
fi
if [[ -z "${e2fsck}" ]]; then
    e2fsck="$(command -v e2fsck)"
fi
if [[ -z "${dumpe2fs}" ]]; then
    dumpe2fs="$(command -v dumpe2fs)"
fi

crane="$(realpath "${crane}")"
guest_binary="$(realpath "${guest_binary}")"
output_image="$(realpath -m "${output_image}")"
evidence_dir="$(realpath -m "${evidence_dir}")"
mkfs_ext4="$(realpath "${mkfs_ext4}")"
e2fsck="$(realpath "${e2fsck}")"
dumpe2fs="$(realpath "${dumpe2fs}")"
[[ -x "${crane}" ]] || { echo "crane is missing or not executable" >&2; exit 3; }
[[ -x "${guest_binary}" ]] || { echo "guest binary is missing or not executable" >&2; exit 3; }
[[ -x "${mkfs_ext4}" ]] || { echo "mke2fs is missing or not executable" >&2; exit 3; }
[[ -x "${e2fsck}" ]] || { echo "e2fsck is missing or not executable" >&2; exit 3; }
[[ -x "${dumpe2fs}" ]] || { echo "dumpe2fs is missing or not executable" >&2; exit 3; }
if [[ -n "${mke2fs_config}" ]]; then
    [[ -f "${mke2fs_config}" && ! -L "${mke2fs_config}" ]] || {
        echo "mke2fs config must be a regular, non-symlink file" >&2
        exit 3
    }
    mke2fs_config="$(realpath "${mke2fs_config}")"
fi
if [[ -n "${e2fsprogs_manifest}" ]]; then
    [[ -f "${e2fsprogs_manifest}" && ! -L "${e2fsprogs_manifest}" ]] || {
        echo "e2fsprogs manifest must be a regular, non-symlink file" >&2
        exit 3
    }
    e2fsprogs_manifest="$(realpath "${e2fsprogs_manifest}")"
fi
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

for executable in chmod chown env jq python3 realpath sha256sum stat tar truncate; do
    command -v "${executable}" >/dev/null
done
tar_version="$(tar --version | head -n 1)"
[[ "${tar_version}" == "tar (GNU tar)"* ]] || {
    echo "GNU tar is required for deterministic rootfs ordering" >&2
    exit 3
}

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
chown 0:0 "${rootfs}"
chmod 0755 "${rootfs}"

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
if ! grep -qw pids /sys/fs/cgroup/cgroup.controllers; then
    echo "pids cgroup controller is unavailable" >&2
    exit 1
fi
if ! grep -qw pids /sys/fs/cgroup/cgroup.subtree_control; then
    printf '+pids\n' >/sys/fs/cgroup/cgroup.subtree_control
fi
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

guest_sha256="$(sha256sum "${guest_binary}" | awk '{print $1}')"
init_sha256="$(sha256sum "${rootfs}/usr/local/bin/ferrobox-init" | awk '{print $1}')"
image_created="$(jq -r '.created // ""' "${staging}/evidence/image-config.json")"
source_date_epoch="${FERROBOX_OCI_SOURCE_DATE_EPOCH:-}"
source_date_origin=environment
if [[ -z "${source_date_epoch}" ]]; then
    if [[ -n "${image_created}" ]]; then
        source_date_epoch="$(python3 - "${image_created}" <<'PY'
import datetime
import sys

raw = sys.argv[1]
if raw.endswith("Z"):
    raw = raw[:-1] + "+00:00"
try:
    instant = datetime.datetime.fromisoformat(raw)
except ValueError as error:
    raise SystemExit(f"OCI config created timestamp is invalid: {error}") from error
if instant.tzinfo is None:
    raise SystemExit("OCI config created timestamp must include a timezone")
epoch = int(instant.timestamp())
if epoch <= 0:
    raise SystemExit("OCI config created timestamp must be after the Unix epoch")
print(epoch)
PY
)"
        source_date_origin=oci-config-created
    else
        source_date_epoch=946684800
        source_date_origin=fixed-fallback-2000-01-01
    fi
fi
[[ "${source_date_epoch}" =~ ^[1-9][0-9]*$ ]] || {
    echo "OCI source date epoch must be a positive decimal integer" >&2
    exit 5
}

materialized_rootfs_tar="${staging}/materialized-rootfs.tar"
materialized_root_uid="$(stat --format '%u' "${rootfs}")"
materialized_root_gid="$(stat --format '%g' "${rootfs}")"
materialized_root_mode="$(stat --format '%04a' "${rootfs}")"
[[ "${materialized_root_uid}" == 0 ]]
[[ "${materialized_root_gid}" == 0 ]]
[[ "${materialized_root_mode}" == 0755 ]]
LC_ALL=C tar \
    --create \
    --file="${materialized_rootfs_tar}" \
    --format=gnu \
    --mtime="@${source_date_epoch}" \
    --numeric-owner \
    --sort=name \
    --directory="${rootfs}" \
    .
materialized_rootfs_sha256="$(sha256sum "${materialized_rootfs_tar}" | awk '{print $1}')"
materialized_rootfs_size="$(stat --format '%s' "${materialized_rootfs_tar}")"

reproducibility_material="${materialized_rootfs_sha256}:${image_bytes}:${source_date_epoch}"
mapfile -t reproducible_uuids < <(python3 - "${reproducibility_material}" <<'PY'
import sys
import uuid

material = sys.argv[1]
base = "https://github.com/nya-a-cat/ferrobox/oci-rootfs/v2/"
print(uuid.uuid5(uuid.NAMESPACE_URL, base + material))
print(uuid.uuid5(uuid.NAMESPACE_URL, base + material + "/directory-hash"))
PY
)
[[ "${#reproducible_uuids[@]}" -eq 2 ]]
filesystem_uuid="${reproducible_uuids[0]}"
directory_hash_seed="${reproducible_uuids[1]}"
[[ "${filesystem_uuid}" =~ ^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$ ]]
[[ "${directory_hash_seed}" =~ ^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$ ]]

install -d -m 0755 "$(dirname -- "${output_image}")" "$(dirname -- "${evidence_dir}")"
truncate --size "${image_bytes}" "${partial_image}"
mkfs_environment=(
    "LC_ALL=C"
    "SOURCE_DATE_EPOCH=${source_date_epoch}"
    "E2FSPROGS_FAKE_TIME=${source_date_epoch}"
)
if [[ -n "${mke2fs_config}" ]]; then
    mkfs_environment+=("MKE2FS_CONFIG=${mke2fs_config}")
fi
{
    env "${mkfs_environment[@]}" "${mkfs_ext4}" -V
} >"${staging}/evidence/mkfs-version.txt" 2>&1
env "${mkfs_environment[@]}" "${mkfs_ext4}" -q -F -t ext4 \
    -L ferrobox-oci \
    -U "${filesystem_uuid}" \
    -E "hash_seed=${directory_hash_seed},lazy_itable_init=0,lazy_journal_init=0" \
    -d "${materialized_rootfs_tar}" \
    "${partial_image}"
"${e2fsck}" -fn "${partial_image}" >"${staging}/evidence/e2fsck.txt"
"${dumpe2fs}" -h "${partial_image}" >"${staging}/evidence/dumpe2fs.txt" 2>&1

rootfs_sha256="$(sha256sum "${partial_image}" | awk '{print $1}')"
crane_version="$("${crane}" version)"
mkfs_version="$(cat "${staging}/evidence/mkfs-version.txt")"
mkfs_sha256="$(sha256sum "${mkfs_ext4}" | awk '{print $1}')"
e2fsck_sha256="$(sha256sum "${e2fsck}" | awk '{print $1}')"
dumpe2fs_sha256="$(sha256sum "${dumpe2fs}" | awk '{print $1}')"
mke2fs_config_sha256=""
e2fsprogs_manifest_sha256=""
if [[ -n "${mke2fs_config}" ]]; then
    mke2fs_config_sha256="$(sha256sum "${mke2fs_config}" | awk '{print $1}')"
fi
if [[ -n "${e2fsprogs_manifest}" ]]; then
    e2fsprogs_manifest_sha256="$(sha256sum "${e2fsprogs_manifest}" | awk '{print $1}')"
fi
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
    --arg image_created "${image_created}" \
    --arg rootfs_tar_sha256 "${rootfs_tar_sha256}" \
    --argjson rootfs_tar_size "${rootfs_tar_size}" \
    --arg materialized_rootfs_sha256 "${materialized_rootfs_sha256}" \
    --argjson materialized_rootfs_size "${materialized_rootfs_size}" \
    --arg materialized_root_uid "${materialized_root_uid}" \
    --arg materialized_root_gid "${materialized_root_gid}" \
    --arg materialized_root_mode "${materialized_root_mode}" \
    --arg tar_version "${tar_version}" \
    --arg guest_sha256 "${guest_sha256}" \
    --arg init_sha256 "${init_sha256}" \
    --arg rootfs_sha256 "${rootfs_sha256}" \
    --argjson rootfs_size "${image_bytes}" \
    --arg source_date_epoch "${source_date_epoch}" \
    --arg source_date_origin "${source_date_origin}" \
    --arg filesystem_uuid "${filesystem_uuid}" \
    --arg directory_hash_seed "${directory_hash_seed}" \
    --arg mkfs_version "${mkfs_version}" \
    --arg mkfs_sha256 "${mkfs_sha256}" \
    --arg e2fsck_sha256 "${e2fsck_sha256}" \
    --arg dumpe2fs_sha256 "${dumpe2fs_sha256}" \
    --arg mke2fs_config_sha256 "${mke2fs_config_sha256}" \
    --arg e2fsprogs_manifest_sha256 "${e2fsprogs_manifest_sha256}" \
    --slurpfile config "${staging}/evidence/image-config.json" \
    --slurpfile layers "${staging}/evidence/layers.json" \
    --slurpfile extraction "${staging}/evidence/extraction.json" \
    '{
        schema_version: 3,
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
            created: (if $image_created == "" then null else $image_created end),
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
        materialized_rootfs: {
            sha256: $materialized_rootfs_sha256,
            size: $materialized_rootfs_size,
            archive_format: "gnu-tar",
            sorted_by_name: true,
            numeric_owner: true,
            mtime_epoch: ($source_date_epoch | tonumber),
            root_uid: ($materialized_root_uid | tonumber),
            root_gid: ($materialized_root_gid | tonumber),
            root_mode: $materialized_root_mode,
            tar_version: $tar_version
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
            e2fsck_read_only: true,
            deterministic_parameters: true,
            source_date_epoch: ($source_date_epoch | tonumber),
            source_date_origin: $source_date_origin,
            filesystem_uuid: $filesystem_uuid,
            directory_hash_seed: $directory_hash_seed,
            lazy_itable_init: false,
            lazy_journal_init: false,
            mkfs_version: $mkfs_version,
            toolchain: {
                mke2fs_sha256: $mkfs_sha256,
                e2fsck_sha256: $e2fsck_sha256,
                dumpe2fs_sha256: $dumpe2fs_sha256,
                mke2fs_config_sha256: (
                    if $mke2fs_config_sha256 == "" then null else $mke2fs_config_sha256 end
                ),
                source_manifest_sha256: (
                    if $e2fsprogs_manifest_sha256 == "" then null else $e2fsprogs_manifest_sha256 end
                )
            }
        }
    }' >"${staging}/evidence/oci-rootfs-evidence.json"

mv -- "${partial_image}" "${output_image}"
mv -- "${staging}/evidence" "${evidence_dir}"
printf 'Built OCI-derived rootfs %s from %s\n' "${output_image}" "${image_reference}"
