#!/usr/bin/env bash
set -euo pipefail

if [[ "${EUID}" -ne 0 ]]; then
    echo "benchmark-firecracker-direct.sh must run as root" >&2
    exit 2
fi

firecracker="${FERROBOX_FIRECRACKER:?FERROBOX_FIRECRACKER is required}"
kernel="${FERROBOX_KERNEL:?FERROBOX_KERNEL is required}"
rootfs_template="${FERROBOX_ROOTFS:?FERROBOX_ROOTFS is required}"
probe="${FERROBOX_MICROVM_PROBE:?FERROBOX_MICROVM_PROBE is required}"
output="${FERROBOX_FIRECRACKER_DIRECT_OUTPUT:?FERROBOX_FIRECRACKER_DIRECT_OUTPUT is required}"
runtime_root="${FERROBOX_FIRECRACKER_DIRECT_RUNTIME_ROOT:-/mnt/ferrobox/runtime/firecracker-direct}"

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

put_json() {
    local socket="$1"
    local path="$2"
    local body="$3"
    curl --fail --silent --show-error \
        --unix-socket "${socket}" \
        --request PUT \
        --header 'content-type: application/json' \
        --data-binary "${body}" \
        "http://localhost${path}" >/dev/null
}

results=()
for iteration in $(seq 1 5); do
    rootfs="${workdir}/rootfs-${iteration}.ext4"
    api="${workdir}/firecracker-${iteration}.sock"
    vsock="${workdir}/vsock-${iteration}.sock"
    log="${workdir}/firecracker-${iteration}.log"
    result="${workdir}/probe-${iteration}.json"
    cp --reflink=always "${rootfs_template}" "${rootfs}"
    launched_unix_nanos="$(date +%s%N)"
    "${firecracker}" --api-sock "${api}" >"${log}" 2>&1 &
    child_pid=$!
    for _ in $(seq 1 200); do
        if [[ -S "${api}" ]]; then
            break
        fi
        if ! kill -0 "${child_pid}" 2>/dev/null; then
            cat "${log}" >&2
            exit 1
        fi
        sleep 0.005
    done
    [[ -S "${api}" ]]

    put_json "${api}" /machine-config \
        '{"vcpu_count":1,"mem_size_mib":512,"smt":false,"track_dirty_pages":false}'
    put_json "${api}" /boot-source \
        "$(jq --null-input \
            --arg kernel "${kernel}" \
            '{
                kernel_image_path: $kernel,
                boot_args: "console=ttyS0 reboot=k panic=1 pci=off root=/dev/vda rw"
            }')"
    put_json "${api}" /drives/rootfs \
        "$(jq --null-input \
            --arg rootfs "${rootfs}" \
            '{
                drive_id: "rootfs",
                path_on_host: $rootfs,
                is_root_device: true,
                is_read_only: false
            }')"
    put_json "${api}" /vsock \
        "$(jq --null-input \
            --arg vsock "${vsock}" \
            '{guest_cid: 3, uds_path: $vsock}')"
    put_json "${api}" /actions '{"action_type":"InstanceStart"}'

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
        runtime: "firecracker-direct",
        ready_us: $ready,
        ready_p50_us: $ready[2],
        ready_p95_us: $ready[4]
    }
' "${results[@]}" >"${output}"

jq --exit-status '
    .schema_version == 2 and
    .runtime == "firecracker-direct" and
    (.ready_us | length == 5) and
    (.exec_true_us | length == 100) and
    (.exec_true_cloned_client_us | length == 100) and
    (.exec_python_us | length == 30)
' "${output}" >/dev/null

cat "${output}"
