#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

cc="${CC:-cc}"
"$cc" -std=c99 -Wall -Wextra -Werror -Isrc \
    -o "$tmpdir/test-state-parser" \
    tests/test-state-parser.c src/zg01_state.c

"$tmpdir/test-state-parser" tests/fixtures/mic-state-persisted.bin
