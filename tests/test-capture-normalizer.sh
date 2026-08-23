#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
actual=$(mktemp)
trap 'rm "$actual"' EXIT

python3 "$repo_root/tools/normalize-usbpcap.py" \
  "$repo_root/tests/fixtures/usb-control.tsv" > "$actual"

python3 - "$actual" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    rows = [json.loads(line) for line in stream]

assert rows == [
    {
        "frame": 1,
        "time": 0.1,
        "device": "1.7.3",
        "direction": "in",
        "request_type": 192,
        "request": 7,
        "value": 0,
        "index": 0,
        "length": 3,
        "data": "80bb00",
    },
    {
        "frame": 2,
        "time": 0.25,
        "device": "1.7.3",
        "direction": "out",
        "request_type": 64,
        "request": 9,
        "value": 1,
        "index": 0,
        "length": 4,
        "data": "01020304",
    },
]
PY

printf 'USBPcap normalizer contract passed\n'
