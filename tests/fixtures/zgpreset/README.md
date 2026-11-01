# `zgpreset` test fixtures

Fixtures for `tests/test-zgpreset.sh` and the `zgpreset` parser are synthetic.
`tests/test-zgpreset.py` builds the MBDF container, chunks and components in
memory, so no binary preset needs to be committed.

The Yamaha factory presets in `Presets*.zgpreset` / `UserPresets*.zgpreset`
and user presets from Yamaha ZG Controller are validation inputs only. They
stay outside Git (see `docs/VENDOR_PACKAGE.md` and the design spec,
`docs/superpowers/specs/2026-10-31-zgpreset-full-controls-design.md`).

Files added here must be synthetic or derived from a documented format fact,
never a copy of a Yamaha or user preset. The container layout they exercise:

- a fixed 0x74-byte header, magic `#YAMAHA MBDFPreset` or
  `#YAMAHA MBDFBackup`, followed by a 36-byte file identifier;
- chunks: 8-byte tag, 4 reserved bytes, 12-byte name, 64-bit big-endian
  length, 4 reserved bytes, payload;
- components inside `#PRESET DATA` / `#CONSOLE` `ChunkData` payloads: an
  optional `Comp\0\0\0\0` envelope, then 24-byte name, 16-bit big-endian
  magic (`0xDA27`, or `0xD927` for Monitor EQ), 16-bit reserved, 16-bit
  little-endian length, 16-bit reserved, payload.
