#!/usr/bin/env bash
# Hardware acceptance test for `zgctl preset load`.
#
# This is the only test that writes the ZG01.  It needs the card and a real
# preset file, both supplied by the machine running the acceptance:
#
#     ZG01_PRESET=/path/to/mic.zgpreset bash tests/test-preset-load.sh
#
# Without a /dev/snd/controlC* node or a zg01 card it skips with a clear
# message and exits 0.  The dry-run path is covered without hardware by
# tests/test-zgctl-load-dry.sh; the conversion helpers are unit-tested in
# tests/test-zgctl-kinds.c.
#
# Before the real preset, a synthetic preset with only a raw row and an
# out-of-range row is loaded for real: every entry is skipped, so the test
# exercises the partial-load summary without writing a control.  `failed>0`
# is not simulated: only a driver rejection (for example a control that
# disappeared between planning and writing) can produce it, and adding a
# test-only control name to the CLI would weaken the shipped code.

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d /tmp/snd-zg01-preset-load.XXXXXX)
trap 'rm -rf "$tmp"' EXIT

skip() {
    printf 'SKIP test-preset-load: %s\n' "$*"
    exit 0
}

fail() {
    printf 'FAIL %s\n' "$*" >&2
    exit 1
}

compgen -G '/dev/snd/controlC*' >/dev/null \
    || skip "no /dev/snd/controlC* control device (is the ZG01 connected?)"
grep -q '\[zg01' /proc/asound/cards \
    || skip "no zg01 ALSA card in /proc/asound/cards"

preset=${ZG01_PRESET:-}

cc -std=c11 -Wall -Wextra -Werror -O2 \
   -o "$tmp/zgctl" \
   "$repo_root/tools/zgctl.c" \
   "$repo_root/tools/zgpreset.c"

# ---------------------------------------------------------- partial load
# A synthetic preset whose entries are all skipped: 0x0f0 is a raw row and
# 0x128 = 400 converts to +22.0 dB, past the +18.0 dB maximum.  Running the
# real load therefore writes no control but must report both skips and exit 1.
python3 - "$tmp/partial.zgpreset" <<'PY'
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


entries = [(0x0F0, 1, 1), (0x128, 2, 400)]
schema = b"\x80\x01\x00\x01\x00\x12" + b"\x41\x01\x00\x80" + b"".join(
    bytes((address >> 8, address & 0xFF, 0x01, width))
    for address, width, _ in entries)
values = b"\x00\x02" + b"Partial".ljust(126, b"\x00") + b"".join(
    value.to_bytes(width, "little") for _, width, value in entries)
info = (b"MIC INPUT".ljust(64, b"\x00") + b"\x00" * 16 + b"\x02"
        + b"Partial" + b"\x00\x00")
out.write_bytes(mbdf([
    chunk("#FILE", "Property", b""),
    chunk("#PRESET ", "General", info),
    chunk("#PRESET ", "ChunkData", ENVELOPE + b"".join((
        component("F_MicEq", schema),
        component("D_MicEq", values, magic=0xD927),
    ))),
    chunk("#END", "", b""),
]))
PY

# Guard the no-write property: if the fixture ever stops being all-skipped,
# fail before the real path can write a control.
set +e
partial_plan=$("$tmp/zgctl" preset load "$tmp/partial.zgpreset" --dry-run 2>&1)
plan_status=$?
set -e
[ "$plan_status" -eq 1 ] \
    || fail "partial dry-run exited $plan_status, want 1"
if grep -q ' (apply)$' <<<"$partial_plan"; then
    fail "the partial fixture would apply a control: $partial_plan"
fi

set +e
partial=$("$tmp/zgctl" preset load "$tmp/partial.zgpreset" 2>&1)
partial_status=$?
set -e
[ "$partial_status" -eq 1 ] \
    || fail "partial load exited $partial_status, want 1"
grep -qx 'applied=0 skipped=2 failed=0' <<<"$partial" \
    || fail "partial load summary missing: $partial"
grep -q '0x0f0 Monitor EQ Band 4 Switch Steps = 1 (raw, skipped)$' <<<"$partial" \
    || fail "partial load did not report the raw skip"
grep -q '0x128 EQ Band 1 Gain = 220 (out_of_range, skipped)$' <<<"$partial" \
    || fail "partial load did not report the out-of-range skip"
if grep -q ' (applied)$' <<<"$partial"; then
    fail "the all-skipped partial preset wrote a control"
fi

# The real-preset acceptance needs a file; the partial-load contract above
# already ran on the card.
[ -n "$preset" ] || skip "partial-load contract passed; ZG01_PRESET is not set"
[ -f "$preset" ] || skip "ZG01_PRESET does not name a file: $preset"

# The plan must be reportable before anything is written.
set +e
plan=$("$tmp/zgctl" preset load "$preset" --dry-run)
plan_status=$?
set -e
[ "$plan_status" -eq 0 ] || [ "$plan_status" -eq 1 ] \
    || fail "dry-run exited $plan_status, want 0 or 1"
grep -Eq '^0x[0-9a-f]{3} .* = -?[0-9]+ \(' <<<"$plan" \
    || fail "dry-run printed no parameter plan"

# Load for real; the summary is the contract for partial loads.
set +e
out=$("$tmp/zgctl" preset load "$preset" 2>&1)
status=$?
set -e

summary=$(grep -E '^applied=[0-9]+ skipped=[0-9]+ failed=[0-9]+$' <<<"$out" \
    | tail -n 1)
[ -n "$summary" ] || fail "load printed no applied/skipped/failed summary"
[ "$(grep -cE '^applied=[0-9]+ skipped=[0-9]+ failed=[0-9]+$' <<<"$out")" \
    -eq 1 ] || fail "load printed more than one summary line"

read -r applied skipped failed < <(
    awk -F'[ =]' '/^applied=/{print $2, $4, $6}' <<<"$summary")

[ "$failed" -eq 0 ] || fail "$summary (the driver rejected a write)"
[ "$applied" -gt 0 ] || fail "$summary (the preset applied nothing)"
grep -q ' (applied)$' <<<"$out" || fail "load reported no applied parameter"

expected=0
[ "$skipped" -eq 0 ] || expected=1
[ "$status" -eq "$expected" ] \
    || fail "load exited $status with $summary, want $expected"

printf 'zgctl preset load hardware contract passed (%s)\n' "$summary"
