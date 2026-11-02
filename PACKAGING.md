# Packaging

Three packages are built from this tree:

- `snd-zg01-dkms` (Architecture: all): the driver source, built by DKMS on
  the target machine.
- `snd-zg01-tools` (Architecture: amd64 arm64): the `zgctl` control CLI, a
  compiled binary.  It is a separate package because a binary cannot ship
  in an architecture-independent package.
- `snd-zg01-gui` (Architecture: amd64 arm64): the GTK4/libadwaita
  `zg01-control` GUI, its desktop entry, AppStream metadata, icon and French
  translation.  It is separate so installing the CLI or the driver never
  pulls GTK, and installing the GUI never pulls the DKMS build
  dependencies.

## Package contents

The `snd-zg01-dkms` package installs:

1. Source files to `/usr/src/snd-zg01-1.0.0/`:
   - `src/zg01_usb.c`, `src/zg01_pcm.c`, `src/zg01_control.c`,
     `src/zg01_usb_discovery.c`, headers, `Makefile`, `dkms.conf`
2. The UCM profile to `/usr/share/alsa/ucm2/conf.d/zg01_usb/`:
   - exposes Game Out, Voice Out, and Voice In as separate PipeWire devices
3. Documentation to `/usr/share/doc/snd-zg01-dkms/`:
   - changelog and copyright (GPL-2+)
4. Post-install actions (`debian/snd-zg01-dkms.postinst`):
   - unloads any loaded `snd-zg01` or old `zg01_*` modules
   - registers, builds, and installs through DKMS
   - copies `src/snd-zg01.conf` to `/etc/modules-load.d/`
   - removes stale `90-zg01.rules` copies from older package versions

The `snd-zg01-tools` package installs `/usr/bin/zgctl` and recommends
`snd-zg01-dkms`.

The `snd-zg01-gui` package maps the GUI build products with
`debian/snd-zg01-gui.install`.  It installs:

1. `/usr/bin/zg01-control`
2. `/usr/share/applications/io.github.bsauvajon.Zg01Control.desktop`
3. `/usr/share/metainfo/io.github.bsauvajon.Zg01Control.metainfo.xml`
4. `/usr/share/icons/hicolor/scalable/apps/io.github.bsauvajon.Zg01Control.svg`
5. `/usr/share/locale/fr/LC_MESSAGES/zg01-control.mo`

It recommends `snd-zg01-dkms` and `pipewire`.  The driver is only
recommended: without it the GUI opens on its connection view.

The postrm script unloads the module, removes the DKMS state, the stale
udev rule copies from older package versions, and the modules-load.d entry.

## Build the package

```bash
sudo apt install debhelper devscripts dh-dkms dkms cargo rustc \
  pkg-config libasound2-dev libgtk-4-dev libadwaita-1-dev gettext
./scripts/build-deb.sh
```

Or manually:

```bash
make clean
debuild -us -uc -b
ls -lh ../snd-zg01-dkms_*.deb ../snd-zg01-tools_*.deb ../snd-zg01-gui_*.deb
```

The build writes `snd-zg01-dkms_*_all.deb`,
`snd-zg01-tools_*_<arch>.deb`, `snd-zg01-gui_*_<arch>.deb`, a buildinfo file,
and a changes file to the parent directory.  `debian/rules` runs `make tools`
for the CLI, `make -C gui build` plus `make -C gui locale` for the GUI, and
skips the kernel build (DKMS does that at install time).

## Arch GUI package

The GUI has its own PKGBUILD at `packaging/arch/zg01-control-git/PKGBUILD`.
It is a VCS package like the driver and CLI recipes:

```bash
cd packaging/arch/zg01-control-git
makepkg --cleanbuild
sudo pacman -U zg01-control-git-*.pkg.tar.zst
```

`depends=(gtk4 libadwaita alsa-lib)`, `makedepends=(git rust cargo pkgconf
gettext)`, and `optdepends=(pipewire snd-zg01-dkms-git)`.  The recipe runs the
same `make -C gui install DESTDIR="$pkgdir" PREFIX=/usr` rules the CI
packaging job validates.  `ZG01_SOURCE` overrides the source URL, so a local
checkout or a `gui-vX.Y.Z` tag can be built without editing the recipe.

## GUI release train

The GUI ships on independent `gui-vX.Y.Z` tags; driver releases keep using
`vX.Y.Z`.  A GUI tag builds only the GUI artifacts:

1. Bump `version` in `gui/Cargo.toml` and `gui/Cargo.lock`.
2. Refresh the catalogue when a source string changed:
   `make -C gui update-po` (xgettext into `gui/po/zg01-control.pot`, msgmerge
   into `gui/po/fr.po`) then fill any new `msgstr`; `make -C gui check-po`
   must pass.
3. Tag: `git tag -a gui-v0.1.0 -m "ZG01 Control 0.1.0"` and push the tag.
4. `.github/workflows/release-gui.yml` builds `zg01-control-git` and
   `snd-zg01-gui`, creates a draft release and uploads only those artifacts.
5. Run the `docs/GUI_TEST.md` checklist on a real ZG01 before publishing.

## Debian GUI packaging fallback

The source package builds the GUI from the committed `gui/Cargo.lock` with
the distribution's `cargo` and `rustc`.  If the target distribution's Rust is
too old for the pinned crates, use one of these fallbacks; do not relax the
lockfile just for the package:

- **Vendored crates**: run `cargo vendor --manifest-path gui/Cargo.toml
  gui/vendor` on a machine with network access, commit or ship the vendor
  directory as a source component, and add a `.cargo/config.toml` that
  replaces `crates-io` with `vendor/`.  The Debian build then runs offline.
- **Flatpak**: build the same `gui/` tree with the GNOME runtime, the
  `org.freedesktop.Sdk.Extension.rust-stable` extension and
  `--device=alsa`.  The app ID is `io.github.bsauvajon.Zg01Control`, so the
  desktop file and metainfo stay valid.  This is the recommended route for
  distributions whose Rust is too old.

The CI packaging job always exercises the plain `make -C gui install` path;
the fallback only changes where the crates come from.

## Release a new version (driver and tools)

1. Update `PACKAGE_VERSION` in `dkms.conf`.
2. Add a changelog entry: `dch -v 1.0.1-1 "Summary of changes"`.
3. Rebuild with `./scripts/build-deb.sh`.
4. Test in a clean environment before publishing:
   `sudo dpkg -i ../snd-zg01-dkms_1.0.1-1_all.deb && dkms status`.

## Debian directory layout

```
debian/
├── changelog                # package version history
├── control                  # metadata, depends on dkms
├── copyright                # GPL-2+
├── rules                    # debhelper + dkms, builds the CLI
├── snd-zg01-dkms.dkms       # DKMS registration
├── snd-zg01-dkms.install    # maps sources to /usr/src/snd-zg01-1.0.0/
├── snd-zg01-dkms.postinst   # build, install, modules-load setup
├── snd-zg01-dkms.postrm     # unload, deregister, clean state
├── snd-zg01-tools.install   # maps tools/zgctl to /usr/bin/
└── snd-zg01-gui.install     # maps the GUI binary, data, icon and fr.mo
```

`debian/rules` skips the normal build, runs `make tools` for the CLI, and
lets DKMS compile the module when the package installs, against the running
kernel's headers.

## Lintian

Known warnings are cosmetic: `unstable` as distribution instead of an Ubuntu
codename, and changelog date format when entries are not made through `dch`.
Use `dch` for new entries and ignore the rest.

## APT repository and releases

`.github/workflows/release-apt.yml` builds the packages on a `v*` tag,
creates a draft GitHub release, uploads the `.deb` files (and the Arch
packages), and publishes a signed APT repository as release assets.

The APT repository is a flat `Packages` index serving every architecture
in one file; APT picks the entry that matches the host, so a single
repository carries both the `all` DKMS package and the architecture-
dependent `snd-zg01-tools` package.  The Arch `zgctl` package is built
separately (`packaging/arch-zgctl/PKGBUILD`).

Trigger a release:

```bash
git tag -a v1.0.0 -m "Release v1.0.0"
git push origin v1.0.0
```

Required repository secrets:

- `GPG_PRIVATE_KEY`: armored GPG private key that signs `Release`/`InRelease`
- `GPG_PASSPHRASE`: passphrase for that key
- `GPG_KEY_ID`: key ID used to set ownertrust
- `GITHUB_TOKEN`: provided by GitHub Actions

The workflow imports the key and runs `gpg` in batch mode. Keep the private
key material in secrets only. Users point APT at the release assets through
the `apt/snd-zg01.sources` file in this repository.

## CI packaging check

`.github/workflows/gui.yml` runs on `gui/**` and the shared control map or
parser. Its packaging job runs `make -C gui install DESTDIR=$(mktemp -d)`,
checks every installed file, then runs `desktop-file-validate` and
`appstreamcli validate`. The build-and-test job runs `cargo fmt --all --
--check`, `cargo clippy --workspace --all-targets -- -D warnings` and
`cargo test --workspace`.

## See also

- `INSTALLATION.md`: user installation guide
- `README.md`: project overview
- `docs/GUI_TEST.md`: GUI hardware acceptance checklist
- `gui/README.md`: GUI build, install and translation notes
