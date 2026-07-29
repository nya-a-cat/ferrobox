#!/usr/bin/env bash
set -euo pipefail

if [[ "${EUID}" -ne 0 ]]; then
    echo "benchmark-kvm.sh must run as root" >&2
    exit 2
fi

node_binary="${FERROBOX_NODE_BINARY:-target/debug/ferrobox-node}"
output="${FERROBOX_BENCHMARK_OUTPUT:?FERROBOX_BENCHMARK_OUTPUT is required}"
iterations="${FERROBOX_BENCHMARK_ITERATIONS:-20}"
python_iterations="${FERROBOX_BENCHMARK_PYTHON_ITERATIONS:-30}"
create_iterations="${FERROBOX_BENCHMARK_CREATE_ITERATIONS:-5}"

"${node_binary}" benchmark \
    --create-iterations "${create_iterations}" \
    --exec-iterations "${iterations}" \
    --python-iterations "${python_iterations}" \
    --firecracker "${FERROBOX_FIRECRACKER:?FERROBOX_FIRECRACKER is required}" \
    --jailer "${FERROBOX_JAILER:?FERROBOX_JAILER is required}" \
    --kernel "${FERROBOX_KERNEL:?FERROBOX_KERNEL is required}" \
    --rootfs "${FERROBOX_ROOTFS:?FERROBOX_ROOTFS is required}" \
    >"${output}"

jq --exit-status '
    .schema_version == 7 and
    (.pool_prepare_us | length > 0) and
    .pool_size > 0 and
    .pool_firecracker_rss_kib > 0 and
    (.create_to_ready_us | length > 0) and
    (.delete_us | length > 0) and
    (.exec_true_us | length > 0) and
    (.exec_python_us | length > 0)
' "${output}" >/dev/null

create_us="$(jq -r '.create_to_ready_p95_us' "${output}")"
prepare_us="$(jq -r '.pool_prepare_p95_us' "${output}")"
exec_p95_us="$(jq -r '.exec_true_p95_us' "${output}")"
delete_us="$(jq -r '.delete_p95_us' "${output}")"

# Initial regression ceilings. They are intentionally recorded separately from
# competitor targets and will be tightened only from retained hosted-KVM data.
(( prepare_us <= 1200000 ))
(( create_us <= 80000 ))
(( exec_p95_us <= 50000 ))
(( delete_us <= 2500000 ))

cat "${output}"
