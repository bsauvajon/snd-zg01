# ZG01 card meters multichannel — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Decode the ZG01's 34 meter channels and expose six meter groups through ALSA, with eight values for the surround groups, then display them in the GUI strips and a new Meters page.

**Architecture:** The kernel parser fills 34 channel slots from the 68 current/peak values of the 192-byte `0x83` frame. Six read-only volatile ALSA controls publish one or eight values per group. The Rust core reads the arrays, detects a legacy driver (four one-value controls) without remapping its values, and the GTK widget draws a card bank plus the existing PipeWire bar.

**Tech Stack:** C kernel module, Python reference decoder, Rust 2021, `zg01-core`, GTK4/libadwaita GUI.

**Spec:** `docs/superpowers/specs/2026-11-11-zg01-card-meters-multichannel-design.md`

## Global Constraints

- The frame is 192 bytes, 48 USB-MIDI packets, a 144-byte SysEx payload, 68 14-bit values.
- The 68 values are 34 channels × (current, peak). Channel `c` current is field `2*c`; group bases in **field** indices: mic in 0, mic out 2, PC in 4, HDMI in 20, game mix 36, stream mix 52.
- Each channel current is payload index `7 + 2*(base + 2*c)` high and the next byte low; `level = clamp(round((code - 16384)/10) + 127, 0, 127)`.
- ALSA controls: `Meter Mic In` (slot 0, count 1), `Meter Mic Out` (slot 1, count 1), `Meter PC In` (slot 2, count 8), `Meter HDMI In` (slot 10, count 8), `Meter Game Mix` (slot 18, count 8), `Meter Stream Mix` (slot 26, count 8). Slots are the 34-channel array, not frame values.
- The controls are `INTEGER`, 0–127, `READ | VOLATILE`, no put. A missing or stale frame is `-ENODATA`, never zero.
- A rejected frame leaves the previous snapshot untouched.
- The GUI never treats `-ENODATA`, a missing control, or a legacy value as zero.
- The legacy driver (no `Meter Game Mix`/`Meter HDMI In`, one value for `Meter PC In`) keeps the old four-meter layout and labels; its values are **not** remapped into the new groups.
- The VOICE strip uses `pc_in` channels 0/1 until the ZG Controller's VOICE meter is confirmed; the Meters page shows every group.
- The Linux meter-session re-arm after suspend is out of scope for this plan; the card meters are unit-tested with fixtures and the Windows captures until that separate phase.
- No new build or runtime dependency; no packaging change.

## Review Focus

- **Field indexing.** A one-off error reads field `c` instead of `2*c`; every group test must pin the known fixture values and every group's array, not only mic.
- **Legacy mislabeling.** An old driver's `Meter PC In` is `stream_mix` channel 2, not PC In channel 0; the legacy layout must keep the old presentation and never remap.
- **Unknown versus zero.** A control that is missing, `-ENODATA`, or short must render as unknown/dashed; a valid frame at the floor is a solid zero.
- **Array/count mismatch.** A partial driver (some count-8, some count-1 controls) must be V2 with missing groups unknown, not a crash and not a silent remap.
- **ALSA ABI change.** `Meter PC In`/`Meter Stream Mix` change from one value to eight; `amixer` and the GUI must read arrays, and the docs must say so.

## File Structure

- `src/zg01_meters.h` — pure frame parser and the 34-channel value struct.
- `src/zg01_meters.c` — URB reader, 34-level atomic snapshot, start/stop.
- `src/zg01.h` — meter slot constants and state.
- `src/zg01_control.c` — six meter kcontrols with counts.
- `tools/decode-meters.py` — reference decoder, groups and array CLI.
- `tests/test-decode-meters.py`, `tests/fixtures/meters/` — Python fixtures and tests.
- `tests/test-zg01-meters.c`, `tests/test-zg01-meters.sh` — C parser contract.
- `tests/test-zg01-meter-controls.sh` — hardware control registration/count check.
- `gui/crates/zg01-core/src/alsa.rs` — element count and multi-value reads.
- `gui/crates/zg01-core/src/meters.rs` — meter ids, layout, snapshot, poller.
- `gui/crates/zg01-core/src/mixage.rs` — `MeterId` and strip mapping.
- `gui/crates/zg01-gui/src/widgets/meter.rs` — card bank widget.
- `gui/crates/zg01-gui/src/ui/mixage.rs`, `mic.rs` — strip and Micro readings.
- `gui/crates/zg01-gui/src/ui/meters.rs` — new Meters page.
- `gui/crates/zg01-gui/src/ui/mod.rs`, `app.rs`, `window.rs` — page wiring.
- `docs/METER_PROTOCOL.md`, `README.md`, `docs/GUI_TEST.md`, `docs/METER_TEST.md`, `gui/README.md` — contract and acceptance.

---

### Task 0: Record the design and the plan

**Files:**
- Add: `docs/superpowers/specs/2026-11-11-zg01-card-meters-multichannel-design.md`
- Add: `docs/superpowers/plans/2026-11-11-zg01-card-meters-multichannel.md`

- [ ] **Step 1: Commit the documents**

```bash
git add docs/superpowers/specs/2026-11-11-zg01-card-meters-multichannel-design.md \
        docs/superpowers/plans/2026-11-11-zg01-card-meters-multichannel.md
git commit -m "docs(gui): design the multichannel card meters"
```

---

### Task 1: Python decoder v2 and synthetic fixtures

**Files:**
- Modify: `tools/decode-meters.py`
- Modify: `tests/test-decode-meters.py`
- Modify: `tests/fixtures/meters/frames.jsonl`
- Modify: `tests/fixtures/meters/frames.h`
- Modify: `tests/fixtures/meters/README.md`

**Interfaces:**
- Produces: `GROUP_BASES = {"mic_in": 0, "mic_out": 2, "pc_in": 4, "hdmi_in": 20, "game_mix": 36, "stream_mix": 52}` and `GROUP_CHANNELS = {"mic_in": 1, "mic_out": 1, "pc_in": 8, "hdmi_in": 8, "game_mix": 8, "stream_mix": 8}`.
- Produces: `decode_frame(data: bytes) -> dict[str, list[int]]` returning one list per group, values 0..127.
- Produces: test helper `synthetic_frame(groups: dict[str, list[int]]) -> bytes` that builds a valid 192-byte frame with the requested control values.

- [ ] **Step 1: Write the failing tests**

Add to `tests/test-decode-meters.py`:

```python
from decode_meters import decode_frame, synthetic_frame

def test_every_group_decodes_at_its_own_base():
    frame = synthetic_frame({
        "mic_in": [50],
        "mic_out": [55],
        "pc_in": [60, 61, 62, 63, 64, 65, 66, 67],
        "hdmi_in": [70, 71, 72, 73, 74, 75, 76, 77],
        "game_mix": [80, 81, 82, 83, 84, 85, 86, 87],
        "stream_mix": [90, 91, 92, 93, 94, 95, 96, 97],
    })
    assert decode_frame(frame) == {
        "mic_in": [50],
        "mic_out": [55],
        "pc_in": [60, 61, 62, 63, 64, 65, 66, 67],
        "hdmi_in": [70, 71, 72, 73, 74, 75, 76, 77],
        "game_mix": [80, 81, 82, 83, 84, 85, 86, 87],
        "stream_mix": [90, 91, 92, 93, 94, 95, 96, 97],
    }

def test_the_old_pc_in_and_stream_mix_are_stream_channels():
    values = decode_frame(bytes.fromhex(MIC_TAP))
    assert values["mic_in"] == [118]
    assert values["mic_out"] == [121]
    assert values["stream_mix"] == [121, 121, 38, 38, 0, 0, 126, 126]
    assert values["pc_in"] == [0] * 8
    assert values["hdmi_in"] == [0] * 8
    assert values["game_mix"] == [0] * 8
```

`MIC_TAP` is the existing `mic-tap` fixture hex. Keep the existing `has_meter_framing` rejection tests.

- [ ] **Step 2: Run the test to verify it fails**

Run: `python3 tests/test-decode-meters.py`
Expected: `ImportError` or `KeyError` for `synthetic_frame`/group keys.

- [ ] **Step 3: Implement the group map and the frame builder**

Replace the four field constants with `GROUP_BASES`/`GROUP_CHANNELS`. Decode channel `c` of group `g` at field `GROUP_BASES[g] + 2*c`; the payload index is `7 + 2*field`, and the raw offset helper is `raw = 4*(payload // 3) + 1 + payload % 3`. `decode_frame` returns the six lists. Add `synthetic_frame` to the test module (not the tool) that starts from a frame of 48 CIN-`04` packets, writes the header, the requested 14-bit codes, and the final CIN-`07`/`F7`.

Update the `decode` CLI to print one line per group and channel. Update `frames.jsonl` with a synthetic `all-groups` fixture and regenerate `frames.h` with the same arrays; update the fixture README.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `python3 tests/test-decode-meters.py`
Expected: all decoder tests pass.

- [ ] **Step 5: Commit**

```bash
git add tools/decode-meters.py tests/test-decode-meters.py tests/fixtures/meters/
git commit -m "feat(meters): decode the 34 card meter channels"
```

---

### Task 2: Kernel parser with 34 channels

**Files:**
- Modify: `src/zg01_meters.h`
- Modify: `tests/test-zg01-meters.c`
- Modify: `tests/fixtures/meters/frames.h`

**Interfaces:**
- Produces: `#define ZG01_METER_CHANNELS 34`, `#define ZG01_METER_GROUP_CHANNELS 8`.
- Produces: `struct zg01_meter_values { int mic_in; int mic_out; int pc_in[8]; int hdmi_in[8]; int game_mix[8]; int stream_mix[8]; };`
- Produces: `static inline int zg01_meter_channel(const unsigned char *buf, unsigned int base, unsigned int channel);` returning the 0..127 control value.
- Keeps: `zg01_meter_parse(buf, len, out)`.

- [ ] **Step 1: Write the failing test**

Replace the four assertions in `tests/test-zg01-meters.c` with group assertions:

```c
assert(zg01_meter_parse(frame_mic_tap, sizeof(frame_mic_tap), &v) == 0);
assert(v.mic_in == 118 && v.mic_out == 121);
assert(v.stream_mix[0] == 121 && v.stream_mix[1] == 121);
assert(v.stream_mix[2] == 38 && v.stream_mix[3] == 38);
assert(v.stream_mix[6] == 126 && v.stream_mix[7] == 126);
assert(v.pc_in[0] == 0 && v.pc_in[7] == 0);

assert(zg01_meter_parse(frame_all_groups, sizeof(frame_all_groups), &v) == 0);
assert(v.mic_in == 50 && v.mic_out == 55);
assert(v.pc_in[0] == 60 && v.pc_in[7] == 67);
assert(v.hdmi_in[0] == 70 && v.hdmi_in[7] == 77);
assert(v.game_mix[0] == 80 && v.game_mix[7] == 87);
assert(v.stream_mix[0] == 90 && v.stream_mix[7] == 97);
```

The `frame_all_groups` array is the synthetic fixture from Task 1. The sentinel for a rejected frame must initialise every field to `-1`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `bash tests/test-zg01-meters.sh`
Expected: compile error for the array fields or missing `frame_all_groups`.

- [ ] **Step 3: Implement the parser**

Add the struct and a helper:

```c
static inline unsigned int zg01_meter_raw(unsigned int payload)
{
    return 4 * (payload / 3) + 1 + payload % 3;
}

static inline int zg01_meter_channel(const unsigned char *buf,
                                     unsigned int base, unsigned int channel)
{
    unsigned int field = base + 2 * channel;
    unsigned int payload = 7 + 2 * field;

    return zg01_meter_level(buf, zg01_meter_raw(payload),
                            zg01_meter_raw(payload + 1));
}
```

Fill `mic_in`/`mic_out` at bases 0/2, then loop 8 channels for bases 4, 20, 36 and 52. Keep the header/CIN/F7 validation and the early `-EINVAL` before touching `out`.

- [ ] **Step 4: Run the test to verify it passes**

Run: `bash tests/test-zg01-meters.sh`
Expected: `zg01 meter parser contract passed`.

- [ ] **Step 5: Commit**

```bash
git add src/zg01_meters.h tests/test-zg01-meters.c tests/fixtures/meters/frames.h
git commit -m "feat(meters): parse all 34 meter channels"
```

---

### Task 3: Kernel snapshot and six multichannel controls

**Files:**
- Modify: `src/zg01.h`
- Modify: `src/zg01_meters.c`
- Modify: `src/zg01_control.c`
- Add: `tests/test-zg01-meter-controls.sh`

**Interfaces:**
- Produces: `#define ZG01_METER_LEVEL_COUNT 34` and the channel slots `MIC_IN 0`, `MIC_OUT 1`, `PC_IN 2`, `HDMI_IN 10`, `GAME_MIX 18`, `STREAM_MIX 26`.
- Produces: `int zg01_meters_values(struct zg01_dev *dev, unsigned int first, unsigned int count, int *out);` returning 0 or `-ENODATA`.
- Produces: the kcontrol table `{ name, first, count }` with the six controls above.
- Keeps: `zg01_meters_value()` for one slot.

- [ ] **Step 1: Write the failing hardware check**

Create `tests/test-zg01-meter-controls.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
command -v amixer >/dev/null || { echo "amixer not installed"; exit 0; }
grep -q '\[zg01' /proc/asound/cards 2>/dev/null || { echo "no zg01 card"; exit 0; }
amixer -c zg01 controls | grep -q "Meter Game Mix"
amixer -c zg01 controls | grep -q "Meter HDMI In"
amixer -c zg01 cget 'name=Meter Mic In' | grep -q 'values=1'
amixer -c zg01 cget 'name=Meter PC In' | grep -q 'values=8'
amixer -c zg01 cget 'name=Meter Stream Mix' | grep -q 'values=8'
echo "zg01 meter controls contract passed"
```

- [ ] **Step 2: Run it to verify it fails**

Run: `bash tests/test-zg01-meter-controls.sh`
Expected: it fails or reports no card while the old driver reports one value per surround control.

- [ ] **Step 3: Implement the snapshot and the controls**

In `src/zg01.h`, replace the four-entry enum with the 34 channel slots and `ZG01_METER_LEVEL_COUNT`. In `src/zg01_meters.c`, change `levels[ZG01_METER_COUNT]` to `levels[ZG01_METER_LEVEL_COUNT]`, store all 34 values, and add `zg01_meters_values` that rejects an invalid snapshot with `-ENODATA` before copying. In `src/zg01_control.c`, add `first`/`count` to `zg01_meter_ctl`, set `uinfo->count = c->count`, and have the getter copy `c->count` values.

- [ ] **Step 4: Build and run the checks**

Run:
```bash
make W=1
bash tests/test-zg01-meter-controls.sh
```
Expected: the module builds without a warning attributed to this repository, and the control names/counts are present on a card; the script skips cleanly without one.

- [ ] **Step 5: Commit**

```bash
git add src/zg01.h src/zg01_meters.c src/zg01_control.c tests/test-zg01-meter-controls.sh
git commit -m "feat(meters): expose six multichannel meter controls"
```

---

### Task 4: Rust ALSA backend reads value arrays

**Files:**
- Modify: `gui/crates/zg01-core/src/alsa.rs`

**Interfaces:**
- Produces: `ControlInfo.count: u32`.
- Produces: `AlsaBackend::read_values(&self, name: &str) -> Result<Vec<i64>, AlsaError>` with a default implementation returning one value.
- Produces: `RawCtl::read_values(id, meta) -> Result<Vec<i64>, AlsaError>`; `RealAlsa` overrides `read_values`.
- Keeps: `AlsaBackend::read` returning the first value.

- [ ] **Step 1: Write the failing test**

Add to the `alsa.rs` test module:

```rust
#[test]
fn a_multichannel_control_reads_all_its_values() {
    let info = ControlInfo {
        name: "Meter PC In".to_string(),
        kind: ControlKind::Integer,
        min: 0,
        max: 127,
        step: 0,
        count: 8,
        writable: false,
    };
    let fake = FakeAlsa::new(vec![info]);
    fake.set_values("Meter PC In", vec![1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(fake.read_values("Meter PC In").unwrap(), vec![1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(fake.read("Meter PC In").unwrap(), 1);
}
```

Add `count: 1` to every existing `ControlInfo` literal, or add a `ControlInfo::mono(...)` helper and use it in the tests.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd gui && cargo test -p zg01-core a_multichannel_control`
Expected: compile error, `count` or `read_values` not found.

- [ ] **Step 3: Implement the array read**

Add `count` to `ControlInfo`, `ElemMeta` and their constructors; read it with `alsa_sys::snd_ctl_elem_info_get_count`. Add `RawCtl::read_values` that allocates the value object, sets the id, calls `snd_ctl_elem_read`, and reads `meta.count` integers. Return `AlsaError::NoData` on `-ENODATA`. Add the trait method and `RealAlsa` override; update the fake backend to store `Vec<i64>`.

- [ ] **Step 4: Run the core tests**

Run: `cd gui && cargo test -p zg01-core`
Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add gui/crates/zg01-core/src/alsa.rs
git commit -m "feat(core): read multichannel ALSA values"
```

---

### Task 5: Rust meter ids, layout, snapshot and poller

**Files:**
- Modify: `gui/crates/zg01-core/src/meters.rs`
- Modify: `gui/crates/zg01-core/src/mixage.rs`
- Test: the `meters.rs` and `mixage.rs` test modules.

**Interfaces:**
- Produces: `MeterId::{MicIn, MicOut, PcIn, HdmiIn, GameMix, StreamMix}`, `ALL: [MeterId; 6]`, `index()`, `channel_count()` (1 or 8), `control()`.
- Produces: `MeterLayout::{V2, Legacy}`.
- Produces: `MeterSnapshot { values: [[Option<i64>; 8]; 6], layout, at, lost, generation }`, `value(MeterId, channel)`, `max(MeterId)`.
- Produces: `METER_CONTROLS: [&str; 6]`.
- Produces: `poll_meters_with_generation()` reading arrays and detecting the layout.

- [ ] **Step 1: Write the failing tests**

In `meters.rs`:

```rust
#[test]
fn the_six_meter_ids_have_stable_slots_and_counts() {
    assert_eq!(MeterId::MicIn.index(), 0);
    assert_eq!(MeterId::MicOut.index(), 1);
    assert_eq!(MeterId::PcIn.index(), 2);
    assert_eq!(MeterId::HdmiIn.index(), 3);
    assert_eq!(MeterId::GameMix.index(), 4);
    assert_eq!(MeterId::StreamMix.index(), 5);
    assert_eq!(MeterId::MicIn.channel_count(), 1);
    assert_eq!(MeterId::PcIn.channel_count(), 8);
    assert_eq!(MeterId::PcIn.control(), "Meter PC In");
}

#[test]
fn a_v2_snapshot_reads_by_group_and_channel() {
    let snapshot = MeterSnapshot {
        values: [[None; 8], [None; 8], [Some(1), Some(2), None, None, None, None, None, None], [None; 8], [None; 8], [None; 8]],
        layout: MeterLayout::V2,
        at: std::time::Instant::now(),
        lost: false,
        generation: 3,
    };
    assert_eq!(snapshot.value(MeterId::PcIn, 1), Some(2));
    assert_eq!(snapshot.max(MeterId::PcIn), Some(2));
    assert_eq!(snapshot.value(MeterId::PcIn, 2), None);
}

#[test]
fn a_legacy_driver_keeps_its_own_layout() {
    let backend = fake_legacy_backend();
    let snapshot = poll_meters_with_generation(&backend, 0);
    assert_eq!(snapshot.layout, MeterLayout::Legacy);
    assert_eq!(snapshot.value(MeterId::PcIn, 0), Some(38));
    assert_eq!(snapshot.value(MeterId::GameMix, 0), None);
}

#[test]
fn a_short_v2_array_leaves_the_rest_unknown() {
    let backend = fake_short_v2_backend();
    let snapshot = poll_meters_with_generation(&backend, 0);
    assert_eq!(snapshot.layout, MeterLayout::V2);
    assert_eq!(snapshot.value(MeterId::PcIn, 0), Some(38));
    assert_eq!(snapshot.value(MeterId::PcIn, 1), None);
}
```

The fake helpers live in the test module: a V2 fake with six controls (counts 1/8) and a legacy fake with four controls (count 1).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd gui && cargo test -p zg01-core meters`
Expected: compile errors for the new variants/fields.

- [ ] **Step 3: Implement the ids, layout and poller**

Add the enum variants, `ALL`, `index`, `channel_count`, `control`. Define `MeterLayout` and extend `MeterSnapshot`. `poll_meters_with_generation` reads each of the six controls with `read_values`; a successful read fills the group's channels, `-ENODATA`/unknown leaves them `None`, and a liveness error sets `lost`. Detect legacy when `Meter Game Mix` and `Meter HDMI In` are unknown and `Meter PC In` returns one value; in that case fill only the four old slots and set `MeterLayout::Legacy`. A short V2 array fills its prefix and leaves the rest `None`.

Update `mixage.rs` to import the new `MeterId` and set the strip map to MIC→MicIn, VOICE→PcIn, GAME→`Some(MeterId::GameMix)` (it is `None` today), STREAM OUT→StreamMix. The old mixage tests that assert `MeterId::MicOut`/`StreamMix` remain valid; add or update the test that currently expects GAME to have no card meter.

- [ ] **Step 4: Run the tests**

Run: `cd gui && cargo test -p zg01-core`
Expected: all tests pass, including the existing mixage tests.

- [ ] **Step 5: Commit**

```bash
git add gui/crates/zg01-core/src/meters.rs gui/crates/zg01-core/src/mixage.rs
git commit -m "feat(core): carry six meter groups and detect the legacy layout"
```

---

### Task 6: Meter widget draws a card bank

**Files:**
- Modify: `gui/crates/zg01-gui/src/widgets/meter.rs`

**Interfaces:**
- Produces: `MeterSpec.card: [Option<f64>; 8]`, `MeterSpec.card_count: usize`, and `MeterSpec::mono(label, card, card_present, linux, linux_present)`.
- Produces: `Meter::set_card(&self, levels: &[Option<f64>])` and `Meter::set_card_mono(&self, level: Option<f64>)`.
- Produces: `meter_width(card_present: bool, card_count: usize, has_linux: bool) -> i32`.
- Keeps: `set_linux_source`, the `PW` badge and the dashed unknown state.

- [ ] **Step 1: Write the failing tests**

Replace the width test and the card-update test:

```rust
#[test]
fn a_bank_width_counts_every_card_bar_and_the_pw_bar() {
    assert_eq!(meter_width(true, 1, false), 19);
    assert_eq!(meter_width(true, 8, false), 19 + 7 * 9);
    assert_eq!(meter_width(true, 8, true), 26 + 7 * 9);
    assert_eq!(meter_width(false, 0, true), 15);
}

#[test]
fn eight_card_channels_keep_every_reading() {
    let mut spec = MeterSpec::mono("PC", None, true, None, false);
    super::apply_card(&mut spec, &[Some(0.1), Some(0.2), None, None, None, None, None, None]);
    assert_eq!(spec.card_count, 8);
    assert_eq!(spec.card[0], Some(0.1));
    assert_eq!(spec.card[1], Some(0.2));
    assert_eq!(spec.card[2], None);
}
```

Update every existing `MeterSpec` literal to `MeterSpec::mono(...)` or to the new fields.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd gui && cargo test -p zg01-gui a_bank_width`
Expected: compile error for `card_count`/`set_card` signature.

- [ ] **Step 3: Implement the bank**

Draw `card_count` bars of `CARD_WIDTH` separated by `CARD_GAP` (2 px), then the existing gap and PW bar. `meter_width` adds `card_count * CARD_WIDTH + (card_count - 1) * CARD_GAP` when the card exists. Store the held peaks per channel (`[Cell<f64>; 8]`, `[Cell<Option<Instant>>; 8]`), and make the peak timer clear each expired channel. The accessible value is the maximum of the card and Linux levels; `set_card_mono` calls `set_card(&[level])` with `card_count = 1`.

- [ ] **Step 4: Run the widget tests**

Run: `cd gui && cargo test -p zg01-gui meter`
Expected: all meter tests pass.

- [ ] **Step 5: Commit**

```bash
git add gui/crates/zg01-gui/src/widgets/meter.rs
git commit -m "feat(gui): draw up to eight card meter bars"
```

---

### Task 7: Wire strips, Micro, the Meters page and the application

**Files:**
- Modify: `gui/crates/zg01-gui/src/ui/mixage.rs`
- Modify: `gui/crates/zg01-gui/src/ui/mic.rs`
- Create: `gui/crates/zg01-gui/src/ui/meters.rs`
- Modify: `gui/crates/zg01-gui/src/ui/mod.rs`
- Modify: `gui/crates/zg01-gui/src/app.rs`
- Modify: `gui/crates/zg01-gui/src/window.rs`

**Interfaces:**
- Produces: `MixageScreen::set_meter(&self, id: MeterId, levels: &[Option<i64>])`, using `max` of channels 0/1 for the strip and `set_card_mono`.
- Produces: `MicScreen::set_meter(&self, id: MeterId, levels: &[Option<i64>])`, using channel 0.
- Produces: `MetersScreen::new(state: &AppState)`, `widget()`, `set_meter(&self, id: MeterId, levels: &[Option<i64>])`, `set_legacy(&self, legacy: bool)`.
- Produces: the bank channel labels `["L", "R", "C", "LFE", "SBL", "SBR", "SL", "SR"]`, from the vendor QML `channelNo` order; the two mic meters are mono.
- Produces: `Page::Meters`; `Page::ALL` has four entries; `Shell::install_meters` and `Shell::set_page_enabled(Page, bool)`.
- Produces: `App.meters: Option<MetersScreen>`; build/install on connect, update on `Msg::Meters`, clear on disconnect.

- [ ] **Step 1: Write the failing tests**

In `app.rs`, update the page round-trip test:

```rust
#[test]
fn the_pages_round_trip_through_their_index() {
    for page in Page::ALL {
        assert_eq!(Page::from_index(page.index()), Some(page));
    }
    assert_eq!(Page::from_index(-1), None);
    assert_eq!(Page::from_index(4), None);
}
```

Add a mixage unit test for the strip selection:

```rust
#[test]
fn a_strip_uses_the_louder_of_the_first_two_channels() {
    assert_eq!(strip_level(&[Some(10), Some(40), Some(90)]), Some(40));
    assert_eq!(strip_level(&[None, Some(40)]), Some(40));
    assert_eq!(strip_level(&[None, None]), None);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd gui && cargo test -p zg01-gui strip_level`
Expected: compile error for `Page::Meters`/`strip_level`.

- [ ] **Step 3: Implement the wiring**

Add `Page::Meters` (index 3, title "Meters", description "The card meter banks.", icon `audio-volume-high-symbolic`, stack name "meters") and update `Page::ALL`. Add `Shell::install_meters` and `set_page_enabled`. Build `MetersScreen` with the six groups; the four surround groups use `MeterSpec` with `card_count = 8`, the two mic groups use mono. In `App`, build it next to Mixage/Micro, install it, pass each `Msg::Meters` array to the three screens, and clear it on disconnect. In `update_meters`, keep the `AppState` meter control value at the group maximum for compatibility, and pass the arrays to the screens. For `MeterLayout::Legacy`, pass only the four legacy slots, set the Meters page to legacy (notice + sidebar row disabled), and never map legacy values into channels 1..8 of the new groups.

- [ ] **Step 4: Build and test**

Run:
```bash
cd gui
cargo fmt
cargo test --workspace
cargo build --workspace
```
Expected: all tests pass and the workspace builds.

- [ ] **Step 5: Commit**

```bash
git add gui/crates/zg01-gui/src/ui/ gui/crates/zg01-gui/src/app.rs gui/crates/zg01-gui/src/window.rs
git commit -m "feat(gui): show the six card meter groups and the Meters page"
```

---

### Task 8: Documentation and acceptance

**Files:**
- Modify: `docs/METER_PROTOCOL.md`
- Modify: `docs/METER_TEST.md`
- Modify: `docs/GUI_TEST.md`
- Modify: `README.md`
- Modify: `gui/README.md`
- Modify: `gui/po/fr.po` (the new Meters page strings)

- [ ] **Step 1: Update the protocol contract**

Rewrite `docs/METER_PROTOCOL.md` around the 34-channel map: the six groups, the field/payload indexing, the current/peak pairs, the control slots and counts, the corrected `stream_mix` channels 2/6, the legacy layout, and the capture evidence (Voice→`stream_mix` 0/1, Game Out→`pc_in`/`game_mix` 0/1, HDMI→`hdmi_in`/`game_mix` 0/1). Keep `hdmi_in`/surround channels 2–7 marked as unmeasured where applicable.

- [ ] **Step 2: Update the user docs and acceptance**

In `README.md`, replace the four-meter section with the six controls and their counts; document the count change and the legacy GUI layout. In `docs/GUI_TEST.md`, add the Meters page checks and the legacy notice. In `docs/METER_TEST.md`, update the control list and the count checks; keep the Linux live-meter blocker note. In `gui/README.md`, document the Meters page and the legacy behavior.

- [ ] **Step 3: Run the full automated checks**

Run:
```bash
make W=1
make -C tools
bash tests/test-zg01-meters.sh
bash tests/test-decode-meters.sh
bash tests/test-capture-normalizer.sh
bash tests/test-usbmon-normalizer.sh
cd gui && cargo fmt --check && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
```
Expected: every command exits 0; `make W=1` reports no warning attributed to this repository.

- [ ] **Step 4: Run the hardware check**

With the new module loaded:
```bash
amixer -c zg01 controls | grep 'Meter '
amixer -c zg01 cget 'name=Meter PC In'
amixer -c zg01 cget 'name=Meter Game Mix'
amixer -c zg01 cget 'name=Meter Mic In'
sudo alsactl store && sudo alsactl restore
```
Expected: six controls; `PC In`/`Game Mix` show eight values; `Mic In` shows one; `alsactl` succeeds and does not restore the meters. With the Linux meter session blocked, the controls return `-ENODATA`; the GUI draws the card banks dashed and the PW bars stay live.

- [ ] **Step 5: Commit**

```bash
git add docs/METER_PROTOCOL.md docs/METER_TEST.md docs/GUI_TEST.md README.md gui/README.md
git commit -m "docs(meters): document the multichannel card meters"
```
