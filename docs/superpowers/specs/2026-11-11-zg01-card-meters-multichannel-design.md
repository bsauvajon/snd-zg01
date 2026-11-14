# ZG01 card meters — multichannel decode and GUI — Design

Status: proposed, 2026-11-11.

## Intent

The GUI draws four card meters and three PipeWire bars, but the card decode
is wrong. The 192-byte frame on EP `0x83` carries **34 channels of level
data**, not eight independent values. `Meter PC In` and `Meter Stream Mix`
read two of the stream-mix channels, and `Meter Game Mix`/`Meter HDMI In`
are not registered at all. The ZG Controller displays six groups, with
8-channel surround views for PC IN and HDMI IN.

The goal is to decode the frame completely, expose every group through
ALSA (one control per group, eight values for the surround groups), and
display them in the GUI: the existing strips plus a Meters page that
matches the ZG Controller's Sound Check view. The PipeWire bars remain the
Linux-side complement.

## Goals

- Correct the frame parser to 34 channels × (current, peak), grouped as
  mic in, mic out, PC in, HDMI in, game mix and stream mix.
- Register six read-only, volatile ALSA integer controls:
  `Meter Mic In` and `Meter Mic Out` with `count = 1`; `Meter PC In`,
  `Meter HDMI In`, `Meter Game Mix` and `Meter Stream Mix` with
  `count = 8`.
- Read the arrays in the GUI and display:
  - Mixage strips: MIC from `mic_in`, VOICE from `pc_in` L/R, GAME from
    `game_mix` L/R, STREAM OUT from `stream_mix` L/R;
  - Micro: `mic_in` and `mic_out`;
  - a new **Meters** page with 8-channel banks for PC IN, HDMI IN, GAME
    MIX and STREAM MIX, plus the two mic levels.
- Keep the PipeWire bars, the dashed unknown state and the "never a fake
  zero" rule.
- Cover the decode and the parser with fixtures; keep `make W=1`,
  `cargo test --workspace`, `clippy -D warnings` and `fmt --check` clean.

## Non-goals

- Re-arming the Linux meter session after a suspend. That is a separate
  reverse-engineering phase (see "Flux dependency"). The controls and the
  GUI are correct while the device does not stream; they show unknown.
- Reproducing the ZG Controller's non-linear bar curve in the driver or
  in the ALSA values. The GUI keeps its own bar mapping.
- Peak-hold in the driver. The driver publishes the current value; the GUI
  already computes peak hold from successive snapshots. The peak fields
  exist in the frame but are not exposed.
- Validating the surround channels 2–7 and the exact ZG Controller VOICE
  strip mapping. The stereo captures cover channels 0/1; the other
  channels are exposed and drawn as unknown when the device reports no
  data.
- Changing the `0x03` write path, the 208+ parameter controls, the preset
  parser or the `.zgpreset` format.

## Constraints

- The frame is 192 bytes: 48 USB-MIDI packets, a 144-byte SysEx payload,
  and 68 14-bit values. Header and framing validation stays as it is
  (`04 f0 43 10 | 04 3e 14 02`, CIN `04`/`07`, final `F7`).
- The device sends a frame about every 50 ms. A read returns the latest
  valid frame; a missing or stale frame is `-ENODATA`, never zero.
- Controls are `SNDRV_CTL_ELEM_TYPE_INTEGER`, range 0–127, access
  `READ | VOLATILE`, no put callback. `alsactl` must not restore them.
- ALSA names are stable and at most 44 bytes. The existing four names stay
  unchanged; the two reserved names become real controls.
- The parser must not read past the buffer; a rejected frame leaves the
  previous snapshot untouched.
- The GUI must distinguish "no control", "control has no value" and
  "value is zero"; all three are different.
- No new build or runtime dependency. No packaging change.

## Decoded protocol (v2)

Each channel has a **current** value followed by a **peak-hold** value.
The 68 values are ordered by channel:

| Values | Group | Channels |
|---|---|---|
| 0–1 | `mic_in` | 1 (current, peak) |
| 2–3 | `mic_out` | 1 |
| 4–19 | `pc_in` | 8 (current/peak interleaved) |
| 20–35 | `hdmi_in` | 8 |
| 36–51 | `game_mix` | 8 |
| 52–67 | `stream_mix` | 8 |

For group base `b` and channel `c`, current is value `b + 2*c`, peak is
`b + 2*c + 1`. The current value at sequence index `k` is the 14-bit field
at payload indices `7 + 2*k` (high) and `8 + 2*k` (low); the raw frame
offset follows the documented CIN stripping.

The scale is unchanged:

```
level_dB = (code - 16384) / 10
control  = clamp(round(level_dB) + 127, 0, 127)
```

### Evidence

`MemAPI_ZG01.dll` (vendor package) registers six meter names with a
descriptor whose constant is `(count << 16) | base`:

| Name | Constant | Base | Values |
|---|---|---|---|
| `meter_mic_input` | `0x00020000` | 0 | 2 |
| `meter_mic_output` | `0x00020002` | 2 | 2 |
| `meter_pc_in` | `0x00100004` | 4 | 16 |
| `meter_hdmi_in` | `0x00100014` | 20 | 16 |
| `meter_game_mix` | `0x00100024` | 36 | 16 |
| `meter_stream_mix` | `0x00100034` | 52 | 16 |

The vendor QML confirms the 8-channel model: `YQMeterDataAccessor` uses
`channelNo` 0, 1, 2, 4, 5, 6, 7, and `PARAM_PC_IN`/`PARAM_HDMI_IN` for the
Sound Check meter. The app's QML `meterPosition()` maps a dB×10 value to a
bar; the driver does not need that curve.

The captures confirm the groups (corrected indexing; see the note below):

| Stimulus | Observed |
|---|---|
| Voice playback (ZG device "Voice") | `stream_mix` 0/1; `mic_out` follows; `pc_in` stays at floor |
| Game Out (USB) | `pc_in` 0/1; `game_mix` 0/1; `stream_mix` 4/5 |
| HDMI input | `hdmi_in` 0/1; `game_mix` 0/1; `stream_mix` 4/5 |
| Microphone | `mic_in`, `mic_out` |

A first pass over the captures used one field per channel and produced
wrong group labels. The values are 34 channels × (current, peak), so
channel `c` is field `2*c`; the table above is the corrected result. The
surround channels 2–7 were not activated by these stereo sources, and the
exact strip mapping for the ZG Controller's VOICE meter is still open
(see "Open items").

### Correction of the previous decode

The previous document treated the 16 varying fields as eight unrelated
current/peak pairs and labelled the pair at payload 119 as `pc_in` and the
pair at payload 135 as `stream_mix`. With the channel map above:

- payload 119 → value 56 → `stream_mix` channel 2;
- payload 135 → value 64 → `stream_mix` channel 6;
- the "copies" E/G/K/O are `stream_mix` channels 0/1/3/6/7.

`pc_in` is values 4–19 and was never exposed by the old driver. The
corrected map is the contract from now on.

## Approaches considered

1. **Single value per group, max of the eight channels.** Rejected: it
   cannot show the surround meters and is not ZG Controller parity.
2. **One ALSA control per group with eight values, plus a Meters page.**
   Chosen. The strips show the group maximum (or L/R), the Meters page
   shows all eight channels. It keeps six names and one control per group.
3. **Eight separate controls per group** (`Meter PC In Ch 1`…`Ch 8`).
   Rejected: 32 extra names, a naming scheme to design, and ALSA already
   supports arrays natively.

## Design

### Kernel: frame parser and snapshot

`struct zg01_meter_values` becomes:

```c
struct zg01_meter_values {
    int mic_in;
    int mic_out;
    int pc_in[8];
    int hdmi_in[8];
    int game_mix[8];
    int stream_mix[8];
};
```

The parser fills every channel from the current value of its current/peak
pair; peaks are ignored. The state stores 34 atomic levels
(`ZG01_METER_LEVEL_COUNT = 34`) with the existing release/acquire
`valid` and `valid_jiffies` fields. A valid frame updates all 34 values in
one completion; a rejected frame updates none.

`zg01_meters_value()` remains for a single slot. A new
`zg01_meters_values(dev, first, count, out)` copies `count` consecutive
levels; the kcontrol getter uses it. A stale or absent snapshot returns
`-ENODATA` before copying anything.

### Kernel: ALSA controls

The parser reduces the 68 frame values to **34 channel slots** (one current
value per channel; peaks are dropped). The `First` column below is a slot
in that 34-channel array, not a frame value index:

| Control | First slot | Count |
|---|---|---|
| `Meter Mic In` | 0 | 1 |
| `Meter Mic Out` | 1 | 1 |
| `Meter PC In` | 2 | 8 |
| `Meter HDMI In` | 10 | 8 |
| `Meter Game Mix` | 18 | 8 |
| `Meter Stream Mix` | 26 | 8 |

The info callback publishes `count`, 0–127 and the existing access flags.
The getter returns `-ENODATA` when the snapshot is invalid, otherwise the
values. `Meter Game Mix` and `Meter HDMI In` stop being reserved names.

The two existing surround controls change from `count = 1` to `count = 8`
and their values move to the corrected channels. This is a deliberate ABI
change, documented in `README.md` and `docs/METER_PROTOCOL.md`; the GUI
and `zgctl` do not read meters today, so nothing in-tree depends on the
old single value.

### Userspace decoder and tests

`tools/decode-meters.py` gains the 34-channel map and prints the six
groups (current and peak). The fixture generator emits frames with known
values for every group and channel. `tests/test-zg01-meters.c` checks the
C parser against those fixtures, including a short frame, a bad CIN, a bad
final byte, and the floor/0 dBFS bounds. `tests/test-decode-meters.py`
checks the Python decoder.

`docs/METER_PROTOCOL.md` is rewritten around the v2 map, with the vendor
DLL evidence and the capture evidence above.

### GUI core: meter ids, snapshots and ALSA arrays

`MeterId` grows to six variants:

```rust
pub enum MeterId {
    MicIn,
    MicOut,
    PcIn,
    HdmiIn,
    GameMix,
    StreamMix,
}
```

`index()` gives the snapshot slot, `channel_count()` gives 1 or 8, and
`control()` gives the ALSA name. `LinuxMeterId` is unchanged.

`MeterSnapshot` holds `[[Option<i64>; 8]; 6]` (only slot 0 is used for the
mic groups), plus `layout: MeterLayout`, `at`, `lost` and `generation`.
`value(group, channel)` returns `None` when the group or channel was not
read. `MeterLayout` is `V2` or `Legacy`, as described below.

`AlsaBackend` gains:

```rust
fn read_values(&self, name: &str) -> Result<Vec<i64>, AlsaError>;
```

The real backend reads the element's value array; the default
implementation calls `read()` and returns one value, so the fake backend
and old tests keep working. `ControlInfo` gains `count: u32`.

`poll_meters` reads each control as an array. `-ENODATA` and a missing
control leave the group empty; a liveness error still sets `lost`.

The snapshot carries a `layout` that says which contract produced it:

- `MeterLayout::V2` when `Meter Game Mix` and `Meter HDMI In` exist and
  `Meter PC In` publishes eight values. The new map applies.
- `MeterLayout::Legacy` when the two new controls are absent and the
  surround controls publish one value. The GUI keeps the current
  four-meter presentation and the old labels, and hides the Meters page.
  It does **not** place the legacy values in channel 0 of the new groups:
  the old `Meter PC In`/`Meter Stream Mix` values are known to be
  `stream_mix` channels 2 and 6, so a silent remap would mislabel them.
- A partial driver (some new controls, some short arrays) is treated as
  V2 with the missing groups unknown and dashed; it is never silently
  remapped either.

The status line / footer says that the card meters are in legacy mode
when the old driver is detected, so the user knows to upgrade the driver.

### GUI: widget and screens

`MeterSpec` grows a card bank:

```rust
card: [Option<f64>; 8],
card_count: usize,
```

`Meter` draws `card_count` solid bars, a gap, then the optional single PW
bar. Peak hold and the clip marker are per card bar. The width calculation
accounts for the bars, the gaps and the PW bar. The existing single-bar
behavior is `card_count = 1`.

Mixage strips keep one card bar, fed with the maximum of the group's L/R
channels (the Meters page shows the individual channels). MIC uses
`mic_in`; VOICE uses `pc_in` 0/1; GAME uses `game_mix` 0/1; STREAM OUT
uses `stream_mix` 0/1. The PW bars keep their current sources.

A new **Meters** page (sidebar, next to Mixage/Micro/Avancé) shows:

- MIC IN and MIC OUT as two mono bars;
- four 8-channel banks for PC IN, HDMI IN, GAME MIX and STREAM MIX;
- channel labels from the vendor QML order (`L`, `R`, `C`, `LFE`, `SBL`,
  `SBR`, `SL`, `SR`) or `CH 1`…`CH 8` if the order is not confirmed;
- a dashed bank when the group has no reading, a solid zero only for a
  captured silence.

The page is card-only; the PipeWire bars stay on the strips. It is built
from the same `MeterSnapshot`, so it adds no worker.

### Flux dependency (Linux)

The controls are correct while the device does not stream: they return
`-ENODATA` and the GUI draws dashed bars. On the current unit the Linux
reader still gets zero-length packets after the 2026-11-01 suspend, while
the Windows ZG Controller streams as soon as it starts. The Windows
capture shows the app's startup: dirty-flag reads (`F0 43 30`, page 1 ids
08–0c), preset blocks (`F0 43 00`), then the keepalive that is followed
20 ms later by the first 192-byte frame.

A separate phase replays that sequence from Linux and bisects the minimal
trigger. It may use a temporary debugfs raw bulk-out facility, which is
removed before merge. The card-meter feature can land and be unit-tested
before that phase; live Linux acceptance is blocked until it completes.

### Failure and edge cases

- Malformed or short frame: `-EINVAL`, previous snapshot unchanged.
- Valid frame with all channels at the floor: controls read 0–127; no
  `-ENODATA`.
- No valid frame for 500 ms, reader stopped, suspend or disconnect:
  `-ENODATA`; the GUI draws dashed.
- Old driver with one value per surround control: the GUI enters the
  legacy four-meter layout, keeps the old labels, and hides the Meters
  page. It does not pretend the legacy value is channel 0 of a new group.
- A missing control in a V2 driver: unknown group, no bar.
- `alsactl store`/`restore`: read-only and volatile; no restore.
- A snapshot from a replaced backend: dropped by generation, as today.

## Tests

- Python: 34-channel fixtures, header/CIN/F7 rejection, floor/0 dBFS,
  group boundaries.
- C: the same fixtures through `zg01_meter_parse`; `make W=1` clean.
- Kernel: control registration on hardware (`amixer` shows the six names,
  counts and 0–127 range); `alsactl` does not restore; disconnect and
  suspend do not crash.
- Rust core: `MeterId` slots and counts, array reads, `-ENODATA`, missing
  control, V2/legacy layout detection, legacy values not remapped into the
  new groups, generation tagging, fake backend.
- GUI: bank geometry, per-bar peak hold, dashed unknown, strip mapping,
  Meters page construction.
- Hardware: decode the Windows captures to the expected levels; after the
  flux phase, the live Linux values match the ZG Controller within two
  steps.

## Compatibility and packaging

- Control names are stable; `Meter PC In`/`Meter Stream Mix` change from
  one value to eight, and their values are corrected.
- `Meter Game Mix`/`Meter HDMI In` are now registered.
- The GUI detects the old driver and keeps the legacy four-meter layout
  and labels, hiding the Meters page; it does not remap the legacy values
  into the new groups. The footer says the card meters are in legacy mode.
- No new dependency, no package change. README and `docs/METER_PROTOCOL.md`
  are updated; the old "GAME and HDMI are not decoded" limitation is
  replaced by the v2 map.

## Acceptance criteria

1. `amixer -c zg01 cget 'name=Meter PC In'` shows eight values;
   `Meter Game Mix` and `Meter HDMI In` exist; `Meter Mic In`/`Meter Mic
   Out` show one.
2. A valid frame updates all 34 values; a missing/stale frame returns
   `-ENODATA`; a malformed frame leaves the previous values.
3. The Mixage strips show MIC, VOICE, GAME and STREAM from the card; the
   Meters page shows the four 8-channel banks; unknown is dashed, never
   zero.
4. The PipeWire bars are unchanged and independent.
5. `make W=1`, `cargo test --workspace`, `cargo clippy -D warnings` and
   `cargo fmt --check` pass.
6. With a live flux, the card values track the ZG Controller within two
   steps; the Windows captures provide the decode evidence until then.

## Decisions made in this session

- Multichannel: eight values per surround group, one per group control.
- The four existing control names stay; Game Mix and HDMI In are added.
- Strips use the group maximum/L-R; the Meters page shows all channels.
- The PipeWire bars are unchanged.

## Open items

- The ZG Controller VOICE strip mapping: `pc_in` 0/1 (PC input) or
  `stream_mix` 0/1 (streaming voice). A screenshot during a Voice playback
  settles it; the GUI uses `pc_in` 0/1 until then.
- The speaker labels for channels 2–7 (the stereo captures only activate
  0/1; the QML channel order is the current evidence).
- The Linux meter-session re-arm after suspend.
