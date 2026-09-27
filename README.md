# Yamaha ZG01 Linux kernel driver

An ALSA driver for the Yamaha ZG01 USB audio interface (VID 0x0499, PID
0x1513). The device uses a vendor-specific protocol, so this driver reverse
engineers it from USB captures instead of using the generic `snd-usb-audio`
class driver.

## How the driver maps the device

The ZG01 carries both playback channels on one isochronous endpoint. The
driver mixes them into the shared URB stream. It exposes one ALSA card with
three PCM devices:

| PCM device | Name | Direction | Rates | Packet |
|---|---|---|---|---|
| 0 | Game Out | playback | 48 kHz | 240 B: 6 frames x 40 B |
| 1 | Voice Out | playback | 48 kHz | shared EP 0x01 with Game Out |
| 2 | Voice In | capture | 48 kHz | 108 B nominal: 8 B header, 5-7 frames x 16 B, 4 B trailer |

All channels run S32_LE stereo. Both sinks work at the same time. Both
packages install a UCM profile, so PipeWire shows the devices as separate
sinks named Game Out, Voice Out, and Voice In. Without it, userspace falls
back to the single generic stereo profile and only Game Out is exposed.

A single out chain serves both playback PCMs. When only Voice Out runs, the
chain sends keepalive silence on the shared endpoint. When both run, the URB
callback mixes both PCM streams into each packet. This preserves the two
sinks without moving the mix into userspace.

## Install

### Arch Linux, CachyOS, Omarchy

Prebuilt packages (`snd-zg01-dkms-git-*.pkg.tar.zst`) are attached to each
[GitHub release](https://github.com/bsauvajon/snd-zg01/releases/latest);
install the downloaded file with `sudo pacman -U <file>`.

To build it yourself instead, install the build tools, DKMS, and the headers
for the running kernel:

```bash
sudo pacman -S --needed base-devel dkms linux-headers git alsa-ucm-conf
```

Other kernel variants need their matching headers instead (for example
`linux-lts-headers` or `linux-zen-headers`). Then build and install:

```bash
cd packaging/arch
makepkg --cleanbuild
sudo pacman -U snd-zg01-dkms-git-*.pkg.tar.zst
```

DKMS hooks build the module for every installed kernel with matching
headers; without them DKMS registers nothing, so the package warns at install
time when the running kernel has no headers. The package installs the
modules-load.d entry and the UCM profile. Reboot after install so `snd-zg01`
registers before the generic Yamaha match claims the device. See
`packaging/arch/README.md` for verification and rollback.

### Debian, Ubuntu

Add the signed APT repository, then install:

```bash
sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL https://raw.githubusercontent.com/bsauvajon/snd-zg01/main/apt/snd-zg01.asc \
  | sudo tee /etc/apt/keyrings/snd-zg01.asc >/dev/null
curl -fsSL https://raw.githubusercontent.com/bsauvajon/snd-zg01/main/apt/snd-zg01.sources \
  | sudo tee /etc/apt/sources.list.d/snd-zg01.sources >/dev/null
sudo apt update
sudo apt install snd-zg01-dkms
```

Or download the `.deb` from the latest release and install it manually:

```bash
sudo dpkg -i snd-zg01-dkms_*.deb
sudo apt-get install -f   # only if dependencies are missing
```

### From source

The module builds against kernel headers. Clang-built kernels need the LLVM
front end; the Makefile reads `CONFIG_CC_IS_CLANG` from the target kernel and
sets it. A user-supplied `LLVM=` value wins over the auto-detection:

```bash
make
sudo modprobe snd-zg01
```

## Verify

With the device connected:

```bash
cat /proc/asound/cards      # one card: zg01
lsmod | grep snd_zg01
journalctl -b -k --grep zg01
```

The card offers three PCM devices: `hw:N,0` Game Out, `hw:N,1` Voice Out,
`hw:N,2` Voice In. With the UCM profile installed, PipeWire names them the
same way.

```bash
# Game Out
speaker-test -D hw:zg01,0 -c 2 -r 48000 -F S32_LE -t sine -f 440 -l 1
# Voice In
arecord -D hw:zg01,2 -f S32_LE -r 48000 -c 2 -d 5 test.wav
```

Voice In logs bursts of `-ECONNRESET` on the first open. They are benign;
the chain resubmits.

## Mic controls

The card exposes the mic DSP as ALSA controls: GATE, COMP and LIMITER
switches and levels, the EQ switch, four EQ band gains, frequencies and
Q, and the low/high shelf types.  They are written to the device through
the interface 4 bulk protocol (`docs/MIC_CONTROL_PROTOCOL.md`).

```bash
make -C tools
./tools/zgctl list
./tools/zgctl set 'EQ Band 1 Gain' +6.0
./tools/zgctl set 'EQ Band 2 Frequency' 1k
./tools/zgctl set 'Limiter' 42
./tools/zgctl save          # persist the current settings to the device
./tools/zgctl reset         # reload the persisted settings
```

`zgctl` accepts dB for gains, Hz/kHz for frequencies, ratios for Q and
on/off for switches; the same controls are reachable with `amixer
cget/cset`.  `zgctl save` (the `Save to ZG01` control) persists the
current settings to the device's non-volatile memory; `zgctl reset`
(the `Reset to ZG01` control) reloads the persisted settings.

Writes reach the device live.  Reading values back is not implemented:
the getters return the **last value the driver wrote** (a cache seeded
with the built-in defaults at load).  Settings changed outside the driver
(ZG Controller on Windows, or `reset`) are therefore not reflected until
the control is written again from Linux.  This is the documented
behaviour; see `docs/MIC_CONTROL_PROTOCOL.md`.

## DKMS

Both packages install the source to DKMS. DKMS rebuilds the module on kernel
updates. Remove the package with `pacman -R snd-zg01-dkms-git` or
`apt remove snd-zg01-dkms`; the hooks unload the module and clean `/usr/src`
and `/var/lib/dkms`.

Upgrades from the old split-driver packages (modules `zg01_usb`,
`zg01_pcm`, `zg01_control`, `zg01_usb_discovery`) unload those modules during
install. If a pre-2026 package left broken DKMS state, remove
`/var/lib/dkms/snd-zg01` and `/usr/src/snd-zg01-*`, then run `depmod -a`.

## Troubleshooting

- Device missing: check `lsusb | grep 0499:1513`, then
  `sudo modprobe snd-zg01` and read `journalctl -b -k --grep zg01`.
- Wrong device claimed the card: check `snd-zg01` loads before
  `snd-usb-audio` (modules-load.d entry).
- No audio on one sink: Game Out and Voice Out share one endpoint; check
  the other sink is not open in exclusive mode.
- Rate problems: playback is 48 kHz only. Voice In is 48 kHz only too.
  Use `plughw:` for format conversion.

## Documentation

- `docs/PROTOCOL_CAPTURE.md`: capture workflow for knobs, buttons, routing
- `docs/INITIALIZATION_ANALYSIS.md`: device USB topology and packet formats
- `docs/MIC_CONTROL_PLAN.md`: mic control implementation plan
- `docs/MIC_CONTROL_PROTOCOL.md`: decoded mic parameter protocol
- `packaging/arch/README.md`: Arch packaging, verification, rollback

## Scope and status

Working: Game Out + Voice Out simultaneous playback, Voice In capture,
suspend/resume, replug, single module, single card, and live write of the
mic GATE/COMP/EQ/LIMITER controls (ALSA + `zgctl`). Not implemented: MIDI,
other sample rates, reading device state back.

Experimental out-of-tree driver. Kernel updates can break the build; report
issues with `dmesg` output.

## Contributing

Issues and pull requests are welcome. For protocol work, follow
`docs/PROTOCOL_CAPTURE.md` and keep raw captures out of Git.

## License

GPL-2.0
