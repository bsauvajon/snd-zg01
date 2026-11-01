#!/usr/bin/env bash
# Compare the Python reference parser and the C parser on shared fixtures.
#
# `tools/zgpreset.py dump FILE` and tests/zgpreset-dump.c print the same JSON
# document.  The fixtures cover the two schema header lengths (six bytes for
# Mic Input and Monitor EQ, two for Game Effect), the end-relative value
# blob, and a schema with no marker (both implementations must reject it).
# No Yamaha or user preset is read or committed.

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d /tmp/snd-zg01-zgpreset-compare.XXXXXX)
trap 'rm -rf "$tmp"' EXIT

python3 - "$tmp" <<'PY'
"""Build the shared MBDF fixtures for the Python/C comparison."""

import pathlib
import struct
import sys

out = pathlib.Path(sys.argv[1])
HEADER_SIZE = 0x74
FILE_ID = b"79f82ba4-a076-47e8-887d-f34571f0fe49"
ENVELOPE = b"Comp\x00\x00\x00\x00"


def chunk(tag, name, payload):
    return b"".join((
        tag.encode("ascii").ljust(8, b"\x00"),
        b"\x00" * 4,
        name.encode("ascii")[:12].ljust(12, b"\x00"),
        struct.pack(">Q", len(payload)),
        b"\x00" * 4,
        payload,
    ))


def mbdf(chunks):
    header = bytearray(HEADER_SIZE)
    header[:len(b"#YAMAHA MBDFPreset")] = b"#YAMAHA MBDFPreset"
    return bytes(header) + FILE_ID + b"".join(chunks)


def component(name, payload, magic=0xDA27):
    return b"".join((
        name.encode("ascii")[:24].ljust(24, b"\x00"),
        struct.pack(">H", magic),
        b"\x00\x00",
        struct.pack("<H", len(payload)),
        b"\x00\x00",
        payload,
    ))


def schema(entries, header):
    return header + b"\x41\x01\x00\x80" + b"".join(
        bytes((address >> 8, address & 0xFF, 0x01, width))
        for address, width in entries)


def info(block, name):
    return b"".join((
        block.encode("ascii").ljust(64, b"\x00"),
        b"\x00" * 16,
        b"\x02",
        name.encode("utf-8"),
        b"\x00\x00",
    ))


def write(name, data):
    (out / name).write_bytes(data)


def preset(block, block_name, payload):
    return mbdf([
        chunk("#PRESET ", "General", info(block, block_name)),
        chunk("#PRESET ", "ChunkData", ENVELOPE + payload),
        chunk("#END", "", b""),
    ])


# Mic Input: six-byte schema header, 1/1/2-byte values.
write("mic.zgpreset", preset("MIC INPUT", "Custom", b"".join((
    component("F_MicEq", schema(
        [(0x120, 1), (0x121, 1), (0x128, 2)], b"\x80\x01\x00\x01\x00\x12")),
    component("D_MicEq",
              b"\x00\x02" + b"Custom".ljust(126, b"\x00")
              + b"\x01\x00\x96\x00", magic=0xD927),
))))

# Game Effect: two-byte schema header, a one-byte longer display name.
write("game.zgpreset", preset("GAME EFFECT", "Game", b"".join((
    component("F_GameEffect", schema(
        [(0x075, 1), (0x076, 1)], b"\x00\x73")),
    component("D_GameEffect",
              b"\x02" + b"GameFxSample1".ljust(127, b"\x00")
              + b"\x03\x01"),
))))

# Monitor EQ: six-byte header and a 2-byte value after two 1-byte values.
write("monitor.zgpreset", preset("MONITOR EQ", "Monitor", b"".join((
    component("F_MonitorEq", schema(
        [(0x0ee, 1), (0x0ef, 1), (0x120, 2)], b"\x80\x01\x00\x01\x00\x12")),
    component("D_MonitorEq",
              b"\x02" + b"Monitor".ljust(120, b"\x00")
              + b"\x01\x02\x34\x12", magic=0xD927),
))))

# A display name longer than 63 bytes, ending with a multi-byte UTF-8
# sequence: Python and C must return the same full name.
write("long-name.zgpreset", preset(
    "MIC INPUT", "A" * 62 + "é tail", b"".join((
        component("F_MicEq", schema([(0x120, 1)],
                                   b"\x80\x01\x00\x01\x00\x12")),
        component("D_MicEq", b"\x00\x01", magic=0xD927),
    ))))

# No schema marker: neither parser may invent a decode.
write("missing-marker.zgpreset", preset("MIC INPUT", "Broken", b"".join((
    component("F_MicEq", b"\x80\x01\x00\x01\x00\x12\x01\x20\x01\x01"),
    component("D_MicEq", b"\x00\x01"),
))))
PY

cc -std=c11 -Wall -Wextra -Werror -O2 \
   -o "$tmp/zgpreset-dump" \
   "$repo_root/tests/zgpreset-dump.c" \
   "$repo_root/tools/zgpreset.c"

for fixture in mic game monitor long-name; do
    python3 "$repo_root/tools/zgpreset.py" dump \
        "$tmp/$fixture.zgpreset" > "$tmp/$fixture.py.json"
    "$tmp/zgpreset-dump" "$tmp/$fixture.zgpreset" > "$tmp/$fixture.c.json"
    if ! cmp -s "$tmp/$fixture.py.json" "$tmp/$fixture.c.json"; then
        printf 'FAIL %s: Python and C dumps differ\n--- Python ---\n%s\n--- C ---\n%s\n' \
            "$fixture" "$(cat "$tmp/$fixture.py.json")" \
            "$(cat "$tmp/$fixture.c.json")" >&2
        exit 1
    fi
done

# Pin the facts the fixtures are meant to cover.
grep -q '"address": 296, "value": 150' "$tmp/mic.py.json" \
    || { echo 'FAIL mic fixture lost the 2-byte value at 0x128' >&2; exit 1; }
grep -q '"address": 117, "value": 3' "$tmp/game.py.json" \
    || { echo 'FAIL game fixture lost the 2-byte-header value at 0x075' >&2; exit 1; }
grep -q '"address": 118, "value": 1' "$tmp/game.py.json" \
    || { echo 'FAIL game fixture lost the 2-byte-header value at 0x076' >&2; exit 1; }
grep -q '"address": 288, "value": 4660' "$tmp/monitor.py.json" \
    || { echo 'FAIL monitor fixture lost the 2-byte value at 0x120' >&2; exit 1; }
python3 - "$tmp/long-name.py.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    document = json.load(handle)
assert document["name"] == "A" * 62 + "é tail", document["name"]
PY

# Both implementations must reject the missing-marker fixture.
set +e
python3 "$repo_root/tools/zgpreset.py" dump \
    "$tmp/missing-marker.zgpreset" >/dev/null 2>&1
python_status=$?
"$tmp/zgpreset-dump" "$tmp/missing-marker.zgpreset" >/dev/null 2>&1
c_status=$?
set -e
[ "$python_status" -ne 0 ] || { echo 'FAIL Python accepted a missing marker' >&2; exit 1; }
[ "$c_status" -ne 0 ] || { echo 'FAIL C accepted a missing marker' >&2; exit 1; }

printf 'zgpreset Python/C comparison contract passed\n'
