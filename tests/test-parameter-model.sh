#!/usr/bin/env bash
# Contract test for tools/vendor/parameter-model-zg01.json.
#
# The model is derived from the vendor MemAPI library and committed so CI
# does not need the DLL.  It must contain exactly the mixer parameters:
# CamelCase names, no persistence mirrors, and device-value bounds within
# what the wire protocol can carry.

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

python3 - "$repo_root" <<'PY'
import json
import re
import sys

root = sys.argv[1]
path = f'{root}/tools/vendor/parameter-model-zg01.json'

try:
    with open(path) as handle:
        model = json.load(handle)
except FileNotFoundError:
    sys.exit(f'missing {path}')

assert isinstance(model, dict), 'parameter model must be a JSON object'

name_re = re.compile(r'[A-Z][A-Za-z0-9_]*')
value_max = 0x1fffff

for name, entry in model.items():
    assert name_re.fullmatch(name), f'{name!r} is not a CamelCase identifier'
    assert not name.endswith('_ForBackup'), f'{name!r} is a persistence mirror'
    assert isinstance(entry, dict), f'{name!r} is not an object'
    assert set(entry) == {'min', 'max', 'default'}, \
        f'{name!r} has unexpected fields: {sorted(entry)}'
    for field in ('min', 'max', 'default'):
        value = entry[field]
        assert type(value) is int, f'{name}.{field} is not an integer: {value!r}'
    assert 0 <= entry['min'] <= entry['default'] <= entry['max'] <= value_max, (
        f"{name!r} has an impossible range: "
        f"min {entry['min']}, default {entry['default']}, max {entry['max']}")

# 331 CamelCase rows in docs/PARAMETER_INVENTORY.md include preset labels
# and display strings (L11, R10, ...), whose min/max/default are pointers,
# not device values.  What remains is the mixer parameter model: 318 base
# rows plus the two parameters whose only descriptor row is a _ForBackup
# mirror (GameFxType and StreamingMicLevel).
assert len(model) == 320, f'expected 320 parameters, found {len(model)}'
assert model['GameFxType'] == {'min': 0, 'max': 100, 'default': 0}
assert model['StreamingMicLevel'] == {'min': 0, 'max': 127, 'default': 0}

print(f'{len(model)} parameters, all ranges valid')
PY

printf 'parameter model contract passed\n'
