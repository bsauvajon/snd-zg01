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
# the source must stay overridable, that is what lets this test build the
# recipe from the tree under test rather than from main
grep -Fq 'ZG01_SOURCE' "$pkgbuild"

if ! command -v makepkg >/dev/null 2>&1; then
  printf 'Arch zgctl PKGBUILD static contract passed (makepkg absent)\n'
  exit 0
fi

test_root=$(mktemp -d /tmp/snd-zg01-zgctl-test.XXXXXX)
trap 'rm -rf "$test_root"' EXIT
cp -a "$repo_root/packaging/arch-zgctl/." "$test_root/"

# The PKGBUILD fetches branch=main, which on a pull request does not carry this
# packaging yet - the `tools` target and this directory arrive with the same
# change - so a package built from the default source could never satisfy this
# contract.  Stage the tree under test as a throwaway git repository instead:
# the source stays a git source (so pkgver() works) but holds this commit.
# Build outputs are left out, otherwise a stale tools/zgctl would satisfy
# `make tools` without compiling anything.
stage="$test_root/src"
mkdir -p "$stage/snd-zg01"
tar -C "$repo_root" \
    --exclude=.git --exclude='*.o' --exclude='*.ko' --exclude='*.mod' \
    --exclude='*.mod.*' --exclude='*.cmd' --exclude=Module.symvers \
    --exclude=.tmp_versions --exclude='tools/zgctl' \
    -cf - . | tar -C "$stage/snd-zg01" -xf -
(
  cd "$stage/snd-zg01"
  git init -q -b main
  git -c user.name=ci -c user.email=ci@example.invalid -c commit.gpgsign=false \
      add -A
  git -c user.name=ci -c user.email=ci@example.invalid -c commit.gpgsign=false \
      commit -qm 'tree under test'
)

(
  cd "$test_root"
  ZG01_SOURCE="git+file://$stage/snd-zg01#branch=main" \
    PKGDEST="$test_root/packages" makepkg --cleanbuild --clean --noconfirm
)

package=$(find "$test_root/packages" -maxdepth 1 -type f -name 'zgctl-git-*.pkg.tar.zst' -print -quit)
test -n "$package"
bsdtar -tf "$package" | grep -Fxq 'usr/bin/zgctl'
bsdtar -xOf "$package" .PKGINFO > "$test_root/PKGINFO"
grep -Fxq 'pkgname = zgctl-git' "$test_root/PKGINFO"

printf 'Arch zgctl package contract passed\n'
