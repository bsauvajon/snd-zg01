#!/usr/bin/env bash

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

cd "$repo_root"
cc -I src -I tests tests/test-zg01-meters.c -o "$work/test-zg01-meters"
"$work/test-zg01-meters"
