#!/usr/bin/env bash
# Capture ZG01 USB traffic from a libvirt/QEMU guest through the host usbmon.
#
# Usage: scripts/capture-usbmon.sh <label> [seconds]
#
# The ZG01 must be passed through to the Windows guest with QEMU usb-host so
# that it stays on the host USB bus; usbmon then records the guest's transfers.
# Without a duration the capture runs until Ctrl-C.

set -euo pipefail

label=${1:?usage: capture-usbmon.sh <label> [seconds]}
duration=${2:-}

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
outdir=${ZG01_CAPTURE_DIR:-$repo_root/captures}
mkdir -p "$outdir"

if ! lsmod | grep -q '^usbmon'; then
    sudo modprobe usbmon
fi

line=$(lsusb | grep '0499:1513' || true)
if [ -z "$line" ]; then
    echo "ZG01 (0499:1513) not found on the host" >&2
    exit 1
fi
bus=$(printf '%s\n' "$line" | sed -E 's/^Bus ([0-9]+) Device ([0-9]+).*/\1/')
dev=$(printf '%s\n' "$line" | sed -E 's/^Bus ([0-9]+) Device ([0-9]+).*/\2/')
bus=$((10#$bus))
dev=$((10#$dev))
iface="usbmon$bus"

file="$outdir/$label.pcap"
echo "device $bus.$dev on $iface -> $file"
if [ -n "$duration" ]; then
    # timeout exits 124 when it stops tcpdump; that is a normal end.
    sudo timeout "$duration" tcpdump -i "$iface" -w "$file" || [ $? -eq 124 ]
else
    echo "press Ctrl-C to stop"
    sudo tcpdump -i "$iface" -w "$file"
fi
sudo chown "$(id -u):$(id -g)" "$file"
printf '%s\n' "$bus.$dev" > "$outdir/$label.device"
echo "captured $file ($bus.$dev)"
echo "normalize: python3 tools/normalize-usbmon.py $file --device $bus.$dev"
