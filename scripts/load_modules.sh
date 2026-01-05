#!/bin/bash

# Unload first to ensure clean state
echo "Unloading existing modules..."
sudo rmmod zg01_usb zg01_usb_discovery zg01_control zg01_pcm 2>/dev/null

echo "Loading Yamaha ZG01 Driver Modules..."

# Load in dependency order
if sudo insmod src/zg01_pcm.ko; then
    echo "  [OK] zg01_pcm.ko"
else
    echo "  [FAIL] zg01_pcm.ko"
    exit 1
fi

if sudo insmod src/zg01_control.ko; then
    echo "  [OK] zg01_control.ko"
else
    echo "  [FAIL] zg01_control.ko"
    exit 1
fi

if sudo insmod src/zg01_usb_discovery.ko; then
    echo "  [OK] zg01_usb_discovery.ko"
else
    echo "  [FAIL] zg01_usb_discovery.ko"
    exit 1
fi

if sudo insmod src/zg01_usb.ko; then
    echo "  [OK] zg01_usb.ko"
else
    echo "  [FAIL] zg01_usb.ko"
    exit 1
fi

echo "Driver loaded successfully!"
echo "Check /proc/asound/cards for devices."