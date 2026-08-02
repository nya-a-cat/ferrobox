#!/usr/bin/env bash
set -euo pipefail

version="0.21.8"
release_commit="2ea098f4b13456cd628460632760b0a74b7488e9"
archive="go-containerregistry_Linux_x86_64.tar.gz"
archive_size="16529684"
archive_sha256="59b59f68ee37aba51f5523d69ec779ee925d9be4e279f9220eca357267f2ee67"
checksums_sha256="fd2c091dc084e28878f59a225f9223442356e6f6de221407b572ab06ddebfb89"
base_url="https://github.com/google/go-containerregistry/releases/download/v${version}"
destination="${1:-${XDG_CACHE_HOME:-${HOME}/.cache}/ferrobox/crane-v${version}}"
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) ;;
    *)
        echo "Ferrobox supports this crane asset only on Linux x86_64." >&2
        exit 2
        ;;
esac

if [[ -e "${destination}" ]]; then
    echo "crane destination already exists: ${destination}" >&2
    exit 3
fi

staging="$(mktemp -d)"
cleanup() {
    rm -rf -- "${staging}"
}
trap cleanup EXIT

for asset in "${archive}" checksums.txt; do
    curl \
        --fail \
        --location \
        --proto '=https' \
        --show-error \
        --silent \
        --tlsv1.2 \
        --output "${staging}/${asset}" \
        "${base_url}/${asset}"
done

printf '%s  %s\n' "${checksums_sha256}" "${staging}/checksums.txt" |
    sha256sum --check --strict
[[ "$(stat --format '%s' "${staging}/${archive}")" == "${archive_size}" ]]
printf '%s  %s\n' "${archive_sha256}" "${staging}/${archive}" |
    sha256sum --check --strict
published_sha256="$({
    awk -v archive="${archive}" '$2 == archive { print $1 }' \
        "${staging}/checksums.txt"
} | tail -n 1)"
[[ "${published_sha256}" == "${archive_sha256}" ]]

python3 "${script_dir}/safe-extract-tar.py" \
    "${staging}/${archive}" \
    "${staging}/unpacked" \
    --max-members 1000 \
    --max-total-bytes 134217728 \
    --max-file-bytes 67108864 \
    --evidence "${staging}/archive-extraction.json" \
    >/dev/null

mapfile -t crane_candidates < <(
    find "${staging}/unpacked" -type f -name crane -perm -u+x -print
)
if [[ "${#crane_candidates[@]}" -ne 1 ]]; then
    echo "The verified archive did not contain exactly one crane executable." >&2
    exit 4
fi

install -d -m 0755 "${destination}"
install -m 0755 "${crane_candidates[0]}" "${destination}/crane"
version_output="$("${destination}/crane" version)"
grep --fixed-strings "${version}" <<<"${version_output}" >/dev/null

cat >"${destination}/SOURCE.manifest" <<EOF
name=crane
version=${version}
release_tag=v${version}
release_commit=${release_commit}
source_url=${base_url}/${archive}
size_bytes=${archive_size}
sha256=${archive_sha256}
checksums_sha256=${checksums_sha256}
upstream_slsa_provenance=absent
EOF

printf '%s\n' "${version_output}"
printf 'Installed verified crane in %s\n' "${destination}"
