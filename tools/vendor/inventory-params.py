#!/usr/bin/env python3
"""Group a MemAPI parameter dump into the functional blocks of the device.

Takes the JSON produced by extract-memapi-params.py and prints (and
optionally writes) an inventory per subsystem, which is what a feature
parity plan needs: how many controls each screen drives and their ranges.

Also separates the legacy snake_case parameter names (a second naming
layer in the same library, including the read-only status entries such as
`sys_samplingrate` or `meter_mic_output`) from the CamelCase ones.

Usage:
    tools/vendor/inventory-params.py params.json [--markdown docs/PARAMETER_INVENTORY.md]
"""
import argparse
import json
import re

FAMILIES = [
    ('Mic (GATE, COMP, EQ, LIMITER, mute, P48V)', r'^Mic|^DirtyFlagMic'),
    ('MIC EFFECT (voice changer, SE, echo, reverb, tone control, transpose)',
     r'^Voice|^Se[A-Z]|^Echo'),
    ('GAME EFFECT (EQ, de-esser, surround)', r'^Game'),
    ('Monitor / headphone', r'^Mon'),
    ('Binaural (3D chat space)', r'^Binaural'),
    ('Streaming output mixer (USB, HDMI, PC)', r'^Streaming|^Usb|^Hdmi|^ToPc'),
    ('Presets and current selection',
     r'^Preset|^UserPreset|^Current|^Factory|^L\d+$|^R\d+$'),
]

IDENTIFIER = re.compile(r'^[A-Za-z][A-Za-z0-9_]{2,}$')
LEGACY = re.compile(r'^[a-z0-9_]+$')


def collect(params):
    """Return (camel, legacy) name lists, mirrors and noise removed."""
    names = set()
    for name in params:
        base = re.sub(r'_ForBackup$', '', name)
        if IDENTIFIER.match(base):
            names.add(base)
    camel = sorted(n for n in names if not LEGACY.match(n))
    legacy = sorted(n for n in names if LEGACY.match(n))
    return camel, legacy


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('params')
    ap.add_argument('--markdown', default=None)
    args = ap.parse_args()

    params = json.load(open(args.params))
    camel, legacy = collect(params)

    groups = {}
    for label, pattern in FAMILIES:
        groups[label] = [n for n in camel if re.search(pattern, n)]
    assigned = {n for members in groups.values() for n in members}
    groups['Other controls'] = [n for n in camel if n not in assigned]

    total = len(camel) + len(legacy)
    print(f'{total} parameters ({len(camel)} CamelCase, {len(legacy)} legacy snake_case)')
    for label, _ in FAMILIES + [('Other controls', None)]:
        print(f'  {len(groups[label]):4}  {label}')
    print()
    print('legacy snake_case names:', ', '.join(legacy))

    if args.markdown:
        def entry(name):
            p = params.get(name) or params.get(name + '_ForBackup')
            return (f'| `{name}` | {p["min"]} | {p["max"]} | {p["default"]} |')

        with open(args.markdown, 'w') as fh:
            fh.write('# ZG01 parameter inventory\n\n')
            fh.write('Every parameter the vendor application can drive, grouped by\n'
                     'screen, extracted from the MemAPI library shipped in *TOOLS for\n'
                     'ZG* (see `docs/VENDOR_PACKAGE.md` for the procedure and the\n'
                     'caveats).  `_ForBackup` mirrors are omitted; they duplicate the\n'
                     'same parameter for the persistence layer.\n\n')
            fh.write(f'Total: **{total}** parameters '
                     f'({len(camel)} CamelCase, {len(legacy)} legacy snake_case).\n\n')
            for label, _ in list(FAMILIES) + [('Other controls', None)]:
                members = groups[label]
                if not members:
                    continue
                fh.write(f'## {label} ({len(members)})\n\n')
                fh.write('| Parameter | Min | Max | Default |\n|---|---|---|---|\n')
                for name in members:
                    fh.write(entry(name) + '\n')
                fh.write('\n')
            if legacy:
                fh.write(f'## Legacy snake_case names ({len(legacy)})\n\n')
                fh.write('A second naming layer present in the same library.  Several\n'
                         'of these are read-only status entries rather than settings,\n'
                         'for example `sys_samplingrate`, `inpt_is_anlg`,\n'
                         '`outpt_is_anlg`, `sys_hs_loopback`, `meter_mic_output`.\n\n')
                fh.write('| Parameter | Min | Max | Default |\n|---|---|---|---|\n')
                for name in legacy:
                    fh.write(entry(name) + '\n')
                fh.write('\n')
        print(f'markdown written to {args.markdown}')


if __name__ == '__main__':
    main()
