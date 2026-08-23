#!/usr/bin/env bash

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
package_dir="$repo_root/packaging/arch"
test_root=$(mktemp -d /tmp/snd-zg01-arch-test.XXXXXX)
trap 'rm -rf "$test_root"' EXIT

test -f "$package_dir/PKGBUILD"
test -f "$package_dir/.SRCINFO"

cp -a "$package_dir/." "$test_root/"

(
  cd "$test_root"
  makepkg --printsrcinfo
) > "$test_root/generated.SRCINFO"
cmp "$test_root/.SRCINFO" "$test_root/generated.SRCINFO"

(
  cd "$test_root"
  PKGDEST="$test_root/packages" makepkg --cleanbuild --clean --noconfirm
)

package=$(find "$test_root/packages" -maxdepth 1 -type f -name 'snd-zg01-dkms-git-*.pkg.tar.zst' -print -quit)
test -n "$package"

pkgver=$(bash -c 'source "$1/PKGBUILD"; printf "%s" "$pkgver"' _ "$test_root")
src_root="usr/src/snd-zg01-$pkgver"

for path in \
  "$src_root/Makefile" \
  "$src_root/dkms.conf" \
  "$src_root/src/zg01_usb.c" \
  "$src_root/src/zg01_pcm.c" \
  "$src_root/src/zg01_control.c" \
  "$src_root/src/zg01_usb_discovery.c" \
  "usr/lib/modules-load.d/snd-zg01.conf" \
  "usr/lib/udev/rules.d/90-zg01.rules"
do
  bsdtar -tf "$package" | grep -Fxq "$path"
done

bsdtar -xOf "$package" "$src_root/dkms.conf" > "$test_root/dkms.conf"
grep -Fxq "PACKAGE_NAME=\"snd-zg01\"" "$test_root/dkms.conf"
grep -Fxq "PACKAGE_VERSION=\"$pkgver\"" "$test_root/dkms.conf"
if grep -q '^CLEAN=' "$test_root/dkms.conf"; then
  printf 'Deprecated DKMS CLEAN directive is still packaged\n' >&2
  exit 1
fi

bsdtar -xOf "$package" .PKGINFO > "$test_root/PKGINFO"
grep -Fxq 'pkgname = snd-zg01-dkms-git' "$test_root/PKGINFO"
grep -Fxq 'depend = dkms' "$test_root/PKGINFO"
grep -Fxq 'makedepend = git' "$test_root/PKGINFO"
grep -Fxq 'provides = snd-zg01-dkms' "$test_root/PKGINFO"
grep -Fxq 'conflict = snd-zg01-dkms' "$test_root/PKGINFO"

mkdir -p "$test_root/root" "$test_root/dkms" "$test_root/modules"
bsdtar -xf "$package" -C "$test_root/root"

dkms add \
  -m snd-zg01 \
  -v "$pkgver" \
  --sourcetree "$test_root/root/usr/src" \
  --dkmstree "$test_root/dkms" \
  --installtree "$test_root/modules"

dkms build \
  -m snd-zg01 \
  -v "$pkgver" \
  -k "$(uname -r)" \
  --sourcetree "$test_root/root/usr/src" \
  --dkmstree "$test_root/dkms" \
  --installtree "$test_root/modules" \
  --kernelsourcedir "/lib/modules/$(uname -r)/build"

test -f "$test_root/dkms/snd-zg01/$pkgver/$(uname -r)/$(uname -m)/module/snd-zg01.ko"

dkms remove \
  -m snd-zg01 \
  -v "$pkgver" \
  --all \
  --sourcetree "$test_root/root/usr/src" \
  --dkmstree "$test_root/dkms" \
  --installtree "$test_root/modules"

test ! -e "$test_root/dkms/snd-zg01/$pkgver"

printf 'Arch DKMS package contract passed: %s\n' "$(basename "$package")"
