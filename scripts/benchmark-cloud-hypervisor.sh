#!/usr/bin/env bash
set -euo pipefail

if [[ "${EUID}" -ne 0 ]]; then
    echo "benchmark-cloud-hypervisor.sh must run as root" >&2
    exit 2
fi

cloud_hypervisor="${FERROBOX_CLOUD_HYPERVISOR:?FERROBOX_CLOUD_HYPERVISOR is required}"
kernel="${FERROBOX_KERNEL:?FERROBOX_KERNEL is required}"
rootfs_template="${FERROBOX_ROOTFS:?FERROBOX_ROOTFS is required}"
probe="${FERROBOX_MICROVM_PROBE:?FERROBOX_MICROVM_PROBE is required}"
output="${FERROBOX_CLOUD_HYPERVISOR_OUTPUT:?FERROBOX_CLOUD_HYPERVISOR_OUTPUT is required}"
runtime_root="${FERROBOX_CLOUD_HYPERVISOR_RUNTIME_ROOT:-/mnt/ferrobox/runtime/cloud-hypervisor}"

mkdir -p "${runtime_root}"
runtime_root="$(realpath "${runtime_root}")"
workdir="$(mktemp -d "${runtime_root}/benchmark.XXXXXX")"
cleanup() {
    if [[ -n "${child_pid:-}" ]]; then
        kill "${child_pid}" 2>/dev/null || true
        wait "${child_pid}" 2>/dev/null || true
    fi
    case "$(realpath -m "${workdir}")" in
        "${runtime_root}"/benchmark.*) rm -rf -- "${workdir}" ;;
        *) echo "refusing to remove unexpected workdir: ${workdir}" >&2 ;;
    esac
}
trap cleanup EXIT

results=()
for iteration in $(seq 1 5); do
    rootfs="${workdir}/rootfs-${iteration}.ext4"
    vsock="${workdir}/vsock-${iteration}.sock"
    log="${workdir}/cloud-hypervisor-${iteration}.log"
    result="${workdir}/probe-${iteration}.json"
    cp --reflink=always "${rootfs_template}" "${rootfs}"
    launched_unix_nanos="$(date +%s%N)"
    "${cloud_hypervisor}" \
        --kernel "${kernel}" \
        --disk "path=${rootfs}" \
        --cmdline "console=hvc0 reboot=k panic=1 root=/dev/vda rw" \
        --cpus boot=1 \
        --memory size=512M \
        --vsock "cid=3,socket=${vsock}" \
        --console off \
        --serial off \
        >"${log}" 2>&1 &
    child_pid=$!
    probe_arguments=(
        --vsock "${vsock}"
        --launched-unix-nanos "${launched_unix_nanos}"
    )
    if [[ "${iteration}" -lt 5 ]]; then
        probe_arguments+=(--health-only)
    fi
    if ! "${probe}" "${probe_arguments[@]}" >"${result}"; then
        cat "${log}" >&2
        exit 1
    fi
    kill "${child_pid}" 2>/dev/null || true
    wait "${child_pid}" 2>/dev/null || true
    child_pid=
    results+=("${result}")
done

jq --slurp '
    (map(.ready_us) | sort) as $ready |
    .[-1] + {
        schema_version: 2,
        runtime: "cloud-hypervisor",
        ready_us: $ready,
        ready_p50_us: $ready[2],
        ready_p95_us: $ready[4]
    }
' "${results[@]}" >"${output}"

jq --exit-status '
    .schema_version == 2 and
    .runtime == "cloud-hypervisor" and
    (.ready_us | length == 5) and
    (.exec_true_us | length == 100) and
    (.exec_true_cloned_client_us | length == 100) and
    (.exec_python_us | length == 30)
' "${output}" >/dev/null

cat "${output}"
