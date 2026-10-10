# ZG01 mixer control protocol (reverse engineered)

Decoded from libvirt + usbmon captures on the MIC SETTINGS screen of ZG
Controller (see `captures/`, normalized with `tools/normalize-usbmon.py`)
and cross-checked against the vendor parameter model shipped in *TOOLS
for ZG* (see `docs/VENDOR_PACKAGE.md`).  This describes the device
protocol, not the driver.

Status: the mic parameter **write** and **save** paths are decoded and EQ
band values are decoded exactly; the read path is partially decoded. The
GATE/COMP/LIMITER sub-parameter ids and the `f0 43 00` preset block are
not mapped yet.

## Transport: USB-MIDI packets carrying Yamaha system exclusive

Two USB paths are involved:

- **EP0 vendor requests** for enumeration/status only (`0xc0 bRequest 7`
  -> `80bb00`, `0xc0 bRequest 4` -> 1-byte poll, `0xc0 bRequest 6` ->
  UTF-16 channel names).
- **Interface 4 bulk** for every mixer parameter:
  - host -> device on EP `0x03`
  - device -> host on EP `0x83`
  - 512-byte frames (some device frames are shorter).

A frame is **not** a vendor-private structure: it is a stream of 4-byte
**USB-MIDI event packets** (USB-MIDI 1.0, cable 0) carrying a plain
**Yamaha system exclusive** message.

Every 4-byte packet starts with a code index number (CIN):

| CIN | Meaning | Data bytes in the packet |
|---|---|---|
| `0x04` | SysEx continues | 3 |
| `0x05` | SysEx ends | 1 |
| `0x06` | SysEx ends | 2 |
| `0x07` | SysEx ends | 3 |
| `0x00` | padding (rest of the 512-byte buffer) | - |

That answers the earlier open question about the "terminator type": the
first byte of the last packet is not a message class, it is how many data
bytes are left when the SysEx ends.  `05 f7` means "one byte left (F7)",
`06 00 f7` means "two bytes left", `07 00 01 f7` and `07 04 00 f7` mean
"three bytes left".

Reassembled, a parameter write is:

```
F0 43 10 | 3E 14 01 | 01 00 <flag> | <id> 00 00 | 00 00 00 | 00 00 00 | <d0> <d1> <d2> | F7
```

`F0 43 10` is Yamaha + device id 0x10, `3E 14` is the parameter-change
command, and `<flag>` is `0x00` for the GATE/COMP/LIMITER block, `0x02`
for the EQ block.

## Value encoding: base-128 digits

Every SysEx data byte is limited to 7 bits, so parameter values travel as
**big-endian base-128 digits** - three of them for the parameters this
driver exposes:

```
value = d0 * 16384 + d1 * 128 + d2      (each digit <= 0x7f)
```

The first version of this analysis read those three digits as a 24-bit
*hexadecimal* number, which produced every "weird" figure in the EQ
table.  The mapping is:

| UI value | frame bytes `d0 d1 d2` | earlier reading (hex) |
|---|---|---|
| 20 Hz | `00 00 14` | 20 |
| 50 Hz | `00 00 32` | 50 |
| 200 Hz | `00 01 48` | 328 |
| 500 Hz | `00 03 74` | 884 |
| 1 kHz | `00 07 68` | 1896 |
| 2 kHz | `00 0F 50` | 3920 |
| 5 kHz | `00 27 08` | 9992 |
| 10 kHz | `00 4E 10` | 19984 |
| 15 kHz | `00 75 18` | 29976 |
| 17 kHz | `01 04 68` | 66664 |
| 20 kHz | `01 1C 20` | 72736 |
| 0.0 dB gain | `00 01 34` | 308 |
| +18.0 dB gain | `00 02 68` | 616 |
| Q 1.00 | `00 00 0C` | 12 |

Two consequences explain the earlier artefacts exactly:

- For a two-digit value the hex reading is `2 * value - d2`, so the
  "small, non-uniform deficit below 16 kHz" **was** the low digit `d2`:
  deficits 72, 116, 104, 80, 8, 16, 24 for 200, 500, 1000, 2000, 5000,
  10000, 15000 Hz.
- Above 16384 Hz the leading digit `d0` becomes 1 and the hex reading
  gains `0x10000`, which the first analysis recorded as a "+32768 offset"
  on its doubled scale (16384 = 2^14 = 128^2 is simply where a third
  digit appears).

There is no table and no piecewise law: the device takes the value in its
own unit and the driver encodes it (`src/zg01_codec.h`).  Values below
128 (all switches, the 0..100 levels, Q) look identical in both readings,
which is why GATE/COMP/LIMITER and Q looked sane and the EQ did not.

## Parameter write (EP 0x03)

```
04 f0 43 10 | 04 3e 14 01 | 04 01 00 <flag> | 04 <id> 00 00
          | 04 00 00 00 | 04 <d0> <d1> <d2> | 05 f7 00 00 | 00...
```

Each group of four bytes is one USB-MIDI packet (`0x04` + three data
bytes); the value occupies the last three data bytes and the final packet
carries the SysEx end (`0x05` + `F7`).  A `0x04` in place of that closing
`0x05` makes the firmware ignore the whole frame.

## Parameter IDs

The mic parameter names, ranges and defaults come from the vendor MemAPI
library (`MicGate*`, `MicComp*`, `MicEq*`, `MicLimiter*`, see
`docs/VENDOR_PACKAGE.md`); the ids are the ones observed on the wire.

| Block | Id | Vendor name | Range / default |
|---|---|---|---|
| GATE | `0x02` | `MicGateOnf` | 0/1, default 0 |
| GATE | `0x03` | `MicGateLevel` | 0..100, default 50 |
| COMP | `0x09` | `MicCompOnf` | 0/1, default 0 |
| COMP | `0x0a` | `MicCompLevel` | 0..100, default 50 |
| EQ | `0x11` | `MicEqOnf` | 0/1, default 0 |
| LIMITER | `0x21` | `MicLimiterOnf` | 0/1, default 1 |
| LIMITER | `0x22` | `MicLimiterLevel` | 0..100, default 50 |

Enable writes use value `0`/`1`.  The gaps suggest per-block id ranges:
GATE `0x02-0x08`, COMP `0x09-0x10`, EQ `0x11-0x20`, LIMITER `0x21+`.

Unmapped ids in the same write space: the blocks' sub-parameters exist
in the vendor model, but their ids were not identified in the captures
yet.  Their names and ranges, useful to label a capture, are
`MicGateThreshold` 1..73 (default 23), `MicGateRange` 0..73 (17),
`MicGateAttack` 57..283 (237), `MicGateHold` 0..213 (101),
`MicGateDecay` 24..300 (188), `MicCompThreshold` 19..73 (52),
`MicCompRatio` 0..120 (14), `MicCompAttack` 57..283 (237),
`MicCompRelease` 24..300 (228), `MicCompKnee` 0..2 (1),
`MicCompOutgain` 0..180 (50), `MicLimiterThreshold` 19..73 (65),
`MicLimiterAttack` 57..283 (237), `MicLimiterRelease` 24..300 (228),
`MicLimiterKnee` 0..2 (1), `MicLimiterOutgain` 0..180 (0).

### EQ ids

EQ edits reach the card live (no `Save to ZG01` needed) and use the same
write frame with `<flag> = 0x02`.

| Id | Parameter | Unit and range | Default |
|---|---|---|---|
| `0x24` | `MicEqV21Shape` (low shape) | 0/1 | 1 |
| `0x27` | `MicEqV24Shape` (high shape) | 0/1 | 1 |
| `0x28`-`0x2b` | band 1-4 gain | 0.1 dB, 0..360 (0 dB = 180) | 180 |
| `0x2c`-`0x2f` | band 1-4 frequency | Hz: 20-1000, 20-20000, 20-20000, 500-20000 | 100, 500, 2500, 10000 |
| `0x30`-`0x33` | band 1-4 Q | 12 units per octave, 0..60 (Q 0.5-16) | 12 |

`MicEqV21`-`MicEqV24` are the four bands of the MIC EQ.  That mapping is
pinned by the vendor model: it lists exactly four bands whose frequency
ranges are 20-1000, 20-20000, 20-20000 and 500-20000 Hz, which is the
order the id groups (`0x28`, `0x2c`, `0x30`) follow.

The band frequency is *not* restricted to a grid by the protocol: the
device takes any Hz value inside the band's range.  The vendor interface
offers a step grid, but that is a ZG Controller choice; the driver
exposes plain Hz.

Shape: `0x24` is the low-frequency shelf type and `0x27` the
high-frequency shelf type; in shelf mode the band Q is hidden (band 1 for
low shape, band 4 for high shape).

The companion `04 f0 43 00` preset block that the app sends after each OK
stores the band-1 gain as `0x0168` (`360`) in what looked like a different
(0.1 dB) scale, and truncates the frequency value to its low bits; the
live write is what the card accepts.  The block's own value encoding is
still unexplained (see open questions).

After each EQ OK the app also sends the full `04 f0 43 00` preset block
and an apply command `04 f0 43 30 | 04 3e 14 03 | 04 02 06 00 | 07 00 01 f7`.

## Save (`Save to ZG01`, EP 0x03)

```
04 f0 43 30 | 04 3e 14 03 | 04 02 01 00 | 07 00 01 f7 | 00...
```

Present exactly once in the two captures where `Save to ZG01` was
pressed, absent otherwise. Settings apply live; Save only persists them.

## Reset / reload (`RESET`, EP 0x03)

```
04 f0 43 30 | 04 3e 14 03 | 04 02 04 00 | 07 00 01 f7 | 00...
```

Same shape as save, with code `0x04` instead of `0x01`. Observed once,
right after disabling the GATE and pressing `RESET`; the app then
re-reads the whole state (type-20 request plus type-30 per-id reads).
It reloads the persisted settings, dropping unsaved edits.

## Reads

The app sends a keepalive about once a second:

```
04 f0 43 10 | 04 3e 14 00 | 07 04 00 f7
```

At screen open the app issues read requests on EP `0x03`:

- `04 f0 43 20 | 04 3e 14 01 | 04 01 00 00 | 04 00 00 <id> | 05 f7`
- `04 f0 43 30 | 04 3e 14 01 | 04 01 00 02 | 04 <id> 00 00 | 06 00 f7`
  (one per id, e.g. `0x08..0x0b`)

The `04 f0 43 30` form is answered on EP `0x83` with a fixed status, not
the parameter value; e.g. for id `0x08` it returned both `04 00 00 00`
and `04 00 00 01`, and for the mic LIMITER id `0x22` it always returns
`04 00 00 01` regardless of the value written.  The mic value space is
therefore not readable through this request.  Reply payloads use the same
base-128 digits as the writes.

The `04 f0 43 20` form makes the device push a **full state dump** on EP
`0x83`: several 512-byte frames that mix parameter descriptors with DSP
coefficient blocks (biquads), not a simple `id -> value` table.  The
dump is not decoded: diffs between known states do not expose the UI
values directly.

## Reading values: current behaviour

The device exposes no usable per-parameter read, and the state dump is
not decoded, so the driver's kcontrol getters return the **last value the
driver wrote** (a cache).  This is the official, documented behaviour.
The cache is seeded with the vendor defaults, not with guesses.

Consequences:

- After a fresh load the cache holds the vendor defaults (frequencies
  100 Hz / 500 Hz / 2.5 kHz / 10 kHz, Q 1, gains 0 dB, shelf shapes on,
  gate/comp/limiter levels 50, limiter switch on, everything else off),
  not the device's current state.
- Settings changed outside the driver (ZG Controller on Windows, or a
  `Reset to ZG01`) are not reflected until the corresponding control is
  written from Linux.
- `Save to ZG01` / `Reset to ZG01` do not refresh the cache.

Decoding the state dump, if ever wanted, is a separate investigation.

## Other observed frames

- `04 f0 43 00 | 04 3e ...` carries the preset block: ASCII strings split
  across words (`_Preset Voice`, `Soprano voice`, `Tenor voice`,
  `_Monitor Eq`) plus per-band values.
- `04 f0 43 10 | 04 3e 14 00 | 04 70 02 00 | ...` carries version/identity
  strings (`BOOT`, `V1.0`, `MAIN`, `ICE`).
- The 192-byte `0x83` frames (`04 f0 43 10 | 04 3e 14 02 | ...`, ~50 ms)
  are level meters, not parameters.

## Open questions

- GATE/COMP/LIMITER sub-parameter ids (attack, release, ratio, knee,
  hold, ...) and their value units.
- The `f0 43 00` preset block layout and value encoding.
- The full 512-byte state dump layout (mixed descriptors and DSP
  coefficients).
- Meaning of the unused ids between the blocks (`0x25`, `0x26`, `0x34+`).
- A GATE drag was once seen to send `0x69` (105) where controlled writes
  give 0..100 (`0x32` = 50): the scale is 0..100 per the vendor model, so
  that 105 was a transient drag value, but it is not proven that the
  firmware clamps above 100.

## Analysis history

The first pass (bus/device usbmon captures) got the transport and the ids
right but misread every value above 127, because the three base-128
digits were interpreted as a hexadecimal 16-bit number.  The symptoms
were a "piecewise frequency table with a small deficit", a "gain scale of
308 at 0 dB", a "different 0.1 dB scale in the preset block" and a
"+32768 offset" above 16 kHz.  They all disappear once the SysEx framing
and the base-128 digits are taken into account; `tests/test-param-codec.sh`
pins the corrected mapping against the captured bytes.

Concretely, the previous encoder sent `00 01 9B` for +6.0 dB of EQ gain
and `00 5D AC` for 12 kHz: `0x9B` and `0xAC` are not valid 7-bit data
bytes, so the firmware rejects those frames.  Where the digits stayed
valid the value was simply wrong (+12.0 dB came out as 257 instead of
300, i.e. +27 dB, and 8 kHz as 8051 Hz).  Only the values that had been
measured by hand - 1 kHz, 0 dB, +-18 dB - happened to be encoded
correctly, which is why the defect was not obvious.
