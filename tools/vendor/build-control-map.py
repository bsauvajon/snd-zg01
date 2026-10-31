#!/usr/bin/env python3
"""Validate the committed ZG01 control map, or seed its initial import.

`tools/vendor/control-map-zg01.json` is the single source of truth for the
parameter surface.  Each row names one device address, its control name, the
vendor model's device values and the user-facing ALSA bounds.

The map is seeded once from the manual control table in
`src/zg01_control.c` and the read-only exclusions, then maintained by hand.
Seeding is deliberately an explicit mode: the build only validates the
committed map, so a change to the control table cannot silently rewrite it.

Usage:
    tools/vendor/build-control-map.py            # validate the committed map
    tools/vendor/build-control-map.py --seed     # rewrite it from the table

`--seed` is for the initial import and for a reviewed re-import; it is never
run by the build or the test.  It refuses to touch a map that contains
hand-maintained rows, so a check-in with the full 375-row map cannot be
overwritten by accident.
"""

import argparse
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MAP = ROOT / 'tools/vendor/control-map-zg01.json'
MODEL = ROOT / 'tools/vendor/parameter-model-zg01.json'
DEVICES = ROOT / 'tools/vendor/device-map-zg01.json'
NAMES = ROOT / 'tools/vendor/name-map-zg01.json'
CONTROLS = ROOT / 'src/zg01_control.c'
ADDRESSES = ROOT / 'tests/test-control-addresses.sh'

VALUE_MAX = 0x1fffff

# The exclusions are a deliberate, closed list: adding one requires a spec
# change and an update to tests/test-control-map.sh.
READ_ONLY = (
    'dirty_flag_mic',
    'dirty_mic_fx',
    'dirty_game_fx',
    'dirty_monitor',
    'dirty_streming',
    'mic_level',
    'mic_monitor_level_knob',
    'mon_level',
)

# Human names for rows that deliberately have no ALSA control.
READ_ONLY_NAMES = {
    'dirty_flag_mic': 'Dirty Flag Mic Status',
    'dirty_mic_fx': 'Dirty Mic FX Status',
    'dirty_game_fx': 'Dirty Game FX Status',
    'dirty_monitor': 'Dirty Monitor Status',
    'dirty_streming': 'Dirty Streaming Status',
    'mic_level': 'Mic Level Status',
    'mic_monitor_level_knob': 'Mic Monitor Level Knob Status',
    'mon_level': 'Monitor Level Status',
}

FIELDS = (
    'address', 'device_name', 'vendor_name', 'control_name', 'type',
    'dev_min', 'dev_max', 'dev_default', 'min', 'max', 'def', 'conv',
    'writable', 'provenance', 'raw_reason',
)
PROVENANCE = ('existing', 'vendor-model', 'raw', 'read-only')
CONVERSIONS = ('id', 'gain', 'freq', 'q')
TYPES = ('boolean', 'integer')


def load_json(path):
    try:
        with path.open() as handle:
            return json.load(handle)
    except FileNotFoundError:
        raise SystemExit(f'missing {path}')


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
    """Integer #defines from the two headers the control table uses."""
    found = {}
    for name in ('src/zg01.h', 'src/zg01_codec.h'):
        with (ROOT / name).open() as handle:
            for line in handle:
                match = re.match(
                    r'#define\s+(\w+)\s+(.+?)\s*(?:/\*.*)?$', line)
                if not match:
                    continue
                value = plain_int(match.group(2))
                if value is not None:
                    found[match.group(1)] = value
    return found


def address_macros(source, defs):
    """ZG01_PARAM_* macro name -> device address."""
    addr_re = re.compile(r'ZG01_ADDR\((\d+),\s*(0x[0-9a-fA-F]+|\d+)\)')
    page_size = defs['ZG01_PARAM_PAGE_SIZE']
    addresses = {}
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
    return addresses


def control_rows():
    """Parse the manual control table into plain rows."""
    defs = definitions()
    with (ROOT / 'src/zg01.h').open() as handle:
        source = handle.read()
    addresses = address_macros(source, defs)

    with CONTROLS.open() as handle:
        source = handle.read()
    table = source.split('zg01_param_ctls[] = {', 1)[1].split('\n};', 1)[0]

    rows = []
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
        fields = [field.strip() for field in tail.split(',') if field.strip()]
        assert len(fields) >= 4, (name, body)

        def resolve(token):
            value = plain_int(token)
            if value is not None:
                return value
            if token in defs:
                return defs[token]
            raise AssertionError(f'cannot resolve {token!r} in {name!r}')

        rows.append({
            'control_name': name,
            'address': address,
            'type': type_match.group(1).lower(),
            'min': resolve(fields[0]),
            'max': resolve(fields[1]),
            'conv': re.search(r'CONV_(\w+)', fields[2]).group(1).lower(),
            'def': resolve(fields[3]),
        })
    return rows


def expected_devices():
    """control name -> device name, from the existing control contract."""
    with ADDRESSES.open() as handle:
        source = handle.read()
    block = source.split('EXPECTED = {', 1)[1].split('\n}', 1)[0]
    pattern = re.compile(
        r"'([^']+)':\s*\('([^']+)',\s*(-?\d+),\s*(-?\d+),\s*(-?\d+)\)")
    return {control: device
            for control, device, _low, _high, _default
            in pattern.findall(block)}


def device_map():
    """device name -> (address, device map record)."""
    data = load_json(DEVICES)
    by_name = {}
    for key, records in data['groups'].items():
        kind, page = (int(part) for part in key.split('/'))
        if kind != 1:
            continue
        for record in records:
            name = record['name']
            assert name not in by_name, f'duplicate device name {name!r}'
            by_name[name] = (page * 256 + record['offset'], record)
    return by_name


def make_row(address, device, vendor, name, kind, device_values,
             user_values, conv, writable, provenance, raw_reason=None):
    dev_min, dev_max, dev_default = device_values
    low, high, default = user_values
    return {
        'address': address,
        'device_name': device,
        'vendor_name': vendor,
        'control_name': name,
        'type': kind,
        'dev_min': dev_min,
        'dev_max': dev_max,
        'dev_default': dev_default,
        'min': low,
        'max': high,
        'def': default,
        'conv': conv,
        'writable': writable,
        'provenance': provenance,
        'raw_reason': raw_reason,
    }


def seed():
    """Write the initial map from the manual table and the exclusions."""
    if MAP.exists():
        current = load_json(MAP)
        if not isinstance(current, list):
            raise SystemExit(f'{MAP.relative_to(ROOT)} must be a JSON array')
        maintained = [
            row for row in current
            if isinstance(row, dict)
            and row.get('provenance') not in ('existing', 'read-only')
        ]
        if maintained:
            raise SystemExit(
                f'refusing to --seed: {MAP.relative_to(ROOT)} has '
                f'{len(maintained)} hand-maintained row(s) '
                f'(vendor-model or raw); move them aside first if a full '
                f're-seed is really intended')

    controls = control_rows()
    expected = expected_devices()
    names = load_json(NAMES)
    model = load_json(MODEL)
    devices = device_map()

    if len(controls) != len(expected):
        raise SystemExit(
            f'{len(controls)} manual controls but {len(expected)} expected '
            f'names in {ADDRESSES.relative_to(ROOT)}')

    rows = []
    for control in controls:
        name = control['control_name']
        if name not in expected:
            raise SystemExit(f'{name!r} has no device name in the contract')
        device = expected[name]
        if device not in devices:
            raise SystemExit(f'{name!r} names unknown device {device!r}')
        address, _record = devices[device]
        if address != control['address']:
            raise SystemExit(
                f'{name!r}: table address {control["address"]:#05x}, '
                f'device map address {address:#05x}')

        vendor = names.get(device)
        if vendor is None:
            if control['conv'] != 'id':
                raise SystemExit(
                    f'{name!r} is converted but has no vendor model row')
            dev_values = (control['min'], control['max'], control['def'])
        else:
            if vendor not in model:
                raise SystemExit(
                    f'{device!r} maps to unknown vendor row {vendor!r}')
            entry = model[vendor]
            dev_values = (entry['min'], entry['max'], entry['default'])

        rows.append(make_row(
            address, device, vendor, name, control['type'], dev_values,
            (control['min'], control['max'], control['def']),
            control['conv'], True, 'existing'))

    for device in READ_ONLY:
        if device not in devices:
            raise SystemExit(f'read-only exclusion {device!r} is not mapped')
        address, _record = devices[device]
        vendor = names.get(device)
        if vendor is not None:
            if vendor not in model:
                raise SystemExit(
                    f'{device!r} maps to unknown vendor row {vendor!r}')
            entry = model[vendor]
            values = (entry['min'], entry['max'], entry['default'])
            kind = 'boolean' if entry['max'] == 1 else 'integer'
        elif device.startswith('dirty_'):
            values = (0, 1, 0)
            kind = 'boolean'
        else:
            values = (0, 0, 0)
            kind = 'integer'
        rows.append(make_row(
            address, device, vendor, READ_ONLY_NAMES[device], kind, values,
            values, 'id', False, 'read-only'))

    rows.sort(key=lambda row: row['address'])
    with MAP.open('w') as handle:
        json.dump(rows, handle, indent=1)
        handle.write('\n')
    print(f'seeded {len(rows)} rows in {MAP.relative_to(ROOT)}')


def validate():
    """Reject a map that is structurally broken or misclassified."""
    rows = load_json(MAP)
    model = load_json(MODEL)
    names = load_json(NAMES)
    devices = device_map()

    if not isinstance(rows, list):
        raise SystemExit(f'{MAP.relative_to(ROOT)} must be a JSON array')

    addresses = set()
    control_names = set()
    for row in rows:
        if not isinstance(row, dict):
            raise SystemExit(f'row is not an object: {row!r}')
        missing = [field for field in FIELDS if field not in row]
        if missing:
            raise SystemExit(
                f"row {row.get('control_name', row.get('address'))!r} is "
                f"missing {missing}")
        extra = sorted(set(row) - set(FIELDS))
        if extra:
            raise SystemExit(
                f"row {row['control_name']!r} has unknown fields {extra}")

        if type(row['address']) is not int:
            raise SystemExit(f'{row["control_name"]!r}: bad address')
        if row['address'] in addresses:
            raise SystemExit(f'duplicate address {row["address"]:#05x}')
        addresses.add(row['address'])

        name = row['control_name']
        if not isinstance(name, str) or not name:
            raise SystemExit(f'{row["address"]:#05x}: missing control name')
        if name in control_names:
            raise SystemExit(f'duplicate control name {name!r}')
        control_names.add(name)

        if row['provenance'] is None:
            raise SystemExit(f'{name!r}: missing provenance')
        if row['provenance'] not in PROVENANCE:
            raise SystemExit(
                f'{name!r}: unknown provenance {row["provenance"]!r}')
        if row['conv'] not in CONVERSIONS:
            raise SystemExit(f'{name!r}: unknown conversion {row["conv"]!r}')
        if row['type'] not in TYPES:
            raise SystemExit(f'{name!r}: unknown type {row["type"]!r}')

        if row['device_name'] not in devices:
            raise SystemExit(
                f'{name!r}: unknown device {row["device_name"]!r}')
        address, _record = devices[row['device_name']]
        if address != row['address']:
            raise SystemExit(
                f'{name!r}: address {row["address"]:#05x} but '
                f'{row["device_name"]} lives at {address:#05x}')

        vendor = row['vendor_name']
        if vendor is not None:
            if vendor not in model:
                raise SystemExit(f'{name!r}: unknown vendor row {vendor!r}')
            entry = model[vendor]
            if (row['dev_min'], row['dev_max'], row['dev_default']) != (
                    entry['min'], entry['max'], entry['default']):
                raise SystemExit(
                    f'{name!r}: device bounds do not match {vendor}')
            if names.get(row['device_name']) != vendor:
                raise SystemExit(
                    f'{name!r}: {row["device_name"]!r} is not mapped to '
                    f'{vendor!r}')

        if row['provenance'] == 'read-only':
            if row['writable'] is not False:
                raise SystemExit(f'{name!r}: read-only row is writable')
        elif row['writable'] is not True:
            raise SystemExit(f'{name!r}: writable row is not writable')

        if row['provenance'] == 'raw':
            if (row['min'], row['max'], row['def']) != (0, VALUE_MAX, 0):
                raise SystemExit(f'{name!r}: raw bounds are not 0..value max')
            if row['conv'] != 'id':
                raise SystemExit(f'{name!r}: raw row is not CONV_ID')
            if not row['raw_reason']:
                raise SystemExit(f'{name!r}: raw row has no reason')
        if row['provenance'] == 'vendor-model' and vendor is None:
            raise SystemExit(f'{name!r}: vendor-model row has no vendor name')

    print(f'{len(rows)} rows, no duplicate address, name or provenance')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--seed', action='store_true',
                        help='rewrite the map from the manual control table')
    args = parser.parse_args()
    if args.seed:
        seed()
    else:
        validate()


if __name__ == '__main__':
    main()
