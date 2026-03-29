# Yamaha ZG01 Linux Kernel Driver

**Status: PRODUCTION READY ✅**

A complete Linux kernel driver for the Yamaha ZG01 USB audio interface (VID: 0x0499, PID: 0x1513), providing **excellent quality** 32-bit audio with three independent channels.

## ✨ Latest Updates

### 🎉 March 29, 2026 - Concurrent Output Mixing + Stability Fixes
**Game + Voice Out simultaneous playback without saccades:**
- **Fixed**: Opening a second app (e.g. Audacity) while audio is playing no longer cuts the stream
- **Fixed**: Game + Voice Out playing together caused saccades (two URB chains on the same USB endpoint)
- **Fixed**: Changing Discord capture device caused a rapid TRIGGER_START/STOP loop crashing the USB device
- **How**: Voice Out now *piggybacks* onto the Game URB stream when both are active — samples are mixed frame-by-frame in the Game callback, so only a single URB chain occupies EP 0x01
- **Result**: Chrome (Game) + Discord (Voice Out + Voice In) all work simultaneously without any glitch

### 🎉 February 7, 2026 - Full Multi-Channel Support!
**All three channels work simultaneously!**
- **Fixed**: Game + Voice Out + Voice In can all operate together without interference
- **Fixed**: Kernel crash when opening control device (card->module not set)
- **Fixed**: Voice Out opening no longer kills Game channel playback
- **Result**: Discord (Voice Out) + Music (Game) + Audacity (Voice In) all work at the same time!

### 🔧 January 18, 2026 - PipeWire START/STOP Loop Fix
**Fixed rapid trigger loop causing audio instability:**
- **Issue**: PipeWire would enter a rapid START/STOP loop when reconfiguring streams, causing audio to fail or become unstable
- **Root Cause**: TRIGGER_STOP kept URBs running (only muting), causing state mismatch when TRIGGER_START tried to skip starting already-active URBs
- **Solution**: 
  - TRIGGER_STOP now properly stops URBs via `zg01_stop_streaming()`
  - Each START/STOP cycle is clean with fresh URB allocation
  - Added trigger loop detection and throttling as safety measure
- **Result**: Stable audio during app start/stop and stream reconfiguration. Multiple applications work correctly without triggering restart loops.

### 🔧 January 17, 2026 - Audio Stream Stability Fix
**Fixed critical issue where audio would stop when starting/stopping applications:**
- **Root Cause**: URBs were not being resubmitted when PCM state wasn't RUNNING, causing USB streaming to stop permanently
- **Solution**: Modified URB callback to always resubmit URBs (sending silence when PCM not running), ensuring continuous USB streaming between TRIGGER_START and TRIGGER_STOP
- **Additional Fixes**:
  - PCM position no longer reset when URBs are already streaming (prevents desync)
  - USB interface setup skipped when streaming is active (prevents disruption)
- **Result**: Multiple applications can play audio simultaneously without interference

### 🎙️ January 11, 2026 - Three Independent Audio Channels!
The driver now provides complete access to all ZG01 audio channels:
- **Game Output**: High-quality playback for gaming/music (96-byte packets)
- **Voice Output**: Secondary playback channel (240-byte packets)
- **Voice Input**: Low-latency microphone capture (108-byte packets)

Each channel appears as a separate ALSA sound card with distinct names in PipeWire/PulseAudio.

### 📦 DKMS Package Production Ready
Professional DKMS packaging with full automation:
- **One-command install**: `sudo dpkg -i snd-zg01-dkms_*.deb`
- **Automatic driver loading**: udev rules handle device detection
- **Kernel updates**: Modules rebuild automatically
- **Proper module structure**: Fixed relocation issues by building from root directory

## 🎉 Features

- **Three Independent Channels**:
  - **Game Output**: Crystal-clear playback for gaming/music
  - **Voice Output**: Secondary playback channel for communication apps
  - **Voice Input**: Low-latency microphone capture
- **Format**: **32-bit Stereo (S32_LE) @ 48kHz** on all channels
- **Architecture**: Asynchronous USB Audio with proper packet handling per channel
- **Integration**: Fully compatible with ALSA, PulseAudio, and PipeWire
- **Naming**: Distinct device names in audio applications via udev rules
- **Quality**: Crystal-clear audio with no clicks or distortion

## 🚀 Quick Start

### Installation via DKMS Package (Recommended)

The easiest way to install the driver is using the pre-built DKMS package:

#### Using apt
```
curl -L https://raw.githubusercontent.com/bsauvajon/snd-zg01/refs/heads/main/apt/snd-zg01.asc | sudo tee /etc/apt/keyrings/snd-zg01.asc
curl -L https://raw.githubusercontent.com/bsauvajon/snd-zg01/refs/heads/main/apt/snd-zg01.sources | sudo tee /etc/apt/sources.list.d/snd-zg01.sources
sudo apt update
sudo apt install snd-zg01-dkms
```

#### Downloading and installing manually

Download the deb archive from the latest release then use these commands

```bash

# Install the package (modules are compiled and installed automatically)
sudo dpkg -i snd-zg01-dkms_1.0.0-1_all.deb

# If there are dependency issues
sudo apt-get install -f

# Verify DKMS built and installed the modules
dkms status snd-zg01

# Plug in your ZG01 device (modules load automatically via udev)
# Or if already connected, unplug and reconnect

# Verify device is detected
cat /proc/asound/cards
# You should see three cards: "zg01game", "zg01voiceout", and "zg01voice"
```

**Benefits of DKMS:**
- ✅ **Fully automated**: Modules compile during installation, load automatically when device is connected
- ✅ **Plug-and-play**: Udev rules automatically load driver when ZG01 is plugged in
- ✅ **Kernel updates**: Automatically rebuilds the driver when kernel is updated
- ✅ **Clean removal**: `sudo apt remove snd-zg01-dkms` removes everything

### Prerequisites
- Ubuntu 22.04+ or compatible Linux system (tested on kernel 6.17.0-8-generic)
- Kernel headers: `sudo apt install linux-headers-$(uname -r)`
- Build tools (only for manual build): `sudo apt install build-essential`

### Manual Build and Install
```bash
# Clone the repository
git clone https://github.com/bsauvajon/snd-zg01.git
cd snd-zg01

# Build the driver
make clean && make -j$(nproc)

# Load the main module (it will load dependencies automatically)
sudo modprobe zg01_usb

# Verify device is detected
cat /proc/asound/cards
# You should see three cards: "zg01game", "zg01voiceout", and "zg01voice"

# Check kernel logs
sudo dmesg | tail -20 | grep zg01
```

### Unload
```bash
sudo modprobe -r zg01_usb
```

### Known Platform Compatibility
- ✅ **Localhost xHCI (Intel)**: Fully functional, perfect audio quality
- ✅ **VM (QEMU/KVM)**: Fully functional, perfect audio quality
- ❓ **Other Controllers**: Likely compatible, please report results!

## 🎧 Usage

### ALSA Device Names
The driver creates three distinct ALSA cards:
1. **Yamaha ZG01 Game**: Primary playback device (hw:zg01game)
2. **Yamaha ZG01 Voice Out**: Secondary playback device (hw:zg01voiceout)
3. **Yamaha ZG01 Voice In**: Capture device (hw:zg01voice)

In PipeWire/PulseAudio, these appear with their full descriptive names thanks to udev rules.

### Testing Audio
**Game Output (Primary Playback):**
```bash
# 1-second 440Hz sine wave test
speaker-test -D hw:zg01game -c 2 -r 48000 -F S32_LE -t sine -f 440 -l 1

# Play audio file
aplay -D plughw:zg01game your_audio.wav
```

**Voice Output (Secondary Playback):**
```bash
# Test voice output channel
speaker-test -D hw:zg01voiceout -c 2 -r 48000 -F S32_LE -t sine -f 440 -l 1

# Play audio file
aplay -D plughw:zg01voiceout your_audio.wav
```

**Voice Input (Microphone Capture):**
```bash
# Record 5 seconds of audio
arecord -D hw:zg01voice -f S32_LE -r 48000 -c 2 -d 5 test.wav

# Or use plughw for automatic format conversion
arecord -D plughw:zg01voice -f S16_LE -r 48000 -c 2 -d 5 test.wav
```

**Multi-Channel Test:**
```bash
# Play simultaneously on both outputs (in separate terminals)
# Terminal 1: Game output
aplay -D hw:zg01game -f S32_LE music.wav

# Terminal 2: Voice output
aplay -D hw:zg01voiceout -f S32_LE voice.wav

# Terminal 3: Record from microphone
arecord -D hw:zg01voice -f S32_LE -r 48000 -c 2 recording.wav
```

## 🔧 Technical Details

- **USB Protocol**: Vendor-specific implementation with interface switching
- **Three Channel Types**:
  - **Game Output**: 96-byte packets (Interface 2,0 → 1,1)
  - **Voice Output**: 240-byte packets (Interface 2,0 → 1,1 → 2,1)
  - **Voice Input**: 108-byte packets (Interface 2,0 → 1,2)
- **Data Format**: 32-bit PCM (S32_LE), stereo @ 48kHz
- **Architecture**: Asynchronous USB Audio with URB-based streaming
- **DKMS Integration**: Automatic build and module loading via udev rules
- **Device Naming**: Unique names per channel via udev ID_MODEL_FROM_DATABASE

## 🐛 Troubleshooting

### Device Not Detected
```bash
# Check if USB device is visible
lsusb | grep 0499:1513

# Check if modules are loaded
lsmod | grep zg01

# Check kernel logs
sudo dmesg | grep -E "(zg01|0499:1513)"

# Manually load driver if needed
sudo modprobe zg01_usb
```

### Only Some Channels Appear
The driver creates three separate sound cards. Check all cards:
```bash
cat /proc/asound/cards
# Should show: zg01game (card 0), zg01voiceout (card 2), zg01voice (card 3)
# Card 1 is typically the built-in USB-Audio driver (MIDI)
```

### Audio Issues
The driver provides crystal-clear audio on compatible hardware. If you experience issues:
- Verify correct device selection in your audio application
- Check sample rate is set to 48kHz
- Ensure format is S32_LE (or use `plughw:` for automatic conversion)
- Check `dmesg` for any USB or driver errors

### Cleaning Up Older Versions

If you installed an older version of the driver (before March 2026) and `dkms status` shows errors or broken modules after uninstalling, you may need to manually clean up the DKMS state. Older packages had incomplete cleanup scripts.

**Symptoms of incomplete removal:**
- `dkms status` shows `snd-zg01` with errors
- `/var/lib/dkms/snd-zg01/` directory still exists after removal
- Module files (`.ko` or `.ko.zst`) remain in `/lib/modules/*/updates/dkms/`

**Complete manual cleanup:**
```bash
# 1. Unload the module if loaded
sudo modprobe -r snd_zg01 2>/dev/null || sudo rmmod snd_zg01 2>/dev/null || true

# 2. Force DKMS removal (ignore errors from broken state)
sudo dkms uninstall snd-zg01/1.0.0 --all 2>/dev/null || true
sudo dkms remove snd-zg01/1.0.0 --all 2>/dev/null || true

# 3. Remove DKMS state and source directories
sudo rm -rf /var/lib/dkms/snd-zg01
sudo rm -rf /usr/src/snd-zg01-*

# 4. Remove compiled module files
sudo find /lib/modules/*/updates/dkms/ -name "snd-zg01.ko*" -delete 2>/dev/null || true
sudo find /lib/modules/*/extra/dkms/ -name "snd-zg01.ko*" -delete 2>/dev/null || true

# 5. Update module dependencies
sudo depmod -a

# 6. Remove udev rules and configs (if package removal didn't do it)
sudo rm -f /etc/udev/rules.d/90-zg01.rules
sudo rm -f /etc/modules-load.d/snd-zg01.conf
sudo udevadm control --reload-rules
sudo udevadm trigger

# 7. Verify everything is clean
dkms status | grep snd-zg01  # Should show nothing
ls -la /var/lib/dkms/ | grep snd  # Should show nothing
ls -la /usr/src/ | grep snd  # Should show nothing
```

After cleanup, you can install the latest version:
```bash
sudo apt update
sudo apt install snd-zg01-dkms
```

## 📚 Documentation

See additional markdown files for detailed information:
- [INSTALLATION.md](INSTALLATION.md): Detailed installation instructions
- [PACKAGING.md](PACKAGING.md): DKMS package building and release process
- [TESTING_GUIDE.md](TESTING_GUIDE.md): Comprehensive testing procedures
- [DEVELOPMENT_STATUS.md](DEVELOPMENT_STATUS.md): Technical details and development notes


## 🤝 Contributing

Contributions are welcome! Please submit pull requests or open issues for any bugs found.

## 📄 License

GPL-2.0

## GitHub Releases & APT repository

A GitHub Actions workflow is included to build Debian packages, create a draft release, upload `.deb` assets, and publish a signed APT repository as release assets. The workflow file is `.github/workflows/release-apt.yml`.

Usage
- Trigger a release by creating a tag that matches `v*`, for example:

```bash
# create a tag and push it
git tag -a v1.0.0 -m "Release v1.0.0"
git push origin v1.0.0
```

Required repository secrets
- `GPG_PRIVATE_KEY`: ASCII-armored GPG private key used to sign `Release`/`InRelease`.
- `GPG_PASSPHRASE`: Passphrase for the private key.
- `GPG_KEY_ID`: The key ID (e.g. `ABCDE123`) used to set ownertrust.
- `GITHUB_TOKEN` (provided by GitHub Actions): used by the `gh` CLI to create and update releases. If you need elevated permissions, provide a personal token in a secret and reference it in the workflow.

Notes and recommendations
- The workflow imports the provided GPG key and uses `gpg` in batch mode to sign non-interactively. Keep `GPG_PRIVATE_KEY` and `GPG_PASSPHRASE` secure.
- If you prefer hosting the APT repo elsewhere (GitHub Pages, S3), the workflow can be adapted to upload the generated APT files to any storage provider.
- The workflow assumes `debian/` packaging files are present and that `dpkg-buildpackage` will generate the `.deb` files at the repository root.

If you want I can add a small `PACKAGING.md` snippet with example `release.conf` for `apt-ftparchive` or modify the workflow to publish to GitHub Pages instead of release assets.