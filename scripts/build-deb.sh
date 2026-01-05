#!/bin/bash

# Build script for snd-zg01 DKMS package

set -e

echo "Building snd-zg01 DKMS package..."

# Check for required tools
if ! command -v debuild &> /dev/null; then
    echo "Error: debuild not found. Please install devscripts:"
    echo "  sudo apt install devscripts debhelper dh-dkms"
    exit 1
fi

# Clean previous builds
make clean || true
rm -rf debian/snd-zg01-dkms debian/.debhelper debian/files debian/*.debhelper* debian/*.substvars
rm -f ../*.deb ../*.dsc ../*.tar.* ../*.buildinfo ../*.changes

# Build the package
echo ""
echo "Building Debian package..."
debuild -us -uc -b

# Clean latest build temporary artifacts
make clean || true
rm -rf debian/snd-zg01-dkms debian/.debhelper debian/files debian/*.debhelper* debian/*.substvars

echo ""
echo "Build complete! Package created:"
ls -lh ../*.deb

echo ""
echo "To install the package, run:"
echo "  sudo dpkg -i ../snd-zg01-dkms_*.deb"
echo "  sudo apt-get install -f  # If there are dependency issues"
