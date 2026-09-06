# USB packet diagnostics

## Bounded IN header trace

The next diagnostic build also exposes `/proc/asound/zg01/in_trace`.
It retains at most 256 records: startup packets, non-108-byte packets, errors,
and the immediate neighbors of length/status transitions. Each record has a
packet sequence number, callback completion timestamp, actual length, status,
and the first eight bytes (the presumed header). It does not copy sample data
under the currently documented framing. Failed or out-of-bounds packets have
no header. This is a diagnostic assumption, not a validated new protocol decoder.

The timestamps describe completion callbacks, not individual USB transfer times.
Packets in one URB share a timestamp. Sequence-number gaps denote unretained
ordinary packets. The buffer is a ring: once full, each new record replaces the
oldest, so the trace always holds the most recent 256 selected records, and
`omitted` counts how many older records were displaced. Reload the module for a
fresh trace.
The trace does not activate IN by itself or implement feedback.

After reloading, reapply Voice Out preallocation with playback closed:

```bash
printf '48\n' | sudo tee /proc/asound/zg01/pcm1p/sub0/prealloc
```

Run capture for 60 seconds in one terminal, then start the tone in another:

```bash
arecord -D hw:zg01,2 -t raw -f S32_LE -c 2 -r 48000 -B 128000 -F 2000 -d 60 /dev/null
```

```bash
aplay -v -D hw:zg01,1 --buffer-size=6144 --period-size=384 /tmp/zg01-997hz.wav
```

After capture completes, save both diagnostics:

```bash
cat /proc/asound/zg01/in_trace > /tmp/zg01-in-trace.txt
cat /proc/asound/zg01/usb_stats > /tmp/zg01-usb-stats.txt
```

The first trace line reads `zg01_in_trace_v2`; v1 froze at capacity instead
of cycling.

This diagnostic build exposes `/proc/asound/zg01/usb_stats` with cumulative,
read-only counters. Counters reset when the ALSA card is recreated, such as
module reload or unplug/replug. Reading does not reset them.

The callbacks count packet results before PCM state and capture-length filters.
They do not print each packet or allocate memory. The proc reader copies both
endpoint snapshots under the device spinlock and formats them after unlocking.

The diagnostic counters describe the current playback and feedback paths. A
consumer underrun is attributed to that playback stream. A shared transport or
feedback-source fault is attributed to the affected chain and schedules XRUN
work for its dependent streams. These events do not prove hardware behavior.

## Build

From the repository root:

```bash
make
```

## Load

Close audio clients. If PipeWire holds the device, stop its services and sockets:

```bash
systemctl --user stop wireplumber.service pipewire-pulse.service pipewire.service pipewire-pulse.socket pipewire.socket
sudo rmmod snd-zg01
sudo insmod /home/softashell/Projects/zg01/snd-zg01-forgejo/snd-zg01.ko
cat /proc/asound/zg01/usb_stats
```

The first line must read `zg01_usb_stats_v1`. If it does not exist, confirm the
module reload succeeded. This requires a kernel with `CONFIG_SND_PROC_FS=y`.

## Reproduce

For playback-only reproduction, use the same direct ALSA test as before:

```bash
speaker-test -D hw:zg01,1 -c 2 -r 48000 -F S32_LE -t pink
```

In another terminal, save a baseline, then watch counters:

```bash
cat /proc/asound/zg01/usb_stats > /tmp/zg01-stats-before.txt
watch -n 1 cat /proc/asound/zg01/usb_stats
```

After a pop, save a second snapshot:

```bash
cat /proc/asound/zg01/usb_stats > /tmp/zg01-stats-after.txt
```

The IN histogram requires the IN endpoint to run. Repeat the test with this
command in another terminal. It activates microphone capture but discards the
samples; it does NOT record a playback loopback:

```bash
arecord -D hw:zg01,2 -t raw -f S32_LE -c 2 -r 48000 -B 128000 -F 2000 /dev/null
```

Compare playback-only and playback-plus-capture separately. Opening capture
changes device activity, so note whether it changes the pop frequency. If the
capture device is busy, another client already owns it. Do not substitute a
different recording device.

Stop test commands with Ctrl+C. Restore desktop audio after testing:

```bash
systemctl --user start pipewire.socket pipewire-pulse.socket pipewire.service pipewire-pulse.service wireplumber.service
```

## Fields

- `snapshot_ns`: monotonic timestamp at the snapshot, in nanoseconds.
- `completions`: every completion, including cancellation and shutdown.
- `cancelled`: aggregate `-ENOENT`, `-ECONNRESET`, or `-ESHUTDOWN` completions.
  Their packet descriptors do not enter transport counters. A shutdown is not
  automatically benign; correlate it with stop, unload, or device removal.
- `urb_errors`: other nonzero aggregate statuses.
- `packets`: packet descriptors in non-cancelled/non-shutdown completions.
- `packet_status STATUS COUNT`: exact packet errno counts, including zero for
  success. These include failed packets inside aggregate-success URBs.
- `unknown_status`: positive or out-of-range packet statuses outside -127..0.
- `in_length BYTES COUNT`: successful IN packets at each actual length, including
  zero. Failures do not enter this histogram.
- `in_length_overflow`: successful IN lengths above the 124-byte allocation.
- `out_length_mismatch`: successful OUT packets whose actual length differs from
  the requested length. This reflects controller reporting, not a device ACK.
- `last_error_ns`: most recent aggregate/packet error or OUT length mismatch.
  It excludes cancellation/shutdown and ordinary IN length variation.

Only nonzero status/length buckets appear. Use snapshot differences for a test
window; lifetime totals may include previous tests.

Non-108-byte IN packets would prove the fixed-length filter discards traffic.
They would not, alone, prove the audio framing or feedback protocol. Packet
captures remain necessary before implementing implicit feedback.
