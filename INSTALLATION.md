# Installation Guide

## Option 1: DKMS Package Installation (Recommended)

### Prerequisites
```bash
sudo apt update
sudo apt install dkms linux-headers-$(uname -r)
```

### Install from .deb Package
```bash
# Download from releases or use the local package
sudo dpkg -i snd-zg01-dkms_1.0.0-1_all.deb

# Fix dependencies if needed
sudo apt-get install -f
```

### Verify Installation
```bash
# Check DKMS status
dkms status snd-zg01

# Should show:
# snd-zg01/1.0.0, 6.17.0-8-generic, x86_64: installed
```

### Load the Modules
```bash
# Load all modules
sudo modprobe zg01_usb zg01_pcm zg01_control zg01_usb_discovery

# Verify modules loaded
lsmod | grep zg01
```

### Verify Audio Devices
```bash
# Check ALSA cards
cat /proc/asound/cards

# Should show:
#  X [zg01game    ]: zg01game - Yamaha ZG01 Game
#                   Yamaha ZG01 Game at usb-XXXX
#  Y [zg01voice   ]: zg01voice - Yamaha ZG01 Voice
#                   Yamaha ZG01 Voice at usb-XXXX
```

### Test Audio
```bash
# Test playback (1-second 440Hz tone)
speaker-test -D hw:zg01game -c 2 -r 48000 -F S32_LE -t sine -f 440 -l 1

# Test capture (5-second recording)
arecord -D hw:zg01voice -f S32_LE -r 48000 -c 2 -d 5 test.wav
```

### Uninstall
```bash
# Remove the package
sudo apt remove snd-zg01-dkms

# DKMS will automatically remove the modules from all kernels
```

---

## Option 2: Manual Build from Source

### Prerequisites
```bash
sudo apt update
sudo apt install build-essential linux-headers-$(uname -r)
```

### Build
```bash
# Clone repository
git clone https://github.com/yourusername/snd-zg01.git
cd snd-zg01

# Clean and build
make clean
make -j$(nproc)
```

### Load Modules
```bash
# Load using the provided script
sudo ./scripts/load_modules.sh

# Or load manually
sudo insmod zg01_usb.ko
sudo insmod zg01_pcm.ko
sudo insmod zg01_control.ko
sudo insmod zg01_usb_discovery.ko
```

### Unload Modules
```bash
# Unload using the provided script
sudo ./scripts/unload_modules.sh

# Or unload manually (in reverse order)
sudo rmmod zg01_usb_discovery
sudo rmmod zg01_control
sudo rmmod zg01_pcm
sudo rmmod zg01_usb
```

---

## Option 3: Build Your Own DKMS Package

### Prerequisites
```bash
sudo apt update
sudo apt install debhelper devscripts dh-dkms dkms linux-headers-$(uname -r)
```

### Build Package
```bash
# From the repository root
./scripts/build-deb.sh

# Package will be created in parent directory
ls -lh ../snd-zg01-dkms_*.deb
```

### Install Your Package
```bash
sudo dpkg -i ../snd-zg01-dkms_1.0.0-1_all.deb
```

---

## Troubleshooting

### DKMS Build Failed
```bash
# Check DKMS logs
sudo dkms status -a
cat /var/lib/dkms/snd-zg01/1.0.0/build/make.log
```

### Modules Won't Load
```bash
# Check kernel logs
sudo dmesg | tail -50

# Check dependencies
modinfo zg01_usb.ko
```

### Device Not Detected
```bash
# Verify USB device is present
lsusb | grep "0499:1513"

# Check module loading order
sudo modprobe zg01_usb
sudo modprobe zg01_pcm
sudo modprobe zg01_control
sudo modprobe zg01_usb_discovery
```

### Permission Issues
```bash
# Add user to audio group (logout/login required)
sudo usermod -a -G audio $USER

# Verify group membership
groups $USER
```

---

## Automatic Module Loading at Boot

### Option 1: Via /etc/modules
```bash
# Add to /etc/modules
echo "zg01_usb" | sudo tee -a /etc/modules
echo "zg01_pcm" | sudo tee -a /etc/modules
echo "zg01_control" | sudo tee -a /etc/modules
echo "zg01_usb_discovery" | sudo tee -a /etc/modules
```

### Option 2: Via systemd service
```bash
# Create service file
sudo tee /etc/systemd/system/zg01-driver.service << 'EOF'
[Unit]
Description=Yamaha ZG01 USB Audio Driver
After=sound.target

[Service]
Type=oneshot
ExecStart=/sbin/modprobe zg01_usb
ExecStart=/sbin/modprobe zg01_pcm
ExecStart=/sbin/modprobe zg01_control
ExecStart=/sbin/modprobe zg01_usb_discovery
RemainAfterExit=yes

[Install]
WantedBy=multi-user.target
EOF

# Enable service
sudo systemctl daemon-reload
sudo systemctl enable zg01-driver.service
sudo systemctl start zg01-driver.service
```

---

## Verifying Installation Success

After installation, you should see:

1. **DKMS Status**: `dkms status` shows module installed
2. **Loaded Modules**: `lsmod | grep zg01` shows 4 modules
3. **ALSA Cards**: `cat /proc/asound/cards` shows zg01game and zg01voice
4. **Kernel Messages**: `dmesg | grep zg01` shows successful initialization
5. **Audio Test**: `speaker-test -D hw:zg01game` plays tone successfully

If all checks pass, your installation is complete! 🎉
