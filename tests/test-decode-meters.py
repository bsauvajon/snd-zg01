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
from contextlib import redirect_stderr, redirect_stdout

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
    # mic decay: the current fields (-25, -19, -15) are below their held peaks
    # (-14, -11, -7); the output must use the current values.
    {"mic_in": 102, "mic_out": 108, "pc_in": 36, "stream_mix": 112},
]
PEAK_HELD_INDEX = 6


def fixture_frames() -> list[bytes]:
    return extract_meter_frames(read_frames(str(FIXTURE)))


def assert_rejected(frame: bytes) -> None:
    try:
        decode_meters.decode_frame(frame)
    except ValueError:
        return
    raise AssertionError("decode_frame accepted an invalid meter frame")


def test_decode_fixture_frames():
    decode_frame = decode_meters.decode_frame
    frames = fixture_frames()
    assert len(frames) == len(FIXTURE_EXPECTED)
    for frame, expected in zip(frames, FIXTURE_EXPECTED):
        assert decode_frame(frame) == expected


def test_decode_omits_undecoded_meters():
    values = decode_meters.decode_frame(fixture_frames()[0])
    assert not {"game_mix", "hdmi_in"} & set(values)


def test_decode_rejects_bad_length_and_header():
    good = fixture_frames()[0]
    assert_rejected(good[:191])
    assert_rejected(good + b"\x00")
    assert_rejected(b"\x00" + good[1:])


def test_decode_rejects_bad_cin():
    good = fixture_frames()[0]
    for offset, value in ((100, 0x05), (184, 0x00), (188, 0x04)):
        bad = bytearray(good)
        bad[offset] = value
        assert_rejected(bytes(bad))


def test_decode_rejects_missing_f7():
    good = fixture_frames()[0]
    for value in (0x00, 0x07, 0xF0):
        bad = bytearray(good)
        bad[191] = value
        assert_rejected(bytes(bad))


def test_decode_rounds_off_grid_codes_to_nearest_db():
    decode_frame = decode_meters.decode_frame
    frame = bytearray(fixture_frames()[0])
    for code, expected in ((16320, 121), (16318, 120)):
        frame[10], frame[11] = divmod(code, 128)
        assert decode_frame(bytes(frame))["mic_in"] == expected


def test_decode_clamps_below_floor_and_top_of_range():
    decode_frame = decode_meters.decode_frame
    frame = bytearray(fixture_frames()[0])
    frame[10], frame[11] = 0x00, 0x00
    assert decode_frame(bytes(frame))["mic_in"] == 0
    frame[10], frame[11] = divmod(16383, 128)
    assert decode_frame(bytes(frame))["mic_in"] == 127


def test_decode_uses_current_not_peak():
    decode_frame = decode_meters.decode_frame
    frame = bytearray(fixture_frames()[PEAK_HELD_INDEX])
    frame[13], frame[14] = divmod(16383, 128)  # mic peak raised to the top code
    assert decode_frame(bytes(frame))["mic_in"] == 102


def test_decode_cli_skips_invalid_frames():
    good = fixture_frames()[0]
    bad = bytearray(good)
    bad[100] = 0x05
    rows = [
        {"endpoint": "0x83", "transfer": "bulk", "direction": "in",
         "data": good.hex(), "time": 1.0},
        {"endpoint": "0x83", "transfer": "bulk", "direction": "in",
         "data": bytes(bad).hex(), "time": 1.1},
    ]
    with tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False) as stream:
        for row in rows:
            stream.write(json.dumps(row) + "\n")
        path = stream.name
    try:
        out = io.StringIO()
        err = io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            status = decode_meters.main(["decode", path])
        assert status == 1
        assert len(out.getvalue().strip().splitlines()) == 1
        assert "skipped frame" in err.getvalue()
    finally:
        os.unlink(path)


if __name__ == "__main__":
    test_is_meter_frame()
    test_extract_keeps_only_valid_meter_frames()
    test_frames_cli_prints_meter_frame_count()
    test_column_stats_reports_only_varying_offsets()
    test_columns_cli_prints_offset_min_max()
    test_decode_fixture_frames()
    test_decode_omits_undecoded_meters()
    test_decode_rejects_bad_length_and_header()
    test_decode_rejects_bad_cin()
    test_decode_rejects_missing_f7()
    test_decode_rounds_off_grid_codes_to_nearest_db()
    test_decode_clamps_below_floor_and_top_of_range()
    test_decode_uses_current_not_peak()
    test_decode_cli_skips_invalid_frames()
    print("decode-meters contract passed")
