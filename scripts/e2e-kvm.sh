#!/usr/bin/env bash
set -euo pipefail

if [[ "${EUID}" -ne 0 ]]; then
    echo "e2e-kvm.sh must run as root" >&2
    exit 2
fi

firecracker="${FERROBOX_FIRECRACKER:?FERROBOX_FIRECRACKER is required}"
jailer="${FERROBOX_JAILER:?FERROBOX_JAILER is required}"
kernel="${FERROBOX_KERNEL:?FERROBOX_KERNEL is required}"
rootfs="${FERROBOX_ROOTFS:?FERROBOX_ROOTFS is required}"
node_binary="${FERROBOX_NODE_BINARY:-target/debug/ferrobox-node}"
runtime_root="${FERROBOX_RUNTIME_ROOT:-/var/lib/ferrobox/runtime}"
chroot_base="${FERROBOX_CHROOT_BASE:-/srv/ferrobox/jailer}"

test -c /dev/kvm
test -r /dev/kvm
test -w /dev/kvm
test -x "${firecracker}"
test -x "${jailer}"
test -f "${kernel}"
test -f "${rootfs}"
test -x "${node_binary}"

before_pids="$(pgrep -x firecracker || true)"
arguments=(
    run-template python
    --firecracker "${firecracker}"
    --jailer "${jailer}"
    --kernel "${kernel}"
    --rootfs "${rootfs}"
    --chroot-base "${chroot_base}"
    --runtime-root "${runtime_root}"
    --jail-uid 1001
    --jail-gid 1001
)
expected="42"
if [[ "${FERROBOX_INTERNET:-0}" == "1" ]]; then
    arguments+=(--internet)
    expected=$'42\ninternet=ok'
fi
output="$(timeout 120s "${node_binary}" "${arguments[@]}")"
[[ "${output}" == "${expected}" ]]

sleep 1
after_pids="$(pgrep -x firecracker || true)"
[[ "${after_pids}" == "${before_pids}" ]]
if ip netns list | grep --quiet '^fb-'; then
    echo "Ferrobox network namespace leaked after E2E" >&2
    exit 3
fi

printf 'Firecracker KVM E2E passed: %s\n' "${output}"
