#!/usr/bin/env bash
# Contract test for tools/vendor/control-map-zg01.json.
#
# The control map is the single source of truth for the parameter surface:
# every row names a device address, a control name and where its bounds and
# default came from.  This test pins the initial import - the 208 controls
# that already exist in src/zg01_control.c - and the read-only exclusions,
# and checks the schema the later layers fill in.

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

python3 - "$repo_root" <<'PY'
import json
import re
import sys

root = sys.argv[1]

MAP = f'{root}/tools/vendor/control-map-zg01.json'
MODEL = f'{root}/tools/vendor/parameter-model-zg01.json'
DEVICES = f'{root}/tools/vendor/device-map-zg01.json'
NAMES = f'{root}/tools/vendor/name-map-zg01.json'
CONTROLS = f'{root}/src/zg01_control.c'
ADDRESSES = f'{root}/tests/test-control-addresses.sh'

VALUE_MAX = 0x1fffff
READ_ONLY = {
    'dirty_flag_mic', 'dirty_mic_fx', 'dirty_game_fx', 'dirty_monitor',
    'dirty_streming', 'mic_level', 'mic_monitor_level_knob', 'mon_level',
}
FIELDS = {
    'address', 'device_name', 'vendor_name', 'control_name', 'type',
    'dev_min', 'dev_max', 'dev_default', 'min', 'max', 'def', 'conv',
    'writable', 'provenance', 'raw_reason',
}
PROVENANCE = {'existing', 'vendor-model', 'raw', 'read-only'}
CONVERSIONS = {'id', 'gain', 'freq', 'q'}
TYPES = {'boolean', 'integer'}

try:
    with open(MAP) as handle:
        rows = json.load(handle)
except FileNotFoundError:
    sys.exit(f'missing {MAP}')

with open(MODEL) as handle:
    model = json.load(handle)
with open(DEVICES) as handle:
    devices = json.load(handle)
with open(NAMES) as handle:
    name_map = json.load(handle)


# ---------------------------------------------------------------- helpers
def plain_int(text):
    """An integer literal, tolerating parentheses and u/l suffixes."""
    token = text.strip()
    while token.startswith('(') and token.endswith(')'):
        token = token[1:-1].strip()
    token = re.sub(r'(?i)[ul]+$', '', token)
    if re.fullmatch(r'[+-]?(0x[0-9a-fA-F]+|\d+)', token):
        return int(token, 0)
    return None


def definitions():
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


def control_rows():
    """The manual control table, resolved to plain integers."""
    defs = definitions()
    addr_re = re.compile(r'ZG01_ADDR\((\d+),\s*(0x[0-9a-fA-F]+|\d+)\)')
    page_size = defs['ZG01_PARAM_PAGE_SIZE']
    addresses = {}
    with open(f'{root}/src/zg01.h') as handle:
        source = handle.read()
    for match in re.finditer(
            r'#define\s+(ZG01_PARAM_\w+)(\(n\))?\s+(.*)', source):
        name, is_fn, value = match.group(1), match.group(2), match.group(3)
        if is_fn:
            continue
        addr = addr_re.search(value)
        if addr:
            addresses[name] = (int(addr.group(1)) * page_size
                               + int(addr.group(2), 0))
    indexed = re.compile(
        r'ZG01_ADDR\(\s*(\d+),\s*(0x[0-9a-fA-F]+)\s*\+\s*\(n\)\s*\)')
    for match in re.finditer(
            r'#define\s+(ZG01_PARAM_\w+)\(n\)\s+(.*)', source):
        addr = indexed.search(match.group(2))
        if addr:
            addresses[match.group(1)] = (int(addr.group(1)) * page_size
                                         + int(addr.group(2), 0))

    with open(CONTROLS) as handle:
        source = handle.read()
    table = source.split('zg01_param_ctls[] = {', 1)[1].split('\n};', 1)[0]
    rows = {}
    row_re = re.compile(r'\{\s*"([^"]+)",\s*(.*?)\},', re.S)
    for name, body in row_re.findall(table):
        body = ' '.join(body.split())
        addr_match = re.match(r'(ZG01_PARAM_\w+)(?:\((\d)\))?', body)
        type_match = re.search(r'SNDRV_CTL_ELEM_TYPE_(\w+)', body)
        assert addr_match and type_match, (name, body)
        macro = addr_match.group(1)
        address = addresses[macro]
        if addr_match.group(2) is not None:
            address += int(addr_match.group(2))
        tail = body[type_match.end():]
        fields = [f.strip() for f in tail.split(',') if f.strip()]
        assert len(fields) >= 4, (name, body)

        def resolve(token):
            value = plain_int(token)
            if value is not None:
                return value
            if token in defs:
                return defs[token]
            raise AssertionError(f'cannot resolve {token!r} in {name!r}')

        rows[name] = {
            'address': address,
            'type': type_match.group(1).lower(),
            'min': resolve(fields[0]),
            'max': resolve(fields[1]),
            'conv': re.search(r'CONV_(\w+)', fields[2]).group(1).lower(),
            'def': resolve(fields[3]),
        }
    return rows


def device_addresses():
    """device name -> address, and address -> device name for page-1 rows."""
    by_name = {}
    by_address = {}
    for key, records in devices['groups'].items():
        kind, page = (int(part) for part in key.split('/'))
        if kind != 1:
            continue
        for record in records:
            address = page * 256 + record['offset']
            name = record['name']
            assert name not in by_name, f'duplicate device name {name!r}'
            by_name[name] = address
            by_address[address] = name
    return by_name, by_address


def expected_device_names():
    """control name -> device name, from the existing control contract."""
    with open(ADDRESSES) as handle:
        source = handle.read()
    block = source.split('EXPECTED = {', 1)[1].split('\n}', 1)[0]
    found = {}
    pattern = re.compile(
        r"'([^']+)':\s*(?:'([^']+)'|\('([^']+)',\s*-?\d+,\s*-?\d+,\s*-?\d+\))")
    for control, direct, legacy in pattern.findall(block):
        found[control] = direct or legacy
    return found


# ------------------------------------------------------- Q conversion
# Port of zg01_q_dev_to_user() from src/zg01_codec.h.  The test needs the
# same integer result the driver reports, not a floating-point approximation.
SQRT2_Q31 = (
    3037000500, 2553695365, 2341987720, 2242581548,
    2194578198, 2171188663, 2159253276, 2153311139,
)
Q_USER_MIN = 50
Q_USER_MAX = 1600
Q_DEV_MAX = 60


def log2_fp(x):
    if x == 0:
        return 0
    e = 0
    y = x
    while y >= 2:
        y >>= 1
        e += 1
    if e == 0:
        return 0
    m = x << (31 - e)
    frac = 0
    for _ in range(9):
        sq = m * m
        frac <<= 1
        if sq >= (1 << 63):
            frac |= 1
            m = sq >> 32
        else:
            m = sq >> 31
    frac = (frac + 1) >> 1
    return e * 256 + frac


def exp2_fp(v):
    i = v >> 8
    f = v & 0xff
    r = 1 << 31
    for k in range(8):
        if f & (1 << (7 - k)):
            r = (r * SQRT2_Q31[k]) >> 31
    return (r >> 23) << i


def q_user_to_dev(q):
    q = min(max(q, Q_USER_MIN), Q_USER_MAX)
    return (12 * (log2_fp(q) - log2_fp(Q_USER_MIN))) // 256


def q_dev_to_user(dev):
    dev = min(dev, Q_DEV_MAX)
    lo, hi = Q_USER_MIN, Q_USER_MAX
    while lo < hi:
        mid = lo + (hi - lo) // 2
        if q_user_to_dev(mid) < dev:
            lo = mid + 1
        else:
            hi = mid
    lower = lo
    if q_user_to_dev(lower) != dev:
        return lower
    lo, hi = Q_USER_MIN, Q_USER_MAX
    while lo < hi:
        mid = lo + (hi - lo + 1) // 2
        if q_user_to_dev(mid) > dev:
            hi = mid - 1
        else:
            lo = mid
    upper = lo
    v = (dev * 256) // 12
    physical = (50 * exp2_fp(v) + 128) >> 8
    return min(max(physical, lower), upper)


def dev_to_user(conv, dev):
    if conv == 'gain':
        return dev - 180
    if conv == 'q':
        return q_dev_to_user(dev)
    return dev


# ----------------------------------------------------------- schema
assert isinstance(rows, list), 'control map must be a JSON array'
assert rows, 'control map is empty'

seen_addresses = set()
seen_names = set()
for row in rows:
    assert isinstance(row, dict), row
    assert set(row) == FIELDS, (
        f"row {row.get('control_name')!r} has fields {sorted(row)}")
    assert type(row['address']) is int, row
    assert isinstance(row['device_name'], str) and row['device_name'], row
    assert row['vendor_name'] is None or (
        isinstance(row['vendor_name'], str) and row['vendor_name']), row
    assert isinstance(row['control_name'], str) and row['control_name'], row
    assert row['type'] in TYPES, row
    if row['type'] == 'boolean':
        assert row['max'] == 1, (
            f"{row['control_name']!r} is boolean but has max {row['max']}")
    for field in ('dev_min', 'dev_max', 'dev_default',
                  'min', 'max', 'def'):
        assert type(row[field]) is int, (row['control_name'], field)
    assert row['conv'] in CONVERSIONS, row
    assert type(row['writable']) is bool, row
    assert row['provenance'] in PROVENANCE, (
        f"{row['control_name']!r} has provenance {row['provenance']!r}")
    assert row['raw_reason'] is None or isinstance(row['raw_reason'], str)
    assert row['address'] not in seen_addresses, (
        f"duplicate address {row['address']:#05x}")
    seen_addresses.add(row['address'])
    assert row['control_name'] not in seen_names, (
        f"duplicate control name {row['control_name']!r}")
    seen_names.add(row['control_name'])
    assert len(row['control_name'].encode()) <= 44, (
        f"{row['control_name']!r} is longer than 44 bytes")
    if row['provenance'] == 'read-only':
        assert row['writable'] is False, row['control_name']
    else:
        assert row['writable'] is True, row['control_name']


# -------------------------------------------------------- device names
by_device, by_address = device_addresses()
for row in rows:
    assert row['device_name'] in by_device, (
        f"{row['control_name']!r} names unknown device {row['device_name']!r}")
    assert by_device[row['device_name']] == row['address'], (
        f"{row['device_name']} lives at {by_device[row['device_name']]:#05x}, "
        f"map says {row['address']:#05x}")
    assert by_address[row['address']] == row['device_name']

# ------------------------------------------------------ completeness
DEVICE_COUNT = 375
assert len(by_address) == DEVICE_COUNT, (
    f'device map has {len(by_address)} parameters, expected {DEVICE_COUNT}')
assert len(rows) == DEVICE_COUNT, (
    f'control map has {len(rows)} rows, expected {DEVICE_COUNT}')
assert seen_addresses == set(by_address), (
    f'missing addresses {sorted(set(by_address) - seen_addresses)}, '
    f'unknown addresses {sorted(seen_addresses - set(by_address))}')

counts = {provenance: 0 for provenance in PROVENANCE}
for row in rows:
    counts[row['provenance']] += 1
assert sum(counts.values()) == DEVICE_COUNT, counts
assert counts == {
    'existing': 208,
    'vendor-model': 106,
    'raw': 53,
    'read-only': len(READ_ONLY),
}, counts


# ------------------------------------------------------- name map
assert isinstance(name_map, dict), 'name map must be a JSON object'
for device, vendor in name_map.items():
    assert device in by_device, f'name map has unknown device {device!r}'
    assert vendor in model, f'name map has unknown vendor name {vendor!r}'
for row in rows:
    if row['vendor_name'] is not None:
        assert row['vendor_name'] in model, (
            f"{row['control_name']!r} names unknown vendor row "
            f"{row['vendor_name']!r}")
        assert name_map.get(row['device_name']) == row['vendor_name'], (
            f"{row['device_name']!r} is not mapped to "
            f"{row['vendor_name']!r} in the name map")


# ---------------------------------------------------- existing 208 rows
controls = control_rows()
expected = expected_device_names()
assert len(controls) == 208, f'expected 208 manual controls, found {len(controls)}'
assert len(expected) == 208, f'expected 208 control names, found {len(expected)}'

existing = {row['control_name']: row for row in rows
            if row['provenance'] == 'existing'}
assert len(existing) == 208, (
    f'expected 208 existing rows, found {len(existing)}')
assert set(existing) == set(controls), (
    f"existing rows differ from the control table: "
    f"missing {sorted(set(controls) - set(existing))}, "
    f"extra {sorted(set(existing) - set(controls))}")

for name, control in controls.items():
    row = existing[name]
    assert row['device_name'] == expected[name], (
        f'{name}: device {row["device_name"]!r}, expected {expected[name]!r}')
    assert row['address'] == control['address'], (
        f'{name}: address {row["address"]:#05x}, '
        f'table says {control["address"]:#05x}')
    assert row['type'] == control['type'], name
    assert row['min'] == control['min'], name
    assert row['max'] == control['max'], name
    assert row['conv'] == control['conv'], name
    assert row['def'] == control['def'], name


# -------------------------------------------------- read-only exclusions
read_only = {row['device_name'] for row in rows
             if row['provenance'] == 'read-only'}
assert read_only == READ_ONLY, (
    f'read-only rows are {sorted(read_only)}, expected {sorted(READ_ONLY)}')
for row in rows:
    if row['provenance'] == 'read-only':
        assert row['writable'] is False, row['device_name']


# -------------------------------- legacy monitor EQ bands are raw steps
# Only the page-0 EQ bands are the legacy 1/12-octave block. The monitor
# select/level/name rows have vendor model rows and stay vendor-model.
legacy_monitor = {row['device_name']: row for row in rows
                  if 0x0f0 <= row['address'] <= 0x101}
assert len(legacy_monitor) == 18, sorted(legacy_monitor)
for name, row in legacy_monitor.items():
    assert row['provenance'] == 'raw', name
    assert row['conv'] == 'id', name
    assert row['vendor_name'] is None, name
    assert name not in name_map, name
assert legacy_monitor['mon_eq_hi_f']['provenance'] == 'raw'
assert legacy_monitor['mon_eq_hi_f']['conv'] == 'id'
assert legacy_monitor['mon_eq_hi_f']['min'] <= 112 <= (
    legacy_monitor['mon_eq_hi_f']['max'])

by_device_name = {row['device_name']: row for row in rows}
monitor_vendors = {
    'mon_select': 'MonitorSelect',
    'mon_hp_vol': 'MonitorHpVolume',
    'mon_sp_vol': 'MonitorSpVolume',
    'mon_vic_lvl': 'MonitorVoiceLevel',
    'mon_gam_lvl': 'MonitorGameLevel',
    'mon_eq_onf': 'MonitorEqOnf',
    'mon_eq_name': 'MonitorEqName',
}
for device, vendor in monitor_vendors.items():
    row = by_device_name[device]
    entry = model[vendor]
    assert row['provenance'] == 'vendor-model', device
    assert row['vendor_name'] == vendor, device
    assert (row['dev_min'], row['dev_max'], row['dev_default']) == (
        entry['min'], entry['max'], entry['default']), device
    assert (row['min'], row['max'], row['def']) == (
        entry['min'], entry['max'], entry['default']), device
    assert row['conv'] == 'id', device
    assert name_map[device] == vendor, device

monitor_v2 = {row['device_name']: row for row in rows
              if row['device_name'].startswith('monitor_eq_v2_')}
assert monitor_v2['monitor_eq_v2_1_F']['provenance'] == 'vendor-model'
assert monitor_v2['monitor_eq_v2_1_F']['conv'] == 'freq'
assert name_map['monitor_eq_v2_1_F'] == 'MonitorEqV21F'

# Two parameters have no base descriptor row, only a _ForBackup mirror.  The
# model publishes the mirror under the base name, so both are vendor-backed.
# Game FX Type is already a manual (existing) control and keeps that
# provenance; Streaming Mic Level moves from raw to vendor-model.
mirror_rows = {
    'gam_fx_type': ('GameFxType', 'existing'),
    'strm_mic_lvl': ('StreamingMicLevel', 'vendor-model'),
}
for device, (vendor, provenance) in mirror_rows.items():
    row = by_device_name[device]
    entry = model[vendor]
    assert row['vendor_name'] == vendor, device
    assert row['provenance'] == provenance, device
    assert (row['dev_min'], row['dev_max'], row['dev_default']) == (
        entry['min'], entry['max'], entry['default']), device
    assert (row['min'], row['max'], row['def']) == (
        entry['min'], entry['max'], entry['default']), device
    assert row['conv'] == 'id', device
    assert name_map[device] == vendor, device
assert by_device_name['gam_fx_type']['max'] == 100
assert by_device_name['strm_mic_lvl']['max'] == 127


# -------------------------------------------------------- layer rules
for row in rows:
    provenance = row['provenance']
    if provenance == 'raw':
        assert row['min'] == 0, row['control_name']
        assert row['max'] == VALUE_MAX, row['control_name']
        assert row['def'] == 0, row['control_name']
        assert row['conv'] == 'id', row['control_name']
        assert row['raw_reason'], row['control_name']
        assert row['vendor_name'] is None, row['control_name']
        assert row['type'] == 'integer', row['control_name']
        assert row['writable'] is True, row['control_name']
    elif provenance == 'vendor-model':
        assert row['vendor_name'] is not None, row['control_name']
        entry = model[row['vendor_name']]
        assert (row['dev_min'], row['dev_max'], row['dev_default']) == (
            entry['min'], entry['max'], entry['default']), row['control_name']
        assert (row['min'], row['max']) == (
            dev_to_user(row['conv'], entry['min']),
            dev_to_user(row['conv'], entry['max'])), row['control_name']
        assert row['def'] == entry['default'], row['control_name']

# New controls name their unit in the suffix: converted rows end in Gain,
# Frequency or Q, and rows published as device steps end in Steps.
UNIT_SUFFIX = {'gain': 'Gain', 'freq': 'Frequency', 'q': 'Q'}
for row in rows:
    if row['provenance'] not in ('vendor-model', 'raw'):
        continue
    if row['conv'] == 'id':
        assert row['control_name'].endswith(' Steps'), row['control_name']
    else:
        assert row['control_name'].endswith(
            ' ' + UNIT_SUFFIX[row['conv']]), row['control_name']

# The existing rows carry the vendor model's device values where the
# correspondence is known.  Checking the conversion keeps the seed honest.
for row in rows:
    if row['provenance'] != 'existing' or row['vendor_name'] is None:
        continue
    entry = model[row['vendor_name']]
    assert (row['dev_min'], row['dev_max'], row['dev_default']) == (
        entry['min'], entry['max'], entry['default']), row['control_name']
    assert (row['min'], row['max']) == (
        dev_to_user(row['conv'], entry['min']),
        dev_to_user(row['conv'], entry['max'])), row['control_name']

# The two action controls are not parameters.
assert 'Save to ZG01' not in seen_names
assert 'Reset to ZG01' not in seen_names

print(f'{len(rows)} rows: {counts["existing"]} existing, '
      f'{counts["vendor-model"]} vendor-model, {counts["raw"]} raw, '
      f'{counts["read-only"]} read-only, schema valid')
PY

python3 "$repo_root/tools/vendor/build-control-map.py"

# The seed path must not overwrite a map with hand-maintained rows.
if python3 "$repo_root/tools/vendor/build-control-map.py" --seed >/dev/null 2>&1; then
    echo 'build-control-map.py --seed overwrote the maintained control map' >&2
    exit 1
fi

printf 'control map contract passed\n'
