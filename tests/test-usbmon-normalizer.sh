#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fixture="$work/usbmon.pcap"
out="$work/out.jsonl"

# Build a small synthetic usbmon (DLT_USB_LINUX_MMAPPED) pcap covering a
# control IN, a control OUT, a bulk IN, an isochronous frame and a second
# device. No binary fixture lives in the repository.
python3 - "$fixture" <<'PY'
import struct
import sys

MAGIC = 0xa1b2c3d4
DLT_USB_LINUX_MMAPPED = 220
HEADER = 64


def frame(urb, record, xfer, epnum, devnum, setup=None, payload=b"", length=0,
          status=0, ndesc=0, descriptors=b""):
    setup = setup or b"\x00" * 8
    body = struct.pack(
        "<QBBBBHccqiiII",
        urb, record, xfer, epnum, devnum, 1,
        b"\x00" if setup else b"-", b"\x00" if payload else b"-",
        0, 0, status, length, len(payload),
    ) + setup
    body += b"\x00" * (HEADER - len(body))
    body = body[:56] + struct.pack("<I", ndesc) + body[60:]
    return body + descriptors + payload


def record(header, payload):
    return struct.pack("<IIII", 0, 0, len(payload), len(payload)) + payload


packets = [
    # Device 5: control IN, request 7 -> 80bb00.
    frame(0x1000, ord("S"), 2, 0x80, 5, b"\xc0\x07\x00\x00\x00\x00\x03\x00", length=3),
    frame(0x1000, ord("C"), 2, 0x80, 5, payload=b"\x80\xbb\x00"),
    # Device 5: control OUT, request 1, 2 bytes.
    frame(0x1001, ord("S"), 2, 0x00, 5, b"\x40\x01\x00\x00\x00\x00\x02\x00", payload=b"\x01\x02", length=2),
    frame(0x1001, ord("C"), 2, 0x00, 5),
    # Device 5: bulk IN.
    frame(0x1002, ord("S"), 3, 0x82, 5, length=512),
    frame(0x1002, ord("C"), 3, 0x82, 5, payload=bytes.fromhex("deadbeef")),
    # Device 5: isochronous, must be skipped by default.
    frame(0x1003, ord("S"), 0, 0x81, 5, length=192, payload=b"\x00" * 192),
    # Device 5: isochronous with one 16-byte descriptor before the data.
    # The descriptor is not audio and must not be emitted as payload.
    frame(0x1006, ord("S"), 0, 0x81, 5, length=4,
          payload=bytes.fromhex("deadbeef"), ndesc=1,
          descriptors=b"\x00" * 16),
    # Device 5: bulk submission error.  E is terminal, like C, and must
    # preserve the failure status instead of looking like a missing C.
    frame(0x1005, ord("S"), 3, 0x03, 5, payload=b"abcd", length=4),
    frame(0x1005, ord("E"), 3, 0x03, 5, status=-32, length=4),
    # Device 6: another control IN, filtered out by --device 1.5.
    frame(0x1004, ord("S"), 2, 0x80, 6, b"\xc0\x07\x00\x00\x00\x00\x03\x00", length=3),
    frame(0x1004, ord("C"), 2, 0x80, 6, payload=b"\x80\xbb\x00"),
]

blob = bytearray(struct.pack("<IHHiIII", MAGIC, 2, 4, 0, 0, 65535, DLT_USB_LINUX_MMAPPED))
blob += b"".join(record(None, packet) for packet in packets)
open(sys.argv[1], "wb").write(blob)
PY

python3 "$repo_root/tools/normalize-usbmon.py" "$fixture" > "$out"

python3 - "$out" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    rows = [json.loads(line) for line in stream]

assert len(rows) == 5, rows

control_in = rows[0]
assert control_in["transfer"] == "control"
assert control_in["direction"] == "in"
assert control_in["request_type"] == 192
assert control_in["request"] == 7
assert control_in["value"] == 0
assert control_in["index"] == 0
assert control_in["length"] == 3
assert control_in["actual_length"] == 3
assert control_in["data"] == "80bb00"
assert control_in["device"] == 5

control_out = rows[1]
assert control_out["direction"] == "out"
assert control_out["request_type"] == 64
assert control_out["request"] == 1
assert control_out["data"] == "0102"

bulk = rows[2]
assert bulk["transfer"] == "bulk"
assert bulk["endpoint"] == "0x82"
assert bulk["data"] == "deadbeef"

# Submission error: E is terminal and keeps its status and data.
failed = rows[3]
assert failed["transfer"] == "bulk"
assert failed["direction"] == "out"
assert failed["status"] == -32
assert failed["data"] == "61626364"

assert rows[4]["device"] == 6
PY

python3 "$repo_root/tools/normalize-usbmon.py" "$fixture" --device 1.5 > "$out"
test "$(wc -l < "$out")" -eq 4

# With --include-iso each usbmon record is emitted unmerged (isochronous
# transfers have many completions per submit).
python3 "$repo_root/tools/normalize-usbmon.py" "$fixture" --include-iso > "$out"
test "$(wc -l < "$out")" -eq 12

python3 - "$out" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    rows = [json.loads(line) for line in stream]

iso = [row for row in rows if row["transfer"] == "isochronous"]
assert iso, "no isochronous records"
assert any(row["data"] == "deadbeef" for row in iso), iso
PY

# A non-usbmon link type must be rejected.
printf 'not a pcap' > "$work/bad.pcap"
if python3 "$repo_root/tools/normalize-usbmon.py" "$work/bad.pcap" >/dev/null 2>&1; then
  printf 'normalizer accepted a truncated pcap\n' >&2
  exit 1
fi

printf 'usbmon normalizer contract passed\n'
