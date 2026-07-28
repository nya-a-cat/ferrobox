#!/usr/bin/env bash
set -euo pipefail

version="1.16.1"
archive="firecracker-v${version}-x86_64.tgz"
expected_sha256="382a02a869e4d6d5cb14c40577f9545e8458021ea8b0b2d3fc10ec14d9c242e6"
base_url="https://github.com/firecracker-microvm/firecracker/releases/download/v${version}"
destination="${1:-${XDG_CACHE_HOME:-${HOME}/.cache}/ferrobox/firecracker-v${version}}"

case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) ;;
    *)
        echo "Ferrobox v0.1 supports this asset only on Linux x86_64." >&2
        exit 2
        ;;
esac

staging="$(mktemp -d)"
cleanup() {
    rm -rf -- "${staging}"
}
trap cleanup EXIT

curl \
    --fail \
    --location \
    --proto '=https' \
    --show-error \
    --silent \
    --tlsv1.2 \
    --output "${staging}/${archive}" \
    "${base_url}/${archive}"

printf '%s  %s\n' "${expected_sha256}" "${staging}/${archive}" |
    sha256sum --check --strict

while IFS= read -r member; do
    case "${member}" in
        /*|../*|*/../*|*/..)
            echo "Unsafe archive member: ${member}" >&2
            exit 3
            ;;
    esac
done < <(tar -tzf "${staging}/${archive}")

mkdir -p -- "${staging}/unpacked"
tar -xzf "${staging}/${archive}" -C "${staging}/unpacked"

firecracker_source="$(
    find "${staging}/unpacked" -type f -name 'firecracker-v*' -perm -u+x -print -quit
)"
jailer_source="$(
    find "${staging}/unpacked" -type f -name 'jailer-v*' -perm -u+x -print -quit
)"

if [[ -z "${firecracker_source}" || -z "${jailer_source}" ]]; then
    echo "The verified archive did not contain both executables." >&2
    exit 4
fi

mkdir -p -- "${destination}"
install -m 0755 "${firecracker_source}" "${destination}/firecracker"
install -m 0755 "${jailer_source}" "${destination}/jailer"
printf '%s  %s\n' "${expected_sha256}" "${archive}" \
    >"${destination}/SOURCE.sha256"
printf '%s\n' "${base_url}/${archive}" >"${destination}/SOURCE.url"

"${destination}/firecracker" --version
"${destination}/jailer" --version
printf 'Installed verified Firecracker tools in %s\n' "${destination}"

