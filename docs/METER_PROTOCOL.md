# ZG01 meter frames (bulk IN 0x83)

Status: **partial**. The frame framing, the header, the 14-bit field
layout and the value quantisation are verified against the captures. The
mapping from the 12 varying series to the six vendor meters, and the
scale, are **not decoded**. `decode_frame` is not implemented yet.

## Transport

- Device-to-host bulk IN on EP `0x83`, interface 4.
- Each frame is 192 bytes: 48 USB-MIDI event packets of 4 bytes.
- Packet 0 byte is the USB-MIDI code index (CIN). Packets 0-46 use CIN
  `0x04` (SysEx start/continue, three data bytes); packet 47 uses CIN
  `0x07` (SysEx end, three data bytes). Verified on all 1414 frames of
  `00-mic-baseline` and `24-state-read`.
- Stripping the CIN bytes gives a 144-byte SysEx message (the payload
  below). All payload bytes except the `F0`/`F7` delimiters are 7-bit
  (`< 0x80`).
- Frames arrive at about 20 Hz (roughly every 50 ms).

Raw frame offset `r` maps to payload index `p` as
`p = 3*(r//4) + (r%4) - 1` for `r % 4 != 0`.
Raw offset `r` maps back to payload index `p` as
`r = 4*(p//3) + 1 + p%3`. The `columns` command reports raw offsets.

## Payload layout

| Payload index | Content |
|---|---|
| 0-5 | `F0 43 10 3E 14 02`: SysEx start, Yamaha ID `43`, then the constant `10 3E 14 02` (meter message). |
| 6 | `03`, constant in all 19,339 captured frames. |
| 7-142 | 68 big-endian 14-bit fields, each as a 7-bit pair `hi lo` (`value = hi*128 + lo`). |
| 143 | `F7`: SysEx end. |

Field `hi` is first. The carry shows this: `79 7e` followed by `7a 08` is
a step of +10 (`0x3CFE` to `0x3D08`), which only works with `hi` first.

Of the 68 pairs, 52 are constant in every captured frame (value `15114`,
i.e. `76 0a`). The other 16 vary. They form 12 distinct series. Four of
the 16 are near-copies of other pairs (see below).

## Varying fields

Raw offsets are the `columns` offsets (index into the 192-byte frame).
Values are the 14-bit integers observed over all 19,339 frames of the 16
captures.

| Payload pair | Raw offsets (hi, lo) | Observed value | `(value-4)/10` |
|---|---|---|---|
| 7,8 | 10, 11 | 15584..16214 | 1558..1621 |
| 9,10 | 13, 14 | 15594..16214 | 1559..1621 |
| 11,12 | 15, 17 | 15114..16124 | 1511..1612 |
| 13,14 | 18, 19 | 15114..16124 | 1511..1612 |
| 111,112 | 149, 150 | 15114..16124 | 1511..1612 |
| 113,114 | 151, 153 | 15114..16124 | 1511..1612 |
| 115,116 | 154, 155 | 15114..16124 | 1511..1612 |
| 117,118 | 157, 158 | 15114..16124 | 1511..1612 |
| 119,120 | 159, 161 | 15464..15524 | 1546..1552 |
| 121,122 | 162, 163 | 15474..15524 | 1547..1552 |
| 123,124 | 165, 166 | 15474..15534 | 1547..1553 |
| 125,126 | 167, 169 | 15474..15534 | 1547..1553 |
| 135,136 | 181, 182 | 15464..16164 | 1546..1616 |
| 137,138 | 183, 185 | 15464..16164 | 1546..1616 |
| 139,140 | 186, 187 | 15474..16164 | 1547..1616 |
| 141,142 | 189, 190 | 15474..16164 | 1547..1616 |

Observations:

- Every observed value satisfies `value % 10 == 4`
  (130,768 of 130,768 field reads). The field is therefore `10*k + 4`,
  and `k` is the quantity that varies. Whether `k` is linear, a dB value
  or a curve is undetermined.
- Pairs 111/113/115/117 are near-copies of 11/13 (98-100% of frames equal
  per capture). Pairs 137/141 are similar to each other but not identical.
  Pairs 119-125 span only 5-6 steps and look like a noise floor.
- The 12 distinct series are the 16 pairs minus the copies (111, 113, 115,
  117). Four pairs (119-125) stay within 5-6 steps, and the other eight
  (7, 9, 11, 13, 135-141) swing across tens to hundreds of steps.

## Unresolved

1. **Meter identity.** Which of the 12 series is `mic_in`, `mic_out`,
   `pc_in`, `hdmi_in`, `game_mix`, `stream_mix`. The only correlation
   available is qualitative: in `10-gate-on`, the GATE enable writes
   (off at +4.6 s, on at +8.0 s, relative to the first `0x03` write) are
   followed by step changes in pairs 9 and 13 at about +4.5 s and +8.9 s.
   This is not conclusive and is a hypothesis, not a decode.
2. **Scale.** The value-to-level relationship is unknown. It cannot be
   measured from the existing captures, which contain no known signal.
3. **Channel layout.** Whether the 12 series are stereo pairs (L/R),
   peak/RMS pairs, or pre/post-fader pairs.

No key is decoded. Leaving `mic_in` and the others out of `decode_frame`
is the honest state until a known-signal capture exists.

## Why no known-signal capture yet

A fresh usbmon capture (kept in `/tmp/opencode`, not committed) played a
1 kHz sine at −60, −40, −20, −10 and −6 dBFS on
`hw:zg01,0` (Game Out) and `hw:zg01,1` (Voice Out), 2 s per level. It
captured 20,740 packets and **zero** `0x83` meter frames.

The running `snd-zg01` driver claims interface 4 but never submits IN
URBs on `0x83`. The only reader, `zg01_param_read`, has no callers. The
meter frames in the existing captures came from a host that did read
`0x83` (the libvirt/Windows ZG Controller session). A userspace reader
needs the interface claimed, which means unbinding `zg01_usb`, and that
disturbs the card's ALSA devices.

Options for the next step:

1. The driver gains a `0x83` IN reader (the Task 3 meter path), and
   known-signal captures are taken with it loaded.
2. A brief, controlled unbind is accepted for a userspace-read capture.

## Validation rules

A frame is a meter frame when all of these hold:

- Length is 192 bytes.
- Bytes 0-7 are `04 f0 43 10 04 3e 14 02`.
- CIN bytes at 0,4,...,184 are `04`, and at 188 is `07`.
- Byte 191 is `F7`.

The `value % 10 == 4` quantisation held in every captured field. It is
an observed invariant, not a rule to reject frames on until the scale is
known.
