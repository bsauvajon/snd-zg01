#!/usr/bin/env bash
# Contract test for `zgctl preset load --dry-run`.
#
# The fixtures are synthetic MBDF files built below; no Yamaha or user preset
# is read or committed.  Dry-run must never open a card, so the test compiles
# the CLI into a temporary directory and uses a bogus card name.

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d /tmp/snd-zg01-zgctl-load-dry.XXXXXX)
trap 'rm -rf "$tmp"' EXIT

python3 - "$tmp" <<'PY'
"""Build synthetic MBDF fixtures for tests/test-zgctl-load-dry.sh."""

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


def schema(entries):
    return b"\x80\x01\x00\x01\x00\x12" + b"\x41\x01\x00\x80" + b"".join(
        bytes((address >> 8, address & 0xFF, 0x01, width))
        for address, width, _ in entries)


def values(entries):
    return b"".join(value.to_bytes(width, "little")
                    for _, width, value in entries)


def info(block, name):
    return b"".join((
        block.encode("ascii").ljust(64, b"\x00"),
        b"\x00" * 16,
        b"\x02",
        name.encode("ascii"),
        b"\x00\x00",
    ))


def preset(block, entries, name="Dry Run"):
    payload = ENVELOPE + b"".join((
        component("F_MicEq", schema(entries)),
        component("D_MicEq",
                  b"\x00\x02" + name.encode("ascii").ljust(126, b"\x00")
                  + values(entries), magic=0xD927),
    ))
    return mbdf([
        chunk("#FILE", "Property", b""),
        chunk("#PRESET ", "General", info(block, name)),
        chunk("#PRESET ", "ChunkData", payload),
        chunk("#END", "", b""),
    ])


def write(name, data):
    (out / name).write_bytes(data)


# Every address is supported: switch, gain, frequency and Q.
write("supported.zgpreset", preset("MIC INPUT", [
    (0x120, 1, 1),
    (0x128, 2, 150),
    (0x12c, 2, 100),
    (0x130, 1, 12),
]))

# EQ Band 1 Gain 400 converts to +22.0 dB, past the +18.0 dB maximum.
write("out-of-range.zgpreset", preset("MIC INPUT", [(0x128, 2, 400)]))

# Monitor EQ Band 4 Switch Steps is a raw vendor-model exception.
write("raw.zgpreset", preset("MIC INPUT", [(0x0f0, 1, 1)]))

# Streaming Mic Level is vendor-model through its _ForBackup mirror, so it
# applies without --allow-raw even though its control name ends in Steps.
write("streaming-mic.zgpreset", preset("STREAMING", [(0x10f, 1, 64)]))

# Mic Level Status is read-only; 0x777 is not in the control map.
write("unsupported.zgpreset", preset("MIC INPUT", [
    (0x028, 1, 5),
    (0x777, 1, 9),
]))

write("monitor.zgpreset", preset("MONITOR EQ", [(0x0ee, 1, 1)]))

write("not-mbdf.zgpreset", b"not an MBDF file")

# A .zg01 backup is out of scope and must be rejected before planning.
backup = ENVELOPE + b"".join((
    component("F_CurrentScene", schema([(0x001, 1, 5)])),
    component("D_CurrentScene", b"\x02Current Scene".ljust(127, b"\x00")
              + b"\x05"),
))
write("backup.zg01", mbdf(
    [chunk("#CONSOLE", "ChunkData", backup)],
    magic=b"#YAMAHA MBDFBackup"))
PY

cc -std=c11 -Wall -Wextra -Werror -O2 \
   -o "$tmp/zgctl" \
   "$repo_root/tools/zgctl.c" \
   "$repo_root/tools/zgpreset.c"

fail() {
    printf 'FAIL %s\n' "$*" >&2
    exit 1
}

# Run a command, require an exact exit status and complete stdout.
expect_run() {
    local want_status=$1
    local want_out=$2
    shift 2

    local got status
    set +e
    got=$("$tmp/zgctl" "$@" 2>"$tmp/stderr")
    status=$?
    set -e

    [ "$status" -eq "$want_status" ] || {
        printf 'FAIL zgctl %s exited %d, want %d\n--- stderr ---\n%s\n' \
            "$*" "$status" "$want_status" "$(cat "$tmp/stderr")" >&2
        exit 1
    }
    [ "$got" = "$want_out" ] || {
        printf 'FAIL zgctl %s\n--- got ---\n%s\n--- want ---\n%s\n' \
            "$*" "$got" "$want_out" >&2
        exit 1
    }
}

expected_supported="0x120 Mic EQ Band 1 Switch Steps = 1 (apply)
0x128 EQ Band 1 Gain = -30 (apply)
0x12c EQ Band 1 Frequency = 100 (apply)
0x130 EQ Band 1 Q = 100 (apply)"

# A supported preset applies every parameter, with or without a card name.
expect_run 0 "$expected_supported" \
    preset load "$tmp/supported.zgpreset" --dry-run
expect_run 0 "$expected_supported" \
    -c no-such-zg01-card preset load "$tmp/supported.zgpreset" --dry-run
# Options may precede the file.
expect_run 0 "$expected_supported" \
    preset load --dry-run "$tmp/supported.zgpreset"

# --block is case-insensitive and whitespace-normalized.
expect_run 0 "$expected_supported" \
    preset load "$tmp/supported.zgpreset" --dry-run --block '  mic   input '

# An out-of-range value is reported, never clamped.
expect_run 1 "0x128 EQ Band 1 Gain = 220 (out_of_range, skipped)" \
    preset load "$tmp/out-of-range.zgpreset" --dry-run

# Raw addresses need --allow-raw.
expect_run 1 "0x0f0 Monitor EQ Band 4 Switch Steps = 1 (raw, skipped)" \
    preset load "$tmp/raw.zgpreset" --dry-run
expect_run 0 "0x0f0 Monitor EQ Band 4 Switch Steps = 1 (apply)" \
    preset load "$tmp/raw.zgpreset" --dry-run --allow-raw

# A vendor-model row with a Steps name needs no opt-in.
expect_run 0 "0x10f Streaming Mic Level Steps = 64 (apply)" \
    preset load "$tmp/streaming-mic.zgpreset" --dry-run

# Read-only and unknown addresses are unsupported.
expect_run 1 "0x028 Mic Level Status = 5 (unsupported, skipped)
0x777 unknown = 9 (unsupported, skipped)" \
    preset load "$tmp/unsupported.zgpreset" --dry-run

# The block must match the parsed preset block.
expect_run 0 "0x0ee Monitor EQ Switch Steps = 1 (apply)" \
    preset load "$tmp/monitor.zgpreset" --dry-run --block ' monitor  eq '

set +e
error=$("$tmp/zgctl" preset load "$tmp/supported.zgpreset" --dry-run \
        --block 'monitor eq' 2>&1)
status=$?
set -e
[ "$status" -eq 2 ] || fail "block mismatch exited $status, want 2"
grep -q 'does not match' <<<"$error" || fail "block mismatch message missing"

# A malformed file is a parse error.
set +e
error=$("$tmp/zgctl" preset load "$tmp/not-mbdf.zgpreset" --dry-run 2>&1)
status=$?
set -e
[ "$status" -eq 2 ] || fail "parse error exited $status, want 2"
grep -q 'cannot parse' <<<"$error" || fail "parse error message missing"

# A .zg01 backup is rejected before any planning or card access.
set +e
error=$("$tmp/zgctl" preset load "$tmp/backup.zg01" --dry-run 2>&1)
status=$?
set -e
[ "$status" -eq 2 ] || fail "backup load exited $status, want 2"
grep -qi 'backup' <<<"$error" || fail "backup rejection message missing"

# Without a card, the real path builds the plan and fails before writing.
set +e
error=$("$tmp/zgctl" -c no-such-zg01-card preset load \
        "$tmp/supported.zgpreset" 2>&1)
status=$?
set -e
[ "$status" -eq 1 ] || fail "load without a card exited $status, want 1"
grep -q 'no ZG01 card found' <<<"$error" \
    || fail "load without a card message missing"
if grep -q 'applied=' <<<"$error"; then
    fail "load without a card printed a summary"
fi

# Unknown options and a missing --block value are usage errors.
for args in "preset load $tmp/supported.zgpreset --dry-run --bogus" \
            "preset load $tmp/supported.zgpreset --dry-run --block"; do
    set +e
    # shellcheck disable=SC2086
    "$tmp/zgctl" $args >/dev/null 2>&1
    status=$?
    set -e
    [ "$status" -eq 2 ] || fail "usage error '$args' exited $status, want 2"
done

printf 'zgctl preset load --dry-run contract passed\n'
