# Effect chain control plan

The microphone effect chain and the game effect chain, what the driver
exposes of them today, how each control was derived, and what is
deliberately left out.  The protocol facts are in
`docs/MIC_CONTROL_PROTOCOL.md`, the address map in
`docs/DEVICE_PARAMETER_MAP.md` and the vendor model in
`docs/VENDOR_PACKAGE.md`; this document is the implementation record for
the two blocks, in the shape of `docs/MIC_CONTROL_PLAN.md`.

Status: the whole microphone effect block (69) and the game effect block
the vendor model describes (100) are implemented as ALSA mixer controls
on the card, reachable through `zgctl`.  They are part of the full
375-address surface: 367 writable controls and 8 read-only status
exclusions (`docs/FEATURE_PARITY.md`).

## What the two blocks are

The vendor library models 383 parameters in eight blocks
(`docs/PARAMETER_INVENTORY.md`).  The microphone DSP block
(GATE/COMP/EQ/LIMITER) was the first one implemented; these are the two
that come next:

| Block | Parameters | Implemented here |
|---|---|---|
| MIC EFFECT: voice changer, sound effect, echo, reverb, tone control, transpose | 69 | 69 |
| GAME EFFECT: EQ, de-esser, surround | 101 | 100 |

The GAME EFFECT block's 101st parameter is `GameEffectName`, a name index
exposed as `Game Effect Name Steps` (0..255) rather than a physical unit.

The application's screens put these in two places: the **MIC FX** page
(the voice changer, the voice reverb, the tone control, the sound effect
and the echo) and the **GAME EFFECT** page (the three effect slots, the
de-esser and the equaliser).

## Two naming layers, and the correspondence

Nothing in the package carries the link between the two names a parameter
has:

- the **vendor model** names it in CamelCase and gives its bounds
  (`MicEqV21G`, `EchoDelayTime`, `GameDeesserMRatio`) - that is what
  `docs/PARAMETER_INVENTORY.md` lists, extracted from the descriptor
  table of `MemAPI_ZG01.dll`;
- the **device** names it in abbreviated snake_case and gives its address
  (`mic_eq_v2_1_G`, `ech_dly_time`, `gam_dee_m_rat`) - that is what the
  library registers when it loads, and what
  `tools/vendor/device-map-zg01.json` holds.

The correspondence is therefore curated, and it is the one part of this
work that neither source can prove.  It follows a regular rule, so the
exceptions are what a reviewer has to check:

| Vendor suffix | Device suffix | Vendor suffix | Device suffix |
|---|---|---|---|
| `Onf` | `onf` / `on` | `Threshold` | `thhold` / `thr` |
| `Shape` | `sh` | `Ratio` | `rat` |
| `Gain` | `g` | `Attack` | `att` |
| `Freq` | `f` | `Release` | `rel` |
| `Qual` | `q` | `LoFreq`/`HiFreq` | `lo`/`hi` |

with these twelve exceptions, which do not follow it:

| Vendor name | Device name | Why |
|---|---|---|
| `VoiceTransHarmOutLevel` | `voice_harm_level` | the four voice output levels sit together on page 1, away from their blocks |
| `VoiceTransPitchOutLevel` | `voice_pitch_level` | idem |
| `VoiceEffectRadioOutLevel` | `voice_radio_level` | idem |
| `VoiceEffectTremoloOutLevel` | `voice_tremolo_level` | idem |
| `VoiceEffectTremoloDepth` | `vic_fx_rdo_dep` | the device name is a typo of `rdo`/`dep` (radio/depth) kept as registered |
| `GameEffectOnfa/b/c` | `game_fx_a/b/c` | the slot letters are part of the device name |
| `GameEqTotalOnf` | `gam_eq_t_onf` | `Total` is the third game EQ family, abbreviated `t` |
| `GameEqV2<n>Onf` | `game_eq_v2_<n>_ONF` | the ten-band EQ spells `V2` out, bands numbered 1-10 |
| `GameEqV2<n>G/F/Q` | `game_eq_v2_<n>_G/F/Q` | idem |
| `GameFxOnf` | `gam_fx_onf` | device abbreviation `gam` |
| `GameFxType` | `gam_fx_type` | idem |
| `GameDeesserSOnf` | `gam_dee_s_onf` | the M instance spells the switch `_on`, the S instance `_onf` |

Every row of the control table carries both names, and
`tests/test-control-addresses.sh` checks each control's address, range
and default against the map and the model, so a wrong row fails the
build.  That test is the exhaustive, reviewable list; this document only
states the rules.

## Units

The rule of the existing controls holds: a control publishes the unit the
vendor model establishes, and the device steps otherwise
(`src/zg01_codec.h`, `src/zg01_control.c`).

| Family | Control unit | Why |
|---|---|---|
| the 0..360 gains (`VoiceTc*Gain`, the voice output levels, the game EQ gains) | 0.1 dB, 0 dB = 180, -18.0..+18.0 dB | the vendor model's scale, already pinned for the mic EQ in `docs/MIC_CONTROL_PROTOCOL.md` |
| the 0..60 bandwidths (`VoiceTc*Qual`, the game EQ M/S bands, the ten-band game EQ) | 0.01 Q, 0.50..16.00 | idem |
| the ten-band game EQ frequencies | Hz | the device takes the frequency (`GameEqV2<n>F` is 20..20000) |
| the tone control and the game EQ M/S band frequencies | **device index** | not Hz: the device holds an index on a 1/12-octave grid, see below |
| echo, reverb, de-esser, switches, levels, types | device steps | the physical unit is not established, so the vendor bounds are published as they are |

The index grid is the vendor's own: `VoiceTcLoFreq` 4..72, `VoiceTcMdFreq`
4..124, `VoiceTcHiFreq` 60..124 and the game EQ M/S bands with the same
bounds, on a grid anchored at 20 Hz with twelve steps per octave, so

```
frequency_hz = 20 * 2 ** ((index - 4) / 12)
```

which puts index 4 at 20 Hz, 32 at 100 Hz, 60 at 500 Hz, 112 at 10 kHz
and 124 at 20 kHz - the ranges the vendor model gives those parameters,
which is what makes the grid credible.  It is not converted at the
control because the grid is a *fit* to the bounds rather than a value
read from the device: a capture of the application moving one tone
control band would turn it into a measurement, and the conversion can
then be added to `zg01_codec.h`.  Until then the control publishes the
index, as the driver does for the other unestablished units.

## The address digit for the high half of page 0

This is the one part of the protocol the effect blocks had to settle
before any of them could be written, because the parameters do not all
live in the half of the address space the mic block used.

A parameter's address is `page * 256 + offset` and the write frame
carries it as two base-128 digits, `flag = address / 128` and
`id = address % 128`, because a system exclusive data byte holds 7 bits
(`docs/MIC_CONTROL_PROTOCOL.md`).  The consequences:

| Address | Example | `flag` | `id` |
|---|---|---|---|
| page 0, offset 0x00-0x7f | `0x003` gate level | `0x00` | `0x03` |
| page 0, offset 0x80-0xff | `0xe9` monitor volume | **`0x01`** | `0x69` |
| page 1, offset 0x00-0x76 | `0x128` EQ band 1 gain | `0x02` | `0x28` |

Only the first and third rows had ever been seen on the wire: the 21
captured ids of the mic block all sit below 0x80 on page 0 or on page 1.
The microphone effect chain stays in the first row (0x2f-0x6f), but the
game effect chain is almost entirely in the second (0x9d-0xe7), so the
`flag 0x01` digit had to be established rather than assumed.

Two facts settle it:

1. **The digit cannot be `0x00`.**  A page 0 offset of 0x80 and above
   does not fit seven bits, so it cannot travel in the `id` byte; the
   only way to carry it in two 7-bit digits is `0x01` in `flag`.  A model
   in which `flag` were the page number (`0x00` for page 0) would make
   every parameter at page 0 offset 0x80-0xff unaddressable.
2. **The application addresses them.**  `MemAPI_ZG01.dll` registers
   those parameters, and ZG Controller writes some of them live: its
   monitor page binds `PARAM_MONITOR_HP_VOLUME` and
   `PARAM_MONITOR_SP_VOLUME` to sliders and `PARAM_MONITOR_SELECT` to a
   button, which are the page 0 parameters `mon_hp_vol` (0xe9),
   `mon_sp_vol` (0xea) and `mon_select` (0xe8).  A volume slider that
   works is a parameter the application writes, so the digit is in use on
   real hardware.

What is still not observed is a *capture* of such a write, so the exact
frame the application sends there is derived, not decoded.  The frame is
the proven one and the digit follows from the encoding; a single usbmon
capture of the monitor volume slider (or of a game EQ slider, same
space) would turn the derivation into a measurement, with
`tools/param-frames.py parse` to read it back:

```sh
tools/capture-usbmon.sh ...            # see docs/PROTOCOL_CAPTURE.md
tools/param-frames.py parse capture.jsonl | grep flag=0x01
```

The contract test pins the derivation for the whole map (all 375
addresses, 128 of them with `flag 0x01`), so the space stays visible and
a regression cannot silently move a parameter.

## Device-step controls and read-only rows

The full map is exposed: "not implemented" is no longer a list of missing
parameters.  Every address is either a writable control or one of the
eight read-only status exclusions.  The rows that publish device steps
instead of an established physical unit are:

| Parameter | Count | What is published |
|---|---|---|
| `gam_eq_t_hi/mh/ml/lo_*`, the four-band "Total" game EQ bands | 18 | raw 0..`ZG01_VALUE_MAX` steps: the vendor *descriptor table* has no row for them (no bounds, no default), so the control map deliberately does not guess the M/S ranges. The family's master switch `GameEqTotalOnf` is vendor-model. |
| `MicEqV2<n>Onf` and `MicEqV2 2/3Shape` | 6 | vendor-model switches, 0..1 steps |
| `game_fx_name`, `mic_eq_name`, `pre_vic_name`, `mon_eq_name` | 4 | vendor-model name indices, 0..255 steps |
| `pre_vic_on1`, `pre_vic_on2` | 2 | vendor-model switches, 0..1 steps |
| the remaining raw block rows (legacy mic EQ, legacy monitor EQ, and the HDMI/streaming gaps `hdmi_4k`, `strm_out_mute`, `strm_mode`) | 35 | 0..`ZG01_VALUE_MAX` steps, requiring `--allow-raw` on load. Together with the 18 Total game EQ rows above, the map has 53 raw rows. |

The monitor, binaural, streaming and mic-chain remainder blocks are also
controls now, with vendor-model bounds where the correspondence is
established and raw steps otherwise.  What remains is *units*, not
surface: a capture of the application moving one value turns a device
step or a grid index into a physical unit, and the conversion can then
be added to `zg01_codec.h`.

The eight read-only status rows (`dirty_flag_mic`, `dirty_mic_fx`,
`dirty_game_fx`, `dirty_monitor`, `dirty_streming`, `mic_level`,
`mic_monitor_level_knob`, `mon_level`) are present in the userspace map
but have no ALSA control.

The M and S game EQ families *are* implemented even though no screen of
the application drives them: the vendor model gives their bounds and
defaults, and the driver already publishes parameters the application
does not expose (`docs/MIC_CONTROL_PROTOCOL.md`, the GATE/COMP/LIMITER
sub-parameters).  Their meaning is not documented anywhere in the
package, so the controls carry the model's own letters and this document
does not invent a reading for them.

## Maintaining the table

The rows are the driver's own data: one line per control in
`zg01_control.c`, one address macro per parameter in `zg01.h`, and one
expectation per control in `tests/test-control-addresses.sh`.  A new
block is added the same way this one was:

1. read the addresses from `tools/vendor/device-map-zg01.json` and the
   bounds from `docs/PARAMETER_INVENTORY.md`;
2. decide the unit for each parameter - established, or device steps;
3. add the macro, the row and the expectation, keeping the three in step
   (the test fails if they are not);
4. state the correspondence and the unit decision in this document.

`tests/test-control-addresses.sh` runs in CI, so the table cannot drift
from the map or from the model without the build failing.
