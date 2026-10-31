#!/usr/bin/env python3
"""Build the committed ZG01 parameter model from a Yamaha MemAPI library.

`extract-memapi-params.py` dumps every descriptor in MemAPI_ZG01.dll,
including `_ForBackup` persistence mirrors, the legacy snake_case naming
layer, and a handful of preset labels and display strings that happen to
share the descriptor layout.  Only the mixer parameters belong in the
model, so this script keeps rows where:

  * the name is a CamelCase identifier (`^[A-Z][A-Za-z0-9_]*$`);
  * the name is not a `_ForBackup` mirror - except for the two mirrors
    (`GameFxType`, `StreamingMicLevel`) that are the only descriptor row
    for their parameter: there the mirror is published under its base
    name;
  * the bounds are device values: 0 <= min <= default <= max <= 2^21 - 1.

The value limit is ZG01_VALUE_MAX, the largest value the wire encoding
(three base-128 digits) can carry.  Display strings fail it because their
min/max/default are pointer-like integers.

The vendor library is never committed.  The output is a derived artifact:

    tools/vendor/build-parameter-model.py \\
        --dll /path/to/MemAPI_ZG01.dll \\
        --out tools/vendor/parameter-model-zg01.json

Usage:
    tools/vendor/build-parameter-model.py --dll PATH --out PATH
"""

import argparse
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ZG01_VALUE_MAX = 0x1fffff
FOR_BACKUP = '_ForBackup'
CAMEL_CASE = re.compile(r'[A-Z][A-Za-z0-9_]*')
EXTRACTOR = Path(__file__).with_name('extract-memapi-params.py')


def is_device_parameter(name, entry):
    """Return True for a CamelCase mixer parameter with device-value bounds."""
    if not CAMEL_CASE.fullmatch(name):
        return False
    low, default, high = entry['min'], entry['default'], entry['max']
    return 0 <= low <= default <= high <= ZG01_VALUE_MAX


def extract(dll):
    """Run the shared extractor and return its raw parameter dump."""
    with tempfile.TemporaryDirectory() as tmp:
        raw_path = Path(tmp) / 'memapi-params.json'
        result = subprocess.run(
            [sys.executable, str(EXTRACTOR), str(dll), str(raw_path)],
            capture_output=True, text=True)
        if result.returncode != 0:
            raise SystemExit(
                f'{EXTRACTOR.name} failed on {dll}:\n{result.stderr.strip()}')
        with raw_path.open() as handle:
            return json.load(handle)


def model_row(entry):
    return {
        'min': entry['min'],
        'max': entry['max'],
        'default': entry['default'],
    }


def build_model(params):
    """Filter the extractor dump down to the device parameter model.

    `_ForBackup` mirrors are normally duplicates and stay out.  A mirror is
    used only when the descriptor table has no base-name row at all, which
    is the case for exactly `GameFxType` and `StreamingMicLevel`; the model
    then exposes it under the base name so the control map can join it.
    """
    model = {
        name: model_row(entry)
        for name, entry in params.items()
        if not name.endswith(FOR_BACKUP)
        and is_device_parameter(name, entry)
    }
    for name, entry in params.items():
        if not name.endswith(FOR_BACKUP):
            continue
        base = name[:-len(FOR_BACKUP)]
        if base in params or base in model:
            continue
        if is_device_parameter(base, entry):
            model[base] = model_row(entry)
    return model


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--dll', required=True,
                        help='MemAPI_ZG01.dll extracted from TOOLS for ZG')
    parser.add_argument('--out', required=True,
                        help='path of the generated parameter-model JSON')
    args = parser.parse_args()

    model = build_model(extract(args.dll))

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open('w') as handle:
        json.dump(model, handle, indent=1, sort_keys=True)
        handle.write('\n')

    print(f'{len(model)} parameters written to {out}')


if __name__ == '__main__':
    main()
