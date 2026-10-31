# ZG01 meter frames (bulk IN 0x83)

Status: **partial**. The framing, the header, the 14-bit field layout and
the dB scale are measured. Three meters are decoded with stated
confidence. Three are not decoded: `mic_in`, `mic_out` and `hdmi_in`.

## Transport

- Device-to-host bulk IN on EP `0x83`, interface 4.
- Each frame is 192 bytes: 48 USB-MIDI event packets of 4 bytes.
- Packet 0 byte is the USB-MIDI code index (CIN). Packets 0-46 use CIN
  `0x04` (SysEx start/continue, three data bytes); packet 47 uses CIN
  `0x07` (SysEx end, three data bytes). Verified on all frames of the
  captures.
- Stripping the CIN bytes gives a 144-byte SysEx message (the payload
  below). Payload bytes are 7-bit (`< 0x80`) except the `F0`/`F7`
  delimiters.
- Frames arrive once the driver's `0x83` reader is running and the 1 Hz
  `0x03` keepalive is sent (driver commits `93ae2cb` and `9f352fe`). The
  sweep frames arrived at about 20 Hz (roughly every 50 ms).

Raw frame offset `r` maps to payload index `p` as `p = 3*(r//4) + (r%4) - 1`
for `r % 4 != 0`. Payload index `p` maps to raw offset `r = 4*(p//3) + 1 + p%3`.
The `columns` command reports raw offsets.

## Payload layout

| Payload index | Content |
|---|---|
| 0-5 | `F0 43 10 3E 14 02`: SysEx start, Yamaha ID `43`, then `10 3E 14 02` (meter message). |
| 6 | `03`, constant over all captured frames. |
| 7-142 | 68 big-endian 14-bit fields as 7-bit pairs `hi lo` (`code = hi*128 + lo`). |
| 143 | `F7`: SysEx end. |

`hi` is first. The carry shows this: `79 7e` followed by `7a 08` is a
step of +10 (`0x3CFE` to `0x3D08`), which only works with `hi` first.

52 of the 68 fields are constant in every frame (code `15114`, i.e. `76 0a`).
The other 16 vary.

## Level scale (measured)

Each meter level field is a dB value:

```
code = hi*128 + lo
level_dB = (code - 16384) / 10        # 16384 = 0 dBFS, 10 codes per dB
```

- The codes are always `== 4 (mod 10)`. Since `16384 == 4 (mod 10)`, the
  quantisation is exact at 1 dB, and the earlier `10*k + 4` observation
  is this scale.
- `code 15114` is -127 dB. Many captures sit at this floor.
- `decode_frame` returns `level_dB + 127` as an integer, clamped to
  `0..127` (0 = -127 dB, 127 = 0 dBFS). This is linear in dB, the device's
  own unit, on a 0-127 range.

The scale was measured with 1 kHz sines played straight into the card
(`hw:zg01,1` Voice Out and `hw:zg01,0` Game Out) at −70 to −6 dBFS. See
"Measurements" for the table.

## Decoded meters

| Key | Field (hi, lo raw offsets) | Series | Confidence |
|---|---|---|---|
| `pc_in` | (159, 161) | payload 119/121/123/125 | High |
| `game_mix` | (181, 182) | payload 135/137/139/141 | Medium-low |
| `stream_mix` | (10, 11) | payload 7/9 | Medium-low |

Not decoded (omitted from `decode_frame`):

- `mic_in`, `mic_out`: no series responded to a mic source. A 1 kHz tone
  from the laptop speakers at −40, −20, −10 dBFS (run3) moved no meter,
  and the `Voice In` capture showed no tone either. No series isolates
  the mic path: the mic path would respond to acoustic input, and no
  series did.
- `hdmi_in`: no HDMI source was available.

### Identity notes

- **`pc_in`**: responds exactly to Voice Out playback (0 dB) and not to Game
  Out. This matches the GUI's VOICE strip (`Meter PC In`, Voice Out sink).
- **`game_mix`**: responds to Game Out playback at −3 dB (−4 dB at the
  top), and also to Voice Out at 0 dB. No series responds to Game Out
  alone. The label is the best match for the GAME path's dominant meter.
  The controller should confirm it against the ZG Controller's Game meter.
- **`stream_mix`**: responds to both playback paths through a power sum,
  and has a noise floor of about −70.5 dB (the group above has −89 dB).
  That floor fits a stream mix that carries a noise term from the mic
  path. The label is medium-low confidence.
- **Series 11/13** (and copies 111/113/115/117): track 7/9 about 54 dB
  lower, with a floor clamped at −127 dB. Their identity is not
  established, so they are not decoded.

### Stream curve (measured)

The stream meter is not linear in dB. Its response fits a power sum:

```
stream_dB ≈ 10*log10( 10^((V + G)/10) + 10^(N/10) )
  V = input level (dBFS), G ≈ −40 dB (sends), N ≈ −70.5 dB (floor)
```

The fit is within 0.7 dB RMS over the voice and game steps. The device
produces this curve, so the decoder returns the device value unchanged.

## Measurements

Voice Out playback, 1 kHz sine, nominal dBFS (run4 and run2). Values are
the decoded level in dB (window: after the ~0.7 s onset).

| Nominal | `pc_in` (119) | 135 group | 7 (`stream`) |
|---|---|---|---|
| −70 | −70.0 | −70.0 | −71.3 |
| −60 | −60.0 | −60.0 | −71.5 |
| −55 | −55.0 | −55.0 | −71.0 |
| −50 | −50.0 | −50.0 | −70.3 |
| −45 | −45.0 | −45.0 | −70.0 |
| −40 | −40.0 | −40.0 | −68.9 |
| −20 | −20.0 | −20.0 | −59.0 |
| −10 | −10.0 | −10.0 | −50.9 |
| −6 | −6.0 | −6.0 | −47.0 |

Game Out playback, same tones:

| Nominal | `pc_in` (119) | 135 group (`game_mix`) | 7 (`stream_mix`) |
|---|---|---|---|
| −70 | floor (−89) | floor (−89) | −70.3 |
| −60 | floor (−89) | −63.0 | −70.1 |
| −55 | floor | −58.0 | -70.8 |
| −50 | floor | −53.0 | −71.2 |
| −45 | floor | −48.0 | −70.2 |
| −40 | floor | −43.0 | −69.0 |
| −20 | floor | −23.0 | −58.6 |
| −10 | floor | −14.0 | −50.0 |
| −6 | floor | −10.0 | −46.0 |

Silence: `pc_in` and the 135 group sit at −89 dB, and the stream at about −70 dB.

### Anomalies and limits

- **Voice −60 dBFS (run2)** read the −89 dB floor on `pc_in` and the 135
  group for the full 2 s step. The same level in run4 read −60.0 exactly. The
  cause is not established. Only run4 is used for the table.
- **Game path threshold**: `game_mix` shows nothing at −70 dBFS and
  tracks from −60 dBFS up.
- **Game offset**: the game path reads 3 dB below nominal (4 dB at −10 and
  −6). The device applies this path gain. The decoder does not correct it.
- **Onset**: meters respond about 0.7 s after `aplay` starts. Measure from
  the onset, not from the start of the step.
- **Mic/HDMI**: not measured (see above).

## Captures used

- Existing captures (`captures/`, not committed): `00-mic-baseline`,
  `10-gate-on`, `12-gate-high`, `20-mic-blocks`, `21-mic-values`,
  `22-mic-eq-*`, `23-mic-*`, `24-state-read*`. They fix the layout, the
  quantisation and the duplicate series. The earlier analysis
  (interpreting them as unresolved) is superseded by the sweeps below.
- Sweeps (not committed; kept in `/tmp/opencode`): `run2` (both playback
  paths, −60 to −6 dBFS), `run4` (voice and game, −70 to −45 dBFS), `run3`
  (laptop-speaker tone, mic test).

## Validation rules

A frame is a meter frame when all of these hold:

- Length is 192 bytes.
- Bytes 0-7 are `04 f0 43 10 04 3e 14 02`.
- CIN bytes at 0,4,...,184 are `04`, and at 188 is `07`.
- Byte 191 is `F7`.

`decode_frame` raises `ValueError` for a frame that fails these rules.
