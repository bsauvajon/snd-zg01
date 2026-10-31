#!/usr/bin/env bash
# Contract test for the generated control tables.
#
# tools/vendor/render-controls.py is the only writer of
# src/zg01_controls_generated.h and tools/zg01_controls_generated.h.  This
# test renders both into a temporary directory and compares them byte for
# byte with the committed headers, then checks the counts the two consumers
# rely on: every one of the 375 addresses is in the userspace table, the
# kernel table holds the 159 writable extras that are not already in
# zg01_param_ctls[], and the writable surface is 208 + 159 = 367 controls.

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d /tmp/snd-zg01-generated.XXXXXX)
trap 'rm -rf "$tmp"' EXIT

python3 "$repo_root/tools/vendor/render-controls.py" \
    "$repo_root/tools/vendor/control-map-zg01.json" \
    "$tmp/zg01_controls_generated.h" \
    "$tmp/zg01_user_controls_generated.h" >/dev/null

cmp "$tmp/zg01_controls_generated.h" \
    "$repo_root/src/zg01_controls_generated.h"
cmp "$tmp/zg01_user_controls_generated.h" \
    "$repo_root/tools/zg01_controls_generated.h"

python3 - "$repo_root" <<'PY'
import json
import re
import sys

root = sys.argv[1]

with open(f'{root}/tools/vendor/control-map-zg01.json') as handle:
    rows = json.load(handle)

by_address = {row['address']: row for row in rows}
writable = [row for row in rows if row['writable']]
extras = [row for row in rows
          if row['writable'] and row['provenance'] != 'existing']

# -------------------------------------------------------------- manual rows
with open(f'{root}/src/zg01_control.c') as handle:
    source = handle.read()
table = source.split('zg01_param_ctls[] = {', 1)[1].split('\n};', 1)[0]
manual_names = re.findall(r'\{\s*"([^"]+)",', table)
assert len(manual_names) == 208, len(manual_names)
assert len(set(manual_names)) == 208, 'duplicate manual control name'

# -------------------------------------------------------------- kernel rows
with open(f'{root}/src/zg01_controls_generated.h') as handle:
    kernel_source = handle.read()

KERNEL_ROW = re.compile(
    r'^\s*\{\s*"([^"]+)",\s*(\d+),\s*'
    r'SNDRV_CTL_ELEM_TYPE_(\w+),\s*(-?\d+),\s*(-?\d+),\s*'
    r'CONV_(\w+),\s*(\d+)\s*\},$', re.M)
kernel_rows = [
    {'name': name, 'address': int(address), 'type': kind.lower(),
     'min': int(lo), 'max': int(hi), 'conv': conv.lower(), 'def': int(default)}
    for name, address, kind, lo, hi, conv, default
    in KERNEL_ROW.findall(kernel_source)
]
assert len(kernel_rows) == len(extras) == 159, len(kernel_rows)
assert 'zg01_extra_param_ctls[]' in kernel_source
assert ('#define ZG01_EXTRA_PARAM_MAX_ADDR '
        f'{max(row["address"] for row in extras)}') in kernel_source

# ----------------------------------------------------------- userspace rows
with open(f'{root}/tools/zg01_controls_generated.h') as handle:
    user_source = handle.read()

USER_ROW = re.compile(
    r'^\s*\{\s*(\d+),\s*"([^"]+)",\s*(-?\d+),\s*(-?\d+),\s*'
    r'CONV_(\w+),\s*(\d+),\s*([^}]+?)\s*\},$', re.M)
user_rows = [
    {'address': int(address), 'name': name, 'min': int(lo), 'max': int(hi),
     'conv': conv.lower(), 'def': int(default),
     'flags': {flag.strip() for flag in flags.split('|')}}
    for address, name, lo, hi, conv, default, flags
    in USER_ROW.findall(user_source)
]
assert len(user_rows) == 375, len(user_rows)
assert 'zg01_user_controls[]' in user_source

# ------------------------------------------------------------- consistency
def expected_flags(row):
    if row['provenance'] == 'raw':
        return {'ZG01_CTL_RAW'}
    if row['provenance'] == 'read-only':
        return {'ZG01_CTL_READONLY'}
    return {'0'}


def check_row(actual, expected):
    for field, key in (('name', 'control_name'), ('address', 'address'),
                       ('min', 'min'), ('max', 'max'), ('conv', 'conv'),
                       ('def', 'def')):
        assert actual[field] == expected[key], (
            f'{expected["control_name"]!r}: {field} is {actual[field]!r}, '
            f'expected {expected[key]!r}')
    if 'type' in actual:
        assert actual['type'] == expected['type'], expected['control_name']
    if 'flags' in actual:
        assert actual['flags'] == expected_flags(expected), \
            expected['control_name']


assert len({row['address'] for row in user_rows}) == 375
assert {row['address'] for row in user_rows} == set(by_address)
for actual in user_rows:
    check_row(actual, by_address[actual['address']])

assert len({row['address'] for row in kernel_rows}) == 159
assert {row['address'] for row in kernel_rows} == {
    row['address'] for row in extras}
for actual in kernel_rows:
    check_row(actual, by_address[actual['address']])

kernel_names = {row['name'] for row in kernel_rows}
assert not kernel_names & set(manual_names), (
    f'generated names already in zg01_param_ctls[]: '
    f'{sorted(kernel_names & set(manual_names))}')
assert len(writable) == 367, len(writable)
assert len(manual_names) + len(kernel_rows) == 367
assert len(user_rows) == 375
assert sum(1 for row in rows if row['provenance'] == 'read-only') == 8

print(f'{len(user_rows)} user controls, {len(kernel_rows)} kernel extras, '
      f'{len(writable)} writable: all match the control map')
PY

printf 'generated control headers match the control map\n'
