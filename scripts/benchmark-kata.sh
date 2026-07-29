#!/usr/bin/env bash
set -euo pipefail

if [[ "${EUID}" -ne 0 ]]; then
    echo "benchmark-kata.sh must run as root" >&2
    exit 2
fi

ctr_binary="${FERROBOX_CTR:?FERROBOX_CTR is required}"
containerd_binary="${FERROBOX_CONTAINERD:?FERROBOX_CONTAINERD is required}"
output="${FERROBOX_KATA_OUTPUT:?FERROBOX_KATA_OUTPUT is required}"
image="${FERROBOX_KATA_IMAGE:-docker.io/library/python:3.11-slim-bookworm}"
runtime_root="${FERROBOX_KATA_RUNTIME_ROOT:-/mnt/ferrobox/runtime/kata}"
containerd_socket="${runtime_root}/containerd.sock"
containerd_root="${runtime_root}/containerd-root"
containerd_state="${runtime_root}/containerd-state"
containerd_log="${runtime_root}/containerd.log"
namespace="ferrobox-kata-benchmark"
runtime="io.containerd.kata.v2"

mkdir -p "${runtime_root}"
runtime_root="$(realpath "${runtime_root}")"
case "${runtime_root}" in
    /mnt/ferrobox/runtime/kata | /var/lib/ferrobox/runtime/kata) ;;
    *) echo "refusing unexpected Kata runtime root: ${runtime_root}" >&2; exit 2 ;;
esac

ctr() {
    "${ctr_binary}" \
        --address "${containerd_socket}" \
        --namespace "${namespace}" \
        "$@"
}

cleanup() {
    if [[ -n "${persistent_id:-}" ]]; then
        ctr tasks kill --signal SIGKILL "${persistent_id}" 2>/dev/null || true
        ctr tasks delete --force "${persistent_id}" 2>/dev/null || true
        ctr containers delete "${persistent_id}" 2>/dev/null || true
    fi
    if [[ -n "${containerd_pid:-}" ]]; then
        kill "${containerd_pid}" 2>/dev/null || true
        wait "${containerd_pid}" 2>/dev/null || true
    fi
}
trap cleanup EXIT

rm -rf -- "${containerd_root}" "${containerd_state}"
mkdir -p "${containerd_root}" "${containerd_state}"
KATA_CONF_FILE=/opt/kata/share/defaults/kata-containers/configuration-qemu.toml \
PATH="/usr/local/bin:/opt/kata/bin:${PATH}" \
"${containerd_binary}" \
    --address "${containerd_socket}" \
    --root "${containerd_root}" \
    --state "${containerd_state}" \
    >"${containerd_log}" 2>&1 &
containerd_pid=$!

for _ in $(seq 1 200); do
    if ctr version >/dev/null 2>&1; then
        break
    fi
    if ! kill -0 "${containerd_pid}" 2>/dev/null; then
        cat "${containerd_log}" >&2
        exit 1
    fi
    sleep 0.05
done
ctr version >/dev/null
ctr images pull "${image}"

cold_job_us=()
for iteration in $(seq 1 5); do
    container_id="kata-cold-${iteration}"
    started_ns="$(date +%s%N)"
    ctr run --rm --runtime "${runtime}" \
        "${image}" "${container_id}" /bin/true
    finished_ns="$(date +%s%N)"
    cold_job_us+=("$(( (finished_ns - started_ns) / 1000 ))")
done

persistent_id="kata-warm"
ctr run --runtime "${runtime}" --detach \
    "${image}" "${persistent_id}" sleep 300

exec_true_us=()
for iteration in $(seq 1 100); do
    started_ns="$(date +%s%N)"
    ctr tasks exec --exec-id "true-${iteration}" \
        "${persistent_id}" /bin/true
    finished_ns="$(date +%s%N)"
    exec_true_us+=("$(( (finished_ns - started_ns) / 1000 ))")
done

ctr tasks exec --exec-id python-warmup \
    "${persistent_id}" python3 -c 'print(42)' >/dev/null
exec_python_us=()
for iteration in $(seq 1 30); do
    started_ns="$(date +%s%N)"
    ctr tasks exec --exec-id "python-${iteration}" \
        "${persistent_id}" python3 -c 'print(42)' >/dev/null
    finished_ns="$(date +%s%N)"
    exec_python_us+=("$(( (finished_ns - started_ns) / 1000 ))")
done

jq --null-input \
    --arg runtime "kata-qemu" \
    --arg image "${image}" \
    --argjson cold_job_us "$(printf '%s\n' "${cold_job_us[@]}" | jq --slurp .)" \
    --argjson exec_true_us "$(printf '%s\n' "${exec_true_us[@]}" | jq --slurp .)" \
    --argjson exec_python_us "$(printf '%s\n' "${exec_python_us[@]}" | jq --slurp .)" \
    '
    ($cold_job_us | sort) as $cold |
    ($exec_true_us | sort) as $true |
    ($exec_python_us | sort) as $python |
    {
        schema_version: 1,
        runtime: $runtime,
        image: $image,
        cold_job_us: $cold,
        cold_job_p50_us: $cold[2],
        cold_job_p95_us: $cold[4],
        exec_true_us: $true,
        exec_true_p50_us: $true[49],
        exec_true_p95_us: $true[94],
        exec_python_us: $python,
        exec_python_p50_us: $python[14],
        exec_python_p95_us: $python[28]
    }
    ' >"${output}"

jq --exit-status '
    .schema_version == 1 and
    .runtime == "kata-qemu" and
    (.cold_job_us | length == 5) and
    (.exec_true_us | length == 100) and
    (.exec_python_us | length == 30)
' "${output}" >/dev/null

cat "${output}"
