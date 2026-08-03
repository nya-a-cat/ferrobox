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
shared_codegen_root="${RUNNER_TEMP:?RUNNER_TEMP is required}/ferrobox-openapi-codegen"
shared_codegen_spec="${shared_codegen_root}/ferrobox-codegen-openapi.json"

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
install -d -m 0755 "${shared_codegen_root}"
if [[ -e "${shared_codegen_spec}" ]]; then
    cmp --silent -- "${codegen_spec}" "${shared_codegen_spec}"
else
    install -m 0644 "${codegen_spec}" "${shared_codegen_spec}"
fi

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
    extra_properties="hideGenerationTimestamp=true,licenseName=Apache-2.0"
    git_repo_id="ferrobox"
    case "${generator}" in
        csharp)
            extra_properties+=",packageName=Ferrobox.Client,packageVersion=0.1.0,packageGuid={DAC7944B-0ADC-4E95-AADD-294DC79ACC69},packageAuthors=Ferrobox contributors,packageCompany=Ferrobox,packageTitle=Ferrobox Client,packageDescription=Generated client for the Ferrobox v1 API"
            ;;
        go)
            extra_properties+=",packageName=ferrobox,packageVersion=0.1.0,withGoMod=true"
            git_repo_id="ferrobox/sdk/go"
            ;;
        java)
            extra_properties+=",groupId=io.github.nyaacat.ferrobox,artifactId=ferrobox-java-client,artifactVersion=0.1.0,invokerPackage=io.github.nyaacat.ferrobox.client,apiPackage=io.github.nyaacat.ferrobox.client.api,modelPackage=io.github.nyaacat.ferrobox.client.model"
            ;;
        kotlin)
            extra_properties+=",groupId=io.github.nyaacat.ferrobox,artifactId=ferrobox-kotlin-client,artifactVersion=0.1.0,packageName=io.github.nyaacat.ferrobox.kotlin"
            ;;
        python)
            extra_properties+=",packageName=ferrobox_client,projectName=ferrobox-client,packageVersion=0.1.0"
            ;;
        rust)
            extra_properties+=",packageName=ferrobox-client,packageVersion=0.1.0"
            ;;
        typescript-fetch)
            extra_properties+=",npmName=@nya-a-cat/ferrobox,npmVersion=0.1.0,supportsES6=true"
            ;;
    esac
    java -jar "${generator_jar}" generate \
        --input-spec "${shared_codegen_spec}" \
        --generator-name "${generator}" \
        --output "${output_root}/${generator}" \
        --git-host github.com \
        --git-user-id nya-a-cat \
        --git-repo-id "${git_repo_id}" \
        --additional-properties "${extra_properties}" \
        --global-property 'apiTests=false,modelTests=false,apiDocs=false,modelDocs=false'
done
