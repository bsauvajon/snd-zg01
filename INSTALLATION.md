# Installation

## Arch Linux, CachyOS, Omarchy

Install build tools, DKMS, and headers for the running kernel:

```bash
sudo pacman -S --needed base-devel dkms linux-headers git
```

Other kernel variants need their matching headers instead. Build and install:

```bash
cd packaging/arch
makepkg --cleanbuild
sudo pacman -U snd-zg01-dkms-git-*.pkg.tar.zst
reboot
```

The reboot lets `snd-zg01` register before the generic Yamaha USB-audio
match claims the device. See `packaging/arch/README.md` for verification,
rollback, and warnings about experimental out-of-tree modules.

## Debian, Ubuntu

Install DKMS and headers, then the package:

```bash
sudo apt install dkms linux-headers-$(uname -r)
sudo dpkg -i snd-zg01-dkms_*.deb
sudo apt-get install -f   # only if dependencies are missing
```

Get the `.deb` from the latest GitHub release. The postinst script builds
the module through DKMS and adds the modules-load.d entry.

## From source

Prerequisites: kernel headers and build tools for the running kernel.

```bash
make
sudo modprobe snd-zg01
```

Unload:

```bash
sudo modprobe -r snd-zg01
```

The module is a single object built from the repository root. Clang-built
kernels need `LLVM=1`; the Makefile detects this from the kernel config.

## Verify

```bash
dkms status snd-zg01
lsmod | grep snd_zg01
cat /proc/asound/cards
```

Expected card state: one card `zg01` with three PCM devices.

```bash
# Playback, Game Out
speaker-test -D hw:zg01,0 -c 2 -r 48000 -F S32_LE -t sine -f 440 -l 1
# Playback, Voice Out
speaker-test -D hw:zg01,1 -c 2 -r 48000 -F S32_LE -t sine -f 440 -l 1
# Capture, Voice In
arecord -D hw:zg01,2 -f S32_LE -r 48000 -c 2 -d 5 test.wav
```

In PipeWire, the UCM profile exposes the devices as Game Out, Voice Out,
and Voice In.

## Automatic loading

`snd-zg01.conf` in modules-load.d pre-loads the module at boot, before
`snd-usb-audio` claims the device through its generic Yamaha alias.

At hotplug time no helper is needed: the module exports the exact
`usb:v0499p1513` modalias, and kmod prefers the exact alias over the
`snd-usb-audio` wildcard, so `modprobe` always resolves to `snd_zg01`.

Manual builds get no mechanism. Add the module to `/etc/modules-load.d/`
yourself if needed.

## Troubleshooting

### DKMS build fails

```bash
dkms status
cat /var/lib/dkms/snd-zg01/1.0.0/build/make.log
```

Check that the headers for the running kernel are installed. Clang-built
kernels log the compiler detection in the Makefile; build once with
`make` by hand to see the full output.

### Device not detected

```bash
lsusb | grep 0499:1513
sudo modprobe snd-zg01
journalctl -b -k --grep zg01
```

### Wrong driver claimed the device

`snd-usb-audio` must not win the bind. Confirm the modules-load.d entry
exists and the module bound to the device:

```bash
cat /etc/modules-load.d/snd-zg01.conf
journalctl -b -k --grep zg01
```

### Permission issues

Add your user to the `audio` group, then log out and back in:

```bash
sudo usermod -aG audio $USER
```

## Uninstall

```bash
# Debian
sudo apt remove snd-zg01-dkms

# Arch
sudo pacman -R snd-zg01-dkms-git
```

The postrm and pacman hooks unload the module and remove the DKMS state and
the modules-load.d entry. Do not delete files under `/usr/src` or
`/var/lib/dkms` by hand.

## Upgrades from old driver versions

Packages older than 2026 installed four modules (`zg01_usb`, `zg01_pcm`,
`zg01_control`, `zg01_usb_discovery`) and three ALSA cards. The current
install unloads those modules. If `dkms status` still shows broken old
state, clean it manually:

```bash
sudo modprobe -r zg01_usb_discovery zg01_control zg01_pcm zg01_usb 2>/dev/null || true
sudo dkms remove snd-zg01/1.0.0 --all 2>/dev/null || true
sudo rm -rf /var/lib/dkms/snd-zg01 /usr/src/snd-zg01-*
sudo find /lib/modules -name 'snd-zg01.ko*' -path '*updates/dkms*' -delete
sudo depmod -a
```
