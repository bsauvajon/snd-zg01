# Feature parity with ZG Controller

What the card exposes, what this driver covers today, and what is still
missing to reach the Windows application.  The parameter list itself is
generated in `docs/PARAMETER_INVENTORY.md`; the protocol facts are in
`docs/MIC_CONTROL_PROTOCOL.md` and the vendor model in
`docs/VENDOR_PACKAGE.md`.

## The surface

The vendor library models **383 parameters** (331 CamelCase plus 52 legacy
snake_case entries; `_ForBackup` mirrors excluded), grouped in the same
blocks as the application screens:

| Block | Parameters | Driver today |
|---|---|---|
| Mic: GATE, COMP, EQ, LIMITER, mute, P48V | 54 | 39 controls: P48V, mute, switches and levels, the GATE/COMP/LIMITER sub-parameters (threshold, range, times, knee, out gain), EQ switch, 2 shapes, 4 gains, 4 frequencies, 4 Q |
| MIC EFFECT: voice changer, SE, echo, reverb, tone control, transpose | 69 | none |
| GAME EFFECT: EQ, de-esser, surround | 101 | none |
| Monitor / headphone: 4-band EQ, levels, select | 27 | none |
| Binaural (3D chat space) | 41 | none |
| Streaming output mixer: USB, HDMI, PC, per-input link/mute/FX/level | 22 | none |
| Presets and current selection | 6 | none |
| Other controls | 11 | none |
| Legacy snake_case names, several of them read-only status | 52 | none |

Plus the device actions (already implemented): `Save to ZG01` and
`Reset to ZG01`.

Read-only status reported by the card, visible in the legacy names:
`sys_samplingrate`, `inpt_is_anlg`, `outpt_is_anlg`, `sys_hs_loopback`,
`sys_is_mtrx_crosstie`, `meter_mic_output`, `meter_pc_in`, plus the level
parameters used as meters (`MicLevel`, `MicMonitorLevel`,
`StreamingMicLevel`, ... 0..127 in the vendor model).  Only the physical
level meters are streamed continuously (the ~50 ms 192-byte frames on EP
`0x83`); nothing in the driver reads them.

## What is already known

**The device address of every parameter is now known**, not just the ones
captured: the vendor library registers all of them when it loads, and
`tools/vendor/extract-device-map.py` reads that registration back
(375 parameters: 256 on page 0, 119 on page 1, with their names).  The
address travels as two base-128 digits, `address = page * 256 + offset`;
`docs/DEVICE_PARAMETER_MAP.md` has the full table and the evidence.

So parity no longer needs a capture per control: a control can be
implemented from the map plus the vendor model's range and default
(`docs/PARAMETER_INVENTORY.md`), and the capture tool
(`tools/param-frames.py`) is only needed to confirm a value encoding.

| Piece | State |
|---|---|
| Transport | interface 4 bulk, EP `0x03` out / `0x83` in, 512-byte frames |
| Framing | USB-MIDI event packets carrying Yamaha SysEx (`F0 43 10 3E 14 ... F7`) |
| Values | three big-endian base-128 digits (see `src/zg01_codec.h`) |
| Message classes | `F0 43 10` write, `F0 43 30` read/save/reset, `F0 43 20` state dump request, `F0 43 00` preset block, `F0 43 40` (unused by the driver) |
| Units and bounds | the whole vendor model, ranges and defaults included |
| Mic DSP addressing | 21 wire ids captured (GATE, COMP, EQ switch/shapes/bands, LIMITER) |
| Cached reads | documented (the device answers reads with a status, not a value) |

## What is missing, and why

1. **The wire id of the other ~360 parameters.**  It is *not* data: the
   descriptor table of `MemAPI_ZG01.dll` holds the name, bounds, default
   and a per-parameter key, but no id (checked: no descriptor slot equals
   any captured id, the key is not a CRC of the name, the table is not
   ordered by address, and each name has exactly one pointer in the whole
   image).  The id is therefore computed by the library's code, which
   means either of:
   - **captures**: one deliberate action per screen, as
     `docs/PROTOCOL_CAPTURE.md` describes - the method that already
     produced the 21 mic ids.  Cost is linear in the number of controls;
   - **disassembly**: `MemAPI_ZG01.dll` is small (261 functions by
     `.pdata`, ~95 KB of `.text`), the message builders are identifiable
     through the SysEx fragment table at `0x18002cf00`, and the parameter
     command dispatcher is a jump table over `0x100..0x122`.  Recovering
     the address computation there would yield every id at once, then a
     few captures confirm the result.  This is the higher-leverage route
     and it needs no hardware.
2. **The read path.**  Per-parameter reads return a status byte, so the
   device state must come from the `F0 43 20` state dump, which is not
   decoded.  Decoding it (still undecoded DSP coefficient blocks) is what
   would let the driver reflect settings changed elsewhere, and it is the
   prerequisite for showing the meters.
3. **Meters.**  The ~50 ms 192-byte `0x83` frames are identified as
   meters but not decoded; the vendor model gives their scale (0..127) and
   names, so a capture with known signals should be enough to pin the
   layout.
4. **Presets.**  Factory and user presets are `*.zgpreset` files in
   Yamaha's MBDF container (chunked, bit-packed).  Not decoded; needed for
   preset selection and for storing user presets, and it is also the
   likely ground truth for the `F0 43 00` block the application sends
   after each edit.

## Suggested order

1. **Mic DSP completion** - the block's remaining ids (gate attack, hold,
   decay, range, threshold; compressor ratio, knee, outgain, attack,
   release, threshold; limiter equivalents) plus mute and P48V.  Same
   frame, same space as the ids already known, so a small capture set or
   the disassembly route closes it.
2. **Streaming output mixer** (22 parameters, switches and levels) - the
   block with the highest user value once the mic is done, and cheap:
   mostly 0/1 and 0..100 values in one space.
3. **Monitor / headphone EQ** - same shape as the MIC EQ (4 bands,
   `MonitorEqV21..V24`) plus `MonitorHpVolume` / `MonitorSpVolume`.
4. **Meters and status** - needs the state dump or the meter frame
   decoding, and it is what makes a GUI or a mixer application possible.
5. **GAME EFFECT** and **MIC EFFECT** - the largest blocks, with
   preset-driven editing; do them once presets are decoded.
6. **Binaural** - 41 parameters, least likely to be needed first.
7. **Presets** - `*.zgpreset` decoding, then read/write through the
   `F0 43 00` block.

## Hardware and operational notes

- The physical knobs and buttons of the ZG01 are not modelled here: they
  are probably not parameters of this library but they do change the same
  state, which is another reason the state dump matters.
- Firmware update traffic is deliberately out of scope, as in
  `docs/MIC_CONTROL_PLAN.md`.
- Every new message must go through the same discipline: decode from a
  capture, state the direction and bounds, add a fixture, expose it as a
  control, and pin the encoding in a host test before it reaches a user.
