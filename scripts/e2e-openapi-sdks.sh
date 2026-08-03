#!/usr/bin/env bash
set -euo pipefail

generated_root="${1:?generated client root is required}"
evidence_dir="${FERROBOX_OPENAPI_SDK_EVIDENCE_DIR:?evidence directory is required}"
api_url="${FERROBOX_API_URL:-http://127.0.0.1:18083}"
repo_root="${GITHUB_WORKSPACE:?GITHUB_WORKSPACE is required}"
work_dir="$(mktemp -d)"
clients_dir="${work_dir}/clients"
locks_dir="${evidence_dir}/locks"
tooling_dir="${evidence_dir}/tooling"
audit_path="${work_dir}/audit/events.jsonl"
api_pid=""
declare -a failures=()

cleanup() {
    status="$?"
    set +e
    if [[ -f "${audit_path}" ]]; then
        install -D -m 0644 "${audit_path}" "${evidence_dir}/audit-events.jsonl"
    fi
    if [[ "${status}" -ne 0 && -f "${work_dir}/api.log" ]]; then
        echo "::group::Generated OpenAPI SDK API log"
        sed -n '1,400p' "${work_dir}/api.log"
        echo "::endgroup::"
    fi
    if [[ -n "${api_pid}" ]]; then
        kill "${api_pid}" 2>/dev/null || true
        wait "${api_pid}" 2>/dev/null || true
    fi
    if [[ -d "${work_dir}/go-mod-cache" ]]; then
        chmod -R u+w -- "${work_dir}/go-mod-cache"
    fi
    rm -rf -- "${work_dir}"
    return "${status}"
}
trap cleanup EXIT

test "${api_url}" = "http://127.0.0.1:18083"
test -x target/debug/ferrobox-api
for command_name in cargo corepack curl dotnet go gofmt java mvn node npm rustc sha256sum uv; do
    command -v "${command_name}" >/dev/null
done
for language in csharp go java kotlin python rust typescript-fetch; do
    test -d "${generated_root}/${language}"
done
mkdir -p "${clients_dir}" "${locks_dir}" "${tooling_dir}"

export FERROBOX_API_URL="${api_url}"
export FERROBOX_AUDIT_LOG="${audit_path}"
export DOTNET_CLI_TELEMETRY_OPTOUT=1
export DOTNET_NOLOGO=1
export NUGET_PACKAGES="${work_dir}/nuget-packages"

target/debug/ferrobox-api \
    --backend process \
    --unsafe-process-runtime \
    --listen 127.0.0.1:18083 \
    --process-root "${work_dir}/sandboxes" \
    --audit-log "${audit_path}" \
    >"${work_dir}/api.log" 2>&1 &
api_pid="$!"

for _ in $(seq 1 120); do
    if curl --fail --silent "${api_url}/healthz" >/dev/null; then
        break
    fi
    if ! kill -0 "${api_pid}" 2>/dev/null; then
        wait "${api_pid}"
    fi
    sleep 0.25
done
curl --fail --silent "${api_url}/healthz" >/dev/null

run_csharp() {
    local client="${clients_dir}/csharp"
    cp -a -- "${generated_root}/csharp" "${client}"
    install -D -m 0644 \
        scripts/openapi-e2e/csharp/FerroboxSdkE2E.csproj \
        "${client}/e2e/FerroboxSdkE2E.csproj"
    install -m 0644 scripts/openapi-e2e/csharp/Program.cs "${client}/e2e/Program.cs"

    dotnet restore "${client}/src/Org.OpenAPITools/Org.OpenAPITools.csproj" \
        --use-lock-file --packages "${NUGET_PACKAGES}"
    dotnet restore "${client}/src/Org.OpenAPITools/Org.OpenAPITools.csproj" \
        --locked-mode --packages "${NUGET_PACKAGES}"
    dotnet restore "${client}/e2e/FerroboxSdkE2E.csproj" \
        --use-lock-file --packages "${NUGET_PACKAGES}"
    dotnet restore "${client}/e2e/FerroboxSdkE2E.csproj" \
        --locked-mode --packages "${NUGET_PACKAGES}"
    install -m 0644 \
        "${client}/src/Org.OpenAPITools/packages.lock.json" \
        "${locks_dir}/csharp-library.packages.lock.json"
    install -m 0644 \
        "${client}/e2e/packages.lock.json" \
        "${locks_dir}/csharp-harness.packages.lock.json"
    FERROBOX_OPENAPI_SDK_EVIDENCE="${evidence_dir}/csharp.json" \
        dotnet run --project "${client}/e2e/FerroboxSdkE2E.csproj" \
            --configuration Release --no-restore
}

run_go() {
    local client="${clients_dir}/go"
    local format_diff="${work_dir}/go-format.diff"
    cp -a -- "${generated_root}/go" "${client}"
    install -D -m 0644 scripts/openapi-e2e/go/main.go "${client}/cmd/ferrobox-e2e/main.go"
    gofmt -d "${client}/cmd/ferrobox-e2e/main.go" | tee "${format_diff}"
    test ! -s "${format_diff}"
    install -m 0644 "${client}/go.sum" "${locks_dir}/go.sum"
    (
        cd "${client}"
        export GOMODCACHE="${work_dir}/go-mod-cache"
        go mod verify
        FERROBOX_OPENAPI_SDK_EVIDENCE="${evidence_dir}/go.json" \
            go run -mod=readonly ./cmd/ferrobox-e2e
    )
}

run_java() {
    local client="${clients_dir}/java"
    local maven_repository="${work_dir}/maven-repository"
    cp -a -- "${generated_root}/java" "${client}"
    install -D -m 0644 \
        scripts/openapi-e2e/java/GeneratedClientE2E.java \
        "${client}/src/test/java/org/openapitools/client/GeneratedClientE2E.java"
    install -m 0644 "${client}/pom.xml" "${locks_dir}/java-pom.xml"
    (
        cd "${client}"
        mvn --batch-mode --no-transfer-progress --strict-checksums \
            -Dmaven.repo.local="${maven_repository}" dependency:go-offline
        mvn --batch-mode --no-transfer-progress --strict-checksums \
            -Dmaven.repo.local="${maven_repository}" \
            org.apache.maven.plugins:maven-dependency-plugin:3.6.1:get \
            -Dartifact=org.apache.maven.surefire:surefire-junit-platform:2.22.2 \
            -Dtransitive=true
        provider_path="${maven_repository}/org/apache/maven/surefire/surefire-junit-platform/2.22.2/surefire-junit-platform-2.22.2.jar"
        test -f "${provider_path}"
        printf 'artifact=%s\nsha256=%s\n' \
            'org.apache.maven.surefire:surefire-junit-platform:2.22.2' \
            "$(sha256sum "${provider_path}" | cut -d' ' -f1)" \
            >"${locks_dir}/java-surefire-provider.sha256"
        mvn --batch-mode --no-transfer-progress --offline \
            -Dmaven.repo.local="${maven_repository}" \
            -DoutputFile="${locks_dir}/java-dependency-tree.txt" dependency:tree
        FERROBOX_OPENAPI_SDK_EVIDENCE="${evidence_dir}/java.json" \
            mvn --batch-mode --no-transfer-progress --offline \
                -Dmaven.repo.local="${maven_repository}" \
                -Dtest=GeneratedClientE2E test
    )
}

run_kotlin() {
    local client="${clients_dir}/kotlin"
    local properties="${client}/gradle/wrapper/gradle-wrapper.properties"
    local wrapper="${client}/gradlew"
    cp -a -- "${generated_root}/kotlin" "${client}"
    install -m 0644 \
        scripts/openapi-e2e/kotlin/FerroboxSdkE2E.kt \
        "${client}/src/main/kotlin/FerroboxSdkE2E.kt"
    test "$(sha256sum "${client}/gradle/wrapper/gradle-wrapper.jar" | cut -d' ' -f1)" = \
        "498495120a03b9a6ab5d155f5de3c8f0d986a449153702fb80fc80e134484f17"
    grep -Fqx \
        'distributionUrl=https\://services.gradle.org/distributions/gradle-8.14.3-all.zip' \
        "${properties}"
    sed -i '$a\distributionSha256Sum=ed1a8d686605fd7c23bdf62c7fc7add1c5b23b2bbc3721e661934ef4a4911d7c' \
        "${properties}"
    install -m 0644 "${properties}" "${locks_dir}/kotlin-gradle-wrapper.properties"
    chmod +x "${wrapper}"
    (
        cd "${client}"
        export GRADLE_USER_HOME="${work_dir}/gradle-home"
        "${wrapper}" --no-daemon \
            --init-script "${repo_root}/scripts/openapi-e2e/kotlin/e2e.init.gradle" \
            ferroboxSdkPrefetch --write-locks
        test -s gradle.lockfile
        install -m 0644 gradle.lockfile "${locks_dir}/kotlin-gradle.lockfile"
        FERROBOX_OPENAPI_SDK_EVIDENCE="${evidence_dir}/kotlin.json" \
            "${wrapper}" --no-daemon --offline \
                --init-script "${repo_root}/scripts/openapi-e2e/kotlin/e2e.init.gradle" \
                ferroboxSdkE2E
    )
}

run_python() {
    FERROBOX_OPENAPI_SDK_EVIDENCE="${evidence_dir}/python.json" \
    FERROBOX_OPENAPI_PYTHON_LOCK="${locks_dir}/python-uv.lock" \
        bash scripts/e2e-openapi-python.sh "${generated_root}/python"
}

run_rust() {
    local client="${clients_dir}/rust"
    local harness="${work_dir}/rust-harness"
    cp -a -- "${generated_root}/rust" "${client}"
    install -D -m 0644 scripts/openapi-e2e/rust/Cargo.toml "${harness}/Cargo.toml"
    install -D -m 0644 scripts/openapi-e2e/rust/main.rs "${harness}/src/main.rs"
    export CARGO_HOME="${work_dir}/cargo-home"
    cargo fmt --manifest-path "${harness}/Cargo.toml" -- --check
    cargo generate-lockfile --manifest-path "${harness}/Cargo.toml"
    install -m 0644 "${harness}/Cargo.lock" "${locks_dir}/rust-Cargo.lock"
    cargo fetch --manifest-path "${harness}/Cargo.toml" --locked
    FERROBOX_OPENAPI_SDK_EVIDENCE="${evidence_dir}/rust.json" \
        cargo run --manifest-path "${harness}/Cargo.toml" --locked --offline
}

run_typescript() {
    local client="${clients_dir}/typescript-fetch"
    cp -a -- "${generated_root}/typescript-fetch" "${client}"
    install -m 0644 scripts/openapi-e2e/typescript/e2e.ts "${client}/e2e.ts"
    install -m 0644 scripts/openapi-e2e/typescript/package.json "${client}/package.json"
    install -m 0644 scripts/openapi-e2e/typescript/tsconfig.json "${client}/tsconfig.json"
    uv run --no-project --python 3.12 python scripts/check-npm-tooling-metadata.py \
        --output "${tooling_dir}/npm-metadata.json"
    node -e 'if (Number(process.versions.node.split(".")[0]) < 20) process.exit(1)'
    (
        cd "${client}"
        export COREPACK_HOME="${work_dir}/corepack"
        export PNPM_HOME="${work_dir}/pnpm-home"
        export PNPM_STORE_DIR="${work_dir}/pnpm-store"
        corepack pnpm@10.15.1 install --lockfile-only --ignore-scripts
        corepack pnpm@10.15.1 fetch --frozen-lockfile
        corepack pnpm@10.15.1 install --offline --frozen-lockfile --ignore-scripts
        install -m 0644 pnpm-lock.yaml "${locks_dir}/typescript-pnpm-lock.yaml"
        corepack pnpm@10.15.1 run build
        FERROBOX_OPENAPI_SDK_EVIDENCE="${evidence_dir}/typescript.json" node dist/e2e.js
    )
}

run_one() {
    local name="$1"
    local function_name="$2"
    local status
    echo "::group::Generated ${name} SDK end-to-end"
    set +e
    (
        set -euo pipefail
        "${function_name}"
    )
    status="$?"
    set -e
    echo "::endgroup::"
    if [[ "${status}" -ne 0 ]]; then
        failures+=("${name}:${status}")
        echo "::error::Generated ${name} SDK end-to-end failed with status ${status}"
    fi
}

run_one csharp run_csharp
run_one go run_go
run_one java run_java
run_one kotlin run_kotlin
run_one python run_python
run_one rust run_rust
run_one typescript run_typescript

run_toolchain_record() {
    uv run --no-project --python 3.12 python scripts/record-openapi-sdk-toolchains.py \
        --kotlin-root "${generated_root}/kotlin" \
        --gradle-wrapper "${clients_dir}/kotlin/gradlew" \
        --output "${evidence_dir}/toolchains.json"
}
run_one toolchains run_toolchain_record

if [[ "${#failures[@]}" -ne 0 ]]; then
    printf 'Generated SDK failures: %s\n' "${failures[*]}" >&2
    exit 1
fi

uv run --no-project --python 3.12 python scripts/check-openapi-sdk-evidence.py \
    --evidence-dir "${evidence_dir}" \
    --audit-log "${audit_path}" \
    --locks-dir "${locks_dir}" \
    --output "${evidence_dir}/matrix.json"
install -m 0644 "${audit_path}" "${evidence_dir}/audit-events.jsonl"
