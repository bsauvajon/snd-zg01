#!/bin/bash

echo "Unloading Yamaha ZG01 Driver Modules..."

# Unload in reverse dependency order
sudo rmmod zg01_usb 2>/dev/null
sudo rmmod zg01_usb_discovery 2>/dev/null
sudo rmmod zg01_control 2>/dev/null
sudo rmmod zg01_pcm 2>/dev/null

echo "Modules unloaded."
