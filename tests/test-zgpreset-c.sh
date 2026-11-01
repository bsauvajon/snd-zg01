#!/usr/bin/env bash
# Contract test for the C MBDF parser in tools/zgpreset.c.
#
# The fixtures are synthetic MBDF files built below; no Yamaha or user preset
# is read or committed.  The same container and value layout is exercised by
# tests/test-zgpreset.py, so the C port and the Python reference stay honest
# about the format rules.

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d /tmp/snd-zg01-zgpreset-c.XXXXXX)
trap 'rm -rf "$tmp"' EXIT

python3 - "$tmp" <<'PY'
"""Build synthetic MBDF fixtures for tests/test-zgpreset.c."""

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


def mbdf(chunks, magic=b"#YAMAHA MBDFPreset"):
    header = bytearray(HEADER_SIZE)
    header[:len(magic)] = magic
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


def schema(entries, header=b"\x80\x01\x00\x01\x00\x12"):
    return header + b"\x41\x01\x00\x80" + b"".join(
        bytes((address >> 8, address & 0xFF, 0x01, width))
        for address, width in entries)


def schema_body(body, header=b"\x80\x01\x00\x01\x00\x12"):
    return header + b"\x41\x01\x00\x80" + body


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


def data_chunk(payload):
    return chunk("#PRESET ", "ChunkData", payload)


mic = ENVELOPE + b"".join((
    component("F_MicEq", schema([(0x120, 1), (0x121, 1), (0x128, 2)])),
    component("D_MicEq",
              b"\x00\x02" + b"Custom".ljust(126, b"\x00")
              + bytes((0x01, 0x00, 0x96, 0x00)), magic=0xD927),
))
write("mic.zgpreset", mbdf([
    chunk("#FILE", "Property", b""),
    chunk("#PRESET ", "General", info("MIC INPUT", "Custom")),
    data_chunk(mic),
    chunk("#END", "", b""),
]))

backup = ENVELOPE + b"".join((
    component("F_CurrentScene", schema([(0x001, 1), (0x006, 2)])),
    component("D_CurrentScene",
              b"\x02Current Scene".ljust(127, b"\x00")
              + b"\x05" + struct.pack("<H", 0x1234)),
))
write("backup.zg01", mbdf(
    [chunk("#CONSOLE", "ChunkData", backup)],
    magic=b"#YAMAHA MBDFBackup"))

# Game Effect has a two-byte header before the marker, and its D_* display
# name is one byte longer than Mic Input's; both rules must still find the
# values at the end of the component.
game = ENVELOPE + b"".join((
    component("F_GameEffect",
              schema([(0x075, 1), (0x076, 1)], header=b"\x00\x73")),
    component("D_GameEffect",
              b"\x02" + b"GameFxSample1".ljust(127, b"\x00")
              + b"\x03\x01"),
))
write("game.zgpreset", mbdf([
    chunk("#PRESET ", "General", info("GAME EFFECT", "Game")),
    data_chunk(game),
]))

# A display name longer than the old 63-byte buffer, ending with a
# multi-byte UTF-8 sequence: the parser must preserve it, not truncate.
long_name = "A" * 62 + "é tail"
write("long-name.zgpreset", mbdf([
    chunk("#PRESET ", "General", info("MIC INPUT", long_name)),
    data_chunk(ENVELOPE + b"".join((
        component("F_MicEq", schema([(0x120, 1)])),
        component("D_MicEq", b"\x00\x01", magic=0xD927),
    ))),
]))

# Metadata-only containers are still readable, but a preset with no
# decoded parameter must not look like a successful empty load.
write("empty.zgpreset", mbdf([
    chunk("#PRESET ", "General", info("MIC INPUT", "Empty")),
    chunk("#END", "", b""),
]))

# A schema with no 41 01 00 80 marker cannot be decoded.
write("missing-marker.zgpreset", mbdf([
    data_chunk(ENVELOPE + b"".join((
        component("F_MicEq", b"\x80\x01\x00\x01\x00\x12\x01\x20\x01\x01"),
        component("D_MicEq", b"\x00\x01"),
    ))),
]))

# A marker followed by a partial 4-byte record is truncated.
write("truncated-schema.zgpreset", mbdf([
    data_chunk(ENVELOPE + b"".join((
        component("F_MicEq", schema_body(b"\x01\x20\x01")),
        component("D_MicEq", b"\x00\x01"),
    ))),
]))

# A chunk length that runs past the end of the file.
overlong = bytearray(mbdf([chunk("#FILE", "x", b"")]))
struct.pack_into(">Q", overlong, HEADER_SIZE + len(FILE_ID) + 24, 1 << 20)
write("overlong.zgpreset", bytes(overlong))

write("bad-magic.zgpreset", mbdf([
    data_chunk(ENVELOPE + component(
        "F_MicEq", schema([(0x120, 1)]), magic=0x1234)),
]))

write("bad-width.zgpreset", mbdf([
    data_chunk(ENVELOPE + b"".join((
        component("F_MicEq", schema([(0x120, 3)])),
        component("D_MicEq", b"\x01"),
    ))),
]))

write("short-values.zgpreset", mbdf([
    data_chunk(ENVELOPE + b"".join((
        component("F_MicEq", schema([(0x120, 2)])),
        component("D_MicEq", b"\x00"),
    ))),
]))

write("missing-values.zgpreset", mbdf([
    data_chunk(ENVELOPE + component("F_MicEq", schema([(0x120, 1)]))),
]))

write("not-mbdf.zgpreset", b"not an MBDF file")
PY

cc -std=c11 -Wall -Wextra -Werror -O2 \
   -o "$tmp/test-zgpreset" \
   "$repo_root/tests/test-zgpreset.c" \
   "$repo_root/tools/zgpreset.c"

"$tmp/test-zgpreset" "$tmp"
