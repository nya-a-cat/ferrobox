#!/usr/bin/env bash
set -euo pipefail

if [[ "${EUID}" -ne 0 ]]; then
    echo "build-python-rootfs.sh must run as root" >&2
    exit 2
fi

guest_binary="${1:?usage: build-python-rootfs.sh GUEST_BINARY OUTPUT_EXT4}"
output_image="${2:?usage: build-python-rootfs.sh GUEST_BINARY OUTPUT_EXT4}"

guest_binary="$(realpath "${guest_binary}")"
output_image="$(realpath -m "${output_image}")"
[[ -x "${guest_binary}" ]] || {
    echo "Guest binary is missing or not executable: ${guest_binary}" >&2
    exit 3
}

staging="$(mktemp -d)"
cleanup() {
    rm -rf -- "${staging}"
}
trap cleanup EXIT

rootfs="${staging}/rootfs"
mkdir -p -- "${rootfs}"
debootstrap \
    --arch=amd64 \
    --variant=minbase \
    --include=ca-certificates,iproute2,python3,systemd-sysv \
    bookworm \
    "${rootfs}" \
    https://deb.debian.org/debian

chroot "${rootfs}" /usr/sbin/useradd \
    --uid 1000 \
    --user-group \
    --create-home \
    --shell /bin/bash \
    sandbox
install -D -m 0755 "${guest_binary}" \
    "${rootfs}/usr/local/bin/ferrobox-guest"
install -d -o 1000 -g 1000 -m 0750 "${rootfs}/home/sandbox"

cat >"${rootfs}/etc/systemd/system/ferrobox-guest.service" <<'UNIT'
[Unit]
Description=Ferrobox Guest Agent
After=local-fs.target

[Service]
Type=simple
ExecStart=/usr/local/bin/ferrobox-guest
Restart=always
RestartSec=250ms
User=root
Group=root
NoNewPrivileges=false

[Install]
WantedBy=multi-user.target
UNIT

mkdir -p "${rootfs}/etc/systemd/system/multi-user.target.wants"
ln -s ../ferrobox-guest.service \
    "${rootfs}/etc/systemd/system/multi-user.target.wants/ferrobox-guest.service"
ln -sf /dev/null \
    "${rootfs}/etc/systemd/system/serial-getty@ttyS0.service"

cat >"${rootfs}/etc/fstab" <<'FSTAB'
proc /proc proc nosuid,nodev,noexec 0 0
sysfs /sys sysfs nosuid,nodev,noexec 0 0
devtmpfs /dev devtmpfs nosuid 0 0
FSTAB

printf 'ferrobox\n' >"${rootfs}/etc/hostname"
rm -f -- "${output_image}"
truncate -s 1G "${output_image}"
mkfs.ext4 -q -F -d "${rootfs}" "${output_image}"
e2fsck -fn "${output_image}"

guest_sha256="$(sha256sum "${guest_binary}" | awk '{print $1}')"
rootfs_sha256="$(sha256sum "${output_image}" | awk '{print $1}')"
cat >"${output_image}.manifest.json" <<EOF
{
  "distribution": "debian",
  "suite": "bookworm",
  "architecture": "amd64",
  "command_uid": 1000,
  "command_gid": 1000,
  "guest_sha256": "${guest_sha256}",
  "rootfs_sha256": "${rootfs_sha256}"
}
EOF

printf 'Built %s\n' "${output_image}"

