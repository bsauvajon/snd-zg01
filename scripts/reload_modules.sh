#!/usr/bin/env bash
set -euo pipefail

# Reload helper for snd-zg01 kernel modules
# Stops PipeWire user services, unbinds ZG01 USB interfaces, rebuilds and reloads modules.

ROOT_CMD="sudo"

echo "Stopping PipeWire user services..."
systemctl --user stop pipewire.service pipewire-pulse.service || true
pkill -u "$USER" wireplumber || true

echo "Waiting a moment for sockets to close..."
sleep 0.5

USB_DRV_PATH="/sys/bus/usb/drivers/zg01_usb"

if [ -d "$USB_DRV_PATH" ]; then
  echo "Unbinding ZG01 USB interfaces (requires root)..."
  for entry in "$USB_DRV_PATH"/*; do
    name=$(basename "$entry")
    # only unbind interface entries (of form '1-5.3.3:1.2' etc.)
    if [[ "$name" =~ : ]]; then
      echo "Unbinding $name"
      echo -n "$name" | ${ROOT_CMD} tee "$USB_DRV_PATH/unbind" >/dev/null || true
    fi
  done
else
  echo "USB driver path $USB_DRV_PATH not present; continuing"
fi

echo "Removing modules in dependency order (if loaded)..."
${ROOT_CMD} rmmod zg01_usb || true
${ROOT_CMD} rmmod zg01_pcm || true
${ROOT_CMD} rmmod zg01_control || true
${ROOT_CMD} rmmod zg01_usb_discovery || true


echo "Building kernel modules (make)..."
make -j$(nproc)

echo "Inserting modules (insmod) from current directory..."
${ROOT_CMD} insmod ./src/zg01_pcm.ko || true
${ROOT_CMD} insmod ./src/zg01_control.ko || true
${ROOT_CMD} insmod ./src/zg01_usb_discovery.ko || true
${ROOT_CMD} insmod ./src/zg01_usb.ko || true

echo "Loaded modules:" 
lsmod | grep zg01 || true

echo "Starting PipeWire user services..."
systemctl --user start pipewire.service pipewire-pulse.service || true

echo "Done. Check dmesg and /proc/asound/cards for device status."
