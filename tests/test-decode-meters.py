#!/usr/bin/env python3
"""Contract tests for tools/decode-meters.py.

The script name is hyphenated (project convention), so it cannot be imported
as a plain module; it is loaded by path with importlib instead.
"""

import importlib.util
import io
import json
import os
import pathlib
import sys
import tempfile
from contextlib import redirect_stdout

ROOT = pathlib.Path(__file__).resolve().parent.parent

_spec = importlib.util.spec_from_file_location(
    "decode_meters", ROOT / "tools" / "decode-meters.py"
)
decode_meters = importlib.util.module_from_spec(_spec)
sys.modules["decode_meters"] = decode_meters
_spec.loader.exec_module(decode_meters)

Frame = decode_meters.Frame
extract_meter_frames = decode_meters.extract_meter_frames
is_meter_frame = decode_meters.is_meter_frame
read_frames = decode_meters.read_frames

METER_HEADER = bytes.fromhex("04f04310043e1402")


def meter_frame(fill: int = 0) -> str:
    data = METER_HEADER + bytes([fill]) * (192 - len(METER_HEADER))
    return data.hex()


def test_is_meter_frame():
    frame = Frame("0x83", "bulk", "in", bytes.fromhex(meter_frame()), 0.0)
    assert is_meter_frame(frame)
    assert not is_meter_frame(Frame("0x83", "bulk", "in", b"\x00" * 192, 0.0))
    assert not is_meter_frame(Frame("0x03", "bulk", "out", bytes.fromhex(meter_frame()), 0.0))
    assert not is_meter_frame(Frame("0x83", "bulk", "in", b"\x00" * 100, 0.0))


def test_extract_keeps_only_valid_meter_frames():
    rows = [
        {"endpoint": "0x83", "transfer": "bulk", "direction": "in",
         "data": meter_frame(1), "time": 1.0},
        {"endpoint": "0x83", "transfer": "bulk", "direction": "in",
         "data": "04f04310043e1401" + "00" * 504, "time": 1.1},
        {"endpoint": "0x03", "transfer": "bulk", "direction": "out",
         "data": meter_frame(2), "time": 1.2},
    ]
    with tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False) as stream:
        for row in rows:
            stream.write(json.dumps(row) + "\n")
        path = stream.name
    try:
        frames = read_frames(path)
        extracted = extract_meter_frames(frames)
        assert len(extracted) == 1
        assert extracted[0] == bytes.fromhex(meter_frame(1))
    finally:
        os.unlink(path)


def test_frames_cli_prints_meter_frame_count():
    rows = [
        {"endpoint": "0x83", "transfer": "bulk", "direction": "in",
         "data": meter_frame(1), "time": 1.0},
        {"endpoint": "0x83", "transfer": "bulk", "direction": "in",
         "data": meter_frame(2), "time": 1.5},
    ]
    with tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False) as stream:
        for row in rows:
            stream.write(json.dumps(row) + "\n")
        path = stream.name
    try:
        out = io.StringIO()
        with redirect_stdout(out):
            status = decode_meters.main(["frames", path])
        assert status == 0
        assert out.getvalue().strip().splitlines() == ["2"]
    finally:
        os.unlink(path)


def test_column_stats_reports_only_varying_offsets():
    column_stats = decode_meters.column_stats
    frames = [
        bytes([0x04, 0x10, 0x20, 0x30]),
        bytes([0x04, 0x11, 0x20, 0x35]),
        bytes([0x04, 0x10, 0x20, 0x31]),
    ]
    assert column_stats(frames) == [(1, 0x10, 0x11), (3, 0x30, 0x35)]


def test_columns_cli_prints_offset_min_max():
    rows = [
        {"endpoint": "0x83", "transfer": "bulk", "direction": "in",
         "data": meter_frame(1), "time": 1.0},
        {"endpoint": "0x83", "transfer": "bulk", "direction": "in",
         "data": meter_frame(9), "time": 1.5},
    ]
    with tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False) as stream:
        for row in rows:
            stream.write(json.dumps(row) + "\n")
        path = stream.name
    try:
        out = io.StringIO()
        with redirect_stdout(out):
            status = decode_meters.main(["columns", path])
        assert status == 0
        assert out.getvalue().strip().splitlines() == [
            "%d %d %d" % (offset, 1, 9) for offset in range(8, 192)
        ]
    finally:
        os.unlink(path)


FIXTURE = ROOT / "tests/fixtures/meters/frames.jsonl"

# Values are level + 127 (0 = -127 dB floor, 127 = 0 dBFS). The fixture frames
# are synthetic and carry the current field of each meter (see
# docs/METER_PROTOCOL.md); the levels come from the measured captures.
FIXTURE_EXPECTED = [
    {"mic_in": 52, "mic_out": 0, "pc_in": 38, "stream_mix": 38},     # gate closed, quiet
    {"mic_in": 49, "mic_out": 51, "pc_in": 39, "stream_mix": 55},    # gate bypassed, quiet
    {"mic_in": 118, "mic_out": 121, "pc_in": 38, "stream_mix": 126}, # mic tap
    {"mic_in": 119, "mic_out": 122, "pc_in": 37, "stream_mix": 126}, # mic speech
    {"mic_in": 68, "mic_out": 13, "pc_in": 107, "stream_mix": 107},  # voice -20 dBFS
    {"mic_in": 81, "mic_out": 26, "pc_in": 37, "stream_mix": 117},   # game -6 dBFS
]


def test_decode_fixture_frames():
    decode_frame = decode_meters.decode_frame
    frames = extract_meter_frames(read_frames(str(FIXTURE)))
    assert len(frames) == len(FIXTURE_EXPECTED)
    for frame, expected in zip(frames, FIXTURE_EXPECTED):
        assert decode_frame(frame) == expected


def test_decode_omits_undecoded_meters():
    values = decode_meters.decode_frame(extract_meter_frames(read_frames(str(FIXTURE)))[0])
    assert not {"game_mix", "hdmi_in"} & set(values)


def test_decode_rejects_non_meter_frames():
    decode_frame = decode_meters.decode_frame
    good = extract_meter_frames(read_frames(str(FIXTURE)))[0]
    for bad in (good[:191], good + b"\x00", b"\x00" + good[1:]):
        try:
            decode_frame(bad)
        except ValueError:
            continue
        raise AssertionError("decode_frame accepted a non-meter frame")


def test_decode_clamps_below_floor():
    decode_frame = decode_meters.decode_frame
    frame = bytearray(extract_meter_frames(read_frames(str(FIXTURE)))[0])
    frame[10], frame[11] = 0x00, 0x00
    assert decode_frame(bytes(frame))["mic_in"] == 0


if __name__ == "__main__":
    test_is_meter_frame()
    test_extract_keeps_only_valid_meter_frames()
    test_frames_cli_prints_meter_frame_count()
    test_column_stats_reports_only_varying_offsets()
    test_columns_cli_prints_offset_min_max()
    test_decode_fixture_frames()
    test_decode_omits_undecoded_meters()
    test_decode_rejects_non_meter_frames()
    test_decode_clamps_below_floor()
    print("decode-meters contract passed")
