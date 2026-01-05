# DKMS Package Build Guide

## Package Information

- **Package Name**: snd-zg01-dkms
- **Version**: 1.0.0-1
- **Architecture**: all (source package, built for each kernel)
- **Package Type**: DKMS (Dynamic Kernel Module Support)
- **Size**: ~19 KB (source files only)

## What is DKMS?

DKMS (Dynamic Kernel Module Support) automatically rebuilds kernel modules when you install a new kernel, ensuring the driver continues to work after kernel updates without manual intervention.

## Package Contents

The package installs:

1. **Source Files** → `/usr/src/snd-zg01-1.0.0/`:
   - `zg01_usb.c` - Main USB driver
   - `zg01_pcm.c` - PCM audio handling (playback & capture)
   - `zg01_control.c` - ALSA control interface
   - `zg01_usb_discovery.c` - Device discovery
   - `zg01.h` - Header file
   - `zg01_pcm.h` - PCM header
   - `zg01_control.h` - Control header
   - `Makefile` - Build configuration
   - `dkms.conf` - DKMS configuration

2. **Documentation** → `/usr/share/doc/snd-zg01-dkms/`:
   - `changelog.Debian.gz` - Package changelog
   - `copyright` - License information (GPL-2+)

3. **Post-Install Actions**:
   - Registers module with DKMS
   - Builds modules for current kernel
   - Displays usage instructions

## Building the Package

### Prerequisites
```bash
sudo apt install debhelper devscripts dh-dkms dkms
```

### Build Command
```bash
./scripts/build-deb.sh
```

Or manually:
```bash
# Clean previous builds
make clean

# Build package
debuild -us -uc -b

# Package created in parent directory
ls -lh ../snd-zg01-dkms_*.deb
```

### Build Output Files

After building, you'll find in the parent directory:
- `snd-zg01-dkms_1.0.0-1_all.deb` - The installable package
- `snd-zg01-dkms_1.0.0-1_amd64.buildinfo` - Build information
- `snd-zg01-dkms_1.0.0-1_amd64.changes` - Changes file

## Installation Flow

When you install the package:

1. **dpkg** extracts files to `/usr/src/snd-zg01-1.0.0/`
2. **dh_dkms** registers the module with DKMS
3. **DKMS** automatically:
   - Detects current kernel version
   - Runs `make` to build the modules
   - Installs modules to `/lib/modules/$(uname -r)/updates/dkms/`
   - Updates module dependencies
4. **postinst** script displays usage instructions

## DKMS Automatic Rebuild

DKMS hooks into kernel updates:

1. When new kernel is installed → DKMS builds modules for new kernel
2. When kernel is removed → DKMS cleans up modules for that kernel
3. Status tracked in `/var/lib/dkms/snd-zg01/`

Check status:
```bash
dkms status snd-zg01
```

Example output:
```
snd-zg01/1.0.0, 6.17.0-8-generic, x86_64: installed
```

## Package Structure

```
debian/
├── changelog              # Package version history
├── control               # Package metadata and dependencies
├── copyright            # License information
├── rules                # Build rules (debhelper + dkms)
├── source/
│   └── format           # Source format (3.0 native)
├── snd-zg01-dkms.install  # File installation list
├── snd-zg01-dkms.postinst # Post-installation script
└── snd-zg01-dkms.postrm   # Post-removal script
```

### Key Files Explained

**debian/control**:
- Defines package name, dependencies (dkms >= 2.1.0.0)
- Architecture: all (source package, not binary)
- Description and features

**debian/changelog**:
- Version: 1.0.0-1 (upstream-debian revision)
- Distribution: unstable
- Lists all achievements (buffer fixes, URB optimization, full-duplex)

**debian/rules**:
- Uses dh (debhelper) with dkms support
- Skips standard build steps (DKMS handles compilation)

**debian/snd-zg01-dkms.install**:
- Maps source files to `/usr/src/snd-zg01-1.0.0/`

**debian/snd-zg01-dkms.postinst**:
- Displays usage instructions after install
- Can add udev rules if needed

**dkms.conf** (root directory):
- Package name and version
- List of 4 modules to build
- Build command: `make`
- Clean command: `make clean`
- Auto-install: yes

## Updating the Package

To release a new version:

1. **Update version in files**:
   ```bash
   # Update dkms.conf
   PACKAGE_VERSION="1.0.1"
   
   # Update debian/changelog
   dch -v 1.0.1-1 "New release with fixes"
   ```

2. **Rebuild package**:
   ```bash
   ./scripts/build-deb.sh
   ```

3. **Test installation**:
   ```bash
   sudo dpkg -i ../snd-zg01-dkms_1.0.1-1_all.deb
   dkms status
   ```

## Distribution

### GitHub Releases
```bash
# Create release
git tag -a v1.0.0 -m "Release 1.0.0"
git push origin v1.0.0

# Upload .deb file to GitHub releases
# Users can download and install with:
wget https://github.com/user/snd-zg01/releases/download/v1.0.0/snd-zg01-dkms_1.0.0-1_all.deb
sudo dpkg -i snd-zg01-dkms_1.0.0-1_all.deb
```

### PPA (Ubuntu Personal Package Archive)
```bash
# Sign package
debuild -S -sa

# Upload to PPA
dput ppa:yourname/snd-zg01 ../snd-zg01-dkms_1.0.0-1_source.changes

# Users install with:
sudo add-apt-repository ppa:yourname/snd-zg01
sudo apt update
sudo apt install snd-zg01-dkms
```

## Lintian Warnings

Current warnings (cosmetic, safe to ignore):

- `bad-distribution-in-changes-file unstable`: Using Debian's "unstable" instead of Ubuntu codename
  - **Fix**: Change `debian/changelog` to use Ubuntu codename (e.g., "jammy")
  
- `debian-changelog-has-wrong-day-of-week`: Date/day mismatch in changelog
  - **Fix**: Use `dch` command which auto-generates correct dates

- `initial-upload-closes-no-bugs`: First upload doesn't close any bugs
  - **Ignore**: Normal for new packages

To fix cosmetic issues:
```bash
# Use dch for changelog entries (auto-formats correctly)
dch -v 1.0.0-1 -D jammy "Initial release"
```

## Troubleshooting Package Build

### Missing dependencies
```bash
sudo apt install debhelper devscripts dh-dkms dkms
```

### Clean build environment
```bash
make clean
rm -rf debian/.debhelper debian/snd-zg01-dkms
./scripts/build-deb.sh
```

### Check build logs
```bash
cat ../snd-zg01-dkms_1.0.0-1_amd64.buildinfo
```

## Benefits of DKMS Packaging

✅ **Automatic kernel updates**: Modules rebuild automatically
✅ **Dependency management**: apt handles dependencies
✅ **Clean uninstall**: `apt remove` cleans everything
✅ **Version tracking**: dpkg tracks installed version
✅ **Distribution ready**: Can publish to PPA or OBS
✅ **Professional**: Standard Linux packaging approach

## See Also

- [INSTALLATION.md](INSTALLATION.md) - User installation guide
- [README.md](README.md) - Project overview
- [DEVELOPMENT_STATUS.md](DEVELOPMENT_STATUS.md) - Development details
