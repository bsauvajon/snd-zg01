# ZG01 Control — MIC state readback

Date: 2026-11-08
Status: approved 2026-11-08
Project: `snd-zg01` (Yamaha ZG01 Linux driver) + `ZG01 Control` GUI

## Intent

The GUI currently shows the driver's **write cache**, not the device's
state. After a module load the cache holds the built-in defaults; settings
changed by the Windows application or by `Reset to ZG01` are invisible.
This is the root of the "Gate looks wrong" doubt: the device persisted
Gate on/60, the cache showed Gate off/60 (attack 70).

The `04 f0 43 20 … 04 00 00 03` request already makes the device push its
MIC state as eight 512-byte frames. The frames were captured and the MIC
fields decoded in this investigation. Use that dump to populate the
driver's cache at startup and on demand, and show the device state in the
GUI.

## Goals

1. After the driver loads, the Micro screen shows the **device's persisted
   MIC state**, not the built-in defaults: the direct gate switch/value,
   compressor, limiter and EQ fields. Transformed fields are Unknown, not
   guessed.
2. A **Read from ZG01** action re-reads the state after an external change
   (ZG Controller, another session) and updates the GUI.
3. `Reset to ZG01` triggers a state read after the reset, so the GUI shows
   the reloaded persisted state instead of all-unknown.
4. Save is safe: the GUI never persists cache defaults for fields it has
   not read. Fields the dump does not expose are explicitly **Unknown**,
   not silently cached.
5. The protocol is documented and testable without the device.

## Scope

In scope:

- the MIC section (type-20 id `0x03`) only;
- the direct fields (gate switch/value/range, compressor
  switch/value/threshold/ratio/knee, limiter switch/value/threshold/knee,
  EQ switch/shapes/Q); the transformed gate threshold/attack/hold/decay,
  compressor/limiter attack/release/out-gain and EQ biquad
  gain/frequency stay Unknown;
- driver state collection, parsing, cache update, a read status and a
  refresh action;
- GUI status, refresh, Unknown provenance, Save guard and Reset flow;
- documentation, fixtures and hardware acceptance steps.

Out of scope:

- Mixage/Game/Stream and Advanced state sections (their type-20 id is not
  captured; the GUI keeps today's cache behaviour there and must say so);
- recovering EQ **gain** and **frequency** from the biquad coefficients;
- the Windows application or its internal parser;
- exporting a `.zgpreset` from the dump;
- live meters (already blocked on this device).

## Decoded protocol

Request (`0x03` bulk out, 512 bytes, zero padded):

```
04 f0 43 20 | 04 3e 14 01 | 04 01 00 00 | 04 00 00 03 | 05 f7
```

Response: eight 512-byte frames on `0x83`. Each frame is 128 four-byte
USB-MIDI packets `[CIN][d0][d1][d2]`; concatenate the three data bytes to
get a 3072-byte payload. The payload starts `f0 43 00 3e 1b 1f 14 …`; the
observed fixture ends in zero padding, not `f7`. The field map below is in
**payload byte offsets**.

Confirmed direct fields (controlled single-control captures, payload index):

| Control | Offset | Notes |
|---|---:|---|
| Gate Capture Switch | 1460 | 0/1 |
| Gate | 1461 | 0..100 |
| Gate Range | 1464 | direct |
| Compressor Capture Switch | 1470 | 0/1, confirmed off/on |
| Compressor | 1472 | direct |
| Compressor Threshold | 1473 | direct |
| Compressor Ratio | 1474 | direct |
| Compressor Knee | 1480 | direct |
| Limiter Capture Switch | 1661 | 0/1 |
| Limiter | 1662 | direct |
| Limiter Threshold | 1664 | direct |
| Limiter Knee | 1669 | direct |
| EQ Capture Switch | 1628 | 0/1, confirmed off/on |
| EQ Low Shape | 1634 | 0/1 |
| EQ High Shape | 1637 | 0/1 |
| EQ Band 1 Q | 1657 | device Q 0..60 |
| EQ Band 2 Q | 1658 | device Q 0..60 |
| EQ Band 3 Q | 1659 | device Q 0..60 |
| EQ Band 4 Q | 1660 | device Q 0..60 |

Transformed fields (do **not** write these into the UI cache slots; mark
them Unknown until an inverse model exists):

| Control | Offset(s) | Observed device value |
|---|---:|---|
| Gate Threshold | 1462 | 80 written → 73 in the dump |
| Gate Attack | 1465 | ALSA range 57..283 exceeds one 7-bit byte |
| Gate Hold | 1467 | ALSA range 0..213 exceeds one 7-bit byte |
| Gate Decay | 1468 | ALSA range 24..300 exceeds one 7-bit byte |
| Compressor Attack | 1475 | 170 → 42 |
| Compressor Release | 1477 | 162 → 34 |
| Compressor Out Gain | 1481 (+1487) | 160 → 32 |
| Limiter Attack | 1665 | 170 → 42 |
| Limiter Release | 1667 | 162 → 34 |
| Limiter Out Gain | 1670 (+1671) | 160 → 32 |
| EQ Band 1 Gain | 1638–1640 | biquad coefficient |
| EQ Band 2 Gain | 1641–1642 | biquad coefficient |
| EQ Band 3 Gain | 1643–1644 | biquad coefficient |
| EQ Band 4 Gain | 1645–1646 | biquad coefficient |
| EQ Band 1 Frequency | 1648–1649 | biquad coefficient |
| EQ Band 2 Frequency | 1650–1651 | biquad coefficient |
| EQ Band 3 Frequency | 1652–1653 | biquad coefficient |
| EQ Band 4 Frequency | 1654–1656 | biquad coefficient |

The bytes at 1463 (`0x40`), 1466 (`0x00`), 1469, 1471 and the other gaps
are stable separators/coefficients in the observed firmware; the parser
skips them. The direct/transformed split is the important guard: a
transformed byte written into a gain/attack cache slot would display a
plausible but wrong fader value.

The dump is the **device state**, not the last write: after Reset the dump
showed Gate on/60 and Limiter on/50 while the ALSA cache still held the
test values (Gate off/60 (attack 70), Limiter off/100). This is the property the
readback relies on. The switch confirmation (Compressor 1470, EQ 1628)
came from an off→on capture in which exactly one payload byte moved.

## Design

### Driver

- New `src/zg01_state.[ch]`:
  - `zg01_state_arm(dev)`: prepare a collector.
  - `zg01_state_frame(dev, buf, len)`: called by the meter reader for each
    512-byte frame whose payload starts `f0 43 00`; collect eight frames.
  - `zg01_state_complete(dev)`: parse the payload and write the confirmed
    fields into `dev->param_cache` under `param_mutex`; record the status.
- The meter reader (`zg01_meters_complete`) distinguishes state frames
  from 192-byte meter frames; a state frame while disarmed is ignored and
  counted as `other`.
- `zg01_param_meter_init` stays the sender. The probe arms the collector
  before sending the type-20 request, so the existing startup read
  populates the cache with no extra traffic.
- A new `State Read` action kcontrol (write 1; get 0, following the
  Save/Reset pattern) arms the collector and
  re-sends the type-20 request; a read-only `State Read Status` returns
  0 (never), 1 (ok), or a negative error. Both are tiny, documented
  controls, not hidden sysfs hooks.
- `Reset to ZG01` automatically arms and re-sends the type-20 request
  after the reset frame; this is the flow the GUI uses.
- The parser only touches fields in the confirmed table. The transformed
  fields stay at the previous cache value; the status control plus a
  per-field known mask tells the GUI which fields were actually read.

### GUI

- The Micro page reads `State Read Status` and the known mask. Fields
  read from the device are `Known`; the transformed fields are `Unknown`
  with the existing dashed style.
- A **Read from ZG01** action in the Micro header triggers `State Read`,
  waits for the status to become ok, and reloads the controls. It is
  enabled only when the card is present.
- At startup the probe read is normally already complete; the GUI shows
  the device state without an action. If the status is 0/error, the Micro
  fields are Unknown rather than defaults.
- `Reset to ZG01`: the driver re-reads after the reset; the GUI waits for
  the new status and reloads, so the values are Known again.
- Save is enabled after a successful read, or when every writable value is
  known (for example a fully applied preset). The Save command itself
  does not write cached fields; it persists the device state. Applying a
  preset still writes the preset values and marks those fields Known.
- The Mixage page is unchanged in this phase and carries a short "état
  non lu (section non décodée)" notice; it must not imply the dumped MIC
  state applies to it.

## Phases and definition of done

### Phase 1 — protocol document and parser fixture

- `docs/STATE_DUMP_PROTOCOL.md` with the request, frame format, payload
  extraction, the table above, the evidence captures and the unresolved
  fields.
- A fixture with the eight captured frames and a host-side parser test
  (Python or Rust) that asserts the gate/comp/limiter/Q values after a
  known state.
- `docs/MIC_CONTROL_PROTOCOL.md` updated: the state dump is partially
  decoded; the cache is no longer the only source for the MIC fields.

DoD: parser test passes on the fixture; doc reviewed; no hardware needed.

### Phase 2 — driver state collection and cache update

- `zg01_state` collector/parser; meter-reader integration; `State Read`
  and `State Read Status` controls; Reset auto-read.
- Unit tests for frame extraction, payload bounds, partial/aborted
  reads, and the known mask.
- `make W=1` clean; no meter regression; module loads on the device.

DoD: after `insmod`, `amixer cget` on the confirmed **direct** MIC
controls returns the persisted device values; `State Read` after an
external change updates them; a failed read reports an error and leaves
the cache untouched.

### Phase 3 — GUI

- Status/known mask, Read action, Startup provenance, Reset flow, Save
  guard, Mixage notice.
- GUI tests for Unknown/refresh/status transitions with a fake backend.
- `GUI_TEST.md` hardware scenario: load module, open GUI, verify Gate
  on/60 (the persisted value at the time of capture), change a value in
  the Windows app or via `amixer`, Read, verify.

DoD: screenshots/manual log show the persisted Gate on/60; EQ gain/freq
dashed; Save disabled before the first successful read and enabled after.

### Phase 4 — packaging and release notes

- Desktop/AppStream text updated: MIC readback, Mixage still cached.
- Release note states the limits and the live-meter blocker.

DoD: validators pass; notes accurate.

## Risks and open questions

- The dump arrives only in response to type-20; a refresh while the meter
  stream is running must not lose meter frames (the reader demultiplexes
  by length/header).
- Firmware variants may move offsets; the parser must reject an unknown
  header/length rather than write wrong values.
- The compressor switch (1470) and EQ switch (1628) are confirmed by an
  off→on capture; firmware variants may still differ, so the header and
  length are validated before any cache write.
- The EQ gain/frequency coefficients are not the UI values; Phase 1
  leaves them Unknown. Recovering them needs a biquad model (Phase 5,
  not committed here).
- The Mixage/Advanced state sections have not been captured; their GUI
  values remain cached and must be labelled honestly.
- The temporary `state_read` module parameter used during the
  investigation is a developer hook only; the shipped trigger is the
  ALSA control.
