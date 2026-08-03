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
packages_dir="${evidence_dir}/packages"
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
for command_name in cargo corepack curl dotnet go gofmt java mvn node npm rustc sha256sum tar uv; do
    command -v "${command_name}" >/dev/null
done
for language in csharp go java kotlin python rust typescript-fetch; do
    test -d "${generated_root}/${language}"
done
mkdir -p "${clients_dir}" "${locks_dir}" "${tooling_dir}" "${packages_dir}"
install -m 0644 openapi/ferrobox-sdk-packages.json \
    "${evidence_dir}/ferrobox-sdk-packages.json"

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
    local consumer="${work_dir}/csharp-consumer"
    local package_dir="${packages_dir}/csharp"
    local project="${client}/src/Ferrobox.Client/Ferrobox.Client.csproj"
    cp -a -- "${generated_root}/csharp" "${client}"
    mkdir -p "${consumer}" "${package_dir}"
    install -D -m 0644 \
        scripts/openapi-e2e/csharp/FerroboxSdkE2E.csproj \
        "${consumer}/FerroboxSdkE2E.csproj"
    install -m 0644 scripts/openapi-e2e/csharp/Program.cs "${consumer}/Program.cs"

    dotnet restore "${project}" \
        --use-lock-file --packages "${NUGET_PACKAGES}"
    dotnet restore "${project}" \
        --locked-mode --packages "${NUGET_PACKAGES}"
    dotnet pack "${project}" --configuration Release --no-restore \
        --output "${package_dir}"
    test -f "${package_dir}/Ferrobox.Client.0.1.0.nupkg"
    dotnet restore "${consumer}/FerroboxSdkE2E.csproj" \
        --use-lock-file --packages "${NUGET_PACKAGES}" \
        --source "${package_dir}" --source https://api.nuget.org/v3/index.json
    dotnet restore "${consumer}/FerroboxSdkE2E.csproj" \
        --locked-mode --packages "${NUGET_PACKAGES}" \
        --source "${package_dir}" --source https://api.nuget.org/v3/index.json
    cmp --silent -- \
        "${package_dir}/Ferrobox.Client.0.1.0.nupkg" \
        "${NUGET_PACKAGES}/ferrobox.client/0.1.0/ferrobox.client.0.1.0.nupkg"
    install -m 0644 \
        "${client}/src/Ferrobox.Client/packages.lock.json" \
        "${locks_dir}/csharp-library.packages.lock.json"
    install -m 0644 \
        "${consumer}/packages.lock.json" \
        "${locks_dir}/csharp-harness.packages.lock.json"
    FERROBOX_OPENAPI_SDK_EVIDENCE="${evidence_dir}/csharp.json" \
        dotnet run --project "${consumer}/FerroboxSdkE2E.csproj" \
            --configuration Release --no-restore
}

run_go() {
    local consumer="${work_dir}/go-consumer"
    local format_diff="${work_dir}/go-format.diff"
    local proxy="${packages_dir}/go/proxy"
    uv run --no-project --python 3.12 python scripts/build-go-sdk-module-proxy.py \
        --source "${generated_root}/go" \
        --module github.com/nya-a-cat/ferrobox/sdk/go \
        --version v0.1.0 \
        --output "${proxy}"
    mkdir -p "${consumer}"
    install -m 0644 scripts/openapi-e2e/go/go.mod "${consumer}/go.mod"
    install -m 0644 scripts/openapi-e2e/go/main.go "${consumer}/main.go"
    gofmt -d "${consumer}/main.go" | tee "${format_diff}"
    test ! -s "${format_diff}"
    (
        cd "${consumer}"
        export GOMODCACHE="${work_dir}/go-mod-cache"
        export GOPROXY="file://${proxy}"
        export GOSUMDB=off
        go mod download all
        go mod verify
        test -s go.sum
        install -m 0644 go.sum "${locks_dir}/go.sum"
        FERROBOX_OPENAPI_SDK_EVIDENCE="${evidence_dir}/go.json" \
            GOPROXY=off go run -mod=readonly .
    )
}

run_java() {
    local client="${clients_dir}/java"
    local consumer="${work_dir}/java-consumer"
    local package_dir="${packages_dir}/java"
    local maven_repository="${work_dir}/maven-repository"
    cp -a -- "${generated_root}/java" "${client}"
    mkdir -p "${consumer}/src/test/java/io/github/nyaacat/ferrobox/client" "${package_dir}"
    install -m 0644 scripts/openapi-e2e/java/pom.xml "${consumer}/pom.xml"
    install -D -m 0644 \
        scripts/openapi-e2e/java/GeneratedClientE2E.java \
        "${consumer}/src/test/java/io/github/nyaacat/ferrobox/client/GeneratedClientE2E.java"
    install -m 0644 "${client}/pom.xml" "${locks_dir}/java-pom.xml"
    (
        cd "${client}"
        mvn --batch-mode --no-transfer-progress --strict-checksums \
            -Dmaven.repo.local="${maven_repository}" dependency:go-offline
        mvn --batch-mode --no-transfer-progress --offline \
            -Dmaven.repo.local="${maven_repository}" -DskipTests install
    )
    install -m 0644 \
        "${client}/target/ferrobox-java-client-0.1.0.jar" \
        "${package_dir}/ferrobox-java-client-0.1.0.jar"
    install -m 0644 "${client}/pom.xml" "${package_dir}/ferrobox-java-client-0.1.0.pom"
    install -m 0644 "${consumer}/pom.xml" "${locks_dir}/java-consumer-pom.xml"
    (
        cd "${consumer}"
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
    local consumer="${work_dir}/kotlin-consumer"
    local package_repository="${packages_dir}/kotlin/repository"
    local properties="${client}/gradle/wrapper/gradle-wrapper.properties"
    local wrapper="${client}/gradlew"
    cp -a -- "${generated_root}/kotlin" "${client}"
    mkdir -p "${consumer}/src/main/kotlin" "${package_repository}"
    install -m 0644 scripts/openapi-e2e/kotlin/consumer.build.gradle \
        "${consumer}/build.gradle"
    install -m 0644 scripts/openapi-e2e/kotlin/consumer.settings.gradle \
        "${consumer}/settings.gradle"
    install -m 0644 scripts/openapi-e2e/kotlin/FerroboxSdkE2E.kt \
        "${consumer}/src/main/kotlin/FerroboxSdkE2E.kt"
    test "$(sha256sum "${client}/gradle/wrapper/gradle-wrapper.jar" | cut -d' ' -f1)" = \
        "498495120a03b9a6ab5d155f5de3c8f0d986a449153702fb80fc80e134484f17"
    grep -Fqx \
        'distributionUrl=https\://services.gradle.org/distributions/gradle-8.14.3-all.zip' \
        "${properties}"
    sed -i '$a\distributionSha256Sum=ed1a8d686605fd7c23bdf62c7fc7add1c5b23b2bbc3721e661934ef4a4911d7c' \
        "${properties}"
    install -m 0644 "${properties}" "${locks_dir}/kotlin-gradle-wrapper.properties"
    chmod +x "${wrapper}"
    export FERROBOX_KOTLIN_PACKAGE_REPOSITORY="${package_repository}"
    (
        cd "${client}"
        export GRADLE_USER_HOME="${work_dir}/gradle-home"
        "${wrapper}" --no-daemon \
            --init-script "${repo_root}/scripts/openapi-e2e/kotlin/e2e.init.gradle" \
            ferroboxSdkPrefetch --write-locks
        test -s gradle.lockfile
        install -m 0644 gradle.lockfile "${locks_dir}/kotlin-gradle.lockfile"
        "${wrapper}" --no-daemon --offline \
            --init-script "${repo_root}/scripts/openapi-e2e/kotlin/e2e.init.gradle" \
            publishMavenPublicationToFerroboxRepository
    )
    test -f \
        "${package_repository}/io/github/nyaacat/ferrobox/ferrobox-kotlin-client/0.1.0/ferrobox-kotlin-client-0.1.0.jar"
    (
        export GRADLE_USER_HOME="${work_dir}/gradle-home"
        "${wrapper}" --no-daemon --project-dir "${consumer}" \
            --init-script "${repo_root}/scripts/openapi-e2e/kotlin/e2e.init.gradle" \
            ferroboxSdkPrefetch --write-locks
        test -s "${consumer}/gradle.lockfile"
        install -m 0644 "${consumer}/gradle.lockfile" \
            "${locks_dir}/kotlin-consumer-gradle.lockfile"
        FERROBOX_OPENAPI_SDK_EVIDENCE="${evidence_dir}/kotlin.json" \
            "${wrapper}" --no-daemon --offline --project-dir "${consumer}" \
                --init-script "${repo_root}/scripts/openapi-e2e/kotlin/e2e.init.gradle" \
                ferroboxSdkE2E
    )
}

run_python() {
    FERROBOX_OPENAPI_SDK_EVIDENCE="${evidence_dir}/python.json" \
    FERROBOX_OPENAPI_PYTHON_LOCK="${locks_dir}/python-uv.lock" \
    FERROBOX_OPENAPI_PYTHON_FREEZE="${locks_dir}/python-consumer-freeze.txt" \
    FERROBOX_OPENAPI_SDK_PACKAGE_DIR="${packages_dir}/python" \
        bash scripts/e2e-openapi-python.sh "${generated_root}/python"
}

run_rust() {
    local client="${clients_dir}/rust"
    local harness="${work_dir}/rust-harness"
    local unpack_root="${work_dir}/rust-package"
    local package_dir="${packages_dir}/rust"
    local crate="${package_dir}/ferrobox-client-0.1.0.crate"
    cp -a -- "${generated_root}/rust" "${client}"
    mkdir -p "${unpack_root}" "${package_dir}"
    export CARGO_HOME="${work_dir}/cargo-home"
    cargo generate-lockfile --manifest-path "${client}/Cargo.toml"
    cargo fetch --manifest-path "${client}/Cargo.toml" --locked
    cargo package --manifest-path "${client}/Cargo.toml" --locked --offline
    install -m 0644 "${client}/target/package/ferrobox-client-0.1.0.crate" "${crate}"
    tar -xzf "${crate}" -C "${unpack_root}"
    test -f "${unpack_root}/ferrobox-client-0.1.0/src/lib.rs"
    install -D -m 0644 scripts/openapi-e2e/rust/Cargo.toml "${harness}/Cargo.toml"
    install -D -m 0644 scripts/openapi-e2e/rust/main.rs "${harness}/src/main.rs"
    cargo fmt --manifest-path "${harness}/Cargo.toml" -- --check
    cargo generate-lockfile --manifest-path "${harness}/Cargo.toml"
    install -m 0644 "${harness}/Cargo.lock" "${locks_dir}/rust-Cargo.lock"
    cargo fetch --manifest-path "${harness}/Cargo.toml" --locked
    FERROBOX_OPENAPI_SDK_EVIDENCE="${evidence_dir}/rust.json" \
        cargo run --manifest-path "${harness}/Cargo.toml" --locked --offline
}

run_typescript() {
    local client="${clients_dir}/typescript-fetch"
    local consumer="${work_dir}/typescript-consumer"
    local runtime_package_dir="${work_dir}/typescript-package"
    local package_dir="${packages_dir}/typescript"
    cp -a -- "${generated_root}/typescript-fetch" "${client}"
    mkdir -p "${consumer}" "${runtime_package_dir}" "${package_dir}"
    install -m 0644 scripts/openapi-e2e/typescript/e2e.ts "${consumer}/e2e.ts"
    install -m 0644 scripts/openapi-e2e/typescript/package.json "${consumer}/package.json"
    install -m 0644 scripts/openapi-e2e/typescript/tsconfig.json "${consumer}/tsconfig.json"
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
        install -m 0644 pnpm-lock.yaml "${locks_dir}/typescript-package-pnpm-lock.yaml"
        corepack pnpm@10.15.1 run build
        corepack pnpm@10.15.1 pack --pack-destination "${runtime_package_dir}"
    )
    test -f "${runtime_package_dir}/nya-a-cat-ferrobox-0.1.0.tgz"
    install -m 0644 "${runtime_package_dir}/nya-a-cat-ferrobox-0.1.0.tgz" \
        "${package_dir}/nya-a-cat-ferrobox-0.1.0.tgz"
    (
        cd "${consumer}"
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

uv run --no-project --python 3.12 python scripts/check-openapi-sdk-packages.py \
    --contract "${evidence_dir}/ferrobox-sdk-packages.json" \
    --packages-dir "${packages_dir}" \
    --evidence-dir "${evidence_dir}" \
    --output "${evidence_dir}/packages.json"
uv run --no-project --python 3.12 python scripts/check-openapi-sdk-evidence.py \
    --evidence-dir "${evidence_dir}" \
    --audit-log "${audit_path}" \
    --locks-dir "${locks_dir}" \
    --packages "${evidence_dir}/packages.json" \
    --output "${evidence_dir}/matrix.json"
install -m 0644 "${audit_path}" "${evidence_dir}/audit-events.jsonl"
