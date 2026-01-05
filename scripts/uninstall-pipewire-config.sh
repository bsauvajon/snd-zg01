#!/bin/bash
# Uninstall script for Yamaha ZG01 PipeWire configuration

set -e

PIPEWIRE_CONF_DIR="/etc/wireplumber/wireplumber.conf.d"
CONF_FILE="50-yamaha-zg01-names.conf"
FULL_PATH="$PIPEWIRE_CONF_DIR/$CONF_FILE"

echo "Uninstalling Yamaha ZG01 PipeWire configuration..."

# Check if running as root
if [[ $EUID -ne 0 ]]; then
   echo "This script must be run as root (use sudo)" 
   exit 1
fi

# Remove configuration file if it exists
if [[ -f "$FULL_PATH" ]]; then
    echo "Removing configuration file: $FULL_PATH"
    rm "$FULL_PATH"
    echo "Configuration removed successfully!"
else
    echo "Configuration file not found: $FULL_PATH"
    echo "Nothing to uninstall."
fi

echo ""
echo "To apply the changes:"
echo "1. Restart WirePlumber for each user:"
echo "   systemctl --user restart wireplumber" 
echo "2. Or reboot the system"
echo ""
echo "After restart, GNOME Sound Settings will revert to showing 'Yamaha ZG01' for both devices."