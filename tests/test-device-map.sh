#!/usr/bin/env bash
# Contract test for tools/vendor/device-map-zg01.json.
#
# The map is extracted from the vendor library, so the only trustworthy
# check is that it reproduces what the wire actually showed: every
# address captured from the vendor application must resolve to the
# parameter name the driver documents for it.

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

python3 - "$repo_root" <<'PY'
import json
import sys

root = sys.argv[1]
data = json.load(open(f'{root}/tools/vendor/device-map-zg01.json'))

params = {}
for key, records in data['groups'].items():
    kind, page = (int(part) for part in key.split('/'))
    if kind != 1:
        continue
    for record in records:
        assert (page, record['offset']) not in params, \
            f"duplicate page {page} offset {record['offset']:#x}"
        params[(page, record['offset'])] = record['name']

# the two parameter pages of the ZG01: 256 offsets and 119 offsets
assert len([k for k in params if k[0] == 0]) == 256, 'page 0 is incomplete'
assert len([k for k in params if k[0] == 1]) == 119, 'page 1 is incomplete'

# (flag, id) as captured on the wire -> the parameter the driver must name
CAPTURED = [
    (0x00, 0x02, 'mic_gate_onf'),
    (0x00, 0x03, 'mic_gate_level'),
    (0x00, 0x04, 'mic_gate_thhold'),
    (0x00, 0x05, 'mic_gate_range'),
    (0x00, 0x06, 'mic_gate_attack'),
    (0x00, 0x07, 'mic_gate_hold'),
    (0x00, 0x08, 'mic_gate_decay'),
    (0x00, 0x09, 'mic_comp_onf'),
    (0x00, 0x0a, 'mic_comp_level'),
    (0x00, 0x0b, 'mic_comp_thhold'),
    (0x00, 0x0c, 'mic_comp_ratio'),
    (0x00, 0x11, 'mic_eq_onf'),
    (0x00, 0x21, 'mic_limt_onf'),
    (0x00, 0x22, 'mic_limt_level'),
    (0x00, 0x23, 'mic_limt_thhold'),
    (0x00, 0x27, 'mic_limt_outgain'),
    (0x02, 0x08, 'dirty_flag_mic'),
    (0x02, 0x0b, 'dirty_monitor'),
    (0x02, 0x24, 'mic_eq_v2_1_SHAPE'),
    (0x02, 0x27, 'mic_eq_v2_4_SHAPE'),
    (0x02, 0x28, 'mic_eq_v2_1_G'),
    (0x02, 0x2b, 'mic_eq_v2_4_G'),
    (0x02, 0x2c, 'mic_eq_v2_1_F'),
    (0x02, 0x2f, 'mic_eq_v2_4_F'),
    (0x02, 0x30, 'mic_eq_v2_1_Q'),
    (0x02, 0x33, 'mic_eq_v2_4_Q'),
]

for flag, param_id, expected in CAPTURED:
    # two base-128 digits on the wire, one device address on the inside
    address = flag * 128 + param_id
    page, offset = divmod(address, 256)
    got = params.get((page, offset))
    assert got == expected, (
        f'wire (0x{flag:02x}, 0x{param_id:02x}) is address 0x{address:03x} = '
        f'page {page} offset {offset:#04x}: expected {expected}, map says {got}')

# names must be printable, unique identifiers
for (page, offset), name in params.items():
    assert name.isascii() and name.replace('_', '').isalnum(), name

print(f'{len(CAPTURED)} captured addresses resolve, '
      f'{len(params)} parameters mapped (pages 0 and 1)')
PY

printf 'device map contract passed\n'
