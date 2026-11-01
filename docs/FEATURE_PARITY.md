# Feature parity with ZG Controller

What the card exposes, what this driver covers today, and what is still
missing to reach the Windows application.  The parameter list itself is
generated in `docs/PARAMETER_INVENTORY.md`; the protocol facts are in
`docs/MIC_CONTROL_PROTOCOL.md` and the vendor model in
`docs/VENDOR_PACKAGE.md`.

## The surface

The vendor library models **383 names** (331 CamelCase plus 52 legacy
snake_case entries; `_ForBackup` mirrors are excluded except for the two
that are the only row for their parameter).  The device map has **375
addressable parameters**.  The control map classifies every one:
**367 writable ALSA controls** and **8 read-only status exclusions**.  Of
the 367, 208 are the original manual controls and 159 are generated from
the control map (106 vendor-model rows and 53 raw device-step rows).

The blocks follow the application screens:

| Block | Vendor names | Driver today |
|---|---|---|
| Mic: GATE, COMP, EQ, LIMITER, mute, P48V | 54 | exposed: the 39 original controls plus the remaining mic rows; `MicLevel` is read-only |
| MIC EFFECT: voice changer, SE, echo, reverb, tone control, transpose | 69 | exposed: the original 69 controls |
| GAME EFFECT: EQ, de-esser, surround | 101 | exposed: the original 100 controls plus `GameEffectName` |
| Monitor / headphone: 4-band EQ, levels, select | 27 | exposed; `MonitorLevel` is read-only |
| Binaural (3D chat space) | 41 | exposed |
| Streaming output mixer: USB, HDMI, PC, per-input link/mute/FX/level | 22 | exposed; the dirty flags are read-only |
| Presets and current selection | 6 | the addressable rows (`CurrentGameEffectButton`, `PresetVoiceName`, `PresetVoiceOnf1/2`) are controls; `L11`/`R10` are display strings, not parameters. Preset files load with `zgctl preset list/show/load` (`docs/PRESET_FORMAT.md`); the application's preset-selection traffic is still unmapped |
| Other controls | 11 | exposed |
| Legacy snake_case names, several of them read-only status | 52 | a second naming layer: the 43 addressable names are covered by their addresses above; the 9 status-only names have no address |

Plus the device actions (already implemented): `Save to ZG01` and
`Reset to ZG01`.

The eight read-only exclusions are `dirty_flag_mic`, `dirty_mic_fx`,
`dirty_game_fx`, `dirty_monitor`, `dirty_streming`, `mic_level`,
`mic_monitor_level_knob` and `mon_level`.  They are present in the
userspace map but have no ALSA control, so `preset load` reports them as
unsupported.  `StreamingMicLevel` is *not* one of them: it is a writable
mixer level, now exposed through its `_ForBackup` model row as
`Streaming Mic Level Steps` (0..127).

Read-only status names carried by the card, with no address in the
375-row map: `sys_samplingrate`, `inpt_is_anlg`, `outpt_is_anlg`,
`sys_hs_loopback`, `sys_is_mtrx_crosstie`, `meter_mic_output`,
`meter_pc_in`.  The level meters are streamed continuously (the ~50 ms
192-byte frames on EP `0x83`); nothing in the driver reads them.

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
| Address digits | derived for every address, not captured: `flag = address / 128` and `id = address % 128`, so page 0 offsets `0x80`-`0xff` travel as `flag 0x01` (`docs/EFFECTS_CONTROL_PLAN.md`) |
| Cached reads | documented (the device answers reads with a status, not a value) |

## What is missing, and why

1. **The addresses are no longer missing.**  An earlier revision of this
   document listed the wire id of the other ~360 parameters as the first
   gap; the device map closed it (`docs/DEVICE_PARAMETER_MAP.md`): the
   library registers every parameter with its address, and the two digits
   the frame carries are that address split into base-128
   (`flag = address / 128`, `id = address % 128`).  What a capture still
   buys is a **value** encoding - the physical unit behind a threshold or
   a time step, which no table holds - and confirmation of the digits
   that were never observed, the page 0 offsets at or above `0x80`
   (`docs/EFFECTS_CONTROL_PLAN.md`).
2. **The read path.**  Per-parameter reads return a status byte, so the
   device state must come from the `F0 43 20` state dump, which is not
   decoded.  Decoding it (still undecoded DSP coefficient blocks) is what
   would let the driver reflect settings changed elsewhere, and it is the
   prerequisite for showing the meters.
3. **Meters.**  The ~50 ms 192-byte `0x83` frames are identified as
   meters but not decoded; the vendor model gives their scale (0..127) and
   names, so a capture with known signals should be enough to pin the
   layout.
4. **Preset export and selection.**  Factory and user presets are
   `*.zgpreset` files in Yamaha's MBDF container.  The container,
   components, schema records and value layout are decoded
   (`docs/PRESET_FORMAT.md`), and `zgctl preset list/show/load` reads and
   applies them.  Exporting a preset from Linux remains out of scope, and
   the application's preset selection is not exposed; the `F0 43 00` block
   it sends after each edit is still unmapped.  `.zg01` backups are
   rejected with a clear error: their multi-group `F_CurrentScene` layout
   is a follow-up, not a supported input.

## Suggested order

1. ~~Mic DSP completion~~, ~~MIC EFFECT~~ and ~~GAME EFFECT~~ - done.  The
   mic block's sub-parameters, the microphone effect chain and the game
   effect chain are implemented and covered by the contract tests
   (`docs/EFFECTS_CONTROL_PLAN.md`).  Together with the generated rows
   they give 367 of the map's 375 parameters; the other eight are the
   read-only status exclusions.
2. **Units for the remaining blocks** - monitor / headphone, streaming,
   binaural, the legacy three-band EQ and the button/name slots are now
   exposed as controls, but several publish device steps or grid indices
   because their physical unit is not established.  A capture of the
   application moving one value pins each unit
   (`docs/EFFECTS_CONTROL_PLAN.md`).
3. **Preset selection and the `F0 43 00` block** - the six preset
   name/index controls are exposed; the application's preset-selection
   traffic is still unmapped.
4. **Meters and status** - needs the state dump or the meter frame
   decoding, and it is what makes a GUI or a mixer application possible.
5. ~~The rest of the mic chain~~ - the legacy three-band EQ, the button
   function slots and the per-band enables are exposed as controls; their
   units are part of item 2.
6. ~~Binaural~~ - the 41 parameters are exposed; their units are part of
   item 2.
7. ~~Presets~~ - `*.zgpreset` decoding and loading are done
   (`docs/PRESET_FORMAT.md`, `zgctl preset load`); exporting a preset and
   the `.zg01` backup layout are follow-ups.

## Hardware and operational notes

- The physical knobs and buttons of the ZG01 are not modelled here: they
  are probably not parameters of this library but they do change the same
  state, which is another reason the state dump matters.
- Firmware update traffic is deliberately out of scope, as in
  `docs/MIC_CONTROL_PLAN.md`.
- Every new message must go through the same discipline: decode from a
  capture, state the direction and bounds, add a fixture, expose it as a
  control, and pin the encoding in a host test before it reaches a user.
