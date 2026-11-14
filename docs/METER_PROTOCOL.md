# ZG01 meter frames (bulk IN 0x83)

Status: **decoded v2; Linux live-meter acceptance is blocked**. The frame
carries 34 channels of level data in six groups. The map comes from the
vendor `MemAPI_ZG01.dll` meter table and is confirmed by USB captures on
Windows with Voice, Game Out, HDMI and microphone stimuli. The current
Linux test unit still returns zero-length packets after a suspend; see
"Current session status".

## Transport

- Device-to-host bulk IN on EP `0x83`, interface 4.
- Each frame is 192 bytes: 48 USB-MIDI event packets of 4 bytes.
- Packet 0 byte is the CIN. Packets 0-46 use `0x04`; packet 47 uses `0x07`.
  Stripping the CIN bytes gives a 144-byte SysEx message.
- Payload byte 0-5 are `F0 43 10 3E 14 02`; byte 6 is `03`; bytes 7-142 are
  68 big-endian 14-bit fields (`code = hi * 128 + lo`); byte 143 is `F7`.
- Frames arrived at about 20 Hz (every 50 ms) in the captures. They start
  once the ZG Controller/ZG01 client has initialized the meter session; a
  keepalive alone is not enough on the current unit.

The raw offset `r` of a payload index `p` (for `p % 3 != 0`) is
`r = 4 * (p // 3) + 1 + p % 3`.

## Channel map

The 68 fields are **34 channels × (current, peak)**. Channel `c` current is
field `2*c`; its peak partner is field `2*c + 1`. A group starts at a
**field** base:

| Group | Base field | Channels | Control |
|---|---|---|---|
| `mic_in` | 0 | 1 | `Meter Mic In` |
| `mic_out` | 2 | 1 | `Meter Mic Out` |
| `pc_in` | 4 | 8 | `Meter PC In` |
| `hdmi_in` | 20 | 8 | `Meter HDMI In` |
| `game_mix` | 36 | 8 | `Meter Game Mix` |
| `stream_mix` | 52 | 8 | `Meter Stream Mix` |

Channel `c` of group `g` is field `base(g) + 2*c`; the current value is at
payload indices `7 + 2*field` (high) and `8 + 2*field` (low). The peak
partner is the next field and is not exposed by the driver.

The vendor DLL registers the groups with a constant
`(field_count << 16) | base`: `0x00020000`, `0x00020002`, `0x00100004`,
`0x00100014`, `0x00100024`, `0x00100034`. The vendor QML uses
`YQMeterDataAccessor` with `channelNo` 0, 1, 2, 4, 5, 6, 7 for its
Sound Check meter.

## Level scale

Each channel is a dB value:

```
level_dB = (code - 16384) / 10
control  = clamp(round(level_dB) + 127, 0, 127)
```

Codes are on a 10-step grid; an off-grid code rounds to the nearest dB.
The two-byte field tops out at code 16383 (−0.1 dB), so the control tops
out at 127. Codes below −127 dB clamp to 0. The device applies its own path
gains; the decoder does not correct them.

## Evidence

The captures on the current Windows session show:

| Stimulus | Observed groups/channels |
|---|---|
| Game Out (USB) | `pc_in` 0/1, `game_mix` 0/1, `stream_mix` 4/5 |
| HDMI input | `hdmi_in` 0/1, `game_mix` 0/1, `stream_mix` 4/5 |
| Voice playback (ZG device "Voice") | `stream_mix` 0/1; `mic_out` follows |
| Microphone | `mic_in`, `mic_out` |

The earlier September fixtures (`gate-closed-quiet`, `mic-tap`, ...) show
`mic_in`, `mic_out` and `stream_mix`. With the corrected indexing, their
old `pc_in` field is `stream_mix` channel 2 and their old `stream_mix` field
is `stream_mix` channel 6; they never contained a PC In group. The
`all-groups` fixture pins every channel of every group.

The stereo captures activate only channels 0/1 (and `stream_mix` 4/5). The
surround channels 2-7 are not measured. The GUI maps the VOICE strip to
`stream_mix` 0/1: a Game Out stimulus also raises `pc_in` and `game_mix`,
so `pc_in` 0/1 must not feed that strip.

## Driver controls

The kernel parser reduces the 68 fields to 34 channel slots and registers
six read-only, volatile integer controls:

| Control | First slot | Count |
|---|---|---|
| `Meter Mic In` | 0 | 1 |
| `Meter Mic Out` | 1 | 1 |
| `Meter PC In` | 2 | 8 |
| `Meter HDMI In` | 10 | 8 |
| `Meter Game Mix` | 18 | 8 |
| `Meter Stream Mix` | 26 | 8 |

A read returns the latest valid frame or `-ENODATA` when no valid frame has
arrived, the reader is stopped, or the last frame is older than 500 ms. A
missing meter is never zero. The controls are `READ | VOLATILE`; `alsactl`
does not restore them.

The GUI detects a legacy driver (four one-value controls, no `Meter Game
Mix`/`Meter HDMI In`) and keeps the old four-meter presentation. It does
not remap the legacy values into the new groups: those values are known to
be `stream_mix` channels 2 and 6.

## Validation rules

`decode_frame` accepts a frame only when all of these hold:

- Length is 192 bytes.
- Bytes 0-7 are `04 f0 43 10 04 3e 14 02`.
- The CIN byte of every packet from 0 to 184 is `04`; byte 188 is `07`.
- Byte 191 is `F7`.

The rules check framing only. A rejected frame leaves the previous driver
snapshot unchanged. The reference decoder is `tools/decode-meters.py`; the
C parser is `src/zg01_meters.h`; the fixtures are
`tests/fixtures/meters/frames.jsonl`.

## Current session status

The Windows ZG Controller streams 192-byte frames as soon as it starts:
the 2026-11-11 captures show the app's startup (dirty-flag reads `F0 43 30`
and preset blocks `F0 43 00`) followed by a keepalive and then meter frames.

The Linux driver on this unit still gets zero-length packets after the
2026-11-01 suspend. The driver's meter reader, keepalive and state request
work, but the device does not enter the streaming meter state. Replaying
the Windows startup sequence from Linux is a separate phase; until it
lands, the Linux card controls answer `-ENODATA` and the GUI draws the card
banks dashed. The `PW` bars from PipeWire remain the live Linux levels.
