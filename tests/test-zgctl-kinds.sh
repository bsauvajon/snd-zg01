#!/usr/bin/env bash
# Contract test for zgctl's unit classification (see test-zgctl-kinds.c).

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
bin=$(mktemp /tmp/snd-zg01-zgctl-kinds.XXXXXX)
trap 'rm -f "$bin"' EXIT

cc -std=c11 -Wall -Wextra -Werror -O2 \
   -o "$bin" "$repo_root/tests/test-zgctl-kinds.c"

"$bin"
