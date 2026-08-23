# Arch Linux and Omarchy

The Arch package is currently a VCS package because upstream has not published
a tagged release. It installs the driver source for DKMS; Arch's DKMS pacman
hooks build the module for every installed kernel that has matching headers.

## Prerequisites

Install the build tools, DKMS, and headers for the running kernel. Omarchy's
stock kernel uses `linux-headers`:

```bash
omarchy pkg add base-devel dkms linux-headers git
```

For another kernel, install its matching headers instead. Do not install
`linux-headers` solely because this package says so; DKMS supports multiple
kernel variants.

## Build without installing

```bash
cd packaging/arch
makepkg --cleanbuild
```

Inspect the resulting package before installation:

```bash
namcap PKGBUILD snd-zg01-dkms-git-*.pkg.tar.zst
```

## First installation

This is an experimental out-of-tree kernel module. Test it when a reboot is
available as a recovery path. Close applications using the ZG01, install the
local package, and reboot so `snd-zg01` registers before the generic Yamaha
USB-audio match:

```bash
sudo pacman -U snd-zg01-dkms-git-*.pkg.tar.zst
reboot
```

After an AUR package is published, Omarchy users can install the same package
through its supported package command:

```bash
omarchy pkg aur add snd-zg01-dkms-git
```

## Verify

With the ZG01 connected:

```bash
dkms status snd-zg01
lsmod | grep '^snd_zg01'
cat /proc/asound/cards
wpctl status
journalctl -b -k --grep='zg01\|0499:1513'
```

Expected ALSA devices are `ZG01 Game`, `ZG01 Voice Out`, and `ZG01 Voice In`.
Test playback and capture at the driver's fixed 48 kHz rate before selecting
the device as the desktop default.

## Roll back

If the module is unstable, stop using the device and reboot. Remove the package
through Omarchy (or pacman), then reboot to restore generic-driver behavior:

```bash
omarchy pkg drop snd-zg01-dkms-git
reboot
```

Do not manually delete files under `/usr/src`, `/var/lib/dkms`, or
`/usr/lib/modules`; the package and DKMS hooks own them.
