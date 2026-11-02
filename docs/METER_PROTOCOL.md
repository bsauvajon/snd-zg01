# ZG01 meter frames (bulk IN 0x83)

Status: **partial; live-meter acceptance is blocked**. The framing, the
header, the 14-bit layout, the dB scale and the current/peak pairing are
measured from earlier captures. Four meters are decoded with stated
confidence: `pc_in` (high), `mic_out` and `mic_in` (medium), and
`stream_mix` (medium). `game_mix` and `hdmi_in` are not decoded. GAME is
deferred to v1.1 (see "Scope and reserved names").

The decode below is still the documented contract, but the latest
hardware session does **not** produce live frames: see "Current session
status". Do not read the earlier playback tables as a claim that the
current device streams.

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
- Frames arrive once the driver's `0x83` meter reader is running and its
  1 Hz `0x03` keepalive is sent. The captures arrived at about 20 Hz
  (roughly every 50 ms).
- Start-up, measured on a cold device that still streamed (earlier
  captures; `docs/METER_TEST.md`): the first `0x83` IN URB may complete at
  once with one 192-byte frame. Then the device returns zero-length
  packets about every 100 ms until the first keepalive. The first 192-byte
  frame after that keepalive arrived 49 ms later, and the zero-length
  packets stopped. An earlier resume after a system suspend showed the
  same sequence. The current device is stuck in the zero-length phase and
  does not reach the 192-byte frames (see "Current session status").

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

`hi` is first. The carry shows this: `79 7e` followed by `7a 08` is a step
of +10 (`0x3CFE` to `0x3D08`), which only works with `hi` first.

52 of the 68 fields are constant in every frame (code `15114`, i.e. `76 0a`).
The other 16 vary, and they form 8 current/peak pairs.

## Level scale (measured)

Each meter field is a dB value:

```
code = hi*128 + lo
level_dB = (code - 16384) / 10        # 16384 = 0 dBFS, 10 codes per dB
```

- Codes are always `== 4 (mod 10)`. Since `16384 == 4 (mod 10)`, the
  quantisation is exact at 1 dB; this is the `10*k + 4` observation.
- `code 15114` is -127 dB, the floor that many captures sit at.
- `decode_frame` returns `level_dB + 127` as an integer, in `0..127`
  (0 = -127 dB, 127 = 0 dBFS). This is linear in dB, the device's own unit,
  on a 0-127 range.
- Rounding: codes on the 10-step grid decode exactly. An off-grid code,
  never observed, rounds to the nearest dB (`round()`, so ties go to the
  even dB).
- Clamping: two bounds apply. The two-byte field tops out at code 16383,
  which is −0.1 dB, so the device cannot encode a level above 0 dBFS and
  the output tops out at 127. Codes below −127 dB clamp to 0. The top
  clamp is a guard, not a reachable case.
- The device applies its own path gains. The decoder does not correct them.

## Current/peak pairs

Each meter is a **current** field followed by a **peak-hold** partner.
The current field rises with the level and decays; the partner holds the
peak for about 1 s, then releases. Across 3,000+ frames of every capture the
peak was never below its current field.

| Pair | Current (payload, raw) | Peak (payload, raw) | Role (see below) |
|---|---|---|---|
| A | 7, (10, 11) | 9, (13, 14) | `mic_in` |
| C | 11, (15, 17) | 13, (18, 19) | `mic_out` |
| E | 111, (149, 150) | 113, (151, 153) | copy of C |
| G | 115, (154, 155) | 117, (157, 158) | copy of C |
| I | 119, (159, 161) | 121, (162, 163) | `pc_in` |
| K | 123, (165, 166) | 125, (167, 169) | second voice-out pair (equal to I in playback sweeps) |
| M | 135, (181, 182) | 137, (183, 185) | `stream_mix` |
| O | 139, (186, 187) | 141, (189, 190) | copy of M |

- E and G equal C in 98% of frames, and O equals M in 98%. F and H equal
  D in 99.6% of frames, and P equals N in 99.3%.
- The fixture frame `mic-decay-peak-held` covers a clear current/peak
  difference (A −25 against its held peak −14). `decode_frame` must return
  the current value there.
- The payload index is for the pair's high byte. The raw offsets are
  the `columns` output.

## Decoded meters

| Key | Current field (raw hi, lo) | Confidence | Evidence |
|---|---|---|---|
| `pc_in` | (159, 161) | High | Exact dB for Voice Out playback (−70..−6 dBFS). Ignores Game Out and the mic. |
| `mic_out` | (15, 17) | Medium | Follows the gate. When the gate is bypassed it equals `mic_in` within 0-2 dB; when the gate closes it falls to −127 dB. |
| `mic_in` | (10, 11) | Medium | Does not follow the gate, so it is pre-gate. Responds to the mic tap and speech. |
| `stream_mix` | (181, 182) | Medium | Responds to the mic (after the gate), Voice Out (0 dB) and Game Out (−3 dB). Its floor when the gate is closed matches the voice floor. |

Not decoded:

- **`game_mix`**: no pair responds to Game Out without also responding to
  the mic. Series A, C and M all respond to the mic. So no pair is the
  game-only tap, and the label is withdrawn.
- **`hdmi_in`**: no HDMI source was available, and no pair is attributable
  to HDMI.

### Identity changes from the previous revision

The previous revision (commit `9b629ba`) labelled the 135 group as
`game_mix` and the 7 group as `stream_mix`. Mic-test and gate evidence
show otherwise:

- The 135 group (now M) responds to the mic, so it is not `game_mix`. It is
  `stream_mix`, the post-gate sum.
- The 7 group (now A) is pre-gate and so is `mic_in`.
- `mic_out` (C) was not decoded before.

## Scope and reserved names

- **GAME is deferred to v1.1.** The decoded frame has no separable
  game-only tap (see `game_mix` above). The GUI's GAME strip uses PipeWire
  `Game Out` in v1.1, not a card meter.
- **`Meter Game Mix` and `Meter HDMI In` are reserved names.** They are not
  registered in v1. A kcontrol is added only when its field is decoded.
- Registered in v1 (the four decoded meters): `Meter Mic In`,
  `Meter Mic Out`, `Meter PC In`, `Meter Stream Mix`.

## Current session status

The latest meter ledger (`.superpowers/sdd/2026-10-31-zg01-meters/`,
2026-11-01) records a **blocked** live-meter session:

- After the suspend that produced `-71` (`EPROTO`) enumeration errors, the
  ZG01 never leaves the "waiting for keepalive" state on this unit.
- EP `0x83` returns zero-length packets about every 100 ms and no 192-byte
  frame. `frames` stays 0 while `other` climbs.
- The driver sends the same `0x03` keepalive and the same 250 ms EP0
  status poll (`bRequest 7`, then `bRequest 4`) as the Windows application,
  and the device answers both. A power cycle, `usbreset`, a hub port cycle
  and a replay of the Windows preamble did not restore frames.
- `amixer cget` on the four meter controls returns `-ENODATA`.

This is a device/session blocker, not a decode change. The GUI must draw
the affected meters as unknown (dashed), never as zero. See
`docs/METER_TEST.md` for the acceptance status and `docs/GUI_TEST.md` for
the GUI-side state.

## Evidence

### Mic test (`mic-test`, user present)

- Tap at +2.36 s: A -9, C -6, M -1 (peak). I, K unchanged at −89.
- Speech at +4.8 to +6.2 s: A and C track the same envelope (about 0-4 dB
  apart). M sits 4-5 dB above C.
- Quiet after speech: A plateaus at about −50 to −58 dB (ambient). C falls
  to −110 to −127 and M to −88 to −91. Both drop together, and A does not.

### Gate writes (`10-gate-on`)

Gate OFF write at +4.61 s, ON write at +7.99 s (relative to the first
`0x03` write):

- Before +4.61 s, C sits at −127 while A carries ambient. The gate is
  closed.
- +4.61 to +7.99 s (gate bypassed): C tracks A to within 0-2 dB, mean
  A −77.4 and C −76.2. M tracks −72.
- After +7.99 s (gate closed): C falls to −126 (mean), while A stays at
  about −74 and M at −89.
- A transient at +4.52 s, while the gate was still closed, reached A
  (−58) and not C or M. C and M rose only after the gate opened at +4.66 s.

### Gate threshold (`12-gate-high`)

With a high threshold, C stays at the floor (17 of 203 frames above −120)
even when A carries transients up to −48 dB. Brief partial openings put C
about 55 dB under A.

### Playback sweeps (`run2`, `run4`, nominal dBFS)

Voice Out playback, decoded dB (`-60` from `run4` only, see Anomalies):

| Nominal | `pc_in` (I) | `stream_mix` (M) | `mic_in` (A) | `mic_out` (C) |
|---|---|---|---|---|
| −70 | −70 | −70 | −71 | floor (−127) |
| −60 | −60 | −60 | −71 | floor |
| −55 | −55 | −55 | −71 | floor |
| −50 | −50 | −50 | −70 | floor |
| −45 | −45 | −45 | −70 | floor |
| −40 | −40 | −40 | −69 | floor |
| −20 | −20 | −20 | −59 | −113 |
| −10 | −10 | −10 | −51 | −105 |
| −6 | −6 | −6 | −47 | −101 |

Game Out playback, decoded dB:

| Nominal | `pc_in` (I) | `stream_mix` (M) | `mic_in` (A) | `mic_out` (C) |
|---|---|---|---|---|
| −70 | floor (−89) | floor (−89) | −70 | floor |
| −60 | floor | −63 | −70 | floor |
| −55 | floor | −58 | −71 | floor |
| −50 | floor | −53 | −71 | floor |
| −45 | floor | −48 | −70 | floor |
| −40 | floor | −43 | −69 | floor |
| −20 | floor | −23 | −59 | −113 |
| −10 | floor | −14 | −50 | −104 |
| −6 | floor | −10 | −46 | −101 |

Game Out is absent from `pc_in`, and Voice Out is 0 dB on `stream_mix`
(−3 dB on the game path). `mic_in` and `mic_out` respond to playback in the
sweeps. That response is not explained by any mic path (see Anomalies).

### Anomalies and limits

- **`mic_in` and `mic_out` respond to playback** in the sweeps (A at
  about −40 dB, C at about −54 dB for Voice Out). A pre-gate mic input
  should not carry playback, so the cause is not established by the
  captures. The approved working hypothesis is **likely acoustic
  coupling**: the playback leaves the speakers and reaches the mic, which
  would show up on the mic fields and not on the playback paths. This is
  unconfirmed. A test with the speakers silent or the mic isolated would
  settle it.
- **Voice −60 dBFS (`run2`)** read the floor on `pc_in`. The same level in
  `run4` read −60.0 exactly. The cause is not established.
- **Stream curve**: `stream_mix` is not linear in dB at low levels. It fits
  a power sum, `10*log10(10^((V+G)/10) + 10^(N/10))`, with G ≈ −40 dB and
  N ≈ −70.5 dB, within 0.7 dB RMS. The device produces this curve.
- **Onset**: meters respond about 0.7 s after `aplay` starts.
- **Mic level scale**: the mic tap and speech are not calibrated to dBFS,
  so only the shared code convention and the gate behaviour are verified
  for the mic fields.

## Captures used

- Existing captures (`captures/`, not committed): `00-mic-baseline`,
  `10-gate-on`, `12-gate-high`, `20-mic-blocks`, `21-mic-values`,
  `22-mic-eq-*`, `23-mic-*`, `24-state-read*`.
- Sweeps (not committed): `run2` (both playback paths, −60 to −6 dBFS),
  `run4` (voice and game, −70 to −45 dBFS), `run3` (laptop-speaker tone).
- Mic test (`mic-test`, not committed): user tap at about +2.4 s and speech
  at about +4.8 to +6.2 s, 558 frames over 30 s.

## Validation rules

`decode_frame` accepts a frame only when all of these hold. It raises
`ValueError` otherwise:

- Length is 192 bytes.
- Bytes 0-7 are `04 f0 43 10 04 3e 14 02`.
- The CIN byte of every packet from 0 to 184 (every multiple of 4) is `04`.
  Byte 188 (packet 47) is `07`.
- Byte 191 is `F7`.

The rules check the framing only. The payload bytes between them are not
checked for the 7-bit range. The `decode` command skips a rejected frame,
reports `decode-meters: skipped frame: ...` on stderr, and exits 1.
