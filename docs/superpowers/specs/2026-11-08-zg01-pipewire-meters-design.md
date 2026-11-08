# ZG01 PipeWire Meters — Design

Status: proposed, 2026-11-08.

## Intent

The GUI already draws the four card meters, but the ZG01 on the test
machine has not streamed 192-byte meter frames on EP `0x83` since the
2026-11-01 suspend. Every meter control answers `-ENODATA`, so the user
sees four dashed tracks and no moving VU meter.

The goal is to make the interface show live levels again **without
waiting for the card-meter unblock**, using the Linux-side PipeWire graph
the design already reserves for v1.1. The card meters stay wired and
become live again on their own once the device streams.

## Goals

- MIC, VOICE and GAME strips show a moving PipeWire level next to the
  card meter (or alone for GAME, which has no card meter).
- The Micro screen shows the `Voice In` level beside `MIC IN`.
- The Linux bar keeps the existing visual contract: thinner than the card
  bar, with the `PW` badge, so the source is never ambiguous.
- No new build dependency. `pipewire-rs` needs `libpipewire-0.3` headers
  that are not installed here, and the GUI package already treats
  PipeWire as an optional runtime dependency.
- No fabricated values: a source that is absent has no bar; a source that
  exists but has no reading yet is dashed; silence reads zero only when a
  captured buffer actually contains silence.
- Closing the GUI, losing the card or quitting never leaves a
  `pw-record` process behind.

## Non-goals

- Decoding or unblocking EP `0x83`. The card meters, `docs/METER_PROTOCOL.md`
  and the driver are untouched.
- The **true pre-transformation microphone input**. `Voice In` is the
  processed capture path; the driver's `Meter Mic In` is documented as
  pre-gate. That remains the target for a later EP `0x83` session, and is
  recorded here so the PipeWire bar is not mistaken for it.
- A Settings toggle for PipeWire meters, compact mode, or meters for
  other applications. The feature is on when the tools and nodes exist.
- Peak data from `pipewire-rs` or a custom SPA client.

## Approaches considered

1. **`pipewire-rs` streams (rejected).** Cleanest in-process design, but
   the crate needs `libpipewire-0.3` development headers. They are not
   installed, and adding a build dependency to the GUI package for an
   optional feature is a packaging cost the current UI does not repay.
2. **`pw-record` child process (chosen).** One process per source, raw
   `s16` stereo on stdout. Verified on the test machine: a −30 dBFS tone
   on `Game Out` reads exactly −30.0 dBFS through the monitor. It needs
   no headers or link-time dependency, and the process is easy to stop.
3. **`parec`/PulseAudio (rejected).** `pactl`/`parec` are not installed;
   the machine runs PipeWire directly. Depending on the Pulse shim would
   be a second optional dependency.

## Design

### Source model

`zg01-core` grows a `LinuxMeterId` next to `MeterId`:

| Variant | PipeWire node | GUI use |
|---|---|---|
| `MicIn` | `Voice In` source | MIC strip, Micro `MIC IN` |
| `VoiceOut` | `Voice Out` sink monitor | VOICE strip |
| `GameOut` | `Game Out` sink monitor | GAME strip |

`StripModel` and `StreamOutputModel` gain `linux_meter: Option<LinuxMeterId>`.
The Mixage mapping is MIC → `MicIn`, VOICE → `VoiceOut`, GAME → `GameOut`,
STREAM OUT → none. GAME therefore gets a meter widget whose card reading
is absent and whose Linux bar is its only bar.

### Discovery

At startup the feature runs `pw-dump`, parses the JSON array, and keeps
the `object.serial` of the first node whose `info.props["node.name"]`
contains `ZG01` and ends with `VoiceIn__source`, `VoiceOut__sink` or
`Game__sink`. The serial is passed to `pw-record --target`; matching the
serial avoids ambiguity between two cards.

If `pw-dump` is missing, returns invalid JSON, or a node is not found,
that source is unavailable: it has no bar, not a zero. A debug log says
which source was not found, once.

### Recording and peak

One `pw-record` child per source:

```text
pw-record --target <serial> -P stream.capture.sink=true \
  --rate 48000 --channels 2 --format s16 --latency 20ms --raw -
```

`stream.capture.sink=true` is set for the two sinks and omitted for the
`Voice In` source.

The reader thread consumes stdout, interprets interleaved `s16` samples
and computes the maximum absolute sample of every 50 ms window. It then
maps that peak to the same scale as the card meters:

```text
peak == 0          -> 0.0
db = 20*log10(peak/32768), clamped to -127..=0
level = (db + 127) / 127
```

Using the card's dB scale keeps the two bars visually comparable: a
−30 dB signal sits at the same height on both. A non-finite result is
not a reading.

Recorders are independent: one that exits (device gone, PipeWire restart)
marks its source `None`, logs once, and is retried with a 1 s backoff
capped at 5 s. A stop flag kills the children and joins the threads.

### Snapshots and wiring

`LinuxMetersSnapshot` carries the backend generation, an `available` flag
per source and its level. The core spawns one event thread that emits a
snapshot every 50 ms; the GUI drops stale generations the same way it
drops stale card snapshots.

`App` starts and stops the Linux poller next to the existing meter pump,
so a disconnect or a reconnect tears both down. `Msg::LinuxMeters` fans
out to:

- `MixageScreen::set_linux_meter(LinuxMeterId, Option<f64>)` for MIC,
  VOICE and GAME;
- `MicScreen::set_linux_meter(MeterId::MicIn, Option<f64>)` for `MIC IN`.

`Metered` in both screens keeps the optional card id and the optional
Linux id. The widget gets a `set_linux_source(present, level)` method that
updates only the Linux bar, preserves the card reading, adjusts the
content width when the source appears, and repaints. The existing
`set_levels` still handles card snapshots and must not clear the Linux
bar.

### Failure and edge cases

- PipeWire absent, `pw-dump` absent, node absent: no Linux bar; card
  meters unchanged.
- Recorder produces no frames: `available` is true after spawn, the level
  is `None`, so the Linux bar is dashed.
- Recorder exits: level `None`, retry with backoff; no stale moving bar.
- App quits or the ZG01 leaves: stop flag, kill children, join.
- GAME has no card meter: the strip builds as soon as it has either a
  card or a Linux meter, so it is no longer labelled `Unavailable`.
- The Linux bar never feeds `MeterStreamMix`; STREAM OUT stays card-only
  and dashed while EP `0x83` is blocked.

### Tests

- `parse_pw_dump` against a fixture with the three ZG01 nodes plus
  unrelated nodes: exact serials, missing nodes, malformed JSON.
- `PeakTracker`: silence, full scale, −30 dBFS, window boundaries,
  non-finite guard.
- Command construction: `stream.capture.sink=true` on sinks only.
- Snapshot mapping and generation tagging.
- Manual acceptance in `docs/GUI_TEST.md`: play to Voice Out and Game
  Out, speak into the mic, see the PW bars move, see STREAM stay dashed,
  and confirm no `pw-record` survives closing the window.

## Compatibility and packaging

- New Cargo feature `pipewire`, enabled by default. Disabling it compiles
  out the module and hides every Linux bar.
- Runtime dependency: `pw-dump` and `pw-record` from the `pipewire`
  package. The Arch and Debian GUI metadata already list PipeWire as
  optional/recommended; no new package or build dependency is added.
- No kernel, ALSA, preset or `.zgpreset` format change.
