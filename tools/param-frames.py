#!/usr/bin/env python3
"""Turn mixer captures into the parameter address table.

``snd-zg01`` captures the vendor application's traffic, and every mixer
write is a USB-MIDI packetised Yamaha SysEx message:

    F0 43 10 3E 14 01 | 01 00 <flag> | <id> 00 00 | 00 00 00
                      | 00 00 00     | <d0> <d1> <d2> | F7

The first byte of each 4-byte packet is a USB-MIDI code index number, the
value is three big-endian base-128 digits, and `<flag> <id>` are the two
base-128 digits of one device address:

    address = page * 256 + offset = flag * 128 + id

(`docs/DEVICE_PARAMETER_MAP.md` maps every address of a ZG01 to the name
of the parameter that lives there.)

This tool reads a capture back into those addresses:

    # what did this capture write?
    tools/param-frames.py parse 40-eq-band2-mid.jsonl

    # one action changed which addresses? (before/after pair)
    tools/param-frames.py diff 00-baseline.jsonl 40-eq-band2-mid.jsonl

    # which vendor parameter has these bounds? (labels the diff)
    tools/param-frames.py diff a.jsonl b.jsonl --label params.json

Input is the JSONL produced by tools/normalize-usbmon.py or
tools/normalize-usbpcap.py, one transfer per line.
"""
import argparse
import json
import sys

EP_OUT_DEFAULT = '0x03'
CODE_INDEX_END = {0x05: 1, 0x06: 2, 0x07: 3}


def iter_transfers(path, endpoint):
    with open(path) as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            record = json.loads(line)
            if record.get('endpoint') != endpoint:
                continue
            if record.get('direction') not in (None, 'out'):
                continue
            data = bytes.fromhex(record.get('data') or '')
            if data:
                yield record, data


def iter_sysex(data):
    """Yield the SysEx byte strings carried by one USB-MIDI frame."""
    stream = []
    index = 0
    while index + 4 <= len(data):
        packet = data[index:index + 4]
        index += 4
        cin = packet[0]
        if cin == 0x00 and not any(packet):
            break
        if cin == 0x04:
            stream.extend(packet[1:4])
            continue
        if cin in CODE_INDEX_END:
            stream.extend(packet[1:1 + CODE_INDEX_END[cin]])
            yield bytes(stream)
            stream = []
            continue
        # unknown code index: resynchronise on the next packet
        stream = []
    if stream:
        yield bytes(stream)


def decode_value(digits):
    value = 0
    for digit in digits:
        value = value * 128 + (digit & 0x7f)
    return value


def parse_sysex(message):
    """Classify a SysEx message: parameter write/read, action, other.

    Bytes, with F0/F7 stripped (``body``, 17 bytes for a write):

        0: 43                    Yamaha
        1: <class>               0x10 write, 0x30 read/action, 0x00 preset
        2: 3e  3: 14             parameter-change command
        4: <command>             0x01 parameter, 0x03 action
        7: <address high digit> page 0 -> 0x00, page 1 -> 0x02
        8: <address low digit>  offset inside the page
        9..13: zero padding
        14..16: value, three big-endian base-128 digits
        (actions instead carry 6: <code> 7: <arg>)
    """
    if len(message) < 6 or message[0] != 0xf0 or message[-1] != 0xf7:
        return None
    body = message[1:-1]
    if len(body) < 6 or body[0] != 0x43:
        return None
    sys_class, command = body[1], body[4]
    info = {'class': sys_class, 'command': command, 'bytes': body.hex()}
    if sys_class == 0x10 and command == 0x01 and len(body) >= 17:
        info['kind'] = 'write'
        info['addr_hi'] = body[7]
        info['addr_lo'] = body[8]
        info['address'] = body[7] * 128 + body[8]
        info['value'] = decode_value(body[-3:])
    elif sys_class == 0x30 and command == 0x01 and len(body) >= 9:
        info['kind'] = 'read-request'
        info['addr_hi'] = body[7]
        info['addr_lo'] = body[8]
        info['address'] = body[7] * 128 + body[8]
    elif sys_class == 0x30 and command == 0x03 and len(body) >= 8:
        info['kind'] = 'action'
        info['code'] = body[6]
        info['arg'] = body[7]
    else:
        info['kind'] = 'other'
    return info


def collect(path, endpoint):
    messages = []
    for record, data in iter_transfers(path, endpoint):
        for message in iter_sysex(data):
            parsed = parse_sysex(message)
            if parsed is None:
                continue
            parsed['time'] = record.get('time')
            messages.append(parsed)
    return messages


def cmd_parse(args):
    writes = [m for m in collect(args.capture, args.endpoint)
              if m['kind'] == 'write']
    for message in writes:
        print(f"address=0x{message['address']:03x} "
              f"(flag=0x{message['addr_hi']:02x} id=0x{message['addr_lo']:02x}) "
              f"value={message['value']}")
    print(f'{len(writes)} parameter write(s)', file=sys.stderr)


def cmd_diff(args):
    before = {(m['address']): m['value']
              for m in collect(args.before, args.endpoint) if m['kind'] == 'write'}
    after = {(m['address']): m['value']
             for m in collect(args.after, args.endpoint) if m['kind'] == 'write'}
    changed = []
    for key in sorted(set(before) | set(after)):
        old = before.get(key)
        new = after.get(key)
        if old != new:
            changed.append((key, old, new))
    for address, old, new in changed:
        print(f'address=0x{address:03x} {old} -> {new}')

    if args.label:
        params = json.load(open(args.label))
        for address, old, new in changed:
            values = [v for v in (old, new) if v is not None]
            candidates = []
            for name, param in params.items():
                if name.endswith('_ForBackup'):
                    continue
                if name[0].islower():
                    continue
                if all(param['min'] <= v <= param['max'] for v in values):
                    candidates.append((param['default'] in values, name,
                                       param['min'], param['max']))
            candidates.sort(reverse=True)
            shown = ', '.join(f'{n} ({mn}..{mx})' for _d, n, mn, mx
                              in candidates[:6])
            print(f'  candidates for address=0x{address:03x}: '
                  f'{shown or "none"}')
    print(f'{len(changed)} changed address(es)', file=sys.stderr)


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--endpoint', default=EP_OUT_DEFAULT,
                    help='USB-MIDI OUT endpoint (default 0x03)')
    sub = ap.add_subparsers(dest='command', required=True)

    one = sub.add_parser('parse', help='list the writes of one capture')
    one.add_argument('capture')
    one.set_defaults(func=cmd_parse)

    two = sub.add_parser('diff', help='addresses that changed between two captures')
    two.add_argument('before')
    two.add_argument('after')
    two.add_argument('--label', help='vendor params JSON to match bounds against')
    two.set_defaults(func=cmd_diff)

    args = ap.parse_args()
    args.func(args)


if __name__ == '__main__':
    main()
