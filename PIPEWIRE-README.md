# Yamaha ZG01 PipeWire Configuration

This directory contains PipeWire configuration files to fix the GNOME Sound Settings display names for the Yamaha ZG01 sound card.

## Problem

The Yamaha ZG01 exposes two audio channels (Game and Voice) as separate USB interfaces. While the kernel driver correctly creates distinctive ALSA card names, GNOME Sound Settings displays both channels as "Yamaha ZG01", making them indistinguishable in the UI.

## Solution

This PipeWire configuration overrides the `device.description` field that GNOME reads, providing distinctive names for each channel.

## Files

- `pipewire-zg01-names.conf` - WirePlumber configuration file
- `install-pipewire-config.sh` - Installation script
- `uninstall-pipewire-config.sh` - Uninstall script
- `PIPEWIRE-README.md` - This documentation

## Installation

### Automatic Installation

```bash
sudo ./scripts/install-pipewire-config.sh
systemctl --user restart wireplumber
```

### Manual Installation

```bash
sudo mkdir -p /etc/wireplumber/wireplumber.conf.d
sudo cp pipewire-zg01-names.conf /etc/wireplumber/wireplumber.conf.d/50-yamaha-zg01-names.conf
systemctl --user restart wireplumber
```

## Result

After installation and restart, GNOME Sound Settings will show:

- **"Yamaha ZG01 Game"** - For gaming/high-bandwidth audio output
- **"Yamaha ZG01 Voice"** - For voice/low-bandwidth audio output

## Uninstallation

```bash
sudo ./scripts/uninstall-pipewire-config.sh
systemctl --user restart wireplumber
```

## Technical Details

The configuration uses WirePlumber rules to match ALSA card devices by their PipeWire device names:

- `alsa_card.usb-Yamaha_Corporation_Yamaha_ZG01-01` → "Yamaha ZG01 Game"
- `alsa_card.usb-Yamaha_Corporation_Yamaha_ZG01-02` → "Yamaha ZG01 Voice"

This approach is clean and doesn't require driver modifications or system-level changes beyond PipeWire configuration.

## Compatibility

- **Required**: PipeWire audio system (default in Ubuntu 22.04+, Fedora 34+, etc.)
- **Supported**: All Linux distributions using PipeWire with WirePlumber
- **GNOME**: All recent versions that use PipeWire for audio

## Troubleshooting

### Configuration Not Applied

1. Verify WirePlumber is running: `systemctl --user status wireplumber`
2. Check configuration file exists: `ls -la /etc/wireplumber/wireplumber.conf.d/50-yamaha-zg01-names.conf`
3. Restart WirePlumber: `systemctl --user restart wireplumber`

### Names Still Not Distinctive

1. Verify ZG01 driver is loaded: `lsmod | grep zg01`
2. Check PipeWire devices: `pw-cli ls | grep -i yamaha`
3. Ensure device names match configuration patterns

### Reverting Changes

Run the uninstall script or manually remove the configuration file and restart WirePlumber.