# ZG01 second capture pair ("Streaming In")

Status: implemented (driver + UCM + docs). The second pair stays silent on
the test unit; that behaviour is device-side and documented in README's
limitations.

## Intent

The ZG01 declares a four-channel capture stream on interface 2. Windows
exposes two recording devices from it: **Voice** (the microphone pair,
channels 1/2) and **Streaming** (the STREAMING OUTPUT MIXER mix, channels
3/4). The Linux driver only reads the first pair; the Mixage screen and the
"GAME" meter make the missing device visible to users but it cannot be
selected as a capture source.

Goal: expose the second pair as its own stereo capture PCM, so PipeWire/OBS
can select "Yamaha ZG01 Streaming In" alongside the existing "Voice In".

## Evidence

- Interface 2 alt 1 carries one isochronous IN endpoint (`0x81`, max packet
  124 B) and declares four 32-bit channels (class-specific descriptors
  `06 24 02 01 04 20` and the AS_GENERAL trailer `... 04 00 00 00`).
- Every 108-byte packet is a 8-byte header, 6 frames of 16 bytes and a
  4-byte trailer; each frame is `[Voice L, Voice R, Streaming L, Streaming
  R]` as 32-bit little-endian samples.
- The Windows driver is a PortCls miniport exposing one "Wave" filter with
  two capture pins whose names come from a per-PID table; the entry for
  `0499:1513` lists exactly `Voice` and `Streaming`. Audacity records the
  microphone from "Voice" and silence from "Streaming".
- On the current test unit the second pair is **always zero**, with the
  vendor stack under Windows (20-minute USB trace, `strm_usb_onf = 1`) as
  well as with this driver, with every relevant routing parameter swept.
  The driver-side feature is therefore implemented against the device's
  declared format; if the pair stays silent, the cause is device-side, not
  in this code.

## Design

Two stereo capture PCMs instead of one four-channel PCM:

| PCM | Name | Channels | Source |
|---|---|---|---|
| 2 | `ZG01 Voice In` | 2 | frame bytes 0-7 (unchanged) |
| 3 | `ZG01 Streaming In` | 2 | frame bytes 8-15 (new) |

Both are capture-only, 48 kHz, `S32_LE`, stereo, with the same buffer and
period constraints as Voice In. They share the single EP `0x81` URB chain:
the chain runs while any capture PCM is running, and the URB callback copies
each consumer's pair from the packet into its own ring buffer.

Why not one four-channel PCM: it would change the existing `hw:N,2` device
contract, force every user (UCM, PipeWire, OBS) to remap channels, and make
a silent pair spoil a working microphone capture. Two stereo devices mirror
the Windows model and keep Voice In byte-for-byte compatible.

Why not a userspace remix in PipeWire: it cannot reproduce the device's
internal stream mix (which includes HDMI input and the DSP effects).

## Constraints

- Same implicit-feedback contract: the IN stream keeps pacing playback, and
  adding or removing the second capture consumer must not reset interfaces
  or drop URBs while the chain is active.
- `runtime->buffer_size` is in frames.
- The URB callback runs in atomic context under `dev->lock`; no allocation.
- One card, one `struct zg01_dev`, private data owned by `snd_card_new`.

## Out of scope

- Device-side investigation of why the pair can stay silent.

## GUI meter (implemented)

The Mixage STREAM OUT block now draws its `PW` bar from the new
`Streaming In` source; the MIC/VOICE/GAME strips keep `Voice In`,
`Voice Out` and `Game Out`. `LinuxMeterId` gained `StreamingIn`, the
PipeWire pump records a fourth source, and the snapshot arrays grew to
four slots.

## Definition of done

- `make` and `tests/test-kernel-build.sh` clean (`W=1`), no new warnings.
- The card exposes `hw:N,3` as a stereo 48 kHz `S32_LE` capture PCM named
  "ZG01 Streaming In"; `hw:N,2` is unchanged.
- With the UCM profile installed, PipeWire lists both sources.
- Voice In capture, both playback PCMs and implicit-feedback pacing show no
  regression on hardware.
- README/AGENTS/UCM/device notes describe the four-channel capture layout.
