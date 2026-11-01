# Full parameter surface and `.zgpreset` loading

Date: 2026-10-31
Status: draft for approval

## Intent

Teach the Linux driver and `zgctl` to understand Yamaha's `*.zgpreset` files,
expose every addressable ZG01 parameter as an ALSA control, and load a preset
file onto the device from Linux.

The preset format is no longer a guess. The Yamaha MBDF container and the
parameter payloads have been inspected, and the controller's reader/writer
were located with REA:

- `sub_140051590` reads a chunk (`tag`, 12-byte name, 64-bit length, payload).
- `sub_1400518b0` writes a chunk.
- `sub_1400513b0` encodes an integer in 1, 2 or 4 bytes.
- `sub_140051cb0`, `sub_1400545c0` and `sub_140053c50` copy/save preset files.
- `sub_140052800`, `sub_140052a80` and `sub_1400522b0` build the `F_*`/`D_*`
  components.

All three preset families decode into the driver's address space. The value
blob starts at `len(D) - sum(schema widths)`, which handles the variable-length
display-name field in front of it; Mic Input, Monitor EQ and Game Effect then
read as plain little-endian 1- or 2-byte device values.

## Goals

1. A complete, read-only parser for `.zgpreset`: container, metadata,
   components, parameter addresses, values and names. `.zg01` backups are
   out of scope for this phase: the parsers and every `zgctl preset`
   command reject a backup with a clear "backup not supported" error, and
   decoding its multi-group current scene is a follow-up.
2. A reference implementation in Python for analysis and fixture generation.
3. A C port used by `zgctl`; no Python dependency at runtime.
4. Every addressable parameter (`tools/vendor/device-map-zg01.json`, 375
   entries) represented in the driver's control surface, except parameters
   that are genuinely read-only status until the state dump is decoded.
5. `zgctl preset show`, `zgctl preset list` and `zgctl preset load`.
6. `zgctl preset load --dry-run` reports every parameter, its target control,
   its converted value and every unsupported or out-of-range entry.
7. Tests, documentation and a hardware run on the ZG01.

## Non-goals

- Exporting a preset from Linux. This is a later, separate feature.
- Decoding the `F0 43 20` state dump and the meter frames.
- A GUI or a replacement for ZG Controller.
- Writing read-only status parameters as if they were mixer settings.
- Preserving unknown fields during a read/write round trip; the first parser is
  read-only.
- Supporting the ZG02. The map and model are ZG01-specific.

## Anti-goals

- Guessing a physical unit for a parameter whose encoding is not established.
  New controls publish device steps and say so.
- Silently clamping a preset value into a control's range. Out-of-range values
  are reported and skipped.
- Making `preset load` persist settings without an explicit `save`.
- Committing Yamaha binaries, factory presets or user presets to Git.

## Constraints

- The driver is an out-of-tree kernel module; every new control must build with
  `make W=1` and must not change the existing 208 control names or units.
- ALSA control names are limited to 44 bytes and must be unique.
- The device writes values as three base-128 digits; the largest representable
  value is `ZG01_VALUE_MAX` (2^21 - 1).
- The driver does not read page 0 values back; getters return the cache. Preset
  load validation therefore compares against the cache and the parser, not a
  device read.
- The vendor archive, `MemAPI_ZG01.dll`, `Yamaha ZG Controller.exe` and the
  `*.zgpreset` files stay out of Git. Derived facts and generated tables may be
  committed.
- The preset map is address-based. Control names may change; the address is the
  stable key.

## Inputs and availability

The work needs hardware and vendor material that are deliberately outside Git.
They are available for this work:

- ZG01 hardware: USB `0499:1513`, ALSA card `zg01`, currently on the
  development machine. It is required for the manual acceptance runs, not for
  CI.
- Vendor package: `TOOLS_for_ZG_V200_Win.zip`, extracted under
  `/tmp/opencode/zg-vendor` (outside the repository). The archive URL is in
  `docs/VENDOR_PACKAGE.md`; the controller is `ZG_IconId.exe` in the CAB, which
  the MSI names `Yamaha ZG Controller.exe`.
- User presets: `/home/brice/OneDrive/Yamaha-zg01-presets.zip`, extracted under
  `/tmp/opencode/zg01-presets`.
- Factory presets: the 26 `Presets*.zgpreset` / `UserPresets*.zgpreset` files
  in the extracted CAB.

CI and committed tests use synthetic MBDF fixtures and derived expected
values. The vendor and user files are validation inputs, never committed. If
the vendor package or hardware is unavailable, Phases 0 and 3 still have a
synthetic-fixture gate; the vendor/hardware parts are reported as not run, not
silently skipped.

## Approaches considered

### Parser

**A. Python reference, then C port (chosen).** Iterate on the remaining format
questions without rebuilding a kernel/userspace binary, use the factory presets
as fixtures, then port the settled format to C. The Python tool remains in
`tools/` for diagnostics. Cost: two implementations, kept honest by shared
fixtures and a comparison test.

**B. C only.** One implementation and no duplicate logic, but every format
experiment needs a rebuild and the reference output is harder to inspect. This
was rejected for the decode phase.

**C. Python only, `zgctl` shells out.** Fastest to build, but adds a Python
runtime dependency to a packaged CLI and makes error handling and testing worse.
Rejected.

### Control surface

**A. Preset-scope controls only.** Add Monitor EQ and binaural plus the missing
names; about 60 controls. Rejected by the user, who wants the full surface.

**B. All addressable parameters (chosen).** Generate the control table from the
device map and the vendor model. The 208 existing controls keep their curated
names, bounds and conversions. The remaining parameters use the vendor model's
bounds and default where the name correspondence is established; only the
addresses without a usable model row are exposed as raw device steps.

**C. Raw write path only.** A new kernel write interface for preset loading;
smaller control surface, but no ALSA visibility and a new ABI. Rejected.

The chosen combination is **A + B**: Python reference then C, and the full
addressable control surface.

## Design

### 1. Format model

`.zgpreset` files start with a fixed `#YAMAHA MBDFPreset` header. A
`#YAMAHA MBDFBackup` header identifies a `.zg01` backup, which the parser
recognizes only to reject it. The parser reads the chunk stream that
follows:

- 8-byte tag (`#FILE`, `#PRESET `, `#CONSOLE`);
- 4 reserved bytes;
- 12-byte chunk name;
- 64-bit big-endian payload length;
- 4 reserved bytes;
- payload.

A `#PRESET ` data chunk (`ChunkData` or `DATA`) contains components:

- 24-byte name (`F_*` or `D_*`);
- 16-bit magic (`DA27`, or `D927` for Monitor EQ);
- 16-bit reserved;
- 16-bit little-endian payload length;
- 16-bit reserved;
- payload.

`F_*` is a schema: a short group header followed by 4-byte records
`address_hi, address_lo, 0x01, width`. The records follow the four-byte marker
`41 01 00 80`; the bytes before the marker are the group header. Mic Input and
Monitor EQ have six header bytes before the marker; Game Effect and the voice
preset have two. The parser finds the marker and starts at `marker + 4`, rather
than assuming a fixed header length.

`D_*` carries a display name, then the values in schema order, each 1 or 2
bytes. The name field has a different length per block, so the value blob is
located from the end: `start = len(D) - sum(widths)`. That rule makes Mic
Input, Monitor EQ and Game Effect decode as plain little-endian device values.

The `.zg01` backup's `F_CurrentScene` is a multi-group schema: it has several
`41 01 00 80` markers and short group trailers. Joining all scene groups to
their values is a follow-up, so this phase rejects the backup magic outright
instead of reporting a partial decode. Only `.zgpreset` is a load target.

The parser resolves addresses through the committed device map
(`tools/vendor/device-map-zg01.json`) and prints the device name, the component
block and the raw value. Preset metadata (`#PRESET INFO`, `#FILE`, timestamps,
UUID, block type) is retained in the parse result.

### 2. Python reference parser

Add `tools/zgpreset.py`:

```
zgpreset.py list FILE
zgpreset.py show FILE
zgpreset.py dump FILE --json
zgpreset.py diff FILE_A FILE_B
```

The parser is a library plus a CLI; tests import the library. It accepts
`.zgpreset` only and rejects `.zg01` with a clear `backup not supported`
error. `show` prints the preset name, block, every parameter, its address and
raw value. `dump --json` is the stable interchange format used to compare the
Python and C implementations.

The decode work includes the Game Effect flag/packing rule. The 26 factory
presets from the vendor package are used locally as additional ground truth;
committed tests use synthetic fixtures and derived expectations, not the
factory or user files.

### 3. C parser and `zgctl` integration

Add `tools/zgpreset.c` and `tools/zgpreset.h`, built by `make -C tools` and
linked into `zgctl`. It uses no external libraries. The public API is:

```c
struct zgpreset;
int zgpreset_open(const char *path, struct zgpreset **out);
size_t zgpreset_count(const struct zgpreset *);
const struct zgpreset_param *zgpreset_param(const struct zgpreset *, size_t);
const char *zgpreset_name(const struct zgpreset *);
const char *zgpreset_block(const struct zgpreset *);
void zgpreset_close(struct zgpreset *);
```

The C parser is a direct port of the Python structure; the test suite compares
both on the same fixtures.

`zgctl` gains:

```
zgctl preset list FILE
zgctl preset show FILE
zgctl preset load FILE [--dry-run] [--block NAME]
```

`preset list` prints the components and parameter count. `preset show` prints
the metadata and values. `preset load` applies values through the existing ALSA
controls. It does not call `Save to ZG01` unless `--save` is added later.

### 4. Control map: one source of truth

The full surface is driven by a new committed file,
`tools/vendor/control-map-zg01.json`. It has one row for each of the 375
addresses:

```json
{
  "address": 18,
  "device_name": "mic_gate_onf",
  "vendor_name": "MicGateOnf",
  "control_name": "Gate Capture Switch",
  "type": "boolean",
  "dev_min": 0,
  "dev_max": 1,
  "dev_default": 0,
  "min": 0,
  "max": 1,
  "def": 0,
  "conv": "id",
  "writable": true,
  "provenance": "vendor-model"
}
```

`dev_min`, `dev_max` and `dev_default` are the vendor model's device values.
`min` and `max` are the user-facing ALSA bounds after the `conv` conversion
(for example a 0..360 gain becomes -180..180 with `CONV_GAIN`), and `def` is
the raw device default that seeds the driver cache. For `existing` rows these
fields are copied from the current control table.

The map is built in three layers:

1. **Existing controls.** The 208 parameter rows already in
   `src/zg01_control.c` are copied in with their current names, types, bounds,
   conversions and defaults. This is a compatibility contract. The two action
   controls (`Save to ZG01`, `Reset to ZG01`) are not parameters and remain
   separate.
2. **New parameters with a vendor-model row.** The remaining writable addresses
   are joined to `tools/vendor/parameter-model-zg01.json` through a curated
   device-name/CamelCase correspondence. The map carries the vendor model's
   bounds and default; the control is exposed with those bounds and
   `CONV_ID` unless a block-specific unit is established.
3. **Addresses with no usable model row.** Only these are marked
   `provenance: "raw"`, with `min = 0`, `max = ZG01_VALUE_MAX`,
   `conv = "id"` and `default = 0`. The map must list each raw address; the
   generator refuses to leave an address unclassified.

`tools/vendor/parameter-model-zg01.json` is a derived artifact generated from
`MemAPI_ZG01.dll` with the existing extractor. It carries the CamelCase name,
min, max and default. It is committed as derived facts; the DLL is not.

Two headers are generated from the control map:

- `src/zg01_controls_generated.h` for the kernel module;
- `tools/zg01_controls_generated.h` for `zgctl`/`zgpreset`.

The driver includes the kernel header; userspace includes the tools header. A
contract test regenerates both and compares them, so the two cannot drift.
The generator also emits an address-to-control-name lookup used by
`zgpreset`/`zgctl`.

New control names must not end in `Gain`, `Frequency` or `Q`; those suffixes
are how `zgctl` recognizes the existing human-unit controls. New names use
`<Block> <Parameter> Steps` (for example `Binaural Voice Position Steps`) and
are classified as raw integers. The existing 208 names and units must not
change.

The full write path uses the existing `CONV_ID`/`CONV_GAIN`/`CONV_FREQ`/
`CONV_Q` conversions. Parameters whose unit is not established are named with
the `Steps` suffix and are classified as raw integers, so `zgctl` prints them
as device steps, not as dB or Hz.

### 5. Read-only exclusions

A parameter is exposed as writable only when the vendor model and the device
map both describe a mixer value. The following are excluded from the control
map and are reported by the parser but skipped by `preset load`:

- `dirty_flag_mic`, `dirty_mic_fx`, `dirty_game_fx`, `dirty_monitor`,
  `dirty_streming`;
- `mic_level`, `mic_monitor_level_knob`, `mon_level`;
- any address added to the exclusion table with a written justification.

The initial list is deliberately small. `strm_*_lvl`, `mon_vic_lvl`,
`mon_gam_lvl`, `hdmi_mute`, `streaming_*_link` and `to_pc_mute_*` are mixer
settings and stay writable. The monitor/headphone block is writable except for
the two read-only entries above. If a later state-dump decode proves another
parameter is a meter or status flag, it moves to the exclusion table with a
test.

### 6. Preset loading

`preset load` performs:

1. Parse the file.
2. Normalize `--block` against the `#PRESET INFO` block type (case-insensitive,
   whitespace-normalized). The canonical labels are `MIC INPUT`,
   `MONITOR EQ`, `GAME EFFECT` and `MIC EFFECT`; a file may contain more than
   one matching component, and all matches are loaded. No match is a usage
   error.
3. For each parameter, look up the control name by address.
4. If the address is excluded or missing, record `unsupported`.
5. Convert the device value to the control's user unit:
   - `CONV_ID`: unchanged;
   - `CONV_GAIN`: `device - 180`;
   - `CONV_FREQ`: unchanged;
   - `CONV_Q`: inverse of the driver's Q encoding.
6. Check the control's min/max; if out of range, record `out_of_range` and skip.
7. If the control is `raw` and `--allow-raw` was not given, record `raw` and
   skip.
8. In `--dry-run`, print the complete plan and exit:
   - `0` if every parameter would be applied;
   - `1` if any parameter would be skipped;
   - `2` for a parse or usage error.
9. Otherwise write each value with the existing ALSA element write path and
   print a summary. Exit `1` if any parameter was skipped or failed.

Load is best-effort and not transactional. A failure leaves earlier writes in
place; the summary identifies them. `--block` limits loading to one block.
`--allow-raw` is the explicit opt-in for addresses whose vendor-model row is
missing.

`preset load` does not persist. The user can follow with `zgctl save`. The
CLI accepts `--dry-run`, `--block NAME` and `--allow-raw`; no `--save` flag is
added in this phase.

### 7. Tests and evidence

Python tests:

- synthetic MBDF fixtures for chunk boundaries, component lengths, names and
  values;
- Mic Input and Monitor EQ expected values;
- Game Effect expected values after the packing rule is pinned;
- malformed/truncated files are rejected with a precise error.

C tests:

- parser output matches the Python JSON on shared fixtures;
- `zgctl preset show/list` output is stable;
- `preset load --dry-run` maps every address and reports exclusions.

Driver tests:

- generated table has 375 unique addresses;
- existing 208 control names, ranges, conversions and defaults are unchanged;
- every writable address has a control; every excluded address is intentional;
- `make W=1` has no warning from the module;
- ALSA control names are unique and within 44 bytes.

Hardware tests:

- load a Mic Input preset and compare `zgctl get` with the parsed values;
- load a Monitor EQ and a Game Effect preset and check the kernel log;
- verify `zgctl preset load --dry-run` before each write.

## Phases and definition of done

### Phase 0: reference parser

- `tools/zgpreset.py` parses all six user presets and the 26 factory presets.
- The `start = len(D) - sum(widths)` rule is documented and tested.
- Mic Input, Monitor EQ and Game Effect values are plausible and consistent
  across the presets.
- Tests and fixture generation are committed.

### Phase 1: C parser and read-only CLI

- `tools/zgpreset.c` produces the same JSON as Python on the shared fixtures.
- `zgctl preset list/show` work on `.zgpreset`; `.zg01` is rejected with a
  clear error.
- No write path is added yet.

### Phase 2: full control surface

- `tools/vendor/parameter-model-zg01.json` is generated from the vendor library
  and committed.
- `tools/vendor/control-map-zg01.json` classifies all 375 addresses: existing
  control, vendor-model control, raw control or read-only exclusion. The
  resulting surface is 367 writable controls and 8 read-only exclusions.
- Generated headers and generator are committed.
- All 375 addresses are either a control or a documented exclusion.
- The 208 existing controls are byte-for-byte compatible in name, range and
  conversion.
- `zgctl list` exposes every writable control.
- `make W=1` and the control contract tests pass.

### Phase 3: preset loading

- `zgctl preset load --dry-run` reports the full plan and exclusions.
- Mic Input, Monitor EQ and Game Effect presets load on hardware.
- Out-of-range values are reported and skipped, never clamped.
- Raw addresses require `--allow-raw`; without it they are reported and
  skipped.
- `preset load` does not save.
- Documentation covers the address-to-control mapping and the read-only
  exclusions.

### Phase 4: follow-ups

- block-by-block units for the raw controls where the vendor model establishes
  them;
- export to `.zgpreset` (nice to have);
- state dump and read-only status, if pursued.

## Decisions left to the builder

- The exact display-name expansion dictionary for device snake_case names. The
  names must be unique, under 44 bytes, and must not end in `Gain`,
  `Frequency` or `Q`.
- Whether the full `.zg01` backup becomes a first-class load target. This
  phase treats `.zgpreset` as the only load target and rejects `.zg01`; the
  multi-group scene decode is a follow-up.
- Whether `preset load` should have a `--save` convenience flag. This spec
  leaves it out; `zgctl save` remains explicit.
- The exact implementation of the device-name/CamelCase join for the remaining
  vendor-model rows. Every row must cite either a vendor-model provenance or a
  raw exception; a heuristic match is not sufficient without a contract test.
