#!/usr/bin/env bash
set -euo pipefail

version="1.47.4"
release_tag="v${version}"
release_commit="7ee1d505ef3b37831215f490411f346fe57e9053"
release_epoch="1772826629"
source_archive="e2fsprogs-${version}.tar.xz"
source_size="7337236"
source_sha256="fd5bf388cbdbe006a3d3b318d983b2948382440acc85a87f1e7d108653e8db0b"
signer_fingerprint="B8868C80BA62A1FFFAF5FDA9632D3A06589DA6B1"
base_url="https://mirrors.edge.kernel.org/pub/linux/kernel/people/tytso/e2fsprogs/v${version}"
key_url="https://keyserver.ubuntu.com/pks/lookup?op=get&search=0x${signer_fingerprint}"
destination="${1:-${XDG_CACHE_HOME:-${HOME}/.cache}/ferrobox/e2fsprogs-v${version}}"
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) ;;
    *)
        echo "Ferrobox supports this e2fsprogs build only on Linux x86_64." >&2
        exit 2
        ;;
esac

if [[ -e "${destination}" ]]; then
    echo "e2fsprogs destination already exists: ${destination}" >&2
    exit 3
fi

for executable in cmp curl find gcc gpg ldd make pkg-config python3 sha256sum stat tar truncate; do
    command -v "${executable}" >/dev/null
done
[[ -f "${script_dir}/safe-extract-tar.py" ]]
libarchive_version="$(pkg-config --modversion libarchive)"
[[ -n "${libarchive_version}" ]]

staging="$(mktemp -d)"
cleanup() {
    rm -rf -- "${staging}"
}
trap cleanup EXIT
install -d -m 0700 "${staging}/gnupg"

curl \
    --fail \
    --location \
    --proto '=https' \
    --show-error \
    --silent \
    --tlsv1.2 \
    --max-filesize 131072 \
    --output "${staging}/release-key.asc" \
    "${key_url}"
curl \
    --fail \
    --location \
    --proto '=https' \
    --show-error \
    --silent \
    --tlsv1.2 \
    --max-filesize 131072 \
    --output "${staging}/sha256sums.asc" \
    "${base_url}/sha256sums.asc"
curl \
    --fail \
    --location \
    --proto '=https' \
    --show-error \
    --silent \
    --tlsv1.2 \
    --max-filesize "${source_size}" \
    --output "${staging}/${source_archive}" \
    "${base_url}/${source_archive}"

mapfile -t key_fingerprints < <(
    gpg \
        --batch \
        --with-colons \
        --import-options show-only \
        --import "${staging}/release-key.asc" 2>/dev/null |
        awk -F: '$1 == "fpr" { print $10 }'
)
printf '%s\n' "${key_fingerprints[@]}" | grep --fixed-strings --line-regexp "${signer_fingerprint}" >/dev/null
gpg --batch --homedir "${staging}/gnupg" --import "${staging}/release-key.asc" \
    >"${staging}/gpg-import.log" 2>&1
gpg \
    --batch \
    --homedir "${staging}/gnupg" \
    --status-fd 1 \
    --verify "${staging}/sha256sums.asc" \
    >"${staging}/gpg-status.log" \
    2>"${staging}/gpg-verify.log"
grep --extended-regexp \
    "^\[GNUPG:\] VALIDSIG ${signer_fingerprint} " \
    "${staging}/gpg-status.log" >/dev/null
gpg \
    --batch \
    --homedir "${staging}/gnupg" \
    --decrypt "${staging}/sha256sums.asc" \
    >"${staging}/sha256sums.txt" \
    2>>"${staging}/gpg-verify.log"

published_sha256="$({
    awk -v archive="${source_archive}" '$2 == archive { print $1 }' \
        "${staging}/sha256sums.txt"
} | tail -n 1)"
[[ "${published_sha256}" == "${source_sha256}" ]]
[[ "$(stat --format '%s' "${staging}/${source_archive}")" == "${source_size}" ]]
printf '%s  %s\n' "${source_sha256}" "${staging}/${source_archive}" |
    sha256sum --check --strict

python3 "${script_dir}/safe-extract-tar.py" \
    "${staging}/${source_archive}" \
    "${staging}/source" \
    --max-members 30000 \
    --max-total-bytes 536870912 \
    --max-file-bytes 134217728 \
    --evidence "${staging}/source-extraction.json" \
    >/dev/null

mapfile -t configure_files < <(
    find "${staging}/source" -maxdepth 3 -type f -name configure -print
)
if [[ "${#configure_files[@]}" -ne 1 ]]; then
    echo "The verified source archive did not contain exactly one configure script." >&2
    exit 4
fi
source_root="$(dirname -- "${configure_files[0]}")"
grep --fixed-strings --line-regexp \
    "#define E2FSPROGS_VERSION \"${version}\"" \
    "${source_root}/version.h" >/dev/null

install -d -m 0755 "${staging}/build" "${destination}/bin" "${destination}/etc"
export LC_ALL=C
export SOURCE_DATE_EPOCH="${release_epoch}"
(
    cd -- "${staging}/build"
    CC=gcc \
    CFLAGS="-O2 -g0 -ffile-prefix-map=${staging}=/usr/src/e2fsprogs -fdebug-prefix-map=${staging}=/usr/src/e2fsprogs" \
        "${source_root}/configure" \
            --disable-fuse2fs \
            --disable-nls \
            --disable-rpath \
            --with-libarchive=direct \
            >"${staging}/configure.log" 2>&1
    make --jobs=2 V=1 >"${staging}/build.log" 2>&1
)

for tool in mke2fs dumpe2fs e2fsck debugfs; do
    case "${tool}" in
        mke2fs | dumpe2fs) source_tool="${staging}/build/misc/${tool}" ;;
        e2fsck) source_tool="${staging}/build/e2fsck/e2fsck" ;;
        debugfs) source_tool="${staging}/build/debugfs/debugfs" ;;
    esac
    [[ -x "${source_tool}" ]]
    install -m 0755 "${source_tool}" "${destination}/bin/${tool}"
done
install -m 0644 "${staging}/build/misc/mke2fs.conf" "${destination}/etc/mke2fs.conf"

{
    for tool in mke2fs dumpe2fs e2fsck debugfs; do
        printf '## %s\n' "${tool}"
        ldd "${destination}/bin/${tool}"
    done
} >"${staging}/ldd.log"
grep --extended-regexp 'libarchive\.so' "${staging}/ldd.log" >/dev/null

smoke_epoch=946684800
install -d -m 0755 "${staging}/smoke-root/fixture"
printf 'ferrobox-e2fsprogs-tar-smoke\n' >"${staging}/smoke-root/fixture/hello.txt"
ln -s hello.txt "${staging}/smoke-root/fixture/link.txt"
tar \
    --create \
    --file="${staging}/smoke-root.tar" \
    --format=gnu \
    --mtime="@${smoke_epoch}" \
    --numeric-owner \
    --sort=name \
    --directory="${staging}/smoke-root" \
    .
for image in "${staging}/smoke-a.ext4" "${staging}/smoke-b.ext4"; do
    truncate --size 67108864 "${image}"
    LC_ALL=C \
    SOURCE_DATE_EPOCH="${smoke_epoch}" \
    E2FSPROGS_FAKE_TIME="${smoke_epoch}" \
    MKE2FS_CONFIG="${destination}/etc/mke2fs.conf" \
        "${destination}/bin/mke2fs" -q -F -t ext4 \
            -L ferrobox-smoke \
            -U 3e1cb9fa-bf5d-51fb-94d7-38013f2b9df1 \
            -E hash_seed=697835af-059e-5082-9085-a7bd9f28a530,lazy_itable_init=0,lazy_journal_init=0 \
            -d "${staging}/smoke-root.tar" \
            "${image}"
    "${destination}/bin/e2fsck" -fn "${image}" >>"${staging}/smoke-e2fsck.log"
done
cmp "${staging}/smoke-a.ext4" "${staging}/smoke-b.ext4"
smoke_contents="$({
    "${destination}/bin/debugfs" -R 'cat /fixture/hello.txt' "${staging}/smoke-a.ext4"
} 2>/dev/null)"
[[ "${smoke_contents}" == "ferrobox-e2fsprogs-tar-smoke" ]]

{
    "${destination}/bin/mke2fs" -V
} >"${staging}/mke2fs-version.txt" 2>&1
grep --fixed-strings "mke2fs ${version}" "${staging}/mke2fs-version.txt" >/dev/null
mke2fs_sha256="$(sha256sum "${destination}/bin/mke2fs" | awk '{print $1}')"
dumpe2fs_sha256="$(sha256sum "${destination}/bin/dumpe2fs" | awk '{print $1}')"
e2fsck_sha256="$(sha256sum "${destination}/bin/e2fsck" | awk '{print $1}')"
debugfs_sha256="$(sha256sum "${destination}/bin/debugfs" | awk '{print $1}')"
mke2fs_config_sha256="$(sha256sum "${destination}/etc/mke2fs.conf" | awk '{print $1}')"

install -m 0644 "${staging}/build.log" "${destination}/BUILD.log"
install -m 0644 "${staging}/configure.log" "${destination}/CONFIGURE.log"
install -m 0644 "${staging}/gpg-import.log" "${destination}/GPG-IMPORT.log"
install -m 0644 "${staging}/gpg-status.log" "${destination}/GPG-STATUS.log"
install -m 0644 "${staging}/gpg-verify.log" "${destination}/GPG-VERIFY.log"
install -m 0644 "${staging}/ldd.log" "${destination}/LDD.log"
install -m 0644 "${staging}/mke2fs-version.txt" "${destination}/MKE2FS-VERSION.txt"
install -m 0644 "${staging}/smoke-e2fsck.log" "${destination}/SMOKE-E2FSCK.log"
install -m 0644 "${staging}/source-extraction.json" "${destination}/SOURCE-EXTRACTION.json"

cat >"${destination}/SOURCE.manifest" <<EOF
name=e2fsprogs
upstream_version=${version}
release_tag=${release_tag}
release_commit=${release_commit}
release_epoch=${release_epoch}
source_url=${base_url}/${source_archive}
source_size_bytes=${source_size}
source_sha256=${source_sha256}
published_checksums_url=${base_url}/sha256sums.asc
published_checksum_signer_fingerprint=${signer_fingerprint}
published_signature_verified=true
key_url=${key_url}
libarchive_version=${libarchive_version}
libarchive_direct_link=true
configure_options=--disable-fuse2fs --disable-nls --disable-rpath --with-libarchive=direct
compiler=$(gcc -dumpfullversion)
source_date_epoch=${release_epoch}
tar_input_smoke_test=true
tar_input_byte_reproducible_twice=true
artifact_sha256_mke2fs=${mke2fs_sha256}
artifact_sha256_dumpe2fs=${dumpe2fs_sha256}
artifact_sha256_e2fsck=${e2fsck_sha256}
artifact_sha256_debugfs=${debugfs_sha256}
artifact_sha256_mke2fs_config=${mke2fs_config_sha256}
EOF

printf 'Built verified e2fsprogs %s in %s\n' "${version}" "${destination}"
