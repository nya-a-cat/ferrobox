#!/usr/bin/env bash
set -euo pipefail

version="7.22.0"
expected_sha256="37f23217f40cabac50c435312ea1d3ff5e61271092edb210695cd6e876a7cc8c"
expected_size="31390141"
destination="${1:?destination directory is required}"

if [[ -e "${destination}" ]]; then
    echo "destination already exists: ${destination}" >&2
    exit 1
fi

install -d -m 0755 "${destination}"
jar="${destination}/openapi-generator-cli-${version}.jar"
url="https://github.com/OpenAPITools/openapi-generator/releases/download/v${version}/openapi-generator-cli-${version}.jar"

curl --fail --location --silent --show-error \
    --output "${jar}" \
    "${url}"
actual_size="$(stat --format='%s' "${jar}")"
actual_sha256="$(sha256sum "${jar}" | awk '{print $1}')"
[[ "${actual_size}" == "${expected_size}" ]]
[[ "${actual_sha256}" == "${expected_sha256}" ]]
[[ "$(java -jar "${jar}" version)" == "${version}" ]]

{
    printf 'name=openapi-generator-cli\n'
    printf 'version=%s\n' "${version}"
    printf 'release_tag=v%s\n' "${version}"
    printf 'release_commit=%s\n' "f4d1cb8c15e1bc0476c75bcbc3febf1edec89b25"
    printf 'source_url=%s\n' "${url}"
    printf 'size_bytes=%s\n' "${actual_size}"
    printf 'sha256=%s\n' "${actual_sha256}"
} >"${destination}/openapi-generator.manifest"

printf '%s\n' "${jar}"
