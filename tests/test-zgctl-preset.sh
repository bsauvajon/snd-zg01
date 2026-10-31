#!/usr/bin/env bash
# Contract test for `zgctl preset list FILE` and `zgctl preset show FILE`.
#
# The fixtures are synthetic MBDF files built below; no Yamaha or user preset
# is read or committed.  The commands are expected to work without a ZG01
# card: the test compiles the CLI into a temporary directory and never opens
# /dev/snd.

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d /tmp/snd-zg01-zgctl-preset.XXXXXX)
trap 'rm -rf "$tmp"' EXIT

python3 - "$tmp" <<'PY'
"""Build synthetic MBDF fixtures for tests/test-zgctl-preset.sh."""

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


def info(block, name):
    return b"".join((
        block.encode("ascii").ljust(64, b"\x00"),
        b"\x00" * 16,
        b"\x02",
        name.encode("ascii"),
        b"\x00\x00",
    ))


def write(name, data):
    (out / name).write_bytes(data)


mic = ENVELOPE + b"".join((
    component("F_MicEq", schema([(0x120, 1), (0x121, 1), (0x128, 2)])),
    component("D_MicEq",
              b"\x00\x02" + b"Custom".ljust(126, b"\x00")
              + bytes((0x01, 0x00, 0x96, 0x00)), magic=0xD927),
))
write("mic.zgpreset", mbdf([
    chunk("#FILE", "Property", b""),
    chunk("#PRESET ", "General", info("MIC INPUT", "Custom")),
    chunk("#PRESET ", "ChunkData", mic),
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

mystery = ENVELOPE + b"".join((
    component("F_Mystery", schema([(0x777, 1)])),
    component("D_Mystery",
              b"\x02Mystery".ljust(127, b"\x00") + b"\x09"),
))
write("mystery.zgpreset", mbdf([
    chunk("#PRESET ", "General", info("MYSTERY", "None")),
    chunk("#PRESET ", "ChunkData", mystery),
]))

write("not-mbdf.zgpreset", b"not an MBDF file")
PY

cc -std=c11 -Wall -Wextra -Werror -O2 \
   -o "$tmp/zgctl" \
   "$repo_root/tools/zgctl.c" \
   "$repo_root/tools/zgpreset.c"

fail() {
    printf 'FAIL %s\n' "$*" >&2
    exit 1
}

# Run a command that must succeed and compare its complete output.
expect_output() {
    local want=$1
    shift
    local got
    got=$("$tmp/zgctl" "$@") || fail "zgctl $* exited nonzero"
    [ "$got" = "$want" ] || {
        printf 'FAIL zgctl %s\n--- got ---\n%s\n--- want ---\n%s\n' \
            "$*" "$got" "$want" >&2
        exit 1
    }
}

expected_list="block: MIC INPUT
name: Custom
params: 3"

expected_show="block: MIC INPUT
name: Custom
0x120 Mic EQ Band 1 Switch Steps = 1
0x121 Mic EQ Band 2 Switch Steps = 0
0x128 EQ Band 1 Gain = 150"

expect_output "$expected_list" preset list "$tmp/mic.zgpreset"
expect_output "$expected_show" preset show "$tmp/mic.zgpreset"

# The commands do not look at the card, so a bogus -c is harmless.
expect_output "$expected_list" -c no-such-zg01-card preset list \
    "$tmp/mic.zgpreset"

# A .zg01 backup is rejected with a clear message, not silently half-read.
for subcommand in show list; do
    set +e
    error=$("$tmp/zgctl" preset "$subcommand" "$tmp/backup.zg01" 2>&1)
    status=$?
    set -e
    [ "$status" -eq 2 ] || fail "preset $subcommand backup exited $status, want 2"
    grep -qi 'backup' <<<"$error" \
        || fail "preset $subcommand backup error is not clear: $error"
done

# Addresses outside the generated control map are reported as unknown.
mystery_out=$("$tmp/zgctl" preset show "$tmp/mystery.zgpreset") \
    || fail "preset show mystery exited nonzero"
grep -qx '0x777 unknown = 9' <<<"$mystery_out" \
    || fail "unknown address line missing"

# A malformed file is a parse error, not a card error.
set +e
error=$("$tmp/zgctl" preset list "$tmp/not-mbdf.zgpreset" 2>&1)
status=$?
set -e
[ "$status" -eq 2 ] || fail "parse error exited $status, want 2"
grep -q 'cannot parse' <<<"$error" || fail "parse error message missing"

printf 'zgctl preset list/show contract passed\n'
