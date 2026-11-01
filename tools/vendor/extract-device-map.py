#!/usr/bin/env python3
"""Extract the device parameter map from a vendor MemAPI library.

The library registers its parameters at load time: one record per
parameter, built by the initialiser as

    mov dword ptr [rip+A], 0x0001SSII   (kind 1, page SS, offset II)
    lea rdx, [rip+N]                    (name string pointer)
    lea rcx, [rip+R]                    (record pointer)
    call <register>

so both the name and the device offset are recoverable by scanning the
code section for those two instruction patterns.  No disassembler is
needed: `c7 05 rel32 imm32` and `48 8d 15 rel32` are fixed encodings.

The address that goes on the wire is ``page * 256 + offset`` encoded as
two big-endian base-128 digits (SysEx data bytes are 7-bit), which is why
the frame's `<flag> <id>` pair is the address high digit followed by the
low digit - see docs/DEVICE_PARAMETER_MAP.md.

Usage:
    tools/vendor/extract-device-map.py VENDOR/MemAPI_ZG01.dll \
        tools/vendor/device-map-zg01.json
"""
import argparse
import json
import struct
import sys

MOVE_DWORD_RIP = b'\xc7\x05'          # mov dword ptr [rip+rel32], imm32
MOVE_QWORD_RIP = b'\x48\xc7\x05'      # mov qword ptr [rip+rel32], imm32
LEA_RDX_RIP = b'\x48\x8d\x15'         # lea rdx, [rip+rel32]
LEA_RCX_RIP = b'\x48\x8d\x0d'         # lea rcx, [rip+rel32]


def parse_pe(data):
    pe = struct.unpack_from('<I', data, 0x3c)[0]
    nsec = struct.unpack_from('<H', data, pe + 6)[0]
    opt_size = struct.unpack_from('<H', data, pe + 20)[0]
    opt = pe + 24
    magic = struct.unpack_from('<H', data, opt)[0]
    base = (struct.unpack_from('<Q', data, opt + 24)[0] if magic == 0x20b
            else struct.unpack_from('<I', data, opt + 28)[0])
    sections = []
    sh = opt + opt_size
    for i in range(nsec):
        off = sh + i * 40
        name = data[off:off + 8].rstrip(b'\0').decode('latin1')
        vsize, vaddr, rsize, raddr = struct.unpack_from('<IIII', data, off + 8)
        sections.append({'name': name, 'va': vaddr, 'vsize': vsize,
                         'off': raddr, 'size': rsize})
    return base, sections


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('library')
    ap.add_argument('output')
    args = ap.parse_args()

    data = open(args.library, 'rb').read()
    base, sections = parse_pe(data)
    text = next((s for s in sections if s['name'] == '.text'), None)
    if text is None:
        raise SystemExit('no .text section')

    def va_of(off):
        return base + text['va'] + (off - text['off'])

    def off_of_va(va):
        rva = va - base
        for section in sections:
            if section['va'] <= rva < section['va'] + section['size']:
                return section['off'] + (rva - section['va'])
        return None

    def read_string(va, limit=64):
        off = off_of_va(va)
        if off is None:
            return None
        end = data.find(b'\0', off, off + limit)
        if end < 0:
            return None
        text_bytes = data[off:end]
        if not text_bytes or not all(32 <= b < 127 for b in text_bytes):
            return None
        return text_bytes.decode('ascii')

    # Records are 40 bytes and the field sits at +0x20 inside the record,
    # so the store target identifies the record.  Names are read from the
    # (name pointer, record pointer, call) triple that precedes each
    # store, which makes the pairing independent of instruction ordering.
    RECORD_SIZE = 0x28
    RECORD_FIELD_OFFSET = 0x20

    end = text['off'] + text['size']
    fields = []
    for pattern, length, rel_at, imm_at in ((MOVE_QWORD_RIP, 11, 3, 7),
                                            (MOVE_DWORD_RIP, 10, 2, 6)):
        cursor = text['off']
        while True:
            pos = data.find(pattern, cursor, end)
            if pos < 0:
                break
            cursor = pos + 1
            immediate = struct.unpack_from('<I', data, pos + imm_at)[0]
            if not 0x10000 <= immediate <= 0xffffff:
                continue
            rel = struct.unpack_from('<i', data, pos + rel_at)[0]
            fields.append({'store': va_of(pos), 'value': immediate,
                           'store_target': va_of(pos) + length + rel,
                           'kind': immediate >> 16,
                           'page': (immediate >> 8) & 0xff,
                           'offset': immediate & 0xff})
    fields.sort(key=lambda field: field['store'])

    # Every registration is a "call <register>" preceded by "lea rcx,
    # [record]" and "lea rdx, [name]", and the record's field was stored
    # at record + 0x20 just before that.  Reading the triple per call site
    # is exact, unlike matching names and stores by proximity.
    CALL = b'\xe8'
    registrations = []
    cursor = text['off']
    while True:
        pos = data.find(LEA_RCX_RIP, cursor, end)
        if pos < 0:
            break
        cursor = pos + 1
        rel = struct.unpack_from('<i', data, pos + 3)[0]
        record_va = va_of(pos) + 7 + rel
        call = data.find(CALL, pos + 7, pos + 0x30)
        if call < 0:
            continue
        name_hit = data.rfind(LEA_RDX_RIP, max(text['off'], pos - 0x40), pos)
        if name_hit < 0:
            continue
        rel = struct.unpack_from('<i', data, name_hit + 3)[0]
        name_va = va_of(name_hit) + 7 + rel
        name = read_string(name_va)
        if name:
            registrations.append({'record_va': record_va, 'name': name,
                                  'name_va': name_va, 'call_va': va_of(call)})
    by_record = {}
    for registration in registrations:
        by_record.setdefault(registration['record_va'], []).append(registration)

    def nearest_registration(record_va, store_va):
        """The registration belonging to this record, in the same block.

        A record can be registered twice (a control and its meter live in
        one record, initialised by two functions); the store and its
        registration sit next to each other, so the closest wins.
        """
        candidates = by_record.get(record_va)
        if not candidates:
            return None
        return min(candidates, key=lambda r: abs(r['call_va'] - store_va))

    out = []
    for field in fields:
        target = field['store_target']
        owner = nearest_registration(target - RECORD_FIELD_OFFSET, field['store'])
        if owner is None:
            candidates = [va for va in by_record
                          if va <= target < va + RECORD_SIZE]
            if not candidates:
                continue
            owner = nearest_registration(max(candidates), field['store'])
            if owner is None:
                continue
        record = dict(field)
        record['name'] = owner['name']
        record['name_va'] = owner['name_va']
        record['record_va'] = owner['record_va']
        out.append(record)

    named = [r for r in out if 'name' in r]
    groups = {}
    for record in out:
        if not record.get('name'):
            continue
        key = f"{record['kind']}/{record['page']}"
        groups.setdefault(key, []).append(record)
    print(f'{len(out)} fields, {len(named)} named', file=sys.stderr)
    for key, recs in sorted(groups.items()):
        offsets = [r['offset'] for r in recs]
        duplicates = len(offsets) - len(set(offsets))
        print(f'  kind/page {key}: {len(recs)} records, '
              f'{len(set(offsets))} distinct offsets, {duplicates} duplicated',
              file=sys.stderr)

    with open(args.output, 'w') as handle:
        json.dump({'library': args.library.split('/')[-1],
                   'image_base': base,
                   'groups': {
                       key: sorted(records, key=lambda r: r['offset'])
                       for key, records in sorted(groups.items())}},
                  handle, indent=1)
    print(f'wrote {args.output}', file=sys.stderr)


if __name__ == '__main__':
    main()
