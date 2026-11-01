#!/usr/bin/env python3
"""Render the extracted device parameter map as the project's markdown table.

Usage:
    tools/vendor/render-device-map.py tools/vendor/device-map-zg01.json \
        docs/DEVICE_PARAMETER_MAP.md
"""
import argparse
import json

PAGE_TITLES = {
    '1/0': 'Page 0 - microphone chain and game DSP',
    '1/1': 'Page 1 - streaming, HDMI, voice and the four-band EQs',
}


def address(page, offset):
    """The device address of a record, as the two SysEx digits it travels as."""
    return page * 256 + offset


def render(groups):
    lines = []
    for key in sorted(groups):
        records = groups[key]
        kind, page = (int(part) for part in key.split('/'))
        if kind != 1:
            continue
        lines.append(f'### {PAGE_TITLES.get(key, f"kind {kind} page {page}")}')
        lines.append('')
        lines.append('| Address | Digits on the wire (`flag`, `id`) | Name |')
        lines.append('|---|---|---|')
        for record in sorted(records, key=lambda r: r['offset']):
            value = address(page, record['offset'])
            flag, param_id = divmod(value, 128)
            lines.append(f"| `0x{value:03x}` | `0x{flag:02x}`, `0x{param_id:02x}` | "
                         f"`{record['name']}` |")
        lines.append('')
    return '\n'.join(lines)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('map_json')
    ap.add_argument('output')
    args = ap.parse_args()
    data = json.load(open(args.map_json))
    with open(args.output, 'w') as handle:
        handle.write(render(data['groups']))
    print(f'wrote {args.output}')


if __name__ == '__main__':
    main()
