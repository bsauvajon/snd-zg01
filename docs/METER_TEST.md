# Meter acceptance checklist

Acceptance for the six card meters: `Meter Mic In` and `Meter Mic Out`
(one value each), and `Meter PC In`, `Meter HDMI In`, `Meter Game Mix`
and `Meter Stream Mix` (eight values each). The v2 decode is in
`docs/METER_PROTOCOL.md`; this file records the hardware checks and what
they showed.

> **Acceptance status: BLOCKED (2026-11-11).** The controls and the v2
> parser are implemented, and the Windows ZG Controller streams 192-byte
> frames. The Linux unit still returns only zero-length packets after the
> 2026-11-01 suspend, so the six controls read `-ENODATA` even after a
> power cycle, `usbreset`, a hub port cycle and a replay of the Windows
> preamble. The earlier results below are kept as decode evidence; their
> `pc_in`/`stream_mix` labels predate the v2 map (those values are
> `stream_mix` channels 2 and 6). Live Linux acceptance stays blocked on
> the device/session, not on the decode. The GUI state for this case is
> the dashed/unknown Meters page documented in `docs/GUI_TEST.md`.

Values are the control's integer, 0..127, where `value = dB + 127`
(0 = −127 dB, 127 = 0 dBFS). Rounded to whole steps; the decode is 1 dB
per step.

## Setup

- Device: Yamaha ZG01 (`0499:1513`), Linux 7.0, worktree build of
  `snd-zg01` (not the installed DKMS build, which has no mixer controls).
- Stop the user audio services before loading the module:
  `systemctl --user stop pipewire.socket pipewire-pulse.socket pipewire pipewire-pulse wireplumber`.
- Load the build: `sudo rmmod snd-zg01 && sudo insmod ./snd-zg01.ko`.
- Restore afterwards: `sudo rmmod snd-zg01 && sudo modprobe snd-zg01`, then
  start the same five units.
- Test signals are synthetic (1 kHz sine, 6 s, stereo S32_LE at 48 kHz)
  generated at −20 and −6 dBFS. No personal audio is used or committed.

Sampling: `amixer -c zg01 cget 'name=Meter …'`, read every 0.5 s. Each row
is the median of 12 samples taken from 1.5 s into a 6 s playback, with
min/max in brackets. The last two samples fall after the sine ends, so
the min values include the decay.

## Checklist

| # | Check | Result |
|---|---|---|
| 1 | Six controls registered, `access=r--v----`, `min=0 max=127`, counts 1/1/8/8/8/8 | **NOT RUN (v2)**; the PASS rows below are the v1 four-control session |
| 2 | v1: Game Out playback moves the two stream channels and not the old `pc_in` pair | PASS (v1) |
| 3 | v1: Voice playback moves the old `pc_in` pair and the stream pair | PASS (v1) |
| 4 | Silence reads the floor (see note) | PASS, with tolerance |
| 5 | Mic In / Mic Out track a deliberate mic input | NOT RUN (see note) |
| 6 | Disconnect while streaming: no crash, stream ends cleanly | PASS |
| 7 | v2: replug recreates the card and the six controls, with arrays | **NOT RUN / BLOCKED** (the v1 session only had four controls) |
| 8 | Suspend/resume restarts the reader and the keepalive | PASS for the reader/keepalive in `freeze` (s2idle); **frames never resumed** in the latest session; `reset_resume` not exercised |
| 9 | v2: `alsactl store` and `restore` succeed; the six controls are not restored | **NOT RUN / BLOCKED** (v1 PASS) |
| 10 | No oops, BUG, WARNING or lockdep splat in the log | PASS |

### Historical v1 results (four controls)

**1. Registration (v1).** The earlier session listed the four controls
`Meter Mic In`, `Meter Mic Out`, `Meter PC In` and `Meter Stream Mix`,
each `values=1`, `type=INTEGER,access=r--v----,min=0,max=127`. The v2
six-control registration and array counts are **NOT RUN** (see below).

**2 and 3. Playback tracks the documented table.** Measured medians from
the earlier decode, with the observed values. The `pc_in`/`stream_mix`
columns use the old single-pair labels; with the v2 map they are
`stream_mix` channels 2 and 6. The current v2 capture evidence is in
`docs/METER_PROTOCOL.md`.

| Stimulus (nominal) | Meter | Documented | Observed (median, min–max) |
|---|---|---|---|
| Game Out −20 dBFS | `stream_mix` | 104 (−23 dB) | 104 (78–104) |
| Game Out −6 dBFS | `stream_mix` | 117 (−10 dB) | 117 (87–117) |
| Game Out −20 dBFS | `pc_in` | ~37 (floor) | 37 (36–38) |
| Voice Out −20 dBFS | `pc_in` | 107 (−20 dB) | 107 (75–107) |
| Voice Out −20 dBFS | `stream_mix` | 107 (−20 dB) | 107 (75–107) |
| Voice Out −6 dBFS | `pc_in` | 121 (−6 dB) | 121 (91–121) |
| Voice Out −6 dBFS | `stream_mix` | 121 (−6 dB) | 121 (89–121) |

Tolerance: within 2 steps of the documented value (the decode's own
"track within 2 steps" bar). Every stimulus matched exactly. The
(min–max) spread is the fast attack and decay of the sine's envelope,
not noise. `pc_in` ignores Game Out, as documented.

**4. Silence and the floor.** At idle, with no playback:

| Meter | Median (min–max) | Documented |
|---|---|---|
| `pc_in` | 38 (37–39) | floor −89 dB (38) |
| `stream_mix` | 38 (37–61) | floor matches the voice floor |
| `mic_in` | 76 (73–86) | ambient, about −51 dB |
| `mic_out` | 19 (15–57) | falls to −127 (0) only when the gate closes |

The `pc_in` and `stream_mix` floors match the documented values. `mic_out`
sits at about −108 dB, not at the −127 floor; its documented floor applies
when the gate closes. The driver does not read the device's gate state
(the controls are a write cache), so the gate state at test time is not
known here. The test does not claim `mic_out` reached 0.

**5. Mic tracking: not run.** No one spoke or tapped the mic during these
runs. The only mic response was in the Voice Out playback sweeps, where
`mic_out` rose from 19 to 84 and `mic_in` from 76 to 86. That matches the
documented acoustic-coupling observation (`METER_PROTOCOL.md`, anomalies).
A deliberate mic tap test is still open: it needs a person at the mic.

**6. Disconnect while streaming.** A 6 s Game Out playback was running when
the device was deauthorised (`echo 0 > …/1-5.3.3/authorized`, the same path
as an unplug). The player exited with `ENODEV`. The card left
`/proc/asound/cards` and no oops, BUG, WARNING or lockdep line followed.

**7. Replug.** In the earlier session, `echo 1 > …/authorized` recreated
the card (`2 [zg01] zg01_usb - ZG01`) and the six controls. The reader
started again, and the controls returned live values (idle `pc_in` 38).
In the latest session the card and the six controls come back, but every
read returns `-ENODATA`: the device sends zero-length packets and no
192-byte frame. The earlier result is evidence for the probe path, not a
claim about the current unit.

**8. Suspend/resume.** Run as a system suspend-to-idle
(`rtcwake -m freeze -s 15`, the `freeze` state, about 17 s). A usbmon capture
of the device's bus showed:

- At 3.285 s the pending `0x83` URB completed with `status -2` (`ENOENT`):
  `zg01_meters_stop` in the suspend callback.
- At 19.341 s, on resume, a new `0x83` URB was submitted: the reader restarted
  in `zg01_resume`.
- The first keepalive came at 20.353 s, one second after the restart, as
  scheduled.
- After resume the device sent one buffered frame, then zero-length packets
  until the keepalive, then 192-byte frames.
- The control reads after resume returned live values.

**Latest session:** this resume was the one after which the device stopped
metering. The reader and keepalive restarted as above, but the device
stayed on zero-length packets and no later frame arrived; reads were
`-ENODATA`. Do not treat this check as a current pass.

Not exercised: `zg01_reset_resume`. The device resumed without a reset, so the
core used plain `resume`. The reset path calls the same `zg01_meters_start`,
but it has not run on hardware. A later system suspend that resets the device
would exercise it.

**9. ALSA state.** `alsactl store` and `restore` both return 0. The restore
trace shows 27 entries (23 writable controls plus the 4 meters). The meters
are stored as read-only snapshots and skipped on restore, so no exclusion is
needed.

**10. Log.** A scan of the kernel log from the start of the session through
restore found no oops, BUG, WARNING, lockdep or refcount message. The one
match for `bug` was `(use no_console_suspend to debug)`, which is a kernel
message, not a fault.

## Notes and limits

- The update rate is the device's frame rate, about 20 Hz (50 ms). It was
  measured on usbmon in the Task 4 captures (mean 50.2 ms). A control read
  returns the latest valid frame; it does not block for the next one.
- A missing meter reports an error, not zero. The driver returns `-ENODATA`
  before the first valid frame, after 500 ms without one, and after the
  reader stops. Those cases were not forced on hardware. The stale path
  relies on the 50 ms frame rate, and the stop path was exercised by
  suspend and disconnect.
- Mic tracking needs a person and a known source. Record the result here
  when it is done.
- GAME and HDMI are registered in v2 as `Meter Game Mix` and
  `Meter HDMI In`, eight values each. The v2 captures show Game Out on
  `pc_in`/`game_mix` 0/1 and HDMI on `hdmi_in`/`game_mix` 0/1; the
  surround channels 2–7 are not measured yet.

### v2 checks (NOT RUN)

- Six controls and counts: **NOT RUN**. The controls cannot be inspected
  while the ZG01 is attached to the Windows VM and the host runs the old
  DKMS module. `tests/test-zg01-meter-controls.sh` is the check to run
  with the new module.
- Replug with the six controls and eight-value arrays: **NOT RUN**.
- `alsactl store`/`restore` with the six controls: **NOT RUN**; the v1
  four-control result is the only evidence, and the controls are still
  read-only and volatile.

## Reproducing

Generate a −20 dBFS sine (6 s, stereo S32_LE, 48 kHz) with any tool, for
example Python's `wave` module. The v2 map makes each surround control an
eight-value array; read the whole array, not one value.

Game Out on the Linux card (`hw:zg01,0`) is the measured path:

```bash
aplay -q -D hw:zg01,0 sine-m20.wav &
sleep 2
amixer -c zg01 cget 'name=Meter PC In'    # channels 0/1 around 107
amixer -c zg01 cget 'name=Meter Game Mix' # channels 0/1 around 107
```

The current captures put Game Out and HDMI on the L/R channels of their
groups; Voice playback was observed on `Meter Stream Mix` 0/1. The exact
Linux PCM-to-VOICE-strip mapping is still open. Raw captures are not
committed. They stay on the test machine.
