# ZG01 Driver Testing Guide

**Last Updated: January 3, 2026**

## 🛠 Setup

### VM Environment (Recommended)
The driver is tested on Ubuntu 25.10 in a VM with USB passthrough.

```bash
# Reboot VM cleanly
virsh destroy ubuntu25.10 && sleep 2 && virsh start ubuntu25.10 && sleep 15

# SSH into VM (from host)
ssh -i /home/brice/repos/snd-zg01/capture/id.key.pub brice@192.168.122.165

# Shared directory is available at: /home/brice/repos/snd-zg01
```

### Build Driver
```bash
cd /home/brice/repos/snd-zg01
make clean && make
# Produces: zg01_usb.ko, zg01_pcm.ko, zg01_control.ko, zg01_usb_discovery.ko
```

### Load Modules
```bash
# Using convenience script (in VM)
cd /home/brice/repos/snd-zg01
sudo ./scripts/load_modules.sh

# Or manually:
sudo insmod zg01_pcm.ko
sudo insmod zg01_control.ko  
sudo insmod zg01_usb_discovery.ko
sudo insmod zg01_usb.ko
```

### Verify Devices
```bash
cat /proc/asound/cards
# Should show:
#  0 [zg01game    ]: zg01_usb - ZG01 Game
#                   Yamaha ZG01 Game Channel
#  1 [ZG01        ]: USB-Audio - Yamaha ZG01
#                   Yamaha Corporation Yamaha ZG01 at usb-0000:02:00.0-4, high speed
#  2 [zg01voice   ]: zg01_usb - ZG01 Voice
#                   Yamaha ZG01 Voice Channel

# Check kernel messages
sudo dmesg | tail -20
# Should see: "Driver loaded successfully!", "Starting Game channel", etc.
```

## 🎧 Audio Testing

### 1. Basic Playback Test (Game Channel)
```bash
# Play a WAV file (in VM)
aplay -D hw:zg01game /path/to/test.wav

# Or use speaker-test for quick verification
speaker-test -D hw:zg01game -c 2 -r 48000 -F S32_LE -t sine -f 440

# Note: Device only supports S32_LE @ 48kHz
# Use plughw:zg01game for automatic conversion if needed
```

**Expected**: Audio output from ZG01 speakers/headphones. May have minor clicking (known issue ~1.25% samples).

### 2. Audio Quality Test
Generate a clean test tone and verify with external recording:

```bash
# On host system:
cd /home/brice/repos/snd-zg01/capture

# Generate 440Hz reference tone
ffmpeg -f lavfi -i 'sine=frequency=440:duration=10' \
  -ar 48000 -ac 2 -acodec pcm_s32le source_440hz.wav -y

# Record ZG01 output with external mic (Yeti on hw:2,0)
arecord -D hw:2,0 -f S24_3LE -r 48000 -c 2 recorded_test.wav &
RECORD_PID=$!
sleep 1

# Play through driver
ssh -i id.key.pub brice@192.168.122.165\
  "aplay -D hw:zg01game /home/brice/repos/snd-zg01/capture/source_440hz.wav"

wait $RECORD_PID

# Analyze quality
ffmpeg -i recorded_test.wav -af "atrim=start=1:end=6,adeclick,astats" -f null - 2>&1 | \
  grep -E "(Detected clicks|RMS level|Peak level)"
```

**Expected Results** (as of Jan 3, 2026):
- Click rate: ~1.25% (5996 clicks per 480000 samples)
- RMS level: -34.6 dB (10dB lower than source)
- Frequency: Correct 440Hz tone
- Status: Usable but not production quality

### 3. Capture Test (Voice Channel)
```bash
# Record from ZG01 microphone (in VM)
arecord -D hw:zg01voice -f S32_LE -r 48000 -c 2 -d 5 /tmp/test_mic.wav

# Playback the recording
aplay -D hw:zg01game /tmp/test_mic.wav
```

**Expected**: Recording of microphone input. File size ~1.9MB for 5 seconds.

### 4. Stress Test
```bash
# Terminal 1: Continuous playback
speaker-test -D hw:zg01game -c 2 -r 48000 -F S32_LE -t sine

# Terminal 2: Continuous capture  
arecord -D hw:zg01voice -f S32_LE -r 48000 -c 2 /tmp/stress_test.wav
```

**Expected**: Both should run without errors for several minutes. Check `dmesg` for any URB errors.

## 🔍 Troubleshooting

### "Robotic" Sound?
- Check kernel logs: `dmesg | tail`
- Ensure no "URB submission failed" errors.
- Verify you are using the latest driver build with 64-packet URBs.

### "Metallic" Sound or 4x Pitch?
- **Issue**: The device may be locked at 192kHz while the driver sends 48kHz.
- **Verification**: Use the Frequency Verification method below to confirm the actual output pitch.
- **Fix**: The driver attempts a "Magic Sequence" in `zg01_set_rate`. Check `dmesg` to ensure "UAC2 Set Rate success" is logged.

## 🔬 Advanced Debugging Methods

### 1. Frequency Verification (External Mic)
To confirm the hardware sample rate mismatch (pitch shift), use an external microphone (e.g., Yeti X) to record the ZG01 output.

```bash
# 1. Start recording with external mic (Bus 2, Device 0 in this example)
arecord -D plughw:2,0 -f S16_LE -r 48000 -c 2 -d 10 /tmp/yeti_test.wav &

# 2. Play 440Hz tone through ZG01
ssh vm-host "aplay -D plughw:0,0 /path/to/test_tone_440hz.wav"
```

Analyze the frequency using Python:
```python
import wave, struct
wf = wave.open('/tmp/yeti_test.wav', 'rb')
frames = wf.readframes(wf.getnframes())
samples = struct.unpack('<' + 'h' * (len(frames)//2), frames)[0::2]
zc = 0
for i in range(1, len(samples)):
    if samples[i-1] <= 0 and samples[i] > 0: zc += 1
print(f"Freq: {zc / (len(samples)/48000):.2f} Hz")
# 440Hz expected. 1760Hz+ indicates 4x pitch shift.
```

### 2. Windows VM Capture (Proprietary Handshaking)
If the device requires a specific initialization sequence, capture the Windows driver behavior using `usbmon` on the host while passing the device to a Windows VM.

```bash
# 1. Identify ZG01 Bus (e.g., Bus 1)
lsusb -d 0499:1513

# 2. Start capture on host
sudo tcpdump -i usbmon1 -w /tmp/zg01_init.pcap &

# 3. Start Windows VM and attach ZG01
virsh start win11
virsh attach-device win11 zg01_attach.xml

# 4. Analyze pcap for UAC2 commands
tshark -r /tmp/zg01_init.pcap -Y "usb.setup.bRequest == 0x01 && usb.setup.wValue == 0x0100"
```
