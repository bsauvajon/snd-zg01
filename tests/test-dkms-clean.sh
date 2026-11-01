#!/usr/bin/env bash
# Contract test for `make clean` in the tree the DKMS packages install.
#
# debian/snd-zg01-dkms.install and packaging/arch/PKGBUILD ship the module
# sources, src/, dkms.conf and the Makefile - no tools/, which is packaged
# separately - and dkms.conf runs `make clean` in that tree.  Cleaning the
# userspace CLI must therefore not be a hard dependency of cleaning the
# module, or every DKMS clean/uninstall reports a failure.
#
# Needs kernel headers; skips when they are absent (a plain container).

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
kernel_version=${KERNEL_VERSION:-$(uname -r)}

kernel_build_dir="/lib/modules/$kernel_version/build"
if [ ! -d "$kernel_build_dir" ] && [ -d "/usr/lib/modules/$kernel_version/build" ]; then
  kernel_build_dir="/usr/lib/modules/$kernel_version/build"
fi

if [ ! -d "$kernel_build_dir" ]; then
  printf 'DKMS clean contract skipped (no kernel headers for %s)\n' "$kernel_version"
  exit 0
fi

test_root=$(mktemp -d /tmp/snd-zg01-dkms-clean.XXXXXX)
trap 'rm -rf "$test_root"' EXIT

# Exactly the files the DKMS packages install into /usr/src/<name>-<version>/.
mkdir -p "$test_root/tree"
cp -a "$repo_root/Makefile" "$repo_root/dkms.conf" "$test_root/tree/"
cp -a "$repo_root/src" "$test_root/tree/src"
test -f "$test_root/tree/dkms.conf"
test ! -e "$test_root/tree/tools"

(
  cd "$test_root/tree"
  make KDIR="$kernel_build_dir" clean > "$test_root/clean.log" 2>&1
) || {
  cat "$test_root/clean.log" >&2
  printf 'make clean failed in a DKMS-shaped tree\n' >&2
  exit 1
}

if grep -qi "no such file or directory" "$test_root/clean.log"; then
  cat "$test_root/clean.log" >&2
  printf 'make clean tried to reach a directory the package does not ship\n' >&2
  exit 1
fi

printf 'DKMS clean contract passed (no tools/ in the tree)\n'
