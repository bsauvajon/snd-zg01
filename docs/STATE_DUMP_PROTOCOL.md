# ZG01 MIC state dump (`F0 43 20`) — decoded layout

Status: MIC section decoded from captures, 2026-11-08. Other sections and
the DSP-transformed fields remain open.

## Request and response

Send one 512-byte bulk frame on EP `0x03`:

```
04 f0 43 20 | 04 3e 14 01 | 04 01 00 00 | 04 00 00 03 | 05 f7
```

The final `03` selects the MIC section. The device answers on EP `0x83`
with **eight 512-byte frames**. Each frame is 128 four-byte USB-MIDI
packets `[CIN][d0][d1][d2]`; concatenate the three data bytes to obtain a
3072-byte payload. The payload starts:

```
f0 43 00 3e 1b 1f 14 01 01 00 …
```

A 192-byte frame is a meter frame, not state. A state frame has CIN `0x04`
and `f0 43 00` in its first packet; the parser rejects anything else.

## What the dump is

The dump is the **device state**, not the last write. When the driver
cache still held test values (Gate off/60, attack 70; Limiter off/100), a
`Reset to ZG01` followed by a read returned Gate on/60 and Limiter on/50.
That is why the readback can replace the cache: it observes what the
device actually applied.

The dump mixes:

- direct parameter bytes (the same raw value the ALSA control uses);
- DSP-transformed bytes (the device's internal coefficient for an attack,
  release, out gain, gate threshold or EQ biquad);
- descriptors, names and separators.

Writing a transformed byte into a direct UI cache slot would show a
plausible but wrong fader. The parser therefore only exposes the direct
fields and marks the others Unknown.

## Direct fields

Offsets are payload indices (0-based). Confirmed by changing exactly one
ALSA control, re-reading the dump and diffing.

| Field | Offset | Range / meaning |
|---|---:|---|
| Gate Capture Switch | 1460 | 0/1 |
| Gate | 1461 | 0..100 |
| Gate Range | 1464 | raw |
| Compressor Capture Switch | 1470 | 0/1 |
| Compressor | 1472 | raw |
| Compressor Threshold | 1473 | raw |
| Compressor Ratio | 1474 | raw |
| Compressor Knee | 1480 | raw |
| Limiter Capture Switch | 1661 | 0/1 |
| Limiter | 1662 | raw |
| Limiter Threshold | 1664 | raw |
| Limiter Knee | 1669 | raw |
| EQ Capture Switch | 1628 | 0/1 |
| EQ Low Shape | 1634 | 0/1 |
| EQ High Shape | 1637 | 0/1 |
| EQ Band 1 Q | 1657 | device Q 0..60 |
| EQ Band 2 Q | 1658 | device Q 0..60 |
| EQ Band 3 Q | 1659 | device Q 0..60 |
| EQ Band 4 Q | 1660 | device Q 0..60 |

The EQ Q is the **device** value (0..60); the ALSA control exposes the
user value (0.50..16.0 in 0.01 steps). The driver's existing
`zg01_dev_to_user(CONV_Q, …)` conversion still applies.

## Transformed fields (do not cache as UI values)

| Field | Offset(s) | Observed |
|---|---:|---|
| Gate Threshold | 1462 | 80 written → 73 |
| Gate Attack | 1465 | 109 observed; ALSA range 57..283 exceeds one 7-bit byte |
| Gate Hold | 1467 | 101 observed; ALSA range 0..213 exceeds one 7-bit byte |
| Gate Decay | 1468 | 60 observed; ALSA range 24..300 exceeds one 7-bit byte |
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

Recovering these needs either an inverse device curve or a biquad model.
Until then the GUI shows them Unknown; it never displays the coefficient
as the user value.

## Driver integration

The driver sends the type-20 request at probe, after submitting the meter
reader URB, and on every write of `State Read`.  The reader treats the
first 512-byte frame with the `f0 43 00` header as the start and the next
seven 512-byte frames as its continuations; the 192-byte meter frames are
unaffected.  A workqueue parses the eight frames and applies the direct
fields to the address-keyed control cache under `param_mutex`.  The
transformed fields are not touched.  `State Read Status` returns 0 while
one read is pending, 1 on success, or a negative errno; `State Read Known`
is the parser known mask.  A timeout of two seconds aborts a read without
touching the cache.  `Reset to ZG01` sends the reset and then a state read.

The GUI must read the status/known mask and display only the direct
fields as device-read; the transformed fields and the non-MIC sections
remain the write cache.

## Evidence

Controlled captures (payload diff, one control at a time):

- `captures/24-state-read*.pcap` — initial state reads;
- `captures/00-mic-baseline.pcap` — reference eight-frame dump;
- local `gate off/50 → on/80`: three bytes changed at 1460/1461/1462;
- local gate sub-parameter sweep: range 1464, attack 1465, hold 1467,
  decay 1468;
- local compressor/limiter/EQ sweep: offsets in the tables above;
- local compressor switch `1470` and EQ switch `1628` off→on: exactly one
  byte each;
- `Reset to ZG01` then read: the persisted state (fixture below).

Fixture: `tests/fixtures/mic-state-persisted.bin` — eight 512-byte frames
of the persisted state after Reset. Parser: `src/zg01_state.c`; host test:
`tests/test-state-parser.c`, run by `tests/test-state-parser.sh`.

## Open questions

- The payload contains descriptors and ASCII names (`F_All`, `D_All`,
  `Cust`, `Soprano voice`, `Tenor voice`, `Monitor Eq`, `Headphone`); the
  full descriptor grammar is not decoded.
- The Mixage/Game/Stream and Advanced sections use a different type-20
  id; none has been captured yet.
- Gate threshold and the compressor/limiter attack/release/out gain use a
  device curve; only the observed pairs are known.
- EQ gain/frequency are biquad coefficients; a filter model is required.
- The separator/gap bytes (1463, 1466, 1469, 1471, …) are stable in the
  observed firmware but their meaning is unknown.
