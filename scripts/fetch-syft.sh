#!/usr/bin/env bash
set -euo pipefail

version="1.50.0"
archive="syft_${version}_linux_amd64.tar.gz"
expected_sha256="bf7b29ff57f06da30918266a0e1c2885a8f99784798d1bdb1628886aa015d788"
base_url="https://github.com/anchore/syft/releases/download/v${version}"
destination="${1:-${XDG_CACHE_HOME:-${HOME}/.cache}/ferrobox/syft-v${version}}"

case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) ;;
    *)
        echo "Ferrobox supports this Syft asset only on Linux x86_64." >&2
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

syft_source="$(
    find "${staging}/unpacked" -type f -name syft -perm -u+x -print -quit
)"
if [[ -z "${syft_source}" ]]; then
    echo "The verified archive did not contain the Syft executable." >&2
    exit 4
fi

mkdir -p -- "${destination}"
install -m 0755 "${syft_source}" "${destination}/syft"
printf '%s  %s\n' "${expected_sha256}" "${archive}" \
    >"${destination}/SOURCE.sha256"
printf '%s\n' "${base_url}/${archive}" >"${destination}/SOURCE.url"

version_output="$(SYFT_CHECK_FOR_APP_UPDATE=false "${destination}/syft" version)"
grep --fixed-strings "${version}" <<<"${version_output}" >/dev/null
printf '%s\n' "${version_output}"
printf 'Installed verified Syft in %s\n' "${destination}"
