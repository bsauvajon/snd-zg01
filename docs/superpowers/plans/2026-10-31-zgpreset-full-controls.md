# Full Parameter Surface and `.zgpreset` Loading Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a complete `.zgpreset` parser, expose all 375 addressable ZG01 parameters as ALSA controls, and load a preset file through `zgctl`.

**Architecture:** A Python reference parser settles and documents the MBDF format and generates test expectations. A C port is linked into `zgctl`. A committed control map joins the device map to the vendor parameter model and generates the kernel and userspace control tables. `zgctl preset load` maps preset addresses to controls and applies values through the existing ALSA element API.

**Tech Stack:** C11 userspace (`zgctl`, `zgpreset`), Python 3 (reference parser and generators), Linux kernel module (C), ALSA control ioctls, shell tests.

**Spec:** `docs/superpowers/specs/2026-10-31-zgpreset-full-controls-design.md`

## Global Constraints

- The existing 208 parameter controls keep their exact names, types, bounds, conversions and defaults. The two action controls (`Save to ZG01`, `Reset to ZG01`) remain separate.
- All 375 addresses from `tools/vendor/device-map-zg01.json` are either a writable control or a documented read-only exclusion.
- ALSA control names are unique and at most 44 bytes.
- New controls that do not use an established unit are named with a `Steps` suffix and classified as raw integers. They must not end in `Gain`, `Frequency` or `Q`.
- Preset values outside a control's range are reported and skipped, never clamped.
- `preset load` does not persist; `zgctl save` remains explicit.
- Raw controls require `--allow-raw` for preset loading.
- Vendor binaries, factory presets and user presets stay out of Git. Derived JSON and generated headers may be committed.
- The driver build must pass `make W=1` with no warning from the module.

## Review Focus

- **Malformed MBDF lengths:** a chunk or component length beyond the file, a truncated header, or a zero-length component must return a clear error and must not read out of bounds. The owning tests are in Tasks 1 and 8.
- **Wrong value-blob start:** the `D_*` display-name field length differs by block; using a fixed offset silently shifts every Game Effect value. The owning tests are in Tasks 2 and 8.
- **Out-of-range preset values:** a preset value above the control's maximum, or a raw address without `--allow-raw`, must be reported and skipped. The owning test is in Task 10.
- **Address and name collisions in the 375-row table:** page 0 offset `0x24` and page 1 offset `0x24` are different parameters, and generated names must not collide with the existing 208. The owning tests are in Tasks 4, 5 and 6.
- **Partial load visibility:** a load that stops halfway must identify applied, skipped and failed entries; no rollback is promised. The owning test is in Task 11.

---

### Task 0: Feature branch and documents

**Files:**
- Existing: `docs/superpowers/specs/2026-10-31-zgpreset-full-controls-design.md`
- Existing: `docs/superpowers/plans/2026-10-31-zgpreset-full-controls.md`

- [ ] **Step 1: Create the feature branch**

From the isolated worktree that contains the design and plan:

```bash
git switch -c feat/zgpreset-controls
git status --short
```

Expected: only the two `docs/superpowers/` files are untracked.

- [ ] **Step 2: Commit the design and plan**

```bash
git add docs/superpowers/specs/2026-10-31-zgpreset-full-controls-design.md \
        docs/superpowers/plans/2026-10-31-zgpreset-full-controls.md
git commit -m "docs: specify the full parameter surface and preset loading"
```

### Task 1: Python MBDF container reader

**Files:**
- Create: `tools/zgpreset.py`
- Create: `tests/test-zgpreset.py`
- Create: `tests/test-zgpreset.sh`
- Create: `tests/fixtures/zgpreset/README.md`

**Interfaces:**
- Produces: `PresetError(Exception)`.
- Produces: `Chunk(tag: str, name: str, payload: bytes)`.
- Produces: `read_chunks(data: bytes) -> list[Chunk]`.
- Produces: `Component(name: str, magic: int, payload: bytes)`.
- Produces: `read_components(payload: bytes) -> list[Component]`.

- [ ] **Step 1: Write the failing tests**

In `tests/test-zgpreset.py`, build synthetic MBDF bytes in memory. Assert:

```python
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

def test_truncated_chunk_is_rejected():
    data = make_mbdf([("#FILE", "x", b"")])
    try:
        read_chunks(data[:-1])
    except PresetError:
        pass
    else:
        raise AssertionError("truncated chunk was accepted")
```

Use plain `assert` and a local helper; do not add pytest as a dependency.

- [ ] **Step 2: Run the test to verify it fails**

Run: `python3 tests/test-zgpreset.py`
Expected: `ImportError: cannot import name 'read_chunks'` or the missing function.

- [ ] **Step 3: Implement the container reader**

In `tools/zgpreset.py`, parse the fixed 0x74-byte header, then read chunks:

```python
class PresetError(Exception): ...
@dataclass
class Chunk:
    tag: str
    name: str
    payload: bytes

def read_chunks(data: bytes) -> list[Chunk]: ...
def read_components(payload: bytes) -> list[Component]: ...
```

Chunk layout: 8-byte tag, 4 reserved bytes, 12-byte name, 64-bit big-endian length, 4 reserved bytes, payload. Component layout: 24-byte name, 16-bit big-endian magic, 16-bit reserved, 16-bit little-endian length, 16-bit reserved, payload. Reject any length that exceeds the remaining bytes, any component with an unknown magic, and any trailing bytes that cannot form a component.

- [ ] **Step 4: Run the test to verify it passes**

Run: `python3 tests/test-zgpreset.py`
Expected: all container tests pass.

- [ ] **Step 5: Add the shell wrapper and commit**

`tests/test-zgpreset.sh` runs `python3 tests/test-zgpreset.py`.

```bash
chmod +x tests/test-zgpreset.sh
bash tests/test-zgpreset.sh
git add tools/zgpreset.py tests/test-zgpreset.py tests/test-zgpreset.sh tests/fixtures/zgpreset/README.md
git commit -m "feat(zgpreset): read the MBDF container"
```

### Task 2: Python component and value decoder, plus `list`/`show`

**Files:**
- Modify: `tools/zgpreset.py`
- Modify: `tests/test-zgpreset.py`
- Create: `tests/fixtures/zgpreset/mic-input.json` (expected decode of a synthetic fixture; generated in the test)

**Interfaces:**
- Produces: `SchemaEntry(address: int, width: int)`.
- Produces: `Param(address: int, width: int, value: int)`.
- Produces: `Preset(name: str, block: str, params: list[Param])`.
- Produces: `parse_preset(path: str) -> Preset`.
- Produces: `decode_values(schema: list[SchemaEntry], d_payload: bytes) -> list[Param]`.
- Produces: `main(argv: list[str]) -> int` with `list FILE`, `show FILE`,
  `dump FILE` (JSON) and `diff FILE_A FILE_B`.

- [ ] **Step 1: Write the failing tests**

Add tests for:

```python
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

def test_game_schema_uses_the_marker():
    # Game Effect has a two-byte header before the marker, not ten.
    f = bytes.fromhex("00 73 41 01 00 80") + bytes.fromhex("00 74 01 01")
    assert parse_schema(f) == [SchemaEntry(0x074, 1)]
```

The test must include a 2-byte value and a 1-byte value, and it must assert the `start = len(D) - sum(widths)` rule. It must also cover the two-byte and six-byte pre-marker headers.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `python3 tests/test-zgpreset.py`
Expected: missing `parse_schema`, `decode_values` or `Param`.

- [ ] **Step 3: Implement the schema and value decoder**

`F_*` payload: find the four-byte marker `41 01 00 80`, then read 4-byte
records `address_hi`, `address_lo`, `0x01`, `width` from `marker + 4`. The
header before the marker is short and differs by block (six bytes for Mic
Input/Monitor EQ, two for Game Effect/voice presets). Reject width values
other than 1 and 2. The `D_*` value start is `len(payload) - sum(widths)`;
read each value little-endian with its width. The `D_*` prefix is a display
name and is not interpreted here. `parse_preset` reads `#PRESET INFO` for the
block type and display name, then pairs each `F_*` with its `D_*`. `.zg01`
backups are out of scope in this task: the parser recognizes the backup
magic and rejects it with a clear `backup not supported` error, because the
multi-group `F_CurrentScene` scene join is a follow-up.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `python3 tests/test-zgpreset.py`
Expected: all tests pass.

- [ ] **Step 5: Add `list`, `show`, `dump` and `diff` CLI tests and implement them**

`zgpreset.py list FILE` prints one line per component:
`F_MicEq 20 params`, `D_MicEq 20 values`.
`zgpreset.py show FILE` prints the block, preset name, then
`0x120 mic_eq_v2_1_ONF = 1` per parameter. Use
`tools/vendor/device-map-zg01.json` for the device name; if the address is
unknown, print `unknown`.
`zgpreset.py dump FILE` prints the same data as stable JSON:
`{"block": "...", "name": "...", "params": [{"address": 288, "value": 1}]}`.
`zgpreset.py diff FILE_A FILE_B` prints one line per changed address:
`0x128 150 -> 211`.

Test all four commands with
`subprocess.run([sys.executable, "tools/zgpreset.py", ...])` against temporary
synthetic files, and assert the exact stdout lines.

- [ ] **Step 6: Add a `.zg01` backup rejection test**

Build a synthetic `#YAMAHA MBDFBackup` with `#CONSOLE` / `ChunkData` /
`F_CurrentScene` + `D_CurrentScene`, then assert `read_chunks` and
`parse_preset` reject it with a `backup not supported` error. Backup
decoding is deferred; this pins the scope instead of claiming support.

- [ ] **Step 7: Run the tests and commit**

```bash
bash tests/test-zgpreset.sh
git add tools/zgpreset.py tests/test-zgpreset.py tests/test-zgpreset.sh
git commit -m "feat(zgpreset): decode preset components and values"
```

### Task 3: Derived vendor parameter model

**Files:**
- Create: `tools/vendor/build-parameter-model.py`
- Create: `tools/vendor/parameter-model-zg01.json`
- Create: `tests/test-parameter-model.sh`

**Interfaces:**
- `build-parameter-model.py --dll PATH --out PATH` writes a JSON object keyed by CamelCase name:
  `{ "name": {"min": int, "max": int, "default": int} }`.
- The committed file contains the CamelCase rows, excluding every legacy snake_case row and every `_ForBackup` duplicate, except the two mirrors (`GameFxType`, `StreamingMicLevel`) that are the only row for their parameter: those are published under the base name.

- [ ] **Step 1: Write the failing contract test**

`tests/test-parameter-model.sh` uses Python to assert:

- the file exists and is valid JSON;
- every value has integer `min`, `max`, `default` with
  `0 <= min <= default <= max <= ZG01_VALUE_MAX`;
- no key ends in `_ForBackup`;
- every key matches `^[A-Z][A-Za-z0-9_]*$`;
- the count is 320: 318 valid CamelCase mixer parameters plus the two rows
  whose only descriptor is a `_ForBackup` mirror (`GameFxType`,
  `StreamingMicLevel`), published under the base name. `docs/PARAMETER_INVENTORY.md`'s
  331 CamelCase names also count preset labels and display strings (`L11`,
  `R10`, `GA Classics - CRUNCH`, `REV-X Hall`, ...), which the generator
  filters out because their min/max/default are not device values.

- [ ] **Step 2: Run the test to verify it fails**

Run: `bash tests/test-parameter-model.sh`
Expected: missing `parameter-model-zg01.json`.

- [ ] **Step 3: Implement the generator and produce the committed JSON**

Reuse the parsing logic from `tools/vendor/extract-memapi-params.py`. The
generator may call that script into a temporary file, then filter and rewrite.
Run it against `/tmp/opencode/zg-vendor/app/files/MemAPI_ZG01.dll`. Commit only
the derived JSON and the generator; never the DLL.

- [ ] **Step 4: Run the test and commit**

```bash
bash tests/test-parameter-model.sh
git add tools/vendor/build-parameter-model.py tools/vendor/parameter-model-zg01.json tests/test-parameter-model.sh
git commit -m "feat(vendor): commit the derived parameter model"
```

### Task 4: Control-map schema, existing 208 rows and read-only exclusions

**Files:**
- Create: `tools/vendor/control-map-zg01.json`
- Create: `tools/vendor/build-control-map.py`
- Create: `tests/test-control-map.sh`
- Create: `tools/vendor/name-map-zg01.json`

**Interfaces:**
- Control map row:
  `{address:int, device_name:str, vendor_name:str|null, control_name:str, type:"boolean"|"integer", dev_min:int, dev_max:int, dev_default:int, min:int, max:int, def:int, conv:"id"|"gain"|"freq"|"q", writable:bool, provenance:"existing"|"vendor-model"|"raw"|"read-only", raw_reason:str|null}`.
  `dev_*` are vendor device values; `min/max` are user-facing ALSA bounds;
  `def` is the raw device default that seeds the cache.
- `name-map-zg01.json` maps `device_name` to `vendor_name` for every row that has a vendor model entry.
- `build-control-map.py` validates the map and rejects duplicate addresses, duplicate control names and a missing provenance.

- [ ] **Step 1: Write the failing map contract test**

`tests/test-control-map.sh` asserts:

- the map parses and every address is unique;
- the 208 existing rows match `src/zg01_control.c` for name, address, type,
  min, max, conv and default;
- the read-only rows are exactly the exclusion list in the spec;
- every `vendor-model` row's `dev_min`, `dev_max` and `dev_default` equal
  `parameter-model-zg01.json`, and its `min`/`max` are the conversion of those
  device bounds by `conv`;
- every `raw` row has `min=0`, `max=ZG01_VALUE_MAX`, `def=0`, `conv="id"`;
- every control name is unique and at most 44 bytes.

- [ ] **Step 2: Run the test to verify it fails**

Run: `bash tests/test-control-map.sh`
Expected: missing `control-map-zg01.json`.

- [ ] **Step 3: Seed the 208 existing rows**

Write a one-off seed step inside `build-control-map.py` (or a separate
`tools/vendor/seed-control-map.py`) that parses the rows of
`src/zg01_control.c` between `zg01_param_ctls[]` and its closing `};`. Fill the
`vendor_name` of each existing row from the expected names already listed in
`tests/test-control-addresses.sh`. Emit the 208 rows with
`provenance: "existing"`. The committed map is the source of truth after this
step; the seed tool is only for the initial import and must not be run by the
build.

- [ ] **Step 4: Add the read-only exclusions**

Add these rows with `writable: false`, `provenance: "read-only"`:
`dirty_flag_mic`, `dirty_mic_fx`, `dirty_game_fx`, `dirty_monitor`,
`dirty_streming`, `mic_level`, `mic_monitor_level_knob`, `mon_level`.
The test asserts this exact set; adding another exclusion requires updating the
test and the spec.

- [ ] **Step 5: Run the test and commit**

```bash
bash tests/test-control-map.sh
git add tools/vendor/control-map-zg01.json tools/vendor/build-control-map.py tests/test-control-map.sh tools/vendor/name-map-zg01.json
git commit -m "feat(vendor): seed the control map and read-only exclusions"
```

### Task 5: Complete the 375-row control map

**Files:**
- Modify: `tools/vendor/control-map-zg01.json`
- Modify: `tools/vendor/name-map-zg01.json`
- Modify: `tools/vendor/build-control-map.py`
- Modify: `tests/test-control-map.sh`

**Interfaces:**
- Consumes: `device-map-zg01.json` (375 rows), `parameter-model-zg01.json`
  (320 rows), `name-map-zg01.json`.
- Produces: the complete 375-row map: 208 existing, 106 vendor-model, 53 raw
  and 8 read-only rows (367 writable controls).

- [ ] **Step 1: Add the completeness tests**

The test asserts:

- every address in `device-map-zg01.json` appears exactly once in the map;
- every map row is `existing`, `vendor-model`, `raw` or `read-only`;
- every `vendor-model` row has a `vendor_name` that exists in
  `parameter-model-zg01.json`, its `dev_min`/`dev_max`/`dev_default` match that
  row, and its user `min`/`max` are the conversion of the device bounds;
- every `raw` row is listed in a `raw_reason` field (why no model row exists);
- the union of existing + vendor-model + raw + read-only is 375.

- [ ] **Step 2: Run the test to verify it fails**

Run: `bash tests/test-control-map.sh`
Expected: missing addresses, because only the 208 existing rows and the
exclusions are present.

- [ ] **Step 3: Fill the name map block by block**

Use the correspondence rule in `docs/EFFECTS_CONTROL_PLAN.md` and the 208
existing rows as anchors. Add the remaining blocks in this order:

1. Monitor / headphone: `mon_eq_*` -> `MonitorEq*`, `mon_*` volumes.
2. Streaming output: `strm_*` -> `Streaming*`, `hdmi_*` -> `Hdmi*`,
   `to_pc_*` -> `ToPc*`.
3. Binaural: `bina_*` -> `Binaural*`.
4. Preset names and current selection: `game_fx_name`, `mic_eq_name`,
   `pre_vic_name`, `mon_eq_name`, `Current*`.
5. Remaining writable parameters: classify each address as `vendor-model`
   with a name-map entry, or `raw` with a written reason.

New controls with `conv: "id"` get `<Block> <Parameter> Steps` names. Example:
`bina_v_pos` -> `Binaural Voice Position Steps`. New controls whose unit is
established use the unit suffix (`Gain`, `Frequency`, `Q`) and the map's
`conv`; for example a Monitor EQ gain is `Monitor EQ Band 1 Gain` with
`conv: "gain"`.

- [ ] **Step 4: Run the test to verify it passes**

Run: `bash tests/test-control-map.sh`
Expected: 375 rows, no duplicate address or name, every vendor row matches the
model.

- [ ] **Step 5: Commit**

```bash
git add tools/vendor/control-map-zg01.json tools/vendor/name-map-zg01.json tools/vendor/build-control-map.py tests/test-control-map.sh
git commit -m "feat(vendor): classify all 375 parameters"
```

### Task 6: Generate headers and expose all controls in the driver

**Files:**
- Create: `tools/vendor/render-controls.py`
- Create: `src/zg01_controls_generated.h`
- Create: `tools/zg01_controls_generated.h`
- Modify: `src/zg01_control.c`
- Modify: `tests/test-control-addresses.sh`
- Create: `tests/test-generated-controls.sh`

**Interfaces:**
- `render-controls.py` reads `control-map-zg01.json` and writes:
  - `src/zg01_controls_generated.h`:
    `static const struct zg01_param_ctl zg01_extra_param_ctls[] = {...};`
  - `tools/zg01_controls_generated.h`:
    `static const struct zg01_user_control zg01_user_controls[] = {...};`
- The userspace struct is:

```c
struct zg01_user_control {
    uint16_t address;
    const char *name;
    long min;
    long max;
    int conv;
    unsigned int def;
    unsigned int flags;
};
```

  with `ZG01_CTL_RAW` and `ZG01_CTL_READONLY` flags.
- The driver keeps `zg01_param_ctls[]` for the 208 existing rows and loops
  over both arrays in `zg01_create_controls`.

- [ ] **Step 1: Write the failing generated-header test**

`tests/test-generated-controls.sh` runs the generator into a temporary
directory and compares the output to the committed headers. It also asserts
that the userspace header contains all 375 addresses, that the kernel header
contains the 159 writable extras, that the writable control set is 367
(208 existing + 159 generated), and that no generated name is already present
in `zg01_param_ctls[]`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `bash tests/test-generated-controls.sh`
Expected: missing generated headers or generator.

- [ ] **Step 3: Implement the generator and generate both headers**

The kernel header emits only writable rows not already in `zg01_param_ctls[]`
(the `existing` rows). The userspace header emits all 375 rows, including the
read-only exclusions, so `zgctl` can report them. Use `CONV_ID` for
`id`, `CONV_GAIN` for `gain`, `CONV_FREQ` for `freq` and `CONV_Q` for `q`.
Emit `ZG01_CTL_RAW` for `provenance == "raw"` and `ZG01_CTL_READONLY` for
`read-only`.

- [ ] **Step 4: Integrate the kernel header**

In `src/zg01_control.c`, include `zg01_controls_generated.h` after the
`struct zg01_param_ctl` definition and the manual table. Add a helper that
iterates the manual table and the generated table. The cache seed and
`BUILD_BUG_ON` checks must cover the generated addresses.

- [ ] **Step 5: Run the driver tests**

Run:

```bash
bash tests/test-control-addresses.sh
bash tests/test-control-map.sh
bash tests/test-generated-controls.sh
bash tests/test-kernel-build.sh
make -C tools
```

Expected: all pass; the existing 208 rows are unchanged; the module builds
with `W=1`.

- [ ] **Step 6: Check the control count on hardware**

Load the module and run `./tools/zgctl list | grep -c '^numid='`.
Expected: 367 writable parameter controls + the two action controls = 369.
The eight read-only exclusions are intentionally absent from ALSA.

- [ ] **Step 7: Commit**

```bash
git add tools/vendor/render-controls.py src/zg01_controls_generated.h tools/zg01_controls_generated.h src/zg01_control.c tests/test-control-addresses.sh tests/test-generated-controls.sh
git commit -m "feat(control): expose the full 375-parameter surface"
```

### Task 7: Userspace control metadata and unit classification

**Files:**
- Modify: `tools/zgctl.c`
- Modify: `tests/test-zgctl-kinds.c`
- Modify: `tests/test-zgctl-kinds.sh` if the compile line needs the generated
  header path.

**Interfaces:**
- Consumes: `tools/zg01_controls_generated.h` from Task 6.
- Produces: `const struct zg01_user_control *zg01_user_control_by_name(const char *name);`
- `kind_of()` returns `KIND_BOOL` when `info->type == SNDRV_CTL_ELEM_TYPE_BOOLEAN`;
  otherwise it looks up the row by name and maps `conv`: `gain -> KIND_GAIN`,
  `freq -> KIND_FREQ`, `q -> KIND_Q`, `id -> KIND_INT`. The generated
  `struct zg01_user_control` has no `type` field; the ALSA type comes from
  `info`. The existing name heuristics become a fallback for the two action
  controls and for any control not in the generated map.

- [ ] **Step 1: Write the failing classification test**

Add cases to `tests/test-zgctl-kinds.c`:

```c
expect_kind(KIND_GAIN, "Monitor EQ V2 Band 1 Gain", SNDRV_CTL_ELEM_TYPE_INTEGER);
expect_kind(KIND_FREQ, "Monitor EQ V2 Band 1 Frequency", SNDRV_CTL_ELEM_TYPE_INTEGER);
expect_kind(KIND_Q, "Monitor EQ V2 Band 1 Q", SNDRV_CTL_ELEM_TYPE_INTEGER);
expect_kind(KIND_INT, "Binaural Voice Position Steps", SNDRV_CTL_ELEM_TYPE_INTEGER);
```

The existing 208 expectations must stay unchanged.

- [ ] **Step 2: Run the test to verify it fails**

Run: `bash tests/test-zgctl-kinds.sh`
Expected: Monitor EQ frequency is classified as `KIND_INT`, not `KIND_FREQ`.

- [ ] **Step 3: Implement the table lookup**

Include `tools/zg01_controls_generated.h` in `tools/zgctl.c` after the
`struct zg01_user_control` definition. Implement the name lookup and change
`kind_of` to use `info->type` for booleans and the row's `conv` for the unit.
Keep the existing `is_gain`, `is_frequency` and Q suffix checks only as the
fallback when no row matches.

- [ ] **Step 4: Run the tests and commit**

```bash
bash tests/test-zgctl-kinds.sh
bash tests/test-param-codec.sh
make -C tools
git add tools/zgctl.c tests/test-zgctl-kinds.c tests/test-zgctl-kinds.sh
git commit -m "feat(zgctl): classify units from the control map"
```

### Task 8: C parser port

**Files:**
- Create: `tools/zgpreset.c`
- Create: `tools/zgpreset.h`
- Modify: `tools/Makefile`
- Create: `tests/test-zgpreset.c`
- Create: `tests/test-zgpreset-c.sh`
- Create: `tests/zgpreset-dump.c` (comparison adapter)
- Create: `tests/test-zgpreset-compare.sh`

**Interfaces:**
- `struct zgpreset_param { uint16_t address; uint8_t width; uint32_t value; };`
- `struct zgpreset { char block[32]; char name[64]; struct zgpreset_param *params; size_t count; };`
- `int zgpreset_open(const char *path, struct zgpreset **out);`
- `void zgpreset_close(struct zgpreset *preset);`
- Returns `0` on success, `-EINVAL` for malformed input, `-ENOMEM` for allocation failure and `-errno` for I/O.

- [ ] **Step 1: Write the failing C test**

`tests/test-zgpreset.c` opens synthetic fixtures generated by
`tests/test-zgpreset-c.sh`, then asserts the block, name, count, addresses and
values. Add a Game Effect fixture whose schema header is two bytes before the
marker (it pins the marker search and the end-relative value start), a
missing-marker fixture, a truncated-schema fixture and a malformed fixture
with a length beyond the file; the malformed ones must return `-EINVAL`, and
a `.zg01` backup must return `-ENOTSUP`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `bash tests/test-zgpreset-c.sh`
Expected: compile failure or `zgpreset_open` missing.

- [ ] **Step 3: Implement the parser and build wiring**

Port the Python container/schema/value logic. Read the whole file into memory,
validate every offset and length, reject unknown component magic and width
values other than 1 or 2. The D value start is
`d_len - sum(widths)`. The parser does not map addresses to names.

Update `tools/Makefile`:

```make
zgctl: zgctl.c zgpreset.c zgpreset.h
	$(CC) $(CFLAGS) -o $@ zgctl.c zgpreset.c
```

- [ ] **Step 4: Run the C and Python tests**

Run:

```bash
bash tests/test-zgpreset-c.sh
bash tests/test-zgpreset.sh
bash tests/test-zgpreset-compare.sh
```

Expected: all pass. The comparison test builds shared fixtures, dumps each
with `tools/zgpreset.py dump` and `tests/zgpreset-dump.c`, and compares the
two JSON documents byte for byte.

- [ ] **Step 5: Commit**

```bash
git add tools/zgpreset.c tools/zgpreset.h tools/Makefile tests/test-zgpreset.c tests/test-zgpreset-c.sh tests/zgpreset-dump.c tests/test-zgpreset-compare.sh
git commit -m "feat(zgpreset): port the parser to C"
```

### Task 9: `zgctl preset list` and `preset show`

**Files:**
- Modify: `tools/zgctl.c`
- Modify: `tools/zgpreset.c`
- Modify: `tools/zgpreset.h`
- Create: `tests/test-zgctl-preset.sh`
- Modify: `tests/test-zgctl-kinds.c` only if the shared parsing helpers change.

**Interfaces:**
- `zgpreset.c` adds `const char *zgpreset_block(const struct zgpreset *);`
  and `const char *zgpreset_name(const struct zgpreset *);`.
- `zgctl` accepts `preset list FILE` and `preset show FILE` without opening an
  ALSA card.

- [ ] **Step 1: Write the failing CLI test**

`tests/test-zgctl-preset.sh` creates a synthetic preset with a Python helper
and asserts:

- `zgctl preset list FILE` prints the block, preset name and parameter count
  (the C parser exposes parameters, not component names);
- `zgctl preset show FILE` prints `block:`, `name:` and one line per parameter
  with its address and value;
- a synthetic `.zg01` backup with `F_CurrentScene`/`D_CurrentScene` is
  rejected by both commands with a clear `backup not supported` error;
- an unknown file exits `2` with a parse error;
- the command works with no ZG01 card present (the test does not open
  `/dev/snd`).

- [ ] **Step 2: Run the test to verify it fails**

Run: `bash tests/test-zgctl-preset.sh`
Expected: `preset` is not a recognized command.

- [ ] **Step 3: Restructure `main` and implement the commands**

Parse `preset list/show` before `find_card()` and return without opening the
control device. Use `zgpreset_open`; print a clear error and return `2` on a
parse failure. The `show` command maps addresses to device names from
`tools/zg01_controls_generated.h`; unknown addresses print `unknown`.

- [ ] **Step 4: Run the test and commit**

```bash
bash tests/test-zgctl-preset.sh
make -C tools
git add tools/zgctl.c tools/zgpreset.c tools/zgpreset.h tests/test-zgctl-preset.sh
git commit -m "feat(zgctl): add preset list and show"
```

### Task 10: `zgctl preset load --dry-run`

**Files:**
- Modify: `tools/zgctl.c`
- Create: `tests/test-zgctl-load-dry.sh`

**Interfaces:**
- `zgctl preset load FILE [--dry-run] [--block NAME] [--allow-raw]`
- `--block` is case-insensitive and whitespace-normalized; canonical labels
  are `MIC INPUT`, `MONITOR EQ`, `GAME EFFECT`, `MIC EFFECT`.
- Dry-run exit codes: `0` all applicable, `1` something would be skipped,
  `2` parse or usage error.
- The command prints one line per parameter:
  `0x120 mic_eq_v2_1_ONF -> Mic EQ Band 1 Switch Steps = 1 (apply)` or
  `0x074 bina_v_onf -> Binaural Voice Switch Steps = 1 (raw, skipped)`.

- [ ] **Step 1: Write the failing dry-run tests**

`tests/test-zgctl-load-dry.sh` creates a synthetic Mic Input preset and
asserts:

- a fully supported preset exits `0` and prints `apply` for every parameter;
- an out-of-range value exits `1` and prints `out_of_range`;
- a raw address exits `1` without `--allow-raw`, and `0` with it;
- `--block "monitor eq"` on a Mic Input file exits `2`;
- malformed input exits `2`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `bash tests/test-zgctl-load-dry.sh`
Expected: `preset load` not implemented.

- [ ] **Step 3: Implement the mapping and dry-run**

Add a lookup by address over `zg01_user_controls[]`. Implement device-to-user
conversion:

```c
case CONV_GAIN: user = (long)value - 180; break;
case CONV_FREQ: user = (long)value; break;
case CONV_Q:    user = zg01_q_user_at_least(value); break;
default:        user = (long)value; break;
```

Include `src/zg01_codec.h` for the Q inverse. Check min/max; raw requires
`--allow-raw`; excluded/read-only rows are `unsupported`. Do not open the card
for dry-run.

- [ ] **Step 4: Run the test and commit**

```bash
bash tests/test-zgctl-load-dry.sh
bash tests/test-zgctl-preset.sh
git add tools/zgctl.c tests/test-zgctl-load-dry.sh
git commit -m "feat(zgctl): plan preset loads with dry-run"
```

### Task 11: Actual preset load, hardware verification and docs

**Files:**
- Modify: `tools/zgctl.c`
- Create: `tests/test-preset-load.sh`
- Create: `docs/PRESET_FORMAT.md`
- Modify: `README.md`
- Modify: `docs/MIC_CONTROL_PROTOCOL.md` (link to the format document)
- Modify: `docs/FEATURE_PARITY.md` (presets move from missing to supported)
- Modify: `tests/test-zgctl-load-dry.sh`
- Modify: `.github/workflows/ci.yml` (run the new contract tests)

**Interfaces:**
- Actual `preset load` opens the card after the dry-run plan is built, writes
  each applicable control, and prints `applied=N skipped=N failed=N`.
- Exit `1` if any parameter was skipped or failed; `0` otherwise.
- `--block` and `--allow-raw` have the same semantics as dry-run.

- [ ] **Step 1: Write the failing hardware-independent test**

Extend `tests/test-zgctl-load-dry.sh` with a test that runs the real path
against a mock ALSA control device? Do not add a mock device. Instead, add a
unit test in `tests/test-zgctl-kinds.c` for the conversion helpers, and keep
the hardware command in `tests/test-preset-load.sh` guarded by the presence of
`/dev/snd/controlC*` and `ZG01_PRESET`.

`tests/test-preset-load.sh` must skip with a clear message when the card or the
preset file is absent. Before the real preset it loads a synthetic
all-skipped preset (a raw row and an out-of-range row) so the partial-load
summary is exercised with `applied=0 skipped=2 failed=0` and exit `1` without
writing a control. `failed>0` is documented as a driver-error path, not
simulated with a test-only control name.

- [ ] **Step 2: Run the test to verify it fails**

Run: `bash tests/test-preset-load.sh`
Expected: skipped without a card, or `preset load` not writing.

- [ ] **Step 3: Implement the write path**

For each applicable parameter, find the control by name, convert the value,
check the range and call the existing `elem_write` helper. Print progress and
the final summary. Do not call `Save to ZG01`.

- [ ] **Step 4: Run the local tests**

```bash
bash tests/test-zgctl-load-dry.sh
bash tests/test-zgctl-kinds.sh
bash tests/test-zgpreset-c.sh
bash tests/test-zgctl-preset.sh
make -C tools
```

Expected: all pass.

- [ ] **Step 5: Add the contract tests to CI**

Add `tests/test-control-map.sh`, `test-generated-controls.sh`,
`test-parameter-model.sh`, `test-zgpreset.sh`, `test-zgpreset-c.sh`,
`test-zgpreset-compare.sh`, `test-zgctl-preset.sh`, `test-zgctl-load-dry.sh`
and `test-preset-load.sh` to `.github/workflows/ci.yml`. The hardware test
skips cleanly without a card, so CI can run it.

- [ ] **Step 6: Run the hardware acceptance**

With the ZG01 connected and the module loaded:

```bash
./tools/zgctl preset load --dry-run /path/to/mic.zgpreset
./tools/zgctl preset load /path/to/mic.zgpreset
./tools/zgctl get 'EQ Band 1 Gain'
sudo dmesg | tail
```

Expected: the dry-run lists every parameter; the load reports
`failed=0`; the getter matches the preset value; no `zg01` error in `dmesg`.
Repeat with a Monitor EQ and a Game Effect preset.

- [ ] **Step 7: Write the format document and update the README**

`docs/PRESET_FORMAT.md` records the container, component, schema, value-start
and address-mapping facts from the spec. `README.md` documents
`zgctl preset list/show/load`, `--dry-run`, `--block`, `--allow-raw` and the
read-only exclusions.

- [ ] **Step 8: Commit**

```bash
git add tools/zgctl.c tests/test-preset-load.sh tests/test-zgctl-load-dry.sh docs/PRESET_FORMAT.md README.md docs/MIC_CONTROL_PROTOCOL.md docs/FEATURE_PARITY.md .github/workflows/ci.yml
git commit -m "feat(zgctl): load zgpreset files"
```

## Self-Review

- Spec coverage: parser (Tasks 1, 2, 8), parameter model and control map
  (Tasks 3, 4, 5), full driver surface (Task 6), userspace units (Task 7),
  CLI list/show (Task 9), dry-run (Task 10), actual load and docs (Task 11).
- Every task has a test that fails before the implementation and a command that
  proves it passes.
- The review-focus failure modes are pinned: malformed lengths (Tasks 1, 8),
  value-blob start (Tasks 2, 8), out-of-range/raw (Task 10), address/name
  collisions (Tasks 4, 5, 6), partial load visibility (Task 11).
- Types and names are consistent: `zgpreset_open`/`zgpreset_close`, `Preset`,
  `Param`, `zg01_user_controls[]`, `ZG01_CTL_RAW`, `ZG01_CTL_READONLY`.
