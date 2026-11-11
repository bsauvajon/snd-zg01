# Arch GUI package

`zg01-control-git` is the standalone GUI package. It is separate from
`snd-zg01-dkms-git` and `zgctl-git`: installing it never pulls DKMS or the
kernel headers, and installing the driver never pulls GTK.

```bash
cd packaging/arch/zg01-control-git
makepkg --cleanbuild
sudo pacman -U zg01-control-git-*.pkg.tar.zst
```

The recipe runs `make -C gui install DESTDIR="$pkgdir" PREFIX=/usr`, so the
binary, desktop entry, AppStream metadata, hicolor icon and `fr.mo` all come
from the same install rules the CI packaging job validates.

The GUI needs `gtk4`, `libadwaita` and `alsa-lib`. `pipewire` is optional and
only feeds the Linux-side meter bars in a later release; v1 shows the card
meters from the driver. `snd-zg01-dkms-git` is optional too: without it the
window opens on the "ZG01 not found" view.
