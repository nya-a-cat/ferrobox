#!/usr/bin/env bash
set -euo pipefail

version="1.7"
release_tag="v${version}"
release_tag_object="2622f760b5b03a868061f85b21964390298a4015"
release_commit="96d12bd0d34a034d6e0b85512422f0d6df3c7c4a"
release_tree="849ba951347671baf7691000e94dfcdffb36fe56"
release_epoch="1762306097"
source_archive="fsverity-utils-${version}.tar.xz"
source_sha256="5778dac5b935bd15f4ec0d17c33b8651217b319d62e8e0432b84f02731b013b2"
source_max_size="131072"
signer_fingerprint="B8868C80BA62A1FFFAF5FDA9632D3A06589DA6B1"
base_url="https://mirrors.edge.kernel.org/pub/linux/kernel/people/ebiggers/fsverity-utils/v${version}"
key_url="https://keyserver.ubuntu.com/pks/lookup?op=get&search=0x${signer_fingerprint}"
destination="${1:-${XDG_CACHE_HOME:-${HOME}/.cache}/ferrobox/fsverity-utils-v${version}}"
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) ;;
    *)
        echo "Ferrobox supports this fsverity-utils build only on Linux x86_64." >&2
        exit 2
        ;;
esac

if [[ -e "${destination}" ]]; then
    echo "fsverity-utils destination already exists: ${destination}" >&2
    exit 3
fi

for executable in awk curl find gcc git gpg ldd make pkg-config python3 sha256sum stat; do
    command -v "${executable}" >/dev/null
done
[[ -f "${script_dir}/safe-extract-tar.py" ]]
libcrypto_version="$(pkg-config --modversion libcrypto)"
[[ -n "${libcrypto_version}" ]]

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
    --max-filesize "${source_max_size}" \
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
printf '%s\n' "${key_fingerprints[@]}" |
    grep --fixed-strings --line-regexp "${signer_fingerprint}" >/dev/null
gpg --batch --homedir "${staging}/gnupg" --import "${staging}/release-key.asc" \
    >"${staging}/gpg-import.log" 2>&1
gpg \
    --batch \
    --homedir "${staging}/gnupg" \
    --status-fd 1 \
    --verify "${staging}/sha256sums.asc" \
    >"${staging}/gpg-status.log" \
    2>"${staging}/gpg-verify.log"
awk -v fingerprint="${signer_fingerprint}" '
    $1 == "[GNUPG:]" && $2 == "VALIDSIG" &&
        ($3 == fingerprint || $NF == fingerprint) { valid = 1 }
    END { exit !valid }
' "${staging}/gpg-status.log"
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
source_size="$(stat --format '%s' "${staging}/${source_archive}")"
[[ "${source_size}" -gt 0 ]]
[[ "${source_size}" -le "${source_max_size}" ]]
printf '%s  %s\n' "${source_sha256}" "${staging}/${source_archive}" |
    sha256sum --check --strict

python3 "${script_dir}/safe-extract-tar.py" \
    "${staging}/${source_archive}" \
    "${staging}/source" \
    --max-members 2000 \
    --max-total-bytes 16777216 \
    --max-file-bytes 4194304 \
    --evidence "${staging}/source-extraction.json" \
    >/dev/null

mapfile -t makefiles < <(
    find "${staging}/source" -maxdepth 3 -type f -name Makefile -print
)
if [[ "${#makefiles[@]}" -ne 1 ]]; then
    echo "The verified source archive did not contain exactly one Makefile." >&2
    exit 4
fi
source_root="$(dirname -- "${makefiles[0]}")"
git -c init.defaultBranch=fsverity-source init --quiet "${source_root}"
git -C "${source_root}" \
    -c core.autocrlf=false \
    -c core.filemode=true \
    add --all --force
extracted_tree="$(git -C "${source_root}" write-tree)"
if [[ "${extracted_tree}" != "${release_tree}" ]]; then
    echo "The signed release archive did not match the pinned upstream Git tree." >&2
    exit 5
fi
rm -rf -- "${source_root}/.git"

export LC_ALL=C
export SOURCE_DATE_EPOCH="${release_epoch}"
build_cflags="-O2 -g0 -ffile-prefix-map=${staging}=/usr/src/fsverity-utils -fdebug-prefix-map=${staging}=/usr/src/fsverity-utils"
(
    cd -- "${source_root}"
    make \
        --jobs=2 \
        V=1 \
        CFLAGS="${build_cflags}" \
        >"${staging}/build.log" 2>&1
    make V=1 CFLAGS="${build_cflags}" check >"${staging}/check.log" 2>&1
)

install -d -m 0755 "${destination}/bin"
install -m 0755 "${source_root}/fsverity" "${destination}/bin/fsverity"
"${destination}/bin/fsverity" --version >"${staging}/version.txt"
grep --fixed-strings --line-regexp "fsverity v${version}" \
    "${staging}/version.txt" >/dev/null
ldd "${destination}/bin/fsverity" >"${staging}/ldd.log"
grep --extended-regexp 'libcrypto\.so' "${staging}/ldd.log" >/dev/null

binary_sha256="$(sha256sum "${destination}/bin/fsverity" | awk '{print $1}')"
install -m 0644 "${staging}/build.log" "${destination}/BUILD.log"
install -m 0644 "${staging}/check.log" "${destination}/CHECK.log"
install -m 0644 "${staging}/gpg-import.log" "${destination}/GPG-IMPORT.log"
install -m 0644 "${staging}/gpg-status.log" "${destination}/GPG-STATUS.log"
install -m 0644 "${staging}/gpg-verify.log" "${destination}/GPG-VERIFY.log"
install -m 0644 "${staging}/ldd.log" "${destination}/LDD.log"
install -m 0644 "${staging}/source-extraction.json" \
    "${destination}/SOURCE-EXTRACTION.json"
install -m 0644 "${staging}/version.txt" "${destination}/VERSION.txt"

cat >"${destination}/SOURCE.manifest" <<EOF
name=fsverity-utils
upstream_version=${version}
release_tag=${release_tag}
release_tag_object=${release_tag_object}
release_commit=${release_commit}
release_git_tree_sha1=${release_tree}
extracted_git_tree_sha1=${extracted_tree}
signed_release_tree_verified=true
release_epoch=${release_epoch}
source_url=${base_url}/${source_archive}
source_size_bytes=${source_size}
source_sha256=${source_sha256}
published_checksums_url=${base_url}/sha256sums.asc
published_checksum_signer_fingerprint=${signer_fingerprint}
published_signature_verified=true
key_url=${key_url}
libcrypto_version=${libcrypto_version}
compiler=$(gcc -dumpfullversion)
source_date_epoch=${release_epoch}
portable_check_passed=true
artifact_sha256_fsverity=${binary_sha256}
EOF

printf 'Built verified fsverity-utils %s in %s\n' "${version}" "${destination}"
