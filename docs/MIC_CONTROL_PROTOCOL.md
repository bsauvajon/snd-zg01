# ZG01 mixer control protocol (reverse engineered)

Decoded from libvirt + usbmon captures on the MIC SETTINGS screen of ZG
Controller (see `captures/`, normalized with `tools/normalize-usbmon.py`)
and cross-checked against the vendor parameter model shipped in *TOOLS
for ZG* (see `docs/VENDOR_PACKAGE.md`).  This describes the device
protocol, not the driver.

Status: the mic parameter **write** and **save** paths are decoded and EQ
band values are decoded exactly; the read path is partially decoded. Every
parameter's address is known from the vendor library rather than from a
capture (`docs/DEVICE_PARAMETER_MAP.md`), so the sub-parameters that were
never captured are labelled from the map; the `f0 43 00` preset block is
still not mapped.

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
command, and `<flag> <id>` is the parameter's address, split into its two
base-128 digits (below).  `0x00` is the high digit of the GATE/COMP/LIMITER
block because those offsets are below `0x80`, and `0x02` the high digit of
the EQ block because it lives on page 1 - the digit is not a block
selector, it is half of the address.

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

A parameter is addressed, not numbered: `<flag> <id>` are the two
base-128 digits of one device address, `address = page * 256 + offset`,
so `flag = address / 128` and `id = address % 128`.  Which gives three
spaces, not two:

| Address | `flag` | `id` |
|---|---|---|
| page 0, offset `0x00`-`0x7f` | `0x00` | the offset |
| page 0, offset `0x80`-`0xff` | `0x01` | `offset - 0x80` |
| page 1, offset `0x00`-`0x76` | `0x02` | the offset |

The middle row is what the game effect, binaural and monitor parameters
need, and it is derived rather than captured: a page 0 offset of `0x80`
and above does not fit the seven bits of the `id` byte, so it cannot
travel any other way, and the application writes into that space (its
monitor page binds `mon_hp_vol` and `mon_sp_vol` to volume sliders).
`docs/EFFECTS_CONTROL_PLAN.md` has the whole argument.  The whole map -
375 parameters with their names - is in
`docs/DEVICE_PARAMETER_MAP.md`, extracted from the vendor library; the
table below is the subset the captures pinned, with the vendor model's
ranges and defaults (`docs/VENDOR_PACKAGE.md`).

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

The rest of the chain is known from the device map
(`docs/DEVICE_PARAMETER_MAP.md`).  That map lists the id of every
parameter, extracted from the vendor library's parameter registration and
checked against these captures: the mic block ids below all come out of
the library, and the ones this driver uses were confirmed on the wire.

| Id | Device name | Vendor model name, range (default) |
|---|---|---|
| `0x00` | `mic_p48v_onf` | P48V on/off |
| `0x01` | `mic_mute_onf` | mute |
| `0x04` | `mic_gate_thhold` | `MicGateThreshold` 1..73 (23) |
| `0x05` | `mic_gate_range` | `MicGateRange` 0..73 (17) |
| `0x06` | `mic_gate_attack` | `MicGateAttack` 57..283 (237) |
| `0x07` | `mic_gate_hold` | `MicGateHold` 0..213 (101) |
| `0x08` | `mic_gate_decay` | `MicGateDecay` 24..300 (188) |
| `0x0b` | `mic_comp_thhold` | `MicCompThreshold` 19..73 (52) |
| `0x0c` | `mic_comp_ratio` | `MicCompRatio` 0..120 (14) |
| `0x0d` | `mic_comp_attack` | `MicCompAttack` 57..283 (237) |
| `0x0e` | `mic_comp_release` | `MicCompRelease` 24..300 (228) |
| `0x0f` | `mic_comp_knee` | `MicCompKnee` 0..2 (1) |
| `0x10` | `mic_comp_outgain` | `MicCompOutgain` 0..180 (50) |
| `0x12` | `mic_eq_name` | the EQ effect name |
| `0x13`-`0x17` | `mic_eq_hi_on/sh/g/f/q` | three-band EQ, high band |
| `0x18`-`0x1b` | `mic_eq_md_*` | three-band EQ, mid band |
| `0x1c`-`0x20` | `mic_eq_lo_*` | three-band EQ, low band |
| `0x23` | `mic_limt_thhold` | `MicLimiterThreshold` 19..73 (65) |
| `0x24` | `mic_limt_attack` | `MicLimiterAttack` 57..283 (237) |
| `0x25` | `mic_limt_release` | `MicLimiterRelease` 24..300 (228) |
| `0x26` | `mic_limt_knee` | `MicLimiterKnee` 0..2 (1) |
| `0x27` | `mic_limt_outgain` | `MicLimiterOutgain` 0..180 (0) |

Note that `0x24`-`0x27` are LIMITER parameters on page 0 while `0x24` is a
MIC EQ shape on page 1: the two pages are separate address spaces and
only the `<flag>` digit tells them apart.

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

The device names behind these ids are the page-1 records
`mic_eq_v2_1_ONF`-`mic_eq_v2_4_ONF` (`0x20`-`0x23`), then
`mic_eq_v2_1_SHAPE` (`0x24`) onwards.  The GAME EQ follows immediately:
`game_eq_v2_1_ONF`-`game_eq_v2_10_ONF` (`0x34`-`0x3d`), then the gains
(`0x3e`-`0x47`), frequencies (`0x48`-`0x51`) and Q (`0x52`-`0x5b`) of the
same ten bands - ten bands, not four, and it is the block the
application's GAME EFFECT EQ page drives.  Both are on page 1, so their
`<flag>` digit is always `0x02` (`docs/DEVICE_PARAMETER_MAP.md`,
`docs/EFFECTS_CONTROL_PLAN.md`).

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

## Driver controls

The driver exposes **208 mixer controls**: the 39 of the microphone DSP
block (P48V and mute, the GATE / COMP / LIMITER switches and levels, the
dynamics sub-parameters and the four-band EQ), the 69 of the microphone
effect chain and the 100 of the game effect chain.  Each row of the table
in `src/zg01_control.c` names a device address from
`docs/DEVICE_PARAMETER_MAP.md`, and
`tests/test-control-addresses.sh` checks that every one of them resolves
to the expected device parameter with the vendor model's bounds.

The two effect blocks have their own record:
`docs/EFFECTS_CONTROL_PLAN.md` - the correspondence between the vendor
names and the device names, the unit chosen for each family, and the
address-digit question the game effect chain raised.

Units are stated only where they are established:

| Control family | Advertised unit | Why |
|---|---|---|
| EQ gain | 0.1 dB, 0 dB = device 180 | vendor model range 0..360, EQ needs cut and boost |
| EQ frequency | Hz | device value is the frequency |
| EQ Q | 0.01 of the vendor ratio, floor 0.50 | device = `12 * log2(Q / 0.5)` |
| switches, levels, and the dynamics sub-parameters | device steps | the physical unit of those steps is not established |

The last row is deliberate: the GATE/COMP/LIMITER thresholds, times, knee
and out gain travel as device steps whose meaning in dB or ms is unknown,
so the controls publish the vendor model's bounds and no conversion.  A
capture of the Windows application moving one of them at a known value
pins the unit (`tools/param-frames.py diff --label`), and the conversion
can then be added to `zg01_codec.h` with the evidence.

Note also that the Windows **MIC SETTINGS** screen only offers a switch
and a 0-100 slider per block: these sub-parameters exist in the device
and in the vendor model, but no screen of the application was captured
driving them, so the driver is ahead of the reference here.  Treat them
as unverified: the frame is the proven one, the values are not.

Two further notes for maintainers:

- the value cache is keyed by *address*, not by an id: page 0 offset
  `0x24` (LIMITER attack) and page 1 offset `0x24` (EQ low shape) are
  different parameters and an id-only cache served one for the other;
- `Mute Capture Switch` follows the device parameter `mic_mute_onf`, so
  writing 1 mutes the microphone.  That polarity is read from the
  vendor's naming and has not been confirmed on hardware yet.

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

The ids the application polls here explain the reply: `flag 0x02` with
ids `0x08`-`0x0b` addresses page 1 offsets `0x08`-`0x0b`, which the device
map names `dirty_flag_mic`, `dirty_mic_fx`, `dirty_game_fx` and
`dirty_monitor`.  The application is not reading a value at all, it is
asking whether a section of the device state changed; the fixed `00 00 00`
/ `00 00 01` answer is that flag.

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

- The value units of the GATE/COMP/LIMITER sub-parameter steps: their ids
  come from the map, the meaning of a threshold or a time in dB or ms does
  not.
- The `f0 43 00` preset block layout and value encoding.
- The full 512-byte state dump layout (mixed descriptors and DSP
  coefficients).
- The exact frame the application sends at page 0 offsets `0x80`-`0xff`:
  the digit follows from the encoding and the space is in use, but no
  capture of such a write has been decoded yet
  (`docs/EFFECTS_CONTROL_PLAN.md`).
- The 1/12-octave grid behind the index-valued frequencies
  (`VoiceTc*Freq`, the game EQ M and S bands): the vendor bounds fit it,
  no value read back from the device has confirmed it.
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
