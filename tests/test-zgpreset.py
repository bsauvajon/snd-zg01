#!/usr/bin/env python3
"""Container, decoder and CLI tests for tools/zgpreset.py.

The fixtures are built in memory from synthetic chunk and component data.
No Yamaha preset file is committed or read.
"""

import json
import pathlib
import struct
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "tests" / "fixtures" / "zgpreset"
sys.path.insert(0, str(ROOT / "tools"))

from zgpreset import (
    Component,
    Param,
    Preset,
    PresetError,
    SchemaEntry,
    decode_values,
    parse_preset,
    parse_schema,
    read_chunks,
    read_components,
)

FILE_MAGIC = "#YAMAHA MBDFPreset"
BACKUP_MAGIC = "#YAMAHA MBDFBackup"
FILE_ID = "79f82ba4-a076-47e8-887d-f34571f0fe49"
HEADER_SIZE = 0x74
FILE_ID_SIZE = 36
CHUNK_START = HEADER_SIZE + FILE_ID_SIZE
ENVELOPE = b"Comp\x00\x00\x00\x00"


def make_chunk(tag, name, payload):
    return b"".join((
        tag.encode("ascii").ljust(8, b"\x00"),
        b"\x00" * 4,
        name.encode("utf-8")[:12].ljust(12, b"\x00"),
        struct.pack(">Q", len(payload)),
        b"\x00" * 4,
        payload,
    ))


def make_mbdf(chunks, magic=FILE_MAGIC, file_id=FILE_ID):
    """Build a synthetic MBDF file from (tag, name, payload) chunks.

    Pass file_id=None to place the chunk stream directly after the
    0x74-byte header, as the format notes describe.
    """
    magic = magic.encode("ascii")
    header = bytearray(HEADER_SIZE)
    header[:len(magic)] = magic
    struct.pack_into(">H", header, 0x1E, HEADER_SIZE)
    header[0x24:0x26] = b"ZG"
    data = bytes(header)
    if file_id is not None:
        data += file_id.encode("ascii").ljust(FILE_ID_SIZE, b"\x00")
    return data + b"".join(make_chunk(*chunk) for chunk in chunks)


def make_component(name, payload, magic=0xDA27):
    return b"".join((
        name.encode("ascii")[:24].ljust(24, b"\x00"),
        struct.pack(">H", magic),
        b"\x00\x00",
        struct.pack("<H", len(payload)),
        b"\x00\x00",
        payload,
    ))


def schema_payload(entries, header=b"\x80\x01\x00\x01\x00\x12"):
    """Build an F_* payload: a block header, the marker, then records."""
    return header + b"\x41\x01\x00\x80" + b"".join(
        bytes((address >> 8, address & 0xFF, 0x01, width))
        for address, width in entries)


def info_payload(block, name):
    """Build a #PRESET General payload with the real 64/16/0x02 layout."""
    return b"".join((
        block.encode("ascii").ljust(64, b"\x00"),
        b"\x00" * 16,
        b"\x02",
        name.encode("utf-8"),
        b"\x00\x00",
    ))


def mic_d_payload(gain=150):
    """A Mic Input D_* payload; the value blob starts at its end."""
    return (b"\x00\x02" + b"Custom".ljust(126, b"\x00")
            + bytes((0x01, 0x00, gain & 0xFF, gain >> 8)))


MIC_ENTRIES = [(0x120, 1), (0x121, 1), (0x128, 2)]


def mic_preset(gain=150, block="MIC INPUT", name="Custom"):
    data = ENVELOPE + b"".join((
        make_component("F_MicEq", schema_payload(MIC_ENTRIES)),
        make_component("D_MicEq", mic_d_payload(gain), magic=0xD927),
    ))
    return make_mbdf([
        ("#FILE", "Property", b""),
        ("#PRESET ", "General", info_payload(block, name)),
        ("#PRESET ", "ChunkData", data),
        ("#END", "", b""),
    ])


def preset_with_entries(entries, values, name="Custom"):
    """A Mic Input preset whose schema and values are supplied by the test."""
    payload = (b"\x00\x02" + b"Custom".ljust(126, b"\x00")
               + b"".join(value.to_bytes(width, "little")
                          for (_, width), value in zip(entries, values)))
    data = ENVELOPE + b"".join((
        make_component("F_MicEq", schema_payload(entries)),
        make_component("D_MicEq", payload, magic=0xD927),
    ))
    return make_mbdf([
        ("#PRESET ", "General", info_payload("MIC INPUT", name)),
        ("#PRESET ", "ChunkData", data),
    ])


def write_file(directory, name, data):
    path = pathlib.Path(directory) / name
    path.write_bytes(data)
    return str(path)


def run_tool(*args):
    return subprocess.run(
        [sys.executable, str(ROOT / "tools" / "zgpreset.py"), *args],
        capture_output=True, text=True, check=False)


def test_file_header_and_chunks():
    data = make_mbdf([
        ("#FILE", "79f82ba4-a076-47e8-887d-f34571f0fe49", b""),
        ("#PRESET ", "INFOGeneral", b"GAME EFFECT"),
        ("#END", "", b""),
    ])
    chunks = read_chunks(data)
    assert [c.tag for c in chunks] == ["#FILE", "#PRESET ", "#END"]
    assert chunks[1].name == "INFOGeneral"
    assert chunks[1].payload == b"GAME EFFECT"


def test_backup_header_is_rejected():
    data = make_mbdf([("#CONSOLE", "ChunkData", b"payload")],
                     magic=BACKUP_MAGIC)
    try:
        read_chunks(data)
    except PresetError as error:
        assert "backup" in str(error).lower()
    else:
        raise AssertionError("a .zg01 backup was accepted")


def test_chunks_directly_after_header():
    data = make_mbdf([("#END", "", b"")], file_id=None)
    chunks = read_chunks(data)
    assert [c.tag for c in chunks] == ["#END"]


def test_chunk_length_is_big_endian():
    payload = bytes(range(256)) + b"length"  # 262 bytes; LE would be 0x0601
    data = make_mbdf([("#PRESET ", "ChunkData", payload)])
    chunks = read_chunks(data)
    assert chunks[0].payload == payload


def test_truncated_chunk_is_rejected():
    data = make_mbdf([("#FILE", "x", b"")])
    try:
        read_chunks(data[:-1])
    except PresetError:
        pass
    else:
        raise AssertionError("truncated chunk was accepted")


def test_truncated_header_is_rejected():
    for data in (FILE_MAGIC.encode("ascii"),
                 b"#NOT AN MBDF FILE".ljust(HEADER_SIZE, b"\x00")):
        try:
            read_chunks(data)
        except PresetError:
            pass
        else:
            raise AssertionError(f"invalid header was accepted: {data!r}")


def test_chunk_length_beyond_file_is_rejected():
    data = bytearray(make_mbdf([("#FILE", "x", b"")]))
    struct.pack_into(">Q", data, CHUNK_START + 24, 1 << 20)
    try:
        read_chunks(bytes(data))
    except PresetError as error:
        assert "remaining" in str(error)
    else:
        raise AssertionError("over-long chunk was accepted")


def test_trailing_chunk_bytes_are_rejected():
    data = make_mbdf([("#END", "", b"")]) + b"\x00\x00\x00"
    try:
        read_chunks(data)
    except PresetError:
        pass
    else:
        raise AssertionError("trailing chunk bytes were accepted")


def test_components():
    payload = ENVELOPE + b"".join((
        make_component("F_MicEq", b"schema", 0xDA27),
        make_component("D_MonitorEq", b"\x01\x02", 0xD927),
    ))
    components = read_components(payload)
    assert components == [
        Component(name="F_MicEq", magic=0xDA27, payload=b"schema"),
        Component(name="D_MonitorEq", magic=0xD927, payload=b"\x01\x02"),
    ]


def test_components_without_envelope():
    payload = make_component("F_GameEffect", b"schema")
    components = read_components(payload)
    assert [c.name for c in components] == ["F_GameEffect"]
    assert components[0].payload == b"schema"


def test_component_length_is_little_endian():
    payload = make_component("D_GameEffect", b"v" * 258)  # 0x0102
    components = read_components(payload)
    assert len(components[0].payload) == 258


def test_truncated_component_is_rejected():
    component = make_component("F_MicEq", b"schema")
    for payload in (ENVELOPE + component[:10],
                    ENVELOPE + component[:-1]):
        try:
            read_components(payload)
        except PresetError:
            pass
        else:
            raise AssertionError("truncated component was accepted")


def test_component_length_beyond_payload_is_rejected():
    payload = ENVELOPE + make_component("F_MicEq", b"schema")
    payload = bytearray(payload)
    struct.pack_into("<H", payload, len(ENVELOPE) + 28, 0x100)
    try:
        read_components(bytes(payload))
    except PresetError as error:
        assert "remaining" in str(error)
    else:
        raise AssertionError("over-long component was accepted")


def test_unknown_component_magic_is_rejected():
    payload = make_component("F_MicEq", b"schema", magic=0x1234)
    try:
        read_components(payload)
    except PresetError as error:
        assert "magic" in str(error)
    else:
        raise AssertionError("unknown component magic was accepted")


def test_zero_length_component_is_rejected():
    payload = make_component("F_MicEq", b"")
    try:
        read_components(payload)
    except PresetError:
        pass
    else:
        raise AssertionError("zero-length component was accepted")


def test_trailing_component_bytes_are_rejected():
    payload = ENVELOPE + make_component("F_MicEq", b"schema") + b"\x00\x00"
    try:
        read_components(payload)
    except PresetError:
        pass
    else:
        raise AssertionError("trailing component bytes were accepted")


def test_schema_and_values():
    f = schema_payload([(0x120, 1), (0x121, 1), (0x128, 2)])
    d = b"\x00\x02" + b"Custom".ljust(126, b"\x00") + b"\x01\x00\x96\x00"
    params = decode_values(parse_schema(f), d)
    assert [(p.address, p.value) for p in params] == [
        (0x120, 1), (0x121, 0), (0x128, 150)]


def test_value_blob_starts_at_the_end():
    # The D_* name field is 127 bytes here, one byte longer than the
    # Mic Input fixture; the values must still be found from the end.
    d = b"\x02" + b"GameFxSample1".ljust(127, b"\x00") + b"\x03\x01"
    params = decode_values([SchemaEntry(0x075, 1), SchemaEntry(0x076, 1)], d)
    assert [p.value for p in params] == [3, 1]


def test_schema_uses_the_marker_end_for_a_short_header():
    f = b"\x00\x73\x41\x01\x00\x80" + b"\x00\x74\x01\x01"
    assert parse_schema(f) == [SchemaEntry(0x074, 1)]


def test_schema_rejects_unknown_width():
    f = b"\x41\x01\x00\x80" + bytes((0x01, 0x20, 0x01, 0x03))
    try:
        parse_schema(f)
    except PresetError as error:
        assert "width" in str(error)
    else:
        raise AssertionError("schema with width 3 was accepted")


def test_schema_rejects_short_payload():
    for payload in (b"\x00" * 9,
                    b"\x00" * 10 + b"\x01\x20\x01",
                    b"\x41\x01\x00\x80" + b"\x01\x20\x01"):
        try:
            parse_schema(payload)
        except PresetError:
            pass
        else:
            raise AssertionError("short schema was accepted")


def test_value_blob_shorter_than_schema_is_rejected():
    d = b"\x01\x00"
    try:
        decode_values([SchemaEntry(0x120, 2), SchemaEntry(0x121, 2)], d)
    except PresetError:
        pass
    else:
        raise AssertionError("short value blob was accepted")


def test_parse_preset_metadata_and_params():
    with tempfile.TemporaryDirectory() as directory:
        path = write_file(directory, "mic.zgpreset", mic_preset())
        preset = parse_preset(path)
    assert preset.block == "MIC INPUT"
    assert preset.name == "Custom"
    assert preset.params == [
        Param(address=0x120, width=1, value=1),
        Param(address=0x121, width=1, value=0),
        Param(address=0x128, width=2, value=150),
    ]


def test_backup_current_scene_is_rejected():
    """A .zg01 backup is out of scope, even with a single scene group."""
    entries = [(0x001, 1), (0x006, 2)]
    d = (b"\x02Current Scene".ljust(127, b"\x00")
         + b"\x05" + struct.pack("<H", 0x1234))
    data = ENVELOPE + b"".join((
        make_component("F_CurrentScene", schema_payload(entries)),
        make_component("D_CurrentScene", d),
    ))
    backup = make_mbdf([("#CONSOLE", "ChunkData", data)],
                       magic=BACKUP_MAGIC)
    with tempfile.TemporaryDirectory() as directory:
        path = write_file(directory, "ref.zg01", backup)
        try:
            parse_preset(path)
        except PresetError as error:
            assert "backup" in str(error).lower()
        else:
            raise AssertionError("a .zg01 backup was parsed")


def test_list_command():
    with tempfile.TemporaryDirectory() as directory:
        path = write_file(directory, "mic.zgpreset", mic_preset())
        result = run_tool("list", path)
    assert result.returncode == 0, result.stderr
    assert result.stdout == "F_MicEq 3 params\nD_MicEq 3 values\n"


def test_show_command():
    with tempfile.TemporaryDirectory() as directory:
        path = write_file(directory, "mic.zgpreset", mic_preset())
        result = run_tool("show", path)
    assert result.returncode == 0, result.stderr
    assert result.stdout == (
        "MIC INPUT\n"
        "Custom\n"
        "0x120 mic_eq_v2_1_ONF = 1\n"
        "0x121 mic_eq_v2_2_ONF = 0\n"
        "0x128 mic_eq_v2_1_G = 150\n"
    )


def test_show_unknown_address():
    entries = [(0x777, 1)]
    data = ENVELOPE + b"".join((
        make_component("F_Mystery", schema_payload(entries)),
        make_component("D_Mystery", b"\x02Mystery".ljust(127, b"\x00")
                       + b"\x09"),
    ))
    preset = make_mbdf([
        ("#PRESET ", "General", info_payload("MYSTERY", "None")),
        ("#PRESET ", "ChunkData", data),
    ])
    with tempfile.TemporaryDirectory() as directory:
        path = write_file(directory, "mystery.zgpreset", preset)
        result = run_tool("show", path)
    assert result.returncode == 0, result.stderr
    assert result.stdout == "MYSTERY\nNone\n0x777 unknown = 9\n"


def test_show_page_zero_ignores_meter_names():
    entries = [(0x004, 1)]
    data = ENVELOPE + b"".join((
        make_component("F_MicEq", schema_payload(entries)),
        make_component("D_MicEq", b"\x02Custom".ljust(127, b"\x00")
                       + b"\x0c"),
    ))
    preset = make_mbdf([
        ("#PRESET ", "General", info_payload("MIC INPUT", "Custom")),
        ("#PRESET ", "ChunkData", data),
    ])
    with tempfile.TemporaryDirectory() as directory:
        path = write_file(directory, "mic.zgpreset", preset)
        result = run_tool("show", path)
    assert result.returncode == 0, result.stderr
    assert result.stdout == (
        "MIC INPUT\nCustom\n0x004 mic_gate_thhold = 12\n")


def test_dump_command_matches_fixture():
    expected = json.loads((FIXTURES / "mic-input.json").read_text())
    with tempfile.TemporaryDirectory() as directory:
        path = write_file(directory, "mic.zgpreset", mic_preset())
        result = run_tool("dump", path)
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout) == expected
    assert result.stdout == json.dumps(expected) + "\n"


def test_diff_command():
    with tempfile.TemporaryDirectory() as directory:
        first = write_file(directory, "a.zgpreset", mic_preset(gain=150))
        second = write_file(directory, "b.zgpreset", mic_preset(gain=211))
        result = run_tool("diff", first, second)
    assert result.returncode == 0, result.stderr
    assert result.stdout == "0x128 150 -> 211\n"


def test_diff_reports_added_and_removed_addresses():
    with tempfile.TemporaryDirectory() as directory:
        first = write_file(
            directory, "a.zgpreset",
            preset_with_entries([(0x120, 1)], [1]))
        second = write_file(
            directory, "b.zgpreset",
            preset_with_entries([(0x121, 1)], [1]))
        result = run_tool("diff", first, second)
    assert result.returncode == 0, result.stderr
    assert result.stdout == "0x120 1 -> -\n0x121 - -> 1\n"


def test_long_utf8_display_name_is_preserved():
    name = "A" * 62 + "é tail"
    with tempfile.TemporaryDirectory() as directory:
        path = write_file(directory, "long.zgpreset",
                          preset_with_entries([(0x120, 1)], [1], name=name))
        preset = parse_preset(path)
    assert preset.name == name


def test_preset_without_decoded_parameters_is_rejected():
    data = make_mbdf([
        ("#PRESET ", "General", info_payload("MIC INPUT", "Empty")),
        ("#END", "", b""),
    ])
    with tempfile.TemporaryDirectory() as directory:
        path = write_file(directory, "empty.zgpreset", data)
        try:
            parse_preset(path)
        except PresetError as error:
            assert "no decoded parameters" in str(error)
        else:
            raise AssertionError("a preset with no parameters was accepted")


def test_bad_file_exits_two():
    with tempfile.TemporaryDirectory() as directory:
        path = write_file(directory, "bad.zgpreset", b"not an MBDF file")
        result = run_tool("show", path)
    assert result.returncode == 2
    assert result.stdout == ""
    assert "zgpreset:" in result.stderr


def test_backup_cli_exits_two_with_a_clear_error():
    backup = make_mbdf([("#CONSOLE", "ChunkData", b"payload")],
                       magic=BACKUP_MAGIC)
    with tempfile.TemporaryDirectory() as directory:
        path = write_file(directory, "ref.zg01", backup)
        result = run_tool("show", path)
    assert result.returncode == 2
    assert result.stdout == ""
    assert "backup" in result.stderr.lower()


def main():
    tests = [value for name, value in sorted(globals().items())
             if name.startswith("test_") and callable(value)]
    for test in tests:
        test()
    print(f"zgpreset: {len(tests)} tests passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
