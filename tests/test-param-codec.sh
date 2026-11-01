#!/usr/bin/env bash

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
bin=$(mktemp /tmp/snd-zg01-param-codec.XXXXXX)
trap 'rm -f "$bin"' EXIT

cc -std=c11 -Wall -Wextra -Werror -O2 \
   -o "$bin" "$repo_root/tests/test-param-codec.c"

"$bin"
