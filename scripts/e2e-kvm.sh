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
output_file="$(mktemp)"
network_diagnostics="${FERROBOX_NETWORK_DIAGNOSTICS:-}"
cleanup() {
    rm -f -- "${output_file}"
}
trap cleanup EXIT

collect_network_diagnostics() {
    if [[ -z "${network_diagnostics}" ]]; then
        return
    fi
    {
        echo "captured_at=$(date --utc --iso-8601=seconds)"
        echo "ip_forward=$(cat /proc/sys/net/ipv4/ip_forward)"
        echo "host_resolv_conf"
        cat /etc/resolv.conf
        if [[ -f /run/systemd/resolve/resolv.conf ]]; then
            echo "systemd_resolv_conf"
            cat /run/systemd/resolve/resolv.conf
        fi
        echo "host_routes"
        ip -4 route show
        echo "host_addresses"
        ip -brief -4 address show
        echo "ferrobox_nft_tables"
        nft list tables | grep 'ferrobox_' || true
        while read -r family table; do
            if [[ "${table}" == ferrobox_* ]]; then
                nft list table "${family}" "${table}" || true
            fi
        done < <(nft list tables | awk '{print $2, $3}')
        echo "host_forward_policy"
        iptables --wait 5 -S FORWARD || true
        echo "network_namespaces"
        ip netns list
        while read -r namespace _; do
            if [[ "${namespace}" == fb-* ]]; then
                echo "namespace=${namespace}"
                ip netns exec "${namespace}" ip -brief -4 address show || true
                ip netns exec "${namespace}" ip -4 route show || true
                ip netns exec "${namespace}" ip -details link show || true
            fi
        done < <(ip netns list)
    } >"${network_diagnostics}" 2>&1
}
set +e
timeout --kill-after=10s 120s \
    "${node_binary}" "${arguments[@]}" >"${output_file}"
status="$?"
set -e
if [[ "${status}" -ne 0 ]]; then
    collect_network_diagnostics
    echo "Firecracker KVM E2E command failed with status ${status}" >&2
    cat "${output_file}" >&2
    exit "${status}"
fi
output="$(cat "${output_file}")"
[[ "${output}" == "${expected}" ]]

sleep 1
after_pids="$(pgrep -x firecracker || true)"
[[ "${after_pids}" == "${before_pids}" ]]
if ip netns list | grep --quiet '^fb-'; then
    echo "Ferrobox network namespace leaked after E2E" >&2
    exit 3
fi
if iptables --wait 5 -S FORWARD | grep --quiet -- '--comment ferrobox:'; then
    echo "Ferrobox forwarding rule leaked after E2E" >&2
    exit 4
fi

printf 'Firecracker KVM E2E passed: %s\n' "${output}"
