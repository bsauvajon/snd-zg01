#!/usr/bin/env python3
"""Dump the parameter model out of a Yamaha MemAPI library.

The Windows "TOOLS for ZG" package ships MemAPI_ZG01.dll (and a ZG02
twin).  That library implements the device parameter API and carries a
table of fixed-size descriptors, one per parameter: the table is what
names MicEqV21G, MicGateThreshold, ... and gives their value bounds.

Only the parameter *metadata* is extracted here - never the vendor
binaries - so the result can be committed or diffed as documentation.

Usage:
    tools/vendor/extract-memapi-params.py <MemAPI_ZG0x.dll> [out.json]

Record layout: 56-byte records with the name pointer at offset 40.  The
14 little-endian u32 slots are reported as-is; slot 4/5/6 hold the
maximum, minimum and default value and slot 12 a per-parameter key.

Reproducing the input, without Windows and without root:
    unzip TOOLS_for_ZG_V200_Win.zip            # -> Install_ZG.exe
    innoextract -e Install_ZG.exe              # -> tmp/Yamaha ZG Application.msi, tmp/YmhZGApp.cab
    cabextract -d app tmp/YmhZGApp.cab         # -> app/MemAPI_ZG01.dll
"""

import json
import re
import struct
import sys

DESCRIPTOR_SIZE = 56
NAME_FIELD = 40


def read_sections(data):
    pe = struct.unpack_from('<I', data, 0x3c)[0]
    if data[pe:pe + 4] != b'PE\0\0':
        raise SystemExit('not a PE file')
    nsec = struct.unpack_from('<H', data, pe + 6)[0]
    opt_size = struct.unpack_from('<H', data, pe + 20)[0]
    opt = pe + 24
    magic = struct.unpack_from('<H', data, opt)[0]
    if magic == 0x20b:
        image_base = struct.unpack_from('<Q', data, opt + 24)[0]
    else:
        image_base = struct.unpack_from('<I', data, opt + 28)[0]

    sections = []
    for i in range(nsec):
        off = opt + opt_size + i * 40
        name = data[off:off + 8].rstrip(b'\0').decode('latin1')
        vsize, vaddr, rsize, raddr = struct.unpack_from('<IIII', data, off + 8)
        sections.append((name, vaddr, vsize, raddr, rsize))
    return image_base, sections


def main():
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    path = sys.argv[1]
    out = sys.argv[2] if len(sys.argv) > 2 else 'memapi_params.json'

    data = open(path, 'rb').read()
    image_base, sections = read_sections(data)

    def off_to_rva(off):
        for _name, vaddr, _vsize, raddr, rsize in sections:
            if raddr <= off < raddr + rsize:
                return vaddr + (off - raddr)
        return None

    params = {}
    for match in re.finditer(rb'[ -~]{3,120}\x00', data):
        off = match.start()
        rva = off_to_rva(off)
        if rva is None:
            continue
        pointer = struct.pack('<Q', image_base + rva)
        ref = data.find(pointer)
        while ref >= 0:
            if ref % DESCRIPTOR_SIZE == NAME_FIELD:
                base = ref - NAME_FIELD
                slots = list(struct.unpack_from('<14I', data, base))
                name = match.group()[:-1].decode('latin1')
                params.setdefault(name, {
                    'record': base,
                    'slots': slots,
                    'max': slots[4],
                    'min': slots[5],
                    'default': slots[6],
                    'key': slots[12] & 0xffff,
                })
            ref = data.find(pointer, ref + 1)

    with open(out, 'w') as handle:
        json.dump(params, handle, indent=1, sort_keys=True)

    mic = sorted(n for n in params if n.startswith('Mic'))
    print(f'{len(params)} parameters extracted, {len(mic)} of them Mic*')
    print(f'written {out}')
    for name in mic:
        p = params[name]
        print(f'  {name:26} {p["min"]:>6} .. {p["max"]:<6} default {p["default"]}')


if __name__ == '__main__':
    main()
