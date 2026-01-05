#!/bin/bash
# Install udev rule to prevent snd-usb-audio from claiming ZG01

echo "Installing udev rule to blacklist snd-usb-audio for ZG01..."
sudo cp /home/brice/repos/snd-zg01/99-zg01-blacklist.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
echo "Done! The rule will take effect on next device plug."
