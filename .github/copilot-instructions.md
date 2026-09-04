# AI coding agent instructions

## Project overview

Linux kernel driver for the Yamaha ZG01 USB audio interface (VID 0x0499,
PID 0x1513). The device is vendor specific and not USB Audio Class, so all
knowledge of it comes from USB captures on Windows. One module `snd-zg01`,
one ALSA card, three PCM devices (Game Out, Voice Out, Voice In).
Distributed through DKMS packages for Debian and Arch.

## Architecture

### Single module, single card

`obj-m := snd-zg01.o` links four objects:

- `src/zg01_usb.c`: probe/disconnect, card creation, suspend/resume
- `src/zg01_pcm.c`: PCM ops, URB chains, the shared-endpoint data path
- `src/zg01_control.c`: vendor init request (bRequest 7, checks 0x80bb00)
- `src/zg01_usb_discovery.c`: descriptor dump, debug only, best effort

Build from the repository root only; `make -C src` fails. `snd_card_new`
allocates `struct zg01_dev` as card private data. Never free it manually;
`card->private_free` handles it. One probe on interface 1 creates the card
and all three PCM devices.

### Shared endpoint 0x01

Game Out and Voice Out both target EP 0x01 OUT. One URB chain (`out_chain`)
serves both PCM devices; Voice In has its own `in_chain` on EP 0x81.
Chain state lives in atomics (`kill`, `inflight`) guarded by
`dev->state_mutex`. The device hardware mixes Game and Voice samples, so the
two playback PCMs stay separate sinks.

- Voice Out alone: chain runs in keepalive mode, sends silence.
- Both PCMs open: the URB callback reads both substream buffers and mixes
  them into each 240-byte packet.
- The out chain starts when either sink prepares and stops when both stop.

### Packet formats

Playback packet: 240 bytes = 6 frames of 40 bytes. Frame: Voice_L(4),
Voice_R(4), Game_L(4), Game_R(4), 24 pad bytes. 32 ISO packets per URB,
4 ms per URB, `MAX_URBS` 16 (64 ms buffering).

Capture packet: 108 bytes = 8-byte header, 6 frames of 16 bytes
(L 4, R 4, pad 8), 4-byte trailer.

### Rates

Playback locks to 48 kHz. Voice In accepts 48 kHz and 16 kHz. The device
clock is shared across interfaces; the driver always initializes it at
48 kHz, including when Voice In opens first at 16 kHz. The vendor handshake
in prepare resets both interfaces to alt 0, so it runs only when no chain is
streaming (`device_initialized` flag plus `inflight` checks under
`state_mutex`).

### Streaming lifecycle

URBs resubmit from the callback until an explicit stop gate
(`disconnecting`, `kill`, `-ESHUTDOWN`, `-ENOENT`, `-ECONNRESET`). Silence
goes to the wire when no PCM is running. Cleanup is deferred to
`zg01_cleanup_wq`; period elapsed runs on `zg01_period_wq` (nonatomic PCM).
Suspend stops both chains and forces re-init on the next prepare;
`reset_resume` covers firmware loss across suspend.

## Pitfalls

- `runtime->buffer_size` is in frames, not bytes. Do not divide by
  `bytes_per_frame`.
- Keep the interface setup out of open paths. The Magic Sequence kills live
  URBs; prepare checks `inflight == 0` before running it.
- USB control buffers must be heap allocated, never on stack.
- Voice In `-ECONNRESET` bursts on first open are benign; do not treat them
  as a stop condition.
- The Makefile auto-detects clang kernels via `CONFIG_CC_IS_CLANG`. Do not
  hardcode `LLVM=1`.
- Do not add `EXPORT_SYMBOL` for intra-module symbols.

## Build and test

```bash
make                                  # from repo root
tests/test-kernel-build.sh            # strict W=1 build check
tests/test-zg01-lifecycle.sh          # open/close stress via pw-cat
tests/test-arch-packaging.sh          # PKGBUILD and .SRCINFO consistency
```

Test on real hardware when possible. PipeWire probing is the most sensitive
area: watch for trigger loops with `journalctl -b -k --grep zg01`.

## References

- `docs/PROTOCOL_CAPTURE.md`: capture workflow for new control messages
- `docs/INITIALIZATION_ANALYSIS.md`: device USB topology and packet formats
- `src/zg01.h`: shared structs, chain state, packet constants
