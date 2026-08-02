#!/usr/bin/env bash
set -euo pipefail

version="0.21.8"
release_commit="2ea098f4b13456cd628460632760b0a74b7488e9"
source_archive="go-containerregistry-${version}.tar.gz"
source_size="4735515"
source_sha256="54d520389ab2e7dbaceafb94fbe5ba151ae51e2dc613d3f3f58689d3bbfce984"
checksums_sha256="fd2c091dc084e28878f59a225f9223442356e6f6de221407b572ab06ddebfb89"
base_url="https://github.com/google/go-containerregistry/releases/download/v${version}"
destination="${1:-${XDG_CACHE_HOME:-${HOME}/.cache}/ferrobox/crane-v${version}}"
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(cd -- "${script_dir}/.." && pwd)"
patch_file="${repository_root}/patches/go-containerregistry-v0.21.8-root-directory.patch"

case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) ;;
    *)
        echo "Ferrobox supports this crane build only on Linux x86_64." >&2
        exit 2
        ;;
esac

if [[ -e "${destination}" ]]; then
    echo "crane destination already exists: ${destination}" >&2
    exit 3
fi
[[ -f "${patch_file}" ]] || { echo "crane source patch is missing" >&2; exit 3; }

for executable in curl find go patch python3 sha256sum stat; do
    command -v "${executable}" >/dev/null
done
[[ "$(go env GOVERSION)" == "go1.26.5" ]]

staging="$(mktemp -d)"
cleanup() {
    rm -rf -- "${staging}"
}
trap cleanup EXIT

for asset in "${source_archive}" checksums.txt; do
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
[[ "$(stat --format '%s' "${staging}/${source_archive}")" == "${source_size}" ]]
printf '%s  %s\n' "${source_sha256}" "${staging}/${source_archive}" |
    sha256sum --check --strict
published_sha256="$({
    awk -v archive="${source_archive}" '$2 == archive { print $1 }' \
        "${staging}/checksums.txt"
} | tail -n 1)"
[[ "${published_sha256}" == "${source_sha256}" ]]

python3 "${script_dir}/safe-extract-tar.py" \
    "${staging}/${source_archive}" \
    "${staging}/source" \
    --max-members 20000 \
    --max-total-bytes 536870912 \
    --max-file-bytes 67108864 \
    --evidence "${staging}/source-extraction.json" \
    >/dev/null

mapfile -t module_files < <(
    find "${staging}/source" -maxdepth 2 -type f -name go.mod -print
)
if [[ "${#module_files[@]}" -ne 1 ]]; then
    echo "The verified source archive did not contain exactly one root Go module." >&2
    exit 4
fi
module_root="$(dirname -- "${module_files[0]}")"
grep --fixed-strings --line-regexp \
    'module github.com/google/go-containerregistry' \
    "${module_root}/go.mod" >/dev/null
grep --fixed-strings --line-regexp 'go 1.26.5' "${module_root}/go.mod" >/dev/null

patch_sha256="$(sha256sum "${patch_file}" | awk '{print $1}')"
install -d -m 0755 "${destination}"
install -m 0644 "${patch_file}" "${destination}/PATCH.diff"
install -m 0644 "${staging}/source-extraction.json" \
    "${destination}/SOURCE-EXTRACTION.json"
(
    cd -- "${module_root}"
    patch --batch --forward --strip=1 <"${patch_file}" 2>&1
) | tee "${destination}/PATCH-APPLY.log"

export CGO_ENABLED=0
export GOARCH=amd64
export GOFLAGS='-mod=readonly'
export GOOS=linux
export GOTOOLCHAIN=local

(
    cd -- "${module_root}"
    go test ./pkg/v1/mutate -count=1 2>&1
) | tee "${destination}/MUTATE-TEST.log"
(
    cd -- "${module_root}"
    for output in "${staging}/crane-a" "${staging}/crane-b"; do
        go build \
            -buildvcs=false \
            -trimpath \
            -ldflags "-s -w -X github.com/google/go-containerregistry/cmd/crane/cmd.Version=v${version}+ferrobox.root-directory.1" \
            -o "${output}" \
            ./cmd/crane
    done
) 2>&1 | tee "${destination}/BUILD.log"
cmp "${staging}/crane-a" "${staging}/crane-b"

install -m 0755 "${staging}/crane-a" "${destination}/crane"
go version -m "${destination}/crane" >"${destination}/BUILDINFO.txt"

version_output="$("${destination}/crane" version)"
[[ "${version_output}" == "v${version}+ferrobox.root-directory.1" ]]
binary_sha256="$(sha256sum "${destination}/crane" | awk '{print $1}')"
binary_size="$(stat --format '%s' "${destination}/crane")"

cat >"${destination}/SOURCE.manifest" <<EOF
name=crane
upstream_version=${version}
version_output=${version_output}
release_tag=v${version}
release_commit=${release_commit}
source_url=${base_url}/${source_archive}
source_size_bytes=${source_size}
source_sha256=${source_sha256}
checksums_sha256=${checksums_sha256}
patch_file=go-containerregistry-v0.21.8-root-directory.patch
patch_sha256=${patch_sha256}
go_version=$(go env GOVERSION)
target=linux/amd64
build_reproducible_twice=true
binary_size_bytes=${binary_size}
binary_sha256=${binary_sha256}
upstream_slsa_provenance=absent
EOF

printf '%s\n' "${version_output}"
printf 'Built verified patched crane in %s\n' "${destination}"
