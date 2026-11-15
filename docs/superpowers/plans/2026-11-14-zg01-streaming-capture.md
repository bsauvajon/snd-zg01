# Plan: ZG01 second capture pair ("Streaming In")

Spec: `docs/superpowers/specs/2026-11-14-zg01-streaming-capture-design.md`.

## Phase 1 — stream plumbing

- `src/zg01.h`: add `ZG01_PCM_STREAMING_IN` (3), `ZG01_STREAMING_IN` stream id
  before `ZG01_N_STREAMS`, and a `capture_offset` byte offset on
  `struct zg01_stream`.
- `src/zg01_usb.c`: initialise the new stream (pcm device 3, capture
  direction, offset 8); Voice In keeps offset 0.

DoD: builds; every array sized from `ZG01_N_STREAMS` still compiles.

## Phase 2 — IN data path

- `src/zg01_pcm.c`: `zg01_iso_in()` copies each frame into every running
  capture consumer (offset 0 and offset 8 within the 16-byte frame), with
  one period work per consumer.
- `chain_consumers_running()` treats the in-chain as busy while either
  capture PCM runs.

DoD: a capture on either PCM alone, or both together, keeps the chain up;
no interface reset while active.

## Phase 3 — PCM device

- `zg01_new_pcm(dev, ZG01_STREAMING_IN, "ZG01 Streaming In", 0, 1, ...)`.
- `stream_name()` reports "Streaming In".
- Managed buffer sizing loop already covers every stream.

DoD: `hw:N,3` exists, stereo, 48 kHz, `S32_LE`, capture only.

## Phase 4 — UCM and docs

- `ucm/conf.d/zg01_usb/zg01_usb.conf`: `SectionDevice."StreamingIn"` with
  `CapturePCM "hw:${CardId},3"` and a lower priority than Voice In.
- `README.md`: PCM table, capture section, meters/Streaming note, known
  limitations (device may leave the pair silent).
- `AGENTS.md`: PCM table and capture chain description.
- `docs/INITIALIZATION_ANALYSIS.md`: four-channel frame layout.

DoD: docs describe the four-channel stream; UCM exposes both sources.

## Phase 5 — GUI meter

- `zg01-core`: `LinuxMeterId::StreamingIn` (slot 3), fourth PipeWire node
  (`StreamingIn__source`), four-slot snapshot arrays, stream-output model
  points at it.
- `zg01-gui`: `set_linux_meter` also updates the stream-output block.
- README: the STREAM OUT `PW` bar is no longer card-only.

DoD: `cargo fmt --check`, `cargo clippy -- -D warnings` and
`cargo test --workspace` green; the app spawns four recorders.

## Phase 6 — verification

- `make KDIR=...` warning-free and `tests/test-kernel-build.sh`.
- Hardware: `arecord -D hw:N,2` and `-D hw:N,3`, both playback PCMs, check
  `/proc/asound/cardN/usb_stats` for feedback/capture counters.
- Remove the temporary probe harness (`src/zg01_probe.c`, Makefile entry,
  probe calls in `zg01_usb.c`/`zg01.h`) before the final commit.

DoD: build gate green, hardware capture verified, probe harness gone.
