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


# The frame carries 34 channels, each with a current and a peak-hold value.
# A group starts at a field index and has one current value per channel.
# Channel c of a group is field (base + 2*c); the peak partner is the next
# field. Each field is value = hi * 128 + lo, and level dB = (value - 16384)
# / 10. See docs/METER_PROTOCOL.md for the captured evidence.
GROUP_BASES = {
    "mic_in": 0,
    "mic_out": 2,
    "pc_in": 4,
    "hdmi_in": 20,
    "game_mix": 36,
    "stream_mix": 52,
}
GROUP_CHANNELS = {
    "mic_in": 1,
    "mic_out": 1,
    "pc_in": 8,
    "hdmi_in": 8,
    "game_mix": 8,
    "stream_mix": 8,
}

LEVEL_ZERO_CODE = 16384
LEVEL_CODES_PER_DB = 10
LEVEL_FLOOR_DB = -127


def raw_offset(payload: int) -> int:
    """Map a SysEx payload index to its raw frame offset."""
    return 4 * (payload // 3) + 1 + payload % 3


def decode_level(data: bytes, field: int) -> int:
    """Return one channel as 0..127, linear in dB (0 = -127 dB, 127 = 0 dBFS).

    `field` is the index of the current value in the 68-value sequence.
    Codes on the 10-step grid decode exactly. Off-grid codes round to the
    nearest dB. Levels above 0 dBFS clamp to 127 and levels below -127 dB
    clamp to 0.
    """
    payload = 7 + 2 * field
    code = data[raw_offset(payload)] * 128 + data[raw_offset(payload + 1)]
    db = round((code - LEVEL_ZERO_CODE) / LEVEL_CODES_PER_DB)
    return max(0, min(127, db - LEVEL_FLOOR_DB))


def has_meter_framing(data: bytes) -> bool:
    """True when the frame has the CIN bytes and the final F7 of a meter frame."""
    if len(data) != METER_LEN or not data.startswith(METER_HEADER):
        return False
    if any(data[i] != 0x04 for i in range(0, METER_LEN - 4, 4)):
        return False
    return data[METER_LEN - 4] == 0x07 and data[METER_LEN - 1] == 0xF7


def decode_frame(data: bytes) -> dict[str, list[int]]:
    """Decode every channel of every group; see docs/METER_PROTOCOL.md.

    The returned lists are the current values (0..127) of each channel, in
    the group's channel order. Raises ValueError for a frame that fails
    has_meter_framing.
    """
    if not has_meter_framing(data):
        raise ValueError("not a valid 192-byte meter frame")
    return {
        group: [
            decode_level(data, GROUP_BASES[group] + 2 * channel)
            for channel in range(GROUP_CHANNELS[group])
        ]
        for group in GROUP_BASES
    }


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

    status = 0
    for frame in meter_frames:
        try:
            values = decode_frame(frame)
        except ValueError as error:
            print("decode-meters: skipped frame: %s" % error, file=sys.stderr)
            status = 1
            continue
        for group, levels in values.items():
            for channel, level in enumerate(levels):
                print("%s %d %d" % (group, channel, level))
    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
