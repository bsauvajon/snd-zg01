#!/usr/bin/env python3
"""Read the MBDF container and values in Yamaha ZG01 preset files.

Only ``.zgpreset`` files are supported.  A file starts with a fixed 0x74-byte
header whose magic is ``#YAMAHA MBDFPreset``.  Real files put a 36-byte file
identifier (a UUID, also used by the ``#FILE`` chunk) between that header and
the chunk stream.  A ``#`` at offset 0x74 means the identifier is absent, as
in synthetic fixtures, so that byte decides where the chunk stream starts.

Chunks and components are parsed conservatively: a length that runs past the
end of its container, an unknown component magic and trailing bytes that do
not form a whole record are all rejected instead of being silently ignored.

An ``F_*`` component is a schema.  Its length varies per block, but the
4-byte marker ``41 01 00 80`` ends the header; the 4-byte
address/marker/width records follow it.  Its paired ``D_*`` component carries
a display name and the little-endian values.  Because the display-name field
differs in length per block, the value blob starts at
``len(D) - sum(widths)``.

``.zg01`` backups use the same container but are not parsed: their
``F_CurrentScene`` schema is multi-group and joining the scene groups to
their values is a follow-up.  ``read_chunks`` rejects the backup magic with
``BackupNotSupported`` so no caller can claim backup support by accident.
"""

from __future__ import annotations

import json
import pathlib
import struct
import sys
from dataclasses import dataclass

HEADER_SIZE = 0x74
FILE_ID_SIZE = 36
PRESET_MAGIC = b"#YAMAHA MBDFPreset"
BACKUP_MAGIC = b"#YAMAHA MBDFBackup"

CHUNK_HEADER_SIZE = 8 + 4 + 12 + 8 + 4

COMPONENT_ENVELOPE = b"Comp\x00\x00\x00\x00"
COMPONENT_HEADER_SIZE = 24 + 2 + 2 + 2 + 2
COMPONENT_MAGICS = frozenset({0xDA27, 0xD927})

SCHEMA_MARKER = b"\x41\x01\x00\x80"
SCHEMA_RECORD_SIZE = 4
RECORD_MARKER = 0x01
VALUE_WIDTHS = (1, 2)

DATA_CHUNK_NAMES = frozenset({"ChunkData", "DATA"})
INFO_CHUNK_NAMES = frozenset({"General", "INFOGeneral"})
BACKUP_UNSUPPORTED = ("Yamaha .zg01 backups are not supported; "
                      "only .zgpreset files can be read")

DEVICE_MAP_PATH = (pathlib.Path(__file__).resolve().parent
                   / "vendor" / "device-map-zg01.json")


class PresetError(Exception):
    """Raised when a file is not a well-formed MBDF preset."""


class BackupNotSupported(PresetError):
    """Raised for a .zg01 backup, which this parser does not decode."""


@dataclass(frozen=True)
class Chunk:
    tag: str
    name: str
    payload: bytes


@dataclass(frozen=True)
class Component:
    name: str
    magic: int
    payload: bytes


@dataclass(frozen=True)
class SchemaEntry:
    address: int
    width: int


@dataclass(frozen=True)
class Param:
    address: int
    width: int
    value: int


@dataclass(frozen=True)
class Preset:
    name: str
    block: str
    params: list[Param]


def _text(raw: bytes) -> str:
    """Decode a fixed-size NUL-padded ASCII field."""
    return raw.rstrip(b"\x00").decode("utf-8", errors="replace")


def _chunk_start(data: bytes) -> int:
    if len(data) < HEADER_SIZE:
        raise PresetError(
            f"file is {len(data)} bytes, shorter than the 0x74-byte header")
    if data.startswith(BACKUP_MAGIC):
        raise BackupNotSupported(BACKUP_UNSUPPORTED)
    if not data.startswith(PRESET_MAGIC):
        raise PresetError("not a Yamaha MBDF preset file")
    start = HEADER_SIZE
    # The 36-byte file identifier follows the header.  A '#' at the header
    # boundary is the start of a chunk tag, so the identifier is absent.
    if (data[start:start + 1] != b"#"
            and len(data) >= start + FILE_ID_SIZE):
        start += FILE_ID_SIZE
    return start


def read_chunks(data: bytes) -> list[Chunk]:
    """Return the chunk stream that follows the MBDF file header."""
    chunks = []
    offset = _chunk_start(data)
    while offset < len(data):
        remaining = len(data) - offset
        if remaining < CHUNK_HEADER_SIZE:
            raise PresetError(
                f"chunk at 0x{offset:x}: truncated header")
        tag = _text(data[offset:offset + 8])
        name = _text(data[offset + 12:offset + 24])
        (length,) = struct.unpack_from(">Q", data, offset + 24)
        payload_start = offset + CHUNK_HEADER_SIZE
        available = len(data) - payload_start
        if length > available:
            raise PresetError(
                f"chunk at 0x{offset:x} ({tag!r}): length {length} exceeds "
                f"the {available} remaining bytes")
        chunks.append(Chunk(
            tag=tag, name=name,
            payload=data[payload_start:payload_start + length]))
        offset = payload_start + length
    return chunks


def read_components(payload: bytes) -> list[Component]:
    """Return the components in a ``#PRESET`` data chunk payload."""
    if payload.startswith(COMPONENT_ENVELOPE):
        payload = payload[len(COMPONENT_ENVELOPE):]
    components = []
    offset = 0
    while offset < len(payload):
        remaining = len(payload) - offset
        if remaining < COMPONENT_HEADER_SIZE:
            raise PresetError(
                f"component at 0x{offset:x}: truncated header")
        name = _text(payload[offset:offset + 24])
        (magic,) = struct.unpack_from(">H", payload, offset + 24)
        (length,) = struct.unpack_from("<H", payload, offset + 28)
        if magic not in COMPONENT_MAGICS:
            raise PresetError(
                f"component at 0x{offset:x} ({name!r}): "
                f"unknown magic 0x{magic:04x}")
        if length == 0:
            raise PresetError(
                f"component at 0x{offset:x} ({name!r}): "
                f"zero-length payload")
        payload_start = offset + COMPONENT_HEADER_SIZE
        available = len(payload) - payload_start
        if length > available:
            raise PresetError(
                f"component at 0x{offset:x} ({name!r}): length {length} "
                f"exceeds the {available} remaining bytes")
        components.append(Component(
            name=name, magic=magic,
            payload=payload[payload_start:payload_start + length]))
        offset = payload_start + length
    return components


def parse_schema(payload: bytes) -> list[SchemaEntry]:
    """Return the address/width records in an ``F_*`` component payload.

    The header length varies (two or six bytes in the real blocks), so the
    records are located relative to the ``41 01 00 80`` marker rather than a
    fixed offset.
    """
    marker_offset = payload.find(SCHEMA_MARKER)
    if marker_offset < 0:
        raise PresetError("schema marker 41 01 00 80 was not found")
    body = payload[marker_offset + len(SCHEMA_MARKER):]
    if len(body) % SCHEMA_RECORD_SIZE:
        raise PresetError(
            f"schema has {len(body)} bytes after the marker, not a whole "
            f"number of {SCHEMA_RECORD_SIZE}-byte records")
    entries = []
    for offset in range(0, len(body), SCHEMA_RECORD_SIZE):
        address_hi, address_lo, marker, width = body[
            offset:offset + SCHEMA_RECORD_SIZE]
        if marker != RECORD_MARKER:
            raise PresetError(
                f"schema record at 0x{offset:x}: unknown marker "
                f"0x{marker:02x}")
        if width not in VALUE_WIDTHS:
            raise PresetError(
                f"schema record at 0x{offset:x}: unknown width {width}")
        entries.append(SchemaEntry(
            address=(address_hi << 8) | address_lo, width=width))
    return entries


def decode_values(schema: list[SchemaEntry],
                  d_payload: bytes) -> list[Param]:
    """Read the little-endian values from the end of a ``D_*`` payload.

    The bytes in front of the value blob are the block's display-name field,
    whose length differs per block; only the total schema width locates the
    blob reliably.
    """
    total_width = 0
    for entry in schema:
        if entry.width not in VALUE_WIDTHS:
            raise PresetError(f"unknown schema width {entry.width}")
        total_width += entry.width
    if total_width > len(d_payload):
        raise PresetError(
            f"value blob is {len(d_payload)} bytes, shorter than the "
            f"{total_width} bytes the schema needs")
    offset = len(d_payload) - total_width
    params = []
    for entry in schema:
        value = int.from_bytes(
            d_payload[offset:offset + entry.width], "little")
        params.append(Param(
            address=entry.address, width=entry.width, value=value))
        offset += entry.width
    return params


def _data_components(chunks: list[Chunk]) -> list[Component]:
    components = []
    for chunk in chunks:
        if not chunk.tag.startswith("#PRESET"):
            continue
        if chunk.name in DATA_CHUNK_NAMES:
            components.extend(read_components(chunk.payload))
    return components


def _read_components(path: str) -> list[Component]:
    with open(path, "rb") as handle:
        return _data_components(read_chunks(handle.read()))


def _preset_info(chunks: list[Chunk]) -> tuple[str, str]:
    for chunk in chunks:
        if (chunk.tag.startswith("#PRESET")
                and chunk.name in INFO_CHUNK_NAMES):
            return _parse_info_payload(chunk.payload)
    return "", ""


def _parse_info_payload(payload: bytes) -> tuple[str, str]:
    if len(payload) < 64:
        return _text(payload), ""
    block = _text(payload[:64])
    # The display name follows a 0x02 marker at offset 80 in every real
    # #PRESET General chunk; the 16 bytes before it are block metadata.
    if len(payload) > 80 and payload[80] == 0x02:
        name = _text(payload[81:])
    else:
        name = ""
    return block, name


def parse_preset(path: str) -> Preset:
    """Parse a ``.zgpreset`` file into its block and values.

    A ``.zg01`` backup raises ``BackupNotSupported`` because its multi-group
    ``F_CurrentScene`` layout is a follow-up.
    """
    with open(path, "rb") as handle:
        chunks = read_chunks(handle.read())
    block, name = _preset_info(chunks)
    components = _data_components(chunks)
    values_by_suffix = {
        component.name[2:]: component
        for component in components if component.name.startswith("D_")}
    params = []
    for component in components:
        if not component.name.startswith("F_"):
            continue
        suffix = component.name[2:]
        values = values_by_suffix.get(suffix)
        if values is None:
            raise PresetError(
                f"component {component.name!r} has no matching D_* values")
        params.extend(
            decode_values(parse_schema(component.payload), values.payload))
    if not params:
        raise PresetError("preset contains no decoded parameters")
    return Preset(name=name, block=block, params=params)


def _device_names() -> dict[int, str]:
    try:
        with open(DEVICE_MAP_PATH, encoding="utf-8") as handle:
            document = json.load(handle)
    except (OSError, ValueError):
        return {}
    names = {}
    for records in document.get("groups", {}).values():
        for record in records:
            # Meter records (kind 2 and above) are not parameters and can
            # share an address with a real page-0 control.
            if record.get("kind") != 1:
                continue
            address = record["page"] * 256 + record["offset"]
            names[address] = record["name"]
    return names


def _address(address: int) -> str:
    return f"0x{address:03x}"


def _component_lines(components: list[Component]) -> list[str]:
    counts = {
        component.name[2:]: len(parse_schema(component.payload))
        for component in components if component.name.startswith("F_")}
    lines = []
    for component in components:
        suffix = component.name[2:]
        if component.name.startswith("F_"):
            lines.append(f"{component.name} {counts[suffix]} params")
        elif component.name.startswith("D_"):
            lines.append(f"{component.name} {counts.get(suffix, 0)} values")
    return lines


def _dump_document(preset: Preset) -> dict:
    return {
        "block": preset.block,
        "name": preset.name,
        "params": [
            {"address": param.address, "value": param.value}
            for param in preset.params],
    }


USAGE = ("usage: zgpreset.py list FILE | show FILE | dump FILE | "
         "diff FILE_A FILE_B")


def main(argv: list[str]) -> int:
    if not argv:
        print(USAGE, file=sys.stderr)
        return 2
    command, args = argv[0], argv[1:]
    try:
        if command == "list" and len(args) == 1:
            for line in _component_lines(_read_components(args[0])):
                print(line)
            return 0
        if command == "show" and len(args) == 1:
            preset = parse_preset(args[0])
            names = _device_names()
            print(preset.block)
            print(preset.name)
            for param in preset.params:
                print(f"{_address(param.address)} "
                      f"{names.get(param.address, 'unknown')} = "
                      f"{param.value}")
            return 0
        if command == "dump" and len(args) == 1:
            preset = parse_preset(args[0])
            print(json.dumps(_dump_document(preset), ensure_ascii=False))
            return 0
        if command == "diff" and len(args) == 2:
            first = parse_preset(args[0])
            second = parse_preset(args[1])
            before = {param.address: param.value for param in first.params}
            after = {param.address: param.value for param in second.params}
            # The union makes schema changes visible: an address present on
            # only one side is a difference too, not an invisible omission.
            for address in sorted(before.keys() | after.keys()):
                old = before.get(address)
                new = after.get(address)
                if old != new:
                    print(f"{_address(address)} "
                          f"{'-' if old is None else old} -> "
                          f"{'-' if new is None else new}")
            return 0
    except (PresetError, OSError) as error:
        print(f"zgpreset: {error}", file=sys.stderr)
        return 2
    print(USAGE, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
