#!/usr/bin/env bash

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
build_log=$(mktemp /tmp/snd-zg01-kernel-build.XXXXXX)
trap 'rm -f "$build_log"' EXIT

make -C "$repo_root" clean >/dev/null 2>&1 || true
make -C "$repo_root" -j"$(nproc)" W=1 2>&1 | tee "$build_log"

if grep -Eiq '(^|[^[:alpha:]])(warning|error):' "$build_log"; then
  printf 'Kernel build emitted a compiler warning or error\n' >&2
  exit 1
fi

test -f "$repo_root/snd-zg01.ko"
modinfo "$repo_root/snd-zg01.ko" | grep -Fq "vermagic:       $(uname -r)"

printf 'Strict kernel build passed for %s\n' "$(uname -r)"
