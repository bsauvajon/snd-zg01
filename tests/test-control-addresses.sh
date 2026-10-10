#!/usr/bin/env bash
# Contract test between the driver's control table and the device map.
#
# src/zg01_control.c declares one row per kcontrol, each naming a device
# address (src/zg01.h) with a range and a default.  This test checks that
# every row
#
#   * names an address that exists in tools/vendor/device-map-zg01.json
#     and resolves to the expected device parameter,
#   * carries the vendor model's bounds and default,
#   * is unique: page 0 offset 0x24 and page 1 offset 0x24 are different
#     parameters (LIMITER attack and EQ low shape), so two controls
#     sharing an address would silently share one cache slot.
#
# The expectations below are the vendor model's values, taken from
# MemAPI_ZG01.dll (docs/PARAMETER_INVENTORY.md); the device names are the
# ones the library registers (docs/DEVICE_PARAMETER_MAP.md).

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

python3 - "$repo_root" <<'PY'
import json
import re
import sys

root = sys.argv[1]

# ---------------------------------------------------------------- headers
def plain_int(text):
    """An integer literal, tolerating parentheses and u/l suffixes."""
    token = text.strip()
    while token.startswith('(') and token.endswith(')'):
        token = token[1:-1].strip()
    token = re.sub(r'(?i)[ul]+$', '', token)
    if re.fullmatch(r'[+-]?(0x[0-9a-fA-F]+|\d+)', token):
        return int(token, 0)
    return None


def definitions(path):
    """Integer #defines, resolved through the files that hold them."""
    found = {}
    for name in ('src/zg01.h', 'src/zg01_codec.h'):
        with open(f'{root}/{name}') as handle:
            for line in handle:
                match = re.match(r'#define\s+(\w+)\s+(.+?)\s*(?:/\*.*)?$', line)
                if not match:
                    continue
                value = plain_int(match.group(2))
                if value is not None:
                    found[match.group(1)] = value
    return found


defs = definitions('src/zg01.h')
ADDR = re.compile(r'ZG01_ADDR\((\d+),\s*(0x[0-9a-fA-F]+|\d+)\)')
PAGE_SIZE = defs['ZG01_PARAM_PAGE_SIZE']

addresses = {}
with open(f'{root}/src/zg01.h') as handle:
    for line in handle:
        match = re.match(r'#define\s+(ZG01_PARAM_\w+)(\(n\))?\s+(.*)$', line)
        if not match:
            continue
        name, is_fn, value = match.group(1), match.group(2), match.group(3)
        if is_fn:
            continue
        addr = ADDR.search(value)
        if addr:
            addresses[name] = (int(addr.group(1)) * PAGE_SIZE
                               + int(addr.group(2), 0))

# indexed macros: ZG01_PARAM_EQ_GAIN(n) ZG01_ADDR(1, 0x28 + (n))
indexed = re.compile(r'ZG01_ADDR\(\s*(\d+),\s*(0x[0-9a-fA-F]+)\s*\+\s*\(n\)\s*\)')
with open(f'{root}/src/zg01.h') as handle:
    for line in handle:
        match = re.match(r'#define\s+(ZG01_PARAM_\w+)\(n\)\s+(.*)$', line)
        if not match:
            continue
        addr = indexed.search(match.group(2))
        if addr:
            addresses[match.group(1)] = (int(addr.group(1)) * PAGE_SIZE
                                         + int(addr.group(2), 0))

assert addresses, 'no parameter addresses found in src/zg01.h'

# ------------------------------------------------------------------ table
ROW = re.compile(r'\{\s*"([^"]+)",\s*(.*?)\},', re.S)
with open(f'{root}/src/zg01_control.c') as handle:
    source = handle.read()
table = source.split('zg01_param_ctls[] = {', 1)[1].split('\n};', 1)[0]

rows = []
for name, body in ROW.findall(table):
    body = ' '.join(body.split())
    addr_match = re.match(r'(ZG01_PARAM_\w+)(?:\((\d)\))?', body)
    type_match = re.search(r'SNDRV_CTL_ELEM_TYPE_(\w+)', body)
    tail = body[type_match.end():]
    fields = [f.strip() for f in tail.split(',') if f.strip()]
    assert addr_match and type_match and len(fields) >= 4, (name, body)
    macro = addr_match.group(1)
    if addr_match.group(2) is not None:
        addr = addresses[macro] + int(addr_match.group(2))
    else:
        addr = addresses[macro]
    conv = re.search(r'CONV_(\w+)', fields[2]).group(1)

    def resolve(token):
        value = plain_int(token)
        if value is not None:
            return value
        if token in defs:
            return defs[token]
        raise AssertionError(f'cannot resolve {token!r} in row {name!r}')

    rows.append({'name': name, 'addr': addr, 'type': type_match.group(1),
                 'min': resolve(fields[0]), 'max': resolve(fields[1]),
                 'conv': conv, 'def': resolve(fields[3])})

# ------------------------------------------------------------------- map
data = json.load(open(f'{root}/tools/vendor/device-map-zg01.json'))
params = {}
for key, records in data['groups'].items():
    kind, page = (int(part) for part in key.split('/'))
    if kind != 1:
        continue
    for record in records:
        params[page * 256 + record['offset']] = record['name']

# --------------------------------------------------------- expectations
# control name -> (device parameter, kcontrol min, kcontrol max, raw default)
#
# min/max are what the control advertises (the codec's user units for the
# converted rows, the device steps for CONV_ID rows); def is always the
# raw device value the cache is seeded with.
EXPECTED = {
    'P48V Capture Switch': ('mic_p48v_onf', 0, 1, 0),
    'Mute Capture Switch': ('mic_mute_onf', 0, 1, 0),
    'Gate Capture Switch': ('mic_gate_onf', 0, 1, 0),
    'Gate': ('mic_gate_level', 0, 100, 50),
    'Gate Threshold': ('mic_gate_thhold', 1, 73, 23),
    'Gate Range': ('mic_gate_range', 0, 73, 17),
    'Gate Attack': ('mic_gate_attack', 57, 283, 237),
    'Gate Hold': ('mic_gate_hold', 0, 213, 101),
    'Gate Decay': ('mic_gate_decay', 24, 300, 188),
    'Compressor Capture Switch': ('mic_comp_onf', 0, 1, 0),
    'Compressor': ('mic_comp_level', 0, 100, 50),
    'Compressor Threshold': ('mic_comp_thhold', 19, 73, 52),
    'Compressor Ratio': ('mic_comp_ratio', 0, 120, 14),
    'Compressor Attack': ('mic_comp_attack', 57, 283, 237),
    'Compressor Release': ('mic_comp_release', 24, 300, 228),
    'Compressor Knee': ('mic_comp_knee', 0, 2, 1),
    'Compressor Out Gain': ('mic_comp_outgain', 0, 180, 50),
    'Limiter Capture Switch': ('mic_limt_onf', 0, 1, 1),
    'Limiter': ('mic_limt_level', 0, 100, 50),
    'Limiter Threshold': ('mic_limt_thhold', 19, 73, 65),
    'Limiter Attack': ('mic_limt_attack', 57, 283, 237),
    'Limiter Release': ('mic_limt_release', 24, 300, 228),
    'Limiter Knee': ('mic_limt_knee', 0, 2, 1),
    'Limiter Out Gain': ('mic_limt_outgain', 0, 180, 0),
    'EQ Capture Switch': ('mic_eq_onf', 0, 1, 0),
    'EQ Low Shape': ('mic_eq_v2_1_SHAPE', 0, 1, 1),
    'EQ High Shape': ('mic_eq_v2_4_SHAPE', 0, 1, 1),
    'EQ Band 1 Gain': ('mic_eq_v2_1_G', -180, 180, 180),
    'EQ Band 2 Gain': ('mic_eq_v2_2_G', -180, 180, 180),
    'EQ Band 3 Gain': ('mic_eq_v2_3_G', -180, 180, 180),
    'EQ Band 4 Gain': ('mic_eq_v2_4_G', -180, 180, 180),
    'EQ Band 1 Frequency': ('mic_eq_v2_1_F', 20, 1000, 100),
    'EQ Band 2 Frequency': ('mic_eq_v2_2_F', 20, 20000, 500),
    'EQ Band 3 Frequency': ('mic_eq_v2_3_F', 20, 20000, 2500),
    'EQ Band 4 Frequency': ('mic_eq_v2_4_F', 500, 20000, 10000),
    'EQ Band 1 Q': ('mic_eq_v2_1_Q', 50, 1600, 12),
    'EQ Band 2 Q': ('mic_eq_v2_2_Q', 50, 1600, 12),
    'EQ Band 3 Q': ('mic_eq_v2_3_Q', 50, 1600, 12),
    'EQ Band 4 Q': ('mic_eq_v2_4_Q', 50, 1600, 12),
}

by_name = {row['name']: row for row in rows}
missing = sorted(set(EXPECTED) - set(by_name))
extra = sorted(set(by_name) - set(EXPECTED))
assert not missing, f'controls promised but absent from the table: {missing}'
assert not extra, f'controls absent from the expectations: {extra}'

seen = {}
for name, (device, vmin, vmax, vdef) in EXPECTED.items():
    row = by_name[name]
    expected_addr = next(addr for addr, dev in params.items() if dev == device)
    assert row['addr'] == expected_addr, (
        f'{name}: address 0x{row["addr"]:03x} but {device} lives at '
        f'0x{expected_addr:03x} (docs/DEVICE_PARAMETER_MAP.md)')
    assert (row['min'], row['max'], row['def']) == (vmin, vmax, vdef), (
        f'{name}: min/max/default {row["min"]}/{row["max"]}/{row["def"]} '
        f'but the vendor model says {vmin}/{vmax}/{vdef}')
    assert row['min'] <= row['max'], f'{name}: empty range'
    if row['conv'] == 'ID':
        # same units on both sides: the default must be settable
        assert row['min'] <= row['def'] <= row['max'], (
            f'{name}: default {row["def"]} outside {row["min"]}..{row["max"]}')
    # the cache is seeded with a device value, which the encoder must take
    assert 0 <= row['def'] <= 0x3fffff, f'{name}: default out of range'
    assert row['addr'] not in seen, (
        f'{name} and {seen[row["addr"]]} share address 0x{row["addr"]:03x}: '
        f'they would share one cache slot')
    seen[row['addr']] = name

# the value the driver would put on the wire, decoded back
for name, row in by_name.items():
    flag, param_id = row['addr'] // 128, row['addr'] % 128
    assert 0 <= flag <= 0x7f and 0 <= param_id <= 0x7f

# the frame builders must derive the two digits from the address: a
# hardcoded page byte is how page 0 parameters used to be unreadable
for func in ('zg01_param_write', 'zg01_param_read', 'zg01_param_parse'):
    segment = source.split(f'{func}(', 1)[1].split('\n}\n', 1)[0]
    for macro in ('ZG01_ADDR_FLAG', 'ZG01_ADDR_ID'):
        assert macro in segment, f'{func} does not derive the digits ({macro})'

print(f'{len(rows)} controls: addresses, bounds and defaults match the device map')
PY

printf 'control address contract passed\n'
