#!/usr/bin/env bash
# Contract test for tools/param-frames.py.
#
# Builds synthetic JSONL captures that carry the mixer writes documented in
# docs/MIC_CONTROL_PROTOCOL.md (USB-MIDI packets around a Yamaha SysEx
# parameter change), then checks that the tool extracts the address and
# value, reports what changed between two captures, and labels the change
# against a vendor parameter model.

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

python3 - "$work" <<'PY'
import json
import sys

out = sys.argv[1]


def digits(value):
    return [(value >> 14) & 0x7f, (value >> 7) & 0x7f, value & 0x7f]


def write_frame(space, param_id, value):
    """USB-MIDI packets of one parameter write (see the protocol doc).

    Eighteen data bytes before F7: f0 43 10 | 3e 14 01 | 01 00 <space>
    | <id> 00 00 | 00 00 00 | 00 <d0> <d1> <d2> ... with the value as
    three base-128 digits in the last three bytes.
    """
    body = [0xf0, 0x43, 0x10, 0x3e, 0x14, 0x01,
            0x01, 0x00, space, param_id] + [0x00] * 5 + digits(value)
    assert len(body) == 18, len(body)
    packets = []
    for i in range(0, 18, 3):
        packets.append([0x04] + body[i:i + 3])
    packets.append([0x05, 0xf7, 0x00, 0x00])
    payload = bytes(b for packet in packets for b in packet)
    return payload + b'\x00' * (512 - len(payload))


def transfer(data, index):
    return json.dumps({'endpoint': '0x03', 'direction': 'out',
                       'actual_length': len(data), 'data': data.hex(),
                       'time': index}, sort_keys=True)


# Baseline: EQ band-1 gain at 0 dB (id 0x28, space 0x02), band-1 frequency
# at 1 kHz (id 0x2c) and a gate level of 50 (id 0x03, space 0x00).  The
# "after" capture only moves the gain to +18 dB; everything else is
# rewritten unchanged, exactly as the application does.
GATE = (0x00, 0x03, 50)
FREQ = (0x02, 0x2c, 1000)
GAIN_BEFORE = (0x02, 0x28, 180)
GAIN_AFTER = (0x02, 0x28, 360)

with open(f'{out}/before.jsonl', 'w') as fh:
    for i, spec in enumerate((GAIN_BEFORE, FREQ, GATE)):
        fh.write(transfer(write_frame(*spec), i) + '\n')
with open(f'{out}/after.jsonl', 'w') as fh:
    for i, spec in enumerate((GAIN_AFTER, FREQ, GATE)):
        fh.write(transfer(write_frame(*spec), i) + '\n')

with open(f'{out}/params.json', 'w') as fh:
    json.dump({
        'MicEqV21G': {'min': 0, 'max': 360, 'default': 180},
        'MicEqV21F': {'min': 20, 'max': 1000, 'default': 100},
        'MicGateLevel': {'min': 0, 'max': 100, 'default': 50},
    }, fh)
PY

parse_out=$(python3 "$repo_root/tools/param-frames.py" parse "$work/before.jsonl" 2>/dev/null)
printf '%s\n' "$parse_out"
grep -q 'address=0x12c (flag=0x02 id=0x2c) value=1000' <<<"$parse_out"
grep -q 'address=0x003 (flag=0x00 id=0x03) value=50' <<<"$parse_out"
grep -q 'address=0x128 (flag=0x02 id=0x28) value=180' <<<"$parse_out"

diff_out=$(python3 "$repo_root/tools/param-frames.py" diff \
             "$work/before.jsonl" "$work/after.jsonl" \
             --label "$work/params.json" 2>/dev/null)
printf '%s\n' "$diff_out"
grep -q 'address=0x128 180 -> 360' <<<"$diff_out"
grep -q 'MicEqV21G (0..360)' <<<"$diff_out"

changed=$(grep -c '^address=' <<<"$diff_out")
if [ "$changed" != "1" ]; then
  printf 'expected exactly one changed address, got %s\n' "$changed" >&2
  exit 1
fi

printf 'param-frames contract passed\n'
