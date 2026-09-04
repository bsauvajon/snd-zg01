# Yamaha ZG01 Linux kernel driver

An ALSA driver for the Yamaha ZG01 USB audio interface (VID 0x0499, PID
0x1513). The device uses a vendor-specific protocol, so this driver reverse
engineers it from USB captures instead of using the generic `snd-usb-audio`
class driver.

## How the driver maps the device

The ZG01 carries both playback channels on one isochronous endpoint and mixes
them in hardware. The driver exposes one ALSA card with three PCM devices:

| PCM device | Name | Direction | Rates | Packet |
|---|---|---|---|---|
| 0 | Game Out | playback | 48 kHz | 240 B: 6 frames x 40 B |
| 1 | Voice Out | playback | 48 kHz | shared EP 0x01 with Game Out |
| 2 | Voice In | capture | 48 kHz, 16 kHz | 108 B: 8 B header, 6 frames x 16 B, 4 B trailer |

All channels run S32_LE stereo. Game Out and Voice Out are mixed by the
device hardware, so use both sinks at the same time. On Arch, the package
also installs a UCM profile, and PipeWire shows the devices as separate
sinks named Game Out, Voice Out, and Voice In.

A single out chain serves both playback PCMs. When only Voice Out runs, the
chain sends keepalive silence on the shared endpoint. When both run, the URB
callback mixes both PCM streams into each packet. This preserves the two
sinks without moving the mix into userspace.

## Install

### Arch Linux, CachyOS, Omarchy

Build the package, then install it. DKMS hooks build the module for every
installed kernel with matching headers:

```bash
cd packaging/arch
makepkg --cleanbuild
sudo pacman -U snd-zg01-dkms-git-*.pkg.tar.zst
```

The package installs the modules-load.d entry and the UCM profile. Reboot
after install so `snd-zg01` registers before the generic Yamaha match claims
the device. See `packaging/arch/README.md` for verification and rollback.

### Debian, Ubuntu

Download the `.deb` from the latest release and install it:

```bash
sudo dpkg -i snd-zg01-dkms_*.deb
sudo apt-get install -f   # only if dependencies are missing
```

The APT repository at `bsauvajon/snd-zg01` predates this fork. Its packages
still describe the old driver.

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
- Rate problems: playback is 48 kHz only. Voice In also accepts 16 kHz.
  Use `plughw:` for format conversion.

## Documentation

- `docs/PROTOCOL_CAPTURE.md`: capture workflow for knobs, buttons, routing
- `docs/INITIALIZATION_ANALYSIS.md`: device USB topology and packet formats
- `packaging/arch/README.md`: Arch packaging, verification, rollback

## Scope and status

Working: Game Out + Voice Out simultaneous playback, Voice In capture,
suspend/resume, replug, single module, single card. Not implemented: MIDI,
other sample rates.

Experimental out-of-tree driver. Kernel updates can break the build; report
issues with `dmesg` output.

## Contributing

Issues and pull requests are welcome. For protocol work, follow
`docs/PROTOCOL_CAPTURE.md` and keep raw captures out of Git.

## License

GPL-2.0
