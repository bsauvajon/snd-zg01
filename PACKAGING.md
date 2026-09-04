# Debian packaging

The Debian package `snd-zg01-dkms` installs the driver source to DKMS. DKMS
builds the module for the current kernel at install time and rebuilds it on
kernel updates.

## Package contents

The package installs:

1. Source files to `/usr/src/snd-zg01-1.0.0/`:
   - `src/zg01_usb.c`, `src/zg01_pcm.c`, `src/zg01_control.c`,
     `src/zg01_usb_discovery.c`, headers, `Makefile`, `dkms.conf`
2. Documentation to `/usr/share/doc/snd-zg01-dkms/`:
   - changelog and copyright (GPL-2+)
3. Post-install actions (`debian/snd-zg01-dkms.postinst`):
   - unloads any loaded `snd-zg01` or old `zg01_*` modules
   - registers, builds, and installs through DKMS
   - copies `src/snd-zg01.conf` to `/etc/modules-load.d/`
   - removes stale `90-zg01.rules` copies from older package versions

The postrm script unloads the module, removes the DKMS state, the copied
udev rules, and the modules-load.d entry.

## Build the package

```bash
sudo apt install debhelper devscripts dh-dkms dkms
./scripts/build-deb.sh
```

Or manually:

```bash
make clean
debuild -us -uc -b
ls -lh ../snd-zg01-dkms_*.deb
```

The build writes `snd-zg01-dkms_*_all.deb`, a buildinfo file, and a changes
file to the parent directory.

## Release a new version

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
├── rules                    # debhelper + dkms, skips the build step
├── snd-zg01-dkms.dkms       # DKMS registration
├── snd-zg01-dkms.install    # maps sources to /usr/src/snd-zg01-1.0.0/
├── snd-zg01-dkms.postinst   # build, install, udev + modules-load setup
└── snd-zg01-dkms.postrm     # unload, deregister, clean state
```

`debian/rules` skips the normal build. DKMS compiles the module when the
package installs, against the running kernel's headers.

## Lintian

Known warnings are cosmetic: `unstable` as distribution instead of an Ubuntu
codename, and changelog date format when entries are not made through `dch`.
Use `dch` for new entries and ignore the rest.

## APT repository and releases

`.github/workflows/release-apt.yml` builds the package on a `v*` tag,
creates a draft GitHub release, uploads the `.deb`, and publishes a signed
APT repository as release assets.

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

## See also

- `INSTALLATION.md`: user installation guide
- `README.md`: project overview
