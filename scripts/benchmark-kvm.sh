#!/usr/bin/env bash
set -euo pipefail

if [[ "${EUID}" -ne 0 ]]; then
    echo "benchmark-kvm.sh must run as root" >&2
    exit 2
fi

node_binary="${FERROBOX_NODE_BINARY:-target/debug/ferrobox-node}"
output="${FERROBOX_BENCHMARK_OUTPUT:?FERROBOX_BENCHMARK_OUTPUT is required}"
iterations="${FERROBOX_BENCHMARK_ITERATIONS:-20}"

"${node_binary}" benchmark \
    --exec-iterations "${iterations}" \
    --firecracker "${FERROBOX_FIRECRACKER:?FERROBOX_FIRECRACKER is required}" \
    --jailer "${FERROBOX_JAILER:?FERROBOX_JAILER is required}" \
    --kernel "${FERROBOX_KERNEL:?FERROBOX_KERNEL is required}" \
    --rootfs "${FERROBOX_ROOTFS:?FERROBOX_ROOTFS is required}" \
    >"${output}"

jq --exit-status '
    .schema_version == 1 and
    .create_to_ready_us > 0 and
    (.exec_true_us | length > 0)
' "${output}" >/dev/null

create_us="$(jq -r '.create_to_ready_us' "${output}")"
exec_p95_us="$(jq -r '.exec_true_p95_us' "${output}")"
delete_us="$(jq -r '.delete_us' "${output}")"

# Initial regression ceilings. They are intentionally recorded separately from
# competitor targets and will be tightened only from retained hosted-KVM data.
(( create_us <= 2750000 ))
(( exec_p95_us <= 50000 ))
(( delete_us <= 2500000 ))

cat "${output}"
