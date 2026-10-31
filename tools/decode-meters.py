#!/usr/bin/env python3
"""Extract and decode ZG01 meter frames from normalized usbmon JSONL.

The input is the JSON Lines schema produced by tools/normalize-usbmon.py:
one JSON object per line with the keys "endpoint", "transfer", "direction",
"data" (hex) and "time".
"""

import argparse
import json
import sys
from dataclasses import dataclass

METER_HEADER = bytes.fromhex("04f04310043e1402")
METER_LEN = 192
METER_EP = "0x83"


@dataclass
class Frame:
    endpoint: str
    transfer: str
    direction: str
    data: bytes
    time: float


def read_frames(path: str) -> list[Frame]:
    """Read a normalized JSONL capture into a list of Frame."""
    frames = []
    with open(path, encoding="utf-8") as stream:
        for line in stream:
            line = line.strip()
            if not line:
                continue
            row = json.loads(line)
            frames.append(Frame(
                row["endpoint"], row["transfer"], row.get("direction", ""),
                bytes.fromhex(row["data"]), row.get("time", 0.0)))
    return frames


def is_meter_frame(frame: Frame) -> bool:
    """True when the frame is a 192-byte bulk IN on the meter endpoint."""
    return (frame.endpoint == METER_EP and frame.transfer == "bulk"
            and frame.direction == "in" and len(frame.data) == METER_LEN
            and frame.data.startswith(METER_HEADER))


def extract_meter_frames(frames: list[Frame]) -> list[bytes]:
    """Return the payloads of the meter frames, in capture order."""
    return [f.data for f in frames if is_meter_frame(f)]


def column_stats(frames: list[bytes]) -> list[tuple[int, int, int]]:
    """Return (offset, min, max) for each byte offset whose value varies."""
    stats = []
    for offset in range(min((len(f) for f in frames), default=0)):
        values = [f[offset] for f in frames]
        if min(values) != max(values):
            stats.append((offset, min(values), max(values)))
    return stats


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    frames_cmd = sub.add_parser(
        "frames", help="print the number of meter frames in a capture")
    frames_cmd.add_argument("file", help="normalized JSONL capture")

    decode_cmd = sub.add_parser(
        "decode", help="print one line per meter frame")
    decode_cmd.add_argument("file", help="normalized JSONL capture")

    columns_cmd = sub.add_parser(
        "columns", help="print 'offset min max' for each varying byte")
    columns_cmd.add_argument("file", help="normalized JSONL capture")

    args = parser.parse_args(argv)

    try:
        frames = read_frames(args.file)
    except (OSError, ValueError, KeyError) as error:
        print("decode-meters: %s" % error, file=sys.stderr)
        return 1

    meter_frames = extract_meter_frames(frames)

    if args.command == "frames":
        print(len(meter_frames))
        return 0

    if args.command == "columns":
        for offset, low, high in column_stats(meter_frames):
            print(offset, low, high)
        return 0

    decode_frame = globals().get("decode_frame")
    if decode_frame is None:
        print("decode-meters: decode_frame is not implemented yet",
              file=sys.stderr)
        return 1

    for frame in meter_frames:
        print(decode_frame(frame))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
