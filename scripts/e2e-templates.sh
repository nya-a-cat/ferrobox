#!/usr/bin/env bash
set -euo pipefail

evidence_dir="${FERROBOX_TEMPLATE_EVIDENCE_DIR:?FERROBOX_TEMPLATE_EVIDENCE_DIR is required}"
runner_temp="${RUNNER_TEMP:?RUNNER_TEMP is required}"
binary="target/debug/ferrobox"
work_dir="$(mktemp -d "${runner_temp}/ferrobox-template.XXXXXX")"
catalog="${work_dir}/catalog"
kernel="${work_dir}/vmlinux"
rootfs="${work_dir}/rootfs.ext4"
rootfs_backup="${work_dir}/rootfs.backup"

mkdir -p "${evidence_dir}"
printf 'ferrobox-template-kernel-v1\n' >"${kernel}"
printf 'ferrobox-template-rootfs-v1\n' >"${rootfs}"
cp "${rootfs}" "${rootfs_backup}"
source_digest="sha256:$(printf 'synthetic-oci-manifest-v1' | sha256sum | awk '{print $1}')"

build_template() {
  "${binary}" template --store "${catalog}" build \
    --name python \
    --version "$1" \
    --alias python-3-12 \
    --source-kind oci \
    --source-reference docker.io/library/python:3.12-slim \
    --source-digest "${source_digest}" \
    --target-arch amd64 \
    --kernel "${kernel}" \
    --rootfs "${rootfs}"
}

build_template 3.12.0 >"${evidence_dir}/build.json"
template_id="$(jq -er '.record.template_id | select(length == 64 and startswith("tpl-"))' "${evidence_dir}/build.json")"
spec_digest="$(jq -er '.record.spec_digest | select(startswith("sha256:") and length == 71)' "${evidence_dir}/build.json")"
jq -e --arg source_digest "${source_digest}" '
  .record.alias == "python-3-12" and
  .record.status == "ready" and
  .record.descriptor.source.kind == "oci" and
  .record.descriptor.source.digest == $source_digest and
  .record.descriptor.platform.architecture == "x86_64" and
  .verification.valid == true
' "${evidence_dir}/build.json" >/dev/null

"${binary}" template --store "${catalog}" list >"${evidence_dir}/list.json"
jq -e --arg template_id "${template_id}" '
  length == 1 and
  .[0].template_id == $template_id and
  .[0].version == "3.12.0" and
  .[0].source.reference == "docker.io/library/python:3.12-slim"
' "${evidence_dir}/list.json" >/dev/null

"${binary}" template --store "${catalog}" inspect "${template_id}" >"${evidence_dir}/inspect.json"
jq -e '.verification.valid == true and .verification.descriptor_valid == true' \
  "${evidence_dir}/inspect.json" >/dev/null

catalog_digest_before="$(
  find "${catalog}" -type f -print0 |
    sort -z |
    xargs -0 sha256sum |
    sha256sum |
    awk '{print $1}'
)"
"${binary}" template --store "${catalog}" render python-3-12 \
  --cpu 2 \
  --memory-mb 1024 \
  --ttl 600 \
  --internet >"${evidence_dir}/render-alias.json"
jq -e \
  --arg template_id "${template_id}" \
  --arg spec_digest "${spec_digest}" \
  --arg source_digest "${source_digest}" '
  .schema_version == 1 and
  .contract_version == "ferrobox-template-render-v1" and
  .requested_template == "python-3-12" and
  .resolved_template_id == $template_id and
  .template_spec_digest == $spec_digest and
  .template_source.kind == "oci" and
  .template_source.digest == $source_digest and
  .template_platform.architecture == "x86_64" and
  .effective_request == {
    template: $template_id,
    cpu_count: 2,
    memory_mb: 1024,
    timeout_seconds: 600,
    network: {internet_access: true}
  } and
  .artifact_verification == "deferred_to_create" and
  .mutation_performed == false
' "${evidence_dir}/render-alias.json" >/dev/null

"${binary}" template --store "${catalog}" render "${template_id}" \
  --cpu 2 \
  --memory-mb 1024 \
  --ttl 600 \
  --internet >"${evidence_dir}/render-id.json"
jq -e --arg template_id "${template_id}" '
  .requested_template == $template_id and
  .resolved_template_id == $template_id
' "${evidence_dir}/render-id.json" >/dev/null
jq -Sc '
  {
    resolved_template_id,
    template_spec_digest,
    template_source,
    template_platform,
    effective_request,
    artifact_verification,
    mutation_performed
  }
' "${evidence_dir}/render-alias.json" >"${evidence_dir}/render-alias-effective.json"
jq -Sc '
  {
    resolved_template_id,
    template_spec_digest,
    template_source,
    template_platform,
    effective_request,
    artifact_verification,
    mutation_performed
  }
' "${evidence_dir}/render-id.json" >"${evidence_dir}/render-id-effective.json"
cmp "${evidence_dir}/render-alias-effective.json" "${evidence_dir}/render-id-effective.json"

"${binary}" template --store "${catalog}" render python-3-12 \
  --cpu 2 \
  --memory-mb 1024 \
  --ttl 600 \
  --internet >"${evidence_dir}/render-repeat.json"
cmp "${evidence_dir}/render-alias.json" "${evidence_dir}/render-repeat.json"
render_digest="sha256:$(sha256sum "${evidence_dir}/render-alias.json" | awk '{print $1}')"

if "${binary}" template --store "${catalog}" render python-3-12 \
  --memory-mb 64 \
  >"${evidence_dir}/render-invalid.stdout" \
  2>"${evidence_dir}/render-invalid.stderr"; then
  echo "invalid rendered sandbox request unexpectedly succeeded" >&2
  exit 1
fi
grep -F 'invalid sandbox request' "${evidence_dir}/render-invalid.stderr" >/dev/null
grep -F 'memory_mb is outside the supported range' \
  "${evidence_dir}/render-invalid.stderr" >/dev/null
catalog_digest_after="$(
  find "${catalog}" -type f -print0 |
    sort -z |
    xargs -0 sha256sum |
    sha256sum |
    awk '{print $1}'
)"
test "${catalog_digest_after}" = "${catalog_digest_before}"

printf 'tamper\n' >>"${rootfs}"
"${binary}" template --store "${catalog}" inspect python-3-12 >"${evidence_dir}/tampered.json"
jq -e '
  .verification.valid == false and
  .verification.kernel.valid == true and
  .verification.rootfs.present == true and
  .verification.rootfs.valid == false
' "${evidence_dir}/tampered.json" >/dev/null
cp "${rootfs_backup}" "${rootfs}"

if build_template 3.12.1 >"${evidence_dir}/alias-conflict.stdout" 2>"${evidence_dir}/alias-conflict.stderr"; then
  echo "template alias mutation unexpectedly succeeded" >&2
  exit 1
fi
grep -F 'template alias python-3-12 already points to' \
  "${evidence_dir}/alias-conflict.stderr" >/dev/null

"${binary}" template --store "${catalog}" delete python-3-12 >"${evidence_dir}/delete.json"
jq -e --arg template_id "${template_id}" '
  .template_id == $template_id and .artifacts_preserved == true
' "${evidence_dir}/delete.json" >/dev/null
test -f "${kernel}"
test -f "${rootfs}"
test "$("${binary}" template --store "${catalog}" list | jq 'length')" -eq 0

build_template 3.12.0 >"${evidence_dir}/rebuild.json"
jq -e --arg template_id "${template_id}" --arg spec_digest "${spec_digest}" '
  .record.template_id == $template_id and
  .record.spec_digest == $spec_digest and
  .verification.valid == true
' "${evidence_dir}/rebuild.json" >/dev/null

jq -n \
  --arg repository "${GITHUB_REPOSITORY:-unknown}" \
  --arg commit "${GITHUB_SHA:-unknown}" \
  --arg run_id "${GITHUB_RUN_ID:-unknown}" \
  --arg template_id "${template_id}" \
  --arg spec_digest "${spec_digest}" \
  --arg source_digest "${source_digest}" \
  --arg render_digest "${render_digest}" \
  '{
    contract_version: "ferrobox-template-catalog-evidence-v1",
    render_contract_version: "ferrobox-template-render-v1",
    repository: $repository,
    commit: $commit,
    run_id: $run_id,
    template_id: $template_id,
    spec_digest: $spec_digest,
    source_digest: $source_digest,
    render_digest: $render_digest,
    checks: [
      {name: "build", success: true},
      {name: "list", success: true},
      {name: "inspect_by_id", success: true},
      {name: "render_alias_resolution", success: true},
      {name: "render_id_resolution", success: true},
      {name: "render_deterministic", success: true},
      {name: "render_catalog_read_only", success: true},
      {name: "render_resource_validation", success: true},
      {name: "artifact_tamper_detection", success: true},
      {name: "immutable_alias", success: true},
      {name: "delete_preserves_artifacts", success: true},
      {name: "identity_stable_after_rebuild", success: true}
    ]
  }' >"${evidence_dir}/evidence.json"

"${binary}" template --store "${catalog}" delete "${template_id}" \
  >"${evidence_dir}/final-delete.json"
