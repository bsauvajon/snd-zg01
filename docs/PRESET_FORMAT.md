# ZG01 preset file format (`.zgpreset`)

Yamaha's ZG Controller stores mixer settings in files with the `.zgpreset`
extension (presets) and `.zg01` extension (backups).  Both use the MBDF
container described here, but **only `.zgpreset` files are parsed**:
`.zg01` backups are rejected with a clear error for now.  This document
records the facts the parser in `tools/zgpreset.c` and the CLI commands
`zgctl preset list/show/load` rely on; it is not a specification from
Yamaha.

The format was decoded from the vendor reader/writer and checked against
factory and user presets.  Vendor binaries and preset files stay out of Git;
committed tests use synthetic fixtures.

## Container

A file starts with a fixed 0x74-byte header.  The header begins with one of
two ASCII magic strings:

| Header magic | Kind | Parser |
|---|---|---|
| `#YAMAHA MBDFPreset` | preset (`.zgpreset`) | accepted |
| `#YAMAHA MBDFBackup` | device backup (`.zg01`) | rejected with `backup not supported` |

Real files place a 36-byte file identifier between that header and the chunk
stream.  Synthetic fixtures may omit it.  The parser decides with the byte
at offset `0x74`: a `#` starts a chunk tag, so the identifier is absent;
any other byte means the 36-byte identifier is present and the stream starts
at `0x74 + 36`.

The chunk stream is a sequence of chunks:

| Field | Size | Encoding |
|---|---|---|
| tag | 8 bytes | ASCII, NUL padded: `#FILE`, `#PRESET `, `#END` (`#CONSOLE` belongs to backups, which are rejected) |
| reserved | 4 bytes | ignored |
| name | 12 bytes | ASCII, NUL padded |
| payload length | 8 bytes | big-endian |
| reserved | 4 bytes | ignored |
| payload | length bytes | depends on the tag |

A `#PRESET ` chunk whose name is `ChunkData` or `DATA` carries the mixer
components.  A `#PRESET ` chunk named `General` carries the preset
metadata: the block label (`MIC INPUT`, `MONITOR EQ`, `GAME EFFECT`,
`MIC EFFECT`, ...), a display name and a UUID.  The parser exposes the block
label and the display name.  The display name is kept at its full payload
length, including a final multi-byte UTF-8 sequence.  There is no literal
`#PRESET DATA` tag: the tag field is eight bytes, and `DATA` is a chunk
*name* inside a `#PRESET ` tag.  A file that yields no decoded parameter is
rejected rather than accepted as an empty preset.

## Components

A `#PRESET ` data chunk payload (after the optional `Comp\0\0\0\0`
envelope) contains components with this header:

| Field | Size | Encoding |
|---|---|---|
| name | 24 bytes | ASCII, NUL padded, `F_*` or `D_*` |
| magic | 2 bytes | big-endian `0xDA27`, or `0xD927` for Monitor EQ |
| reserved | 2 bytes | ignored |
| payload length | 2 bytes | little-endian |
| reserved | 2 bytes | ignored |
| payload | length bytes | see below |

Component names come in pairs.  An `F_*` component is the **schema**: it
lists the parameter addresses and their value widths.  The matching `D_*`
component (**data**) carries the values in that order.  For example
`F_MicEq` describes the parameters whose values live in `D_MicEq`.

### Schema: addresses and widths

The four-byte marker

```
41 01 00 80
```

ends the schema header.  The records that follow it are four bytes each:

| Field | Size | Meaning |
|---|---|---|
| address high | 1 byte | high byte of the device address |
| address low | 1 byte | low byte of the device address |
| marker | 1 byte | always `0x01` |
| width | 1 byte | value width in bytes, `1` or `2` |

The number of header bytes before the marker differs per block (six for Mic
Input and Monitor EQ, two for Game Effect and the voice preset), so the
parser searches for the marker instead of assuming a fixed header length.
A file with no marker, a truncated record or a width other than 1 or 2 is
rejected.

### Data: display name and values

A `D_*` payload starts with a display-name field, then the values in schema
order.  Each value is one or two bytes, **little-endian**.  The display-name
length differs between blocks (it is not a fixed prefix), so the value blob
is located from the end of the component:

```
value_start = len(D_payload) - sum(schema widths)
```

The bytes before `value_start` are the display name and its small framing
prefix.  This rule is what makes Mic Input, Monitor EQ and Game Effect
decode as plain device values; it is pinned by
`tests/test-zgpreset.py` and `tests/test-zgpreset-c.sh`.

For example, a schema with widths `1, 2` has a three-byte value blob: one
byte for the first parameter and two little-endian bytes for the second.

## Address mapping

Schema addresses are the same 16-bit device addresses used by the mixer
protocol:

```
address = page * 256 + offset
```

`tools/vendor/device-map-zg01.json` names every address and
`tools/vendor/control-map-zg01.json` maps each one to an ALSA control name,
conversion and bounds.  `tools/zg01_controls_generated.h` is the userspace
view of that map, included by `zgctl`.  Address is the stable key: control
names may change, but a preset value is always resolved by address.

`zgctl preset show` prints the address and the mapped control name (or
`unknown`).  `zgctl preset load` converts the raw device value to the
control's user unit with the same rules as the driver:

| Conversion | Device value to user value |
|---|---|
| `CONV_ID` | unchanged |
| `CONV_GAIN` | `device - 180` (0.1 dB, 0 dB = 180) |
| `CONV_FREQ` | unchanged (Hz) |
| `CONV_Q` | inverse of `device = 12 * log2(Q / 0.5)` |

## Loading

`zgctl preset load FILE [--dry-run] [--block NAME] [--allow-raw]` builds
one plan for every parameter and then writes the applicable entries through
their ALSA controls:

- an address with no control, or a read-only status row, is **unsupported**
  and skipped;
- a converted value outside the control's `min..max` is **out_of_range** and
  skipped, never clamped;
- a control marked `raw` in the control map is skipped unless `--allow-raw`
  is given;
- `--block NAME` limits the load to the file's block label (case-insensitive,
  whitespace-normalized); a mismatch is a usage error;
- `--dry-run` prints the complete plan and writes nothing.

The summary is `applied=N skipped=N failed=N`.  Loading is best-effort and
not transactional: a failed write leaves earlier writes in place, and the
command exits `1` if anything was skipped or failed.  A `failed` entry only
occurs when the driver rejects a write (for example a control that vanished
between planning and writing); the synthetic tests exercise the skip path,
not a driver error.  Load never calls `Save to ZG01`; persistence stays an
explicit `zgctl save`.

The eight read-only exclusions are the five `dirty_*` flags, `mic_level`,
`mic_monitor_level_knob` and `mon_level`.  They appear in the userspace map
with `ZG01_CTL_READONLY` but have **no ALSA control**, so `preset show`
reports them and `preset load` marks them `unsupported` and skips them.
`Streaming Mic Level Steps` is not one of them: it is a writable
vendor-model level (0..127).

## Backups

`.zg01` backups use the same container and component framing, but they are
**not parsed**.  Their `F_CurrentScene` schema is a multi-group layout with
several `41 01 00 80` markers and short group trailers; joining the scene
groups to their values is a follow-up.  `zgctl preset list`, `show` and
`load` reject a backup with a clear `backup not supported` error rather
than reporting a partial or invented decode.  Preset loading targets
`.zgpreset` files only.
