#!/usr/bin/env python3
"""Probe a MemAPI dump for the wire address of a parameter.

The captures give the wire id of a few parameters; this script checks the
three places where it could plausibly live in the vendor data, and reports
that none of them holds it - which is why the ids have to come from a
capture or from disassembling the library.

Checks, given a params JSON (extract-memapi-params.py) and the library:

  1. does any descriptor slot equal a captured id?
  2. is the per-parameter key (slot 12, low 16 bits) a CRC or checksum of
     the parameter name?
  3. is the descriptor table ordered by wire address?
  4. does any name have a second pointer in the image (a parallel table)?

Usage: tools/vendor/probe-wire-id.py params.json MemAPI_ZG01.dll
"""
import binascii
import json
import re
import struct
import sys

# Wire ids observed in the ZG01 captures (docs/MIC_CONTROL_PROTOCOL.md).
KNOWN = {
    'MicGateOnf': 0x02, 'MicGateLevel': 0x03,
    'MicCompOnf': 0x09, 'MicCompLevel': 0x0a, 'MicEqOnf': 0x11,
    'MicLimiterOnf': 0x21, 'MicLimiterLevel': 0x22,
    'MicEqV21Shape': 0x24, 'MicEqV24Shape': 0x27,
    'MicEqV21G': 0x28, 'MicEqV22G': 0x29, 'MicEqV23G': 0x2a, 'MicEqV24G': 0x2b,
    'MicEqV21F': 0x2c, 'MicEqV22F': 0x2d, 'MicEqV23F': 0x2e, 'MicEqV24F': 0x2f,
    'MicEqV21Q': 0x30, 'MicEqV22Q': 0x31, 'MicEqV23Q': 0x32, 'MicEqV24Q': 0x33,
}
DESCRIPTOR_SIZE = 56
NAME_FIELD = 40


def crc16_arc(data):
    crc = 0
    for byte in data:
        crc ^= byte
        for _ in range(8):
            crc = (crc >> 1) ^ 0xA001 if crc & 1 else crc >> 1
    return crc


def crc16_ccitt(data, init=0xFFFF):
    crc = init
    for byte in data:
        crc ^= byte << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021) & 0xFFFF if crc & 0x8000 \
                else (crc << 1) & 0xFFFF
    return crc


def fletcher16(data):
    a = b = 0
    for byte in data:
        a = (a + byte) % 255
        b = (b + a) % 255
    return (b << 8) | a


def main():
    params = json.load(open(sys.argv[1]))
    image = open(sys.argv[2], 'rb').read()

    print('1. descriptor slots holding a captured wire id')
    for slot in range(14):
        hits = [n for n, wid in KNOWN.items()
                if params.get(n) and params[n]['slots'][slot] == wid]
        print(f'   slot {slot:2}: {len(hits)} match(es) {hits[:4]}')

    print('2. is the per-parameter key a checksum of the name?')
    for label, fn in (('crc16_arc', crc16_arc), ('ccitt_F', lambda d: crc16_ccitt(d)),
                      ('ccitt_0', lambda d: crc16_ccitt(d, 0)),
                      ('xmodem', lambda d: binascii.crc_hqx(d, 0)),
                      ('fletcher16', fletcher16), ('sum', lambda d: sum(d) & 0xFFFF)):
        hits = sum(1 for n, p in params.items()
                   if fn(n.encode()) == (p['slots'][12] & 0xFFFF))
        print(f'   {label:11} {hits}/{len(params)}')

    print('3. is the table ordered by wire address?')
    marks = sorted((p['record'], wid) for n, wid in KNOWN.items()
                   if params.get(n) for p in [params[n]])
    ordered = all(marks[i][1] < marks[i + 1][1] for i in range(len(marks) - 1))
    print(f'   increasing with record offset: {ordered}')

    print('4. second pointer per parameter name?')
    ident = re.compile(r'^[A-Za-z][A-Za-z0-9_]{2,}$')
    checked = extra = stray = 0
    for name, p in params.items():
        if name.endswith('_ForBackup') or not ident.match(name):
            continue
        checked += 1
        needle = image[p['record'] + NAME_FIELD:p['record'] + NAME_FIELD + 8]
        refs = [m.start() for m in re.finditer(re.escape(needle), image)]
        others = [r for r in refs if r != p['record'] + NAME_FIELD]
        if others:
            extra += 1
        if any((r - NAME_FIELD) % DESCRIPTOR_SIZE != 0 for r in others):
            stray += 1
    print(f'   {checked} identifier names checked, {extra} with a second pointer, '
          f'{stray} of them outside the descriptor table')
    print('   (the numeric label strings do have a shared pointer array, but '
          'they are display labels, not parameters)')


if __name__ == '__main__':
    main()
