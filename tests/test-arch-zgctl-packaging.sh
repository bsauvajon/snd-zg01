#!/usr/bin/env bash

# Contract test for the Arch zgctl package.  makepkg only exists on Arch,
# so when it is missing (e.g. a Debian/Ubuntu CI runner) fall back to a
# static check of the PKGBUILD.

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
pkgbuild="$repo_root/packaging/arch-zgctl/PKGBUILD"

test -f "$pkgbuild"

grep -Fxq "pkgname=zgctl-git" "$pkgbuild"
grep -Fq "arch=('x86_64' 'aarch64')" "$pkgbuild"
grep -Fq 'install -Dm755 "${_pkgname}/tools/zgctl"' "$pkgbuild"
grep -Fq 'make -C "${_pkgname}" tools' "$pkgbuild"

if ! command -v makepkg >/dev/null 2>&1; then
  printf 'Arch zgctl PKGBUILD static contract passed (makepkg absent)\n'
  exit 0
fi

test_root=$(mktemp -d /tmp/snd-zg01-zgctl-test.XXXXXX)
trap 'rm -rf "$test_root"' EXIT
cp -a "$repo_root/packaging/arch-zgctl/." "$test_root/"

(
  cd "$test_root"
  PKGDEST="$test_root/packages" makepkg --cleanbuild --clean --noconfirm
)

package=$(find "$test_root/packages" -maxdepth 1 -type f -name 'zgctl-git-*.pkg.tar.zst' -print -quit)
test -n "$package"
bsdtar -tf "$package" | grep -Fxq 'usr/bin/zgctl'
bsdtar -xOf "$package" .PKGINFO > "$test_root/PKGINFO"
grep -Fxq 'pkgname = zgctl-git' "$test_root/PKGINFO"

printf 'Arch zgctl package contract passed\n'
