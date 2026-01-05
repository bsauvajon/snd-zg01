#!/bin/bash
# Installation script for Yamaha ZG01 PipeWire configuration
# This script installs the PipeWire configuration to fix GNOME display names

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PIPEWIRE_CONF_DIR="/etc/wireplumber/wireplumber.conf.d"
CONF_FILE="50-yamaha-zg01-names.conf"
SOURCE_FILE="$SCRIPT_DIR/../pipewire-zg01-names.conf"

echo "Installing Yamaha ZG01 PipeWire configuration..."

# Check if running as root
if [[ $EUID -ne 0 ]]; then
   echo "This script must be run as root (use sudo)" 
   exit 1
fi

# Create PipeWire config directory if it doesn't exist
if [[ ! -d "$PIPEWIRE_CONF_DIR" ]]; then
    echo "Creating PipeWire configuration directory: $PIPEWIRE_CONF_DIR"
    mkdir -p "$PIPEWIRE_CONF_DIR"
fi

# Copy configuration file
echo "Installing configuration file: $PIPEWIRE_CONF_DIR/$CONF_FILE"
cp "$SOURCE_FILE" "$PIPEWIRE_CONF_DIR/$CONF_FILE"
chmod 644 "$PIPEWIRE_CONF_DIR/$CONF_FILE"

echo "Configuration installed successfully!"
echo ""
echo "To apply the changes:"
echo "1. Restart WirePlumber for each user:"
echo "   systemctl --user restart wireplumber"
echo "2. Or reboot the system"
echo ""
echo "After restart, GNOME Sound Settings will show:"
echo "- 'Yamaha ZG01 Game' for gaming/high-bandwidth audio"
echo "- 'Yamaha ZG01 Voice' for voice/low-bandwidth audio"