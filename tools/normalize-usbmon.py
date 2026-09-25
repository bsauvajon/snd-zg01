#!/usr/bin/env python3
"""Convert a Linux usbmon capture into stable JSON Lines.

Unlike tools/normalize-usbpcap.py, which consumes a tshark TSV keyed on
USB idVendor/idProduct, this tool parses a classic pcap written by
tcpdump/tshark on a usbmon interface. usbmon records do not expose the
vendor/product IDs, so records are keyed on bus/device numbers and the
device of interest is selected with --device.

Isochronous transfers are skipped by default: on the ZG01 they are the
audio streams and would drown the control and bulk traffic. Pass
--include-iso to keep them (each record is emitted unmerged).

Only classic pcap is supported. Capture with tcpdump, or convert a
pcapng first:  tshark -r in.pcapng -F pcap -w out.pcap
"""

import argparse
import json
import struct
import sys
from pathlib import Path

# libpcap link types for Linux usbmon.
DLT_USB_LINUX = 189
DLT_USB_LINUX_MMAPPED = 220

# usbmon header sizes: the mmapped layout appends fields after the setup
# block and pads to 64 bytes; the legacy layout stops at 48.
HEADER_MMAPPED = 64
HEADER_LEGACY = 48

TRANSFER_TYPES = {0: "isochronous", 1: "interrupt", 2: "control", 3: "bulk"}

MAGIC_LE = b"\xd4\xc3\xb2\xa1"
MAGIC_BE = b"\xa1\xb2\xc3\xd4"
MAGIC_NS_LE = b"\x4d\x3c\xb2\xa1"
MAGIC_NS_BE = b"\xa1\xb2\x3c\x4d"


def read_pcap(path):
    """Yield (timestamp_seconds, raw_frame, linktype) for a classic pcap."""
    data = path.read_bytes()
    if len(data) < 24:
        raise ValueError("truncated pcap global header")

    magic = data[0:4]
    if magic in (MAGIC_LE, MAGIC_NS_LE):
        endian, nanos = "<", magic == MAGIC_NS_LE
    elif magic in (MAGIC_BE, MAGIC_NS_BE):
        endian, nanos = ">", magic == MAGIC_NS_BE
    else:
        raise ValueError(
            "not a classic pcap (pcapng?): %s" % magic.hex()
        )

    snaplen, linktype = struct.unpack_from(endian + "II", data, 16)
    if linktype not in (DLT_USB_LINUX, DLT_USB_LINUX_MMAPPED):
        raise ValueError(
            "unsupported link type %d (expected usbmon)" % linktype
        )

    offset = 24
    scale = 1e-9 if nanos else 1e-6
    total = len(data)
    while offset + 16 <= total:
        ts_sec, ts_frac, incl_len, _orig_len = struct.unpack_from(
            endian + "IIII", data, offset
        )
        offset += 16
        frame = data[offset:offset + incl_len]
        offset += incl_len
        yield ts_sec + ts_frac * scale, frame, linktype


def parse_usbmon_frame(frame, linktype):
    """Return the usbmon header fields and the transfer payload."""
    header_size = (
        HEADER_MMAPPED if linktype == DLT_USB_LINUX_MMAPPED else HEADER_LEGACY
    )
    if len(frame) < header_size:
        raise ValueError("truncated usbmon frame")

    (
        urb_id,
        record_type,
        xfer_type,
        epnum,
        devnum,
        busnum,
        _flag_setup,
        _flag_data,
        ts_sec,
        ts_usec,
        status,
        length,
        len_cap,
    ) = struct.unpack_from("<QBBBBHccqiiII", frame, 0)

    setup = frame[40:48]
    payload = frame[header_size:header_size + len_cap]

    return {
        "urb_id": urb_id,
        "record_type": chr(record_type),
        "transfer": TRANSFER_TYPES.get(xfer_type, "unknown(%d)" % xfer_type),
        "endpoint": epnum,
        "direction": "in" if epnum & 0x80 else "out",
        "device": devnum,
        "bus": busnum,
        "time": ts_sec + ts_usec * 1e-6,
        "status": status,
        "length": length,
        "actual_length": len(payload),
        "setup": setup,
        "payload": payload,
    }


def control_fields(setup):
    if len(setup) != 8:
        return {}
    request_type = setup[0]
    return {
        "request_type": request_type,
        "direction": "in" if request_type & 0x80 else "out",
        "request": setup[1],
        "value": setup[2] | (setup[3] << 8),
        "index": setup[4] | (setup[5] << 8),
        "length": setup[6] | (setup[7] << 8),
    }


def emit(record, payload, actual_length, status, time):
    transfer = record["transfer"]
    out = {
        "frame": record["frame"],
        "time": time,
        "bus": record["bus"],
        "device": record["device"],
        "endpoint": "0x%02x" % (record["endpoint"] & 0xff),
        "transfer": transfer,
        "urb_id": "0x%016x" % record["urb_id"],
        "status": status,
        "actual_length": actual_length,
        "data": payload.hex(),
    }
    if transfer == "control":
        out.update(control_fields(record["setup"]))
    else:
        out["direction"] = record["direction"]
        out["length"] = record["length"]
    return out


def normalize(path, device=None, include_iso=False):
    pending = {}
    order = []
    for frame_number, (time, frame, linktype) in enumerate(read_pcap(path), 1):
        rec = parse_usbmon_frame(frame, linktype)
        rec["frame"] = frame_number

        if device is not None and (rec["bus"], rec["device"]) != device:
            continue
        if rec["transfer"] == "isochronous" and not include_iso:
            continue

        if include_iso:
            yield emit(rec, rec["payload"], rec["actual_length"], rec["status"], time)
            continue

        key = rec["urb_id"]
        if rec["record_type"] == "S":
            pending[key] = (rec, time)
            order.append(key)
        elif rec["record_type"] == "C":
            entry = pending.pop(key, None)
            if entry is None:
                continue
            rec0, time0 = entry
            # For IN transfers the setup is on the submit and the data on
            # the completion; for OUT transfers the data is on the submit.
            if rec0["direction"] == "in":
                payload = rec["payload"]
                actual = rec["actual_length"]
                status = rec["status"]
                timestamp = time
            else:
                payload = rec0["payload"]
                actual = rec0["actual_length"]
                status = rec["status"]
                timestamp = time0
            merged = dict(rec0)
            merged["frame"] = rec0["frame"]
            yield emit(merged, payload, actual, status, timestamp)

    # Transfers whose completion was not captured: emit with what we have.
    for key in order:
        entry = pending.pop(key, None)
        if entry is None:
            continue
        rec0, time0 = entry
        yield emit(rec0, rec0["payload"], rec0["actual_length"], None, time0)


def parse_device(text):
    try:
        bus, dev = text.split(".")
        return int(bus, 0), int(dev, 0)
    except ValueError as error:
        raise argparse.ArgumentTypeError("expected BUS.DEV, e.g. 1.78") from error


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    parser.add_argument(
        "--device",
        type=parse_device,
        help="keep only this BUS.DEV, e.g. --device 1.78",
    )
    parser.add_argument(
        "--include-iso",
        action="store_true",
        help="include isochronous audio transfers (default: skip)",
    )
    args = parser.parse_args()

    try:
        for transfer in normalize(args.capture, args.device, args.include_iso):
            print(json.dumps(transfer, separators=(",", ":"), sort_keys=True))
    except (OSError, ValueError) as error:
        print("normalize-usbmon: %s" % error, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
