# Experimental implicit feedback: evidence and hardware test

## Scope

Keep the three ALSA devices: Game Out (0), Voice Out (1), Voice In (2).
The device mixes both playback streams. No software mix or new PipeWire profile.
This experiment addresses variable IN framing and asynchronous playback pacing.
A successful build or unit test does not establish that the audible pops are fixed.

## Measured protocol evidence

A 60-second capture on firmware bcdDevice 1.50 contained:

| IN actual length | Successful packets | Second header LE32 |
| --- | ---: | --- |
| 0 | 3 | none |
| 108 | 480001 | 96 (0x60) in retained headers |
| 124 | 60 | 112 (0x70) in retained headers |

The trace retained all 60 length excursions and their neighbors, with no trace
buffer overflow. The first LE32 word matched packet sequence (offset by the
three initial empty packets). The second word matched actual length minus 12.
The known framing has eight header bytes, 16 bytes per audio frame, and four
trailer bytes. This supports six- and seven-frame packets. It does not validate
all possible firmware variants, frame counts, or trailer values.

The device descriptors identify OUT 0x01 as asynchronous data, maximum 280
bytes, and IN 0x81 as implicit-feedback data, maximum 124 bytes. OUT frames
occupy 40 bytes each, including the slots for both playback streams.

The IN sequence carried 60 extra frames relative to six frames per nonempty
USB interval, roughly 21 ppm. This is relative to USB intervals, not a calibrated
measurement against the host wall clock. No OUT or IN packet errors occurred.

Earlier tests still stuttered with a verified 6144-frame ALSA playback ring.
Merely opening IN capture did not help the fixed-size driver. Neither result
proves that clock mismatch is the only remaining audio defect.

## Initial implementation contract

- Accept only successful IN packets whose bounded length and second header word
  agree with 5, 6, or 7 frames. Unknown formats must not become playback timing.
- Consume ordered frame-count plans, not an averaged rate or fixed extra frame.
- Separate callback submission gates from PCM capture activity. IN must remain
  alive until all dependent playback transport and capture consumers stop.
- Account queued and completed PCM contributions separately and tag each URB
  contribution with the consumer generation. Old completions must not advance
  a newly prepared stream.
- The experiment exposes capture at 48 kHz only. The old advertised 16 kHz mode
  had no rate conversion and cannot satisfy a 16 kHz sample-rate contract.
- Retain read-only diagnostics. Invalid feedback, queue overflow, submission
  failures, and starvation must remain observable, not masquerade as clean audio.

## First playback-only check

After the user loads the experimental module, start with no `arecord` process.
The driver must supply its own IN feedback traffic during playback.

With playback closed, set the same ALSA ring used for the last baseline test:

```bash
printf '48\n' | sudo tee /proc/asound/zg01/pcm1p/sub0/prealloc
aplay -v -D hw:zg01,1 --buffer-size=6144 --period-size=384 /tmp/zg01-997hz.wav
```

In another terminal:

```bash
cat /proc/asound/zg01/usb_stats
```

Read the diagnostics again while playback continues. Successful IN packet counts
must advance despite Voice In's PCM status being closed. Do not change USB port,
headphone level, tone, or ALSA ring for this first comparison.

## Hardware test criteria

1. Close test clients before module replacement. The user performs privileged
   unload/load commands. Do not install or push this experimental build yet.
2. Use direct ALSA Voice Out playback first. Verify 48 kHz stereo S32_LE and
   actual negotiated buffer/period sizes in verbose output or proc hw_params.
3. Run without an application capturing Voice In. The driver must nevertheless
   keep IN active for feedback during playback.
4. Check diagnostic counters before and after playback. Confirm successful
   non-six-frame feedback and variable OUT scheduling without repeated starvation,
   queue overflow, resubmit failures, or stream XRUNs.
5. Listen beyond the previous stutter window. Record approximate pop times and
   preserve counters. No pops in one short test is not a regression pass.
6. Add Game Out while Voice Out continues. Stop either playback stream and check
   that the other remains audible and synchronized.
7. Start and stop Voice In capture while playback continues. Capture close must
   not kill the feedback endpoint or alter the remaining playback ownership.
8. Test capture alone, then playback joining it. Test playback restart after all
   streams close. Old queued audio must not replay after restart.
9. Only after basic tests pass, test suspend/resume and unplug/replug. Save kernel
   logs for any failure. Do not rely on clean aggregate URB status alone.

The tone file `/tmp/zg01-997hz.wav` has a quiet 997 Hz tone with short end fades.
Keep headphone volume comfortable. Startup and teardown clicks are separate
observations from steady-playback stutters.

## Rollback

The pre-feedback diagnostic module is saved locally at
`/tmp/snd-zg01-before-feedback.ko` for the running 7.3.0-rc1 kernel.
With all audio clients closed, the user can restore it:

```bash
sudo rmmod snd-zg01
sudo insmod /tmp/snd-zg01-before-feedback.ko
```

This is a local test rollback, not a DKMS package installation. Module reload
recreates the ALSA card and resets preallocation and diagnostic counters.

## References

- Linux implicit-feedback FIFO handling: sound/usb/endpoint.c,
  snd_usb_handle_sync_urb and snd_usb_queue_pending_output_urbs.
- Linux USB packet status semantics:
  https://docs.kernel.org/driver-api/usb/error-codes.html
- Diagnostic capture procedure: USB_STATS.md.
