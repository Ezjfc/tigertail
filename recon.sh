#!/usr/bin/env bash
# Device recon for tigertail: checks whether the tablet's kernel can host a
# configfs HID gadget and how the stock USB gadget is assembled.
#
# Usage: [SSH_KEY=~/.ssh/id_ed25519.remarkable] ./recon.sh [root@10.11.99.1]
# Run from the flake devShell (provides ssh). Findings go to NOTES.md by hand.
set -u

HOST="${1:-root@10.11.99.1}"
SSH_KEY="${SSH_KEY:-$HOME/.ssh/id_ed25519.remarkable}"

run() {
    echo "=== $* ==="
    # shellcheck disable=SC2029
    ssh -i "$SSH_KEY" -o BatchMode=yes "$HOST" "$*" 2>&1
    echo
}

echo "# tigertail recon against $HOST — $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo

run "cat /proc/device-tree/model; echo"
run "uname -a"
run "cat /etc/os-release | head -5"

# Kernel gadget support. /proc/config.gz needs CONFIG_IKCONFIG; fall back to
# module listing if absent.
run "zcat /proc/config.gz 2>/dev/null | grep -i -E 'CONFIG_USB_CONFIGFS|CONFIG_USB_GADGET|F_HID|G_ETHER|LIBCOMPOSITE' || echo 'no /proc/config.gz'"
run "ls /lib/modules/\$(uname -r)/kernel/drivers/usb/gadget/ 2>/dev/null || echo 'no gadget module dir'"
run "find /lib/modules -name '*hid*' -o -name 'libcomposite*' -o -name 'u_ether*' -o -name 'usb_f_*' 2>/dev/null | head -30"
run "lsmod"

# How the stock ethernet gadget is set up: legacy g_ether vs configfs.
run "ls -R /sys/kernel/config/usb_gadget/ 2>/dev/null || echo 'no configfs usb_gadget dir'"
run "mount | grep -E 'configfs|functionfs'"
run "cat /sys/class/udc/* 2>/dev/null; ls /sys/class/udc/"
run "systemctl status busybox-ifplugd 2>/dev/null | head -3; systemctl list-units | grep -i -E 'usb|gadget' | head -10"

# Input devices: confirm the pen event node.
run "cat /proc/bus/input/devices"
