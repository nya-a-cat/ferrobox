#!/usr/bin/env bash
set -euo pipefail

generator_jar="${1:?OpenAPI Generator JAR is required}"
spec="${2:?OpenAPI specification is required}"
output_root="${3:?output root is required}"
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"
overlay="${repo_root}/openapi/ferrobox-codegen-overlay.json"
codegen_overlay="${output_root}/.ferrobox-codegen-overlay.json"
codegen_spec="${output_root}/.ferrobox-codegen-openapi.json"

test -f "${generator_jar}"
test -f "${spec}"
test -f "${overlay}"
if [[ -e "${output_root}" ]]; then
    echo "output root already exists: ${output_root}" >&2
    exit 1
fi
install -d -m 0755 "${output_root}"
install -m 0644 "${overlay}" "${codegen_overlay}"
uv run --no-project --python 3.12 python \
    "${repo_root}/scripts/openapi_codegen_projection.py" \
    "${spec}" \
    "${codegen_overlay}" \
    "${codegen_spec}"

generators=(
    csharp
    go
    java
    kotlin
    python
    rust
    typescript-fetch
)

for generator in "${generators[@]}"; do
    extra_properties="hideGenerationTimestamp=true"
    if [[ "${generator}" == "csharp" ]]; then
        extra_properties+=",packageGuid={DAC7944B-0ADC-4E95-AADD-294DC79ACC69}"
    fi
    if [[ "${generator}" == "python" ]]; then
        extra_properties+=",packageName=ferrobox_client,projectName=ferrobox-client,packageVersion=0.1.0"
    fi
    java -jar "${generator_jar}" generate \
        --input-spec "${codegen_spec}" \
        --generator-name "${generator}" \
        --output "${output_root}/${generator}" \
        --additional-properties "${extra_properties}" \
        --global-property 'apiTests=false,modelTests=false,apiDocs=false,modelDocs=false'
done
