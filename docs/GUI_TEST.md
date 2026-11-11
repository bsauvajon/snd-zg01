# GUI hardware acceptance

Manual checklist for `zg01-control` on a real Yamaha ZG01. Run it before a
GUI release; it complements `cargo test` and the packaging job, which cannot
open a card.

Record the machine, distribution, kernel, driver commit, GUI commit and
`/proc/asound/cards` in the release notes. A failed step is a release
blocker unless the report says why it is environmental.

## Prerequisites

- The `snd-zg01` module is loaded and `/proc/asound/cards` shows the ZG01.
- PipeWire or PulseAudio is running if the test needs playback/capture.
- The GUI is installed (`snd-zg01-gui` or `zg01-control-git`) or built with
  `make -C gui build`.
- The user is a normal desktop user. Do **not** run the GUI as root and do
  not install a setuid helper; it must talk to ALSA as the logged-in user.
- Close ZG Controller and any other application that writes the same
  controls.

## 1. Launch and landing screen

1. Start `zg01-control` from the application menu and from a terminal.
2. Wait for the window.

Expected:

- The window opens on **Mixage**, 1100x680 by default.
- The header shows `ZG01`, the card index and the driver name.
- The sidebar shows **Mixage**, **Micro** and **Avancé**.
- The footer says `MIC lu · Mixage/Avancé en cache` /
  `MIC read · Mixage/Advanced cache` after a successful MIC state read. If
  the read failed it says `MIC inconnu · Mixage/Avancé en cache`; on a
  driver without the readback it still says `Valeurs du driver (cache)`.
- No stack trace on stderr; `journalctl --user -b` has no panic. A
  `ZG01 connected` toast may appear on the first connection.

## 2. Live meters

1. Play audio to the ZG01 and speak into the microphone.
2. Watch MIC IN, MIC OUT, VOICE, GAME and the stream meter.

Expected:

- The card meters move at about 20 Hz while audio flows.
- The thin `PW` bars move at the same rate: MIC from `Voice In`, VOICE from
  `Voice Out`, GAME from `Game Out`.
- A silent input falls back to the bottom; it does not stay frozen.
- A meter whose control is missing or unreadable is drawn dashed, never as a
  solid zero.
- A control that reads `-ENODATA` (the driver has no valid frame) is also
  dashed/unknown, and the value label shows `—`; it is never shown as zero.
- The Mixage strips use `mic_in`, `pc_in` L/R, `game_mix` L/R and
  `stream_mix` L/R. `GAME` now has a card bar; do not report it as missing.
- Open **Meters**. It shows MIC IN, MIC OUT and the eight channels of
  PC IN, HDMI IN, GAME MIX and STREAM MIX. Every channel is labelled
  L/R/C/LFE/SBL/SBR/SL/SR; a channel with no reading is dashed.
- An old driver without `Meter Game Mix`/`Meter HDMI In` shows the legacy
  four-meter layout and a notice on the Meters page; the page row is
  disabled and no legacy value is remapped.
- STREAM OUT has no Linux source: it stays card-only.
- A `PW` source that exists but has no frames is dashed; a source that
  PipeWire does not expose has no bar at all.
- Closing the window leaves no `pw-record` process behind
  (`pgrep -af pw-record` is empty).

> **Current hardware note (2026-11-11).** The v2 decode is complete and
> the Windows ZG Controller streams 192-byte frames, but the current Linux
> unit stops streaming after a suspend: EP `0x83` returns zero-length
> packets and the six meter groups read `-ENODATA`. The expected GUI
> presentation is therefore dashed/unknown card bars on Mixage and Meters;
> the `PW` bars are the live levels on this unit. Record that as the
> observed state; card-meter acceptance stays **blocked** until a session
> brings the 192-byte frames back (see `docs/METER_TEST.md`). A moving card
> bar cannot be required from this unit, and a dashed card bar is not a
> code failure.

## 3. Fader writes

1. Move a Mixage send fader with the pointer.
2. Drag with Shift held, scroll the wheel, and use the arrow keys.
3. Press `Page Up`, `Page Down`, `Home` and `End`.
4. Double-click the fader.
5. Check the same value from a terminal:
   `amixer -c zg01 cget 'name=Streaming Mic Level Steps'`.

Expected:

- The value under the fader follows the pointer and the keyboard.
- Arrow keys move one device step; `Page Up`/`Page Down` move ten; `Home` and
  `End` go to the control bounds.
- Shift makes pointer drags finer.
- Double-click returns to the neutral value (bottom for a send, 0 dB for a
  gain).
- `amixer` reports the same value once the write lands.
- No other control changes because of the drag.

> **Mixage is a stream mixer, not a local monitor.** The MIC/VOICE/GAME
> faders change the USB/stream mix; they are not the headphone/speaker
> volume. Do not fail this check because the local audio does not change.
> Confirm the write with `amixer`; the local monitor is on the physical
> knobs and the `Monitor *` controls in Avancé.
>
> The driver does not expose the ZG01's separate USB **Streaming** capture
> device. OBS therefore builds the stream from `Voice In` plus the
> `Game Out`/`Voice Out` monitors, or captures the HDMI OUT.

## 4. Micro screen

1. Click the MIC strip (or select **Micro** in the sidebar).
2. Toggle GATE, COMP and LIMIT; move their faders.
3. Open the **Avancé** expander and change one threshold.
4. Change an EQ band gain, frequency and Q; toggle the low/high shape.

Expected:

- The chain reads `GATE › COMP › EQ › LIMITER`.
- Each switch and fader writes its own control; no cross-talk.
- The expander shows the block's out-gain and threshold controls.
- A control the driver does not publish is hidden; the screen never shows a
  widget bound to a missing control.
- The Micro header has **Lire depuis le ZG01** / **Read from ZG01**.
- The direct fields (gate switch/value/range, compressor
  switch/value/threshold/ratio/knee, limiter switch/value/threshold/knee,
  EQ switch/shapes/Q) show the persisted device state, not the built-in
  defaults. In the current unit Gate reads on/60 and Limiter on/50.
- The transformed fields (gate threshold/attack/hold/decay,
  compressor/limiter attack/release/out gain, EQ gain/frequency) stay
  dashed unknown; they are not populated from the dump.
- Returning to Mixage keeps the values and does not reload the card.

## 5. Preset apply

1. Click **Charger** / **Load** in the Mixage preset bar.
2. Choose a `.zgpreset` file.
3. Review the preview: block, name, applied count, skipped rows and reasons.
4. Apply it, first without and then with **Inclure les paramètres bruts**.

Expected:

- The chooser filters `*.zgpreset`; a `.zg01` backup is rejected with a clear
  message, not parsed.
- The preview counts match the file; skips name the address, control and
  reason.
- Applying changes only the listed controls; the report says what was
  applied, skipped and failed.
- The preset bar names the applied preset.
- Nothing is persisted until Save; the report reminds the user.

## 6. MIC state readback

1. Open **Micro** and note the direct values and the dashed transformed
   fields.
2. Change the MIC state outside the GUI (ZG Controller on Windows, or the
   device's RESET), or press **Réinitialiser** / **Reset** in the GUI.
3. Click **Lire depuis le ZG01** / **Read from ZG01** and watch the footer.
4. Simulate a driver timeout (or read while the card is being removed)
   and recover; physical unplug/replug is covered in section 8.

Expected:

- The direct fields update to the persisted state; the current unit reads
  Gate on/60, Limiter on/50 and the EQ Q values from the dump.
- The transformed fields stay dashed unknown; the GUI never displays a
  DSP coefficient as an attack or a gain.
- A successful read sets the footer to `MIC lu · Mixage/Avancé en cache`
  and enables Save even though the transformed fields remain unknown.
- A failed/timeout read leaves the MIC chain dashed, the footer at
  `MIC inconnu · Mixage/Avancé en cache`, Save disabled, and shows an
  error toast.
- The Mixage page carries the notice that its section still uses the
  driver cache.
- Reset triggers its own post-reset read; the direct fields come back
  known and Save is enabled again.

## 7. Save and Reset

1. Change a control, then click **Sauvegarder** / **Save**.
2. Wait for the toast, then close and reopen the GUI.
3. Change a control again and click **Réinitialiser** / **Reset**.
4. Observe the banner and the Save button.

Expected:

- Save writes `Save to ZG01` and the toast confirms it.
- Reopening shows the saved MIC values from the state read, not the
  built-in defaults.
- Reset reloads the persisted settings and triggers a post-reset state
  read: the direct MIC fields become known again, Save is enabled after
  that read, and the transformed fields stay dashed unknown; Mixage
  keeps the driver cache and its notice.
- The unknown banner clears when the post-reset read succeeds; it stays
  while the read is pending or failed.
- Save never writes the cache defaults for the transformed fields or the
  Mixage section; it only persists the device state.
- A failed Save or Reset shows an error toast and does not claim success.

## 8. Disconnect and reconnect

1. While the GUI is open, unplug the ZG01.
2. Wait five seconds, then plug it back in.

Expected:

- The header shows `Déconnecté` / `Disconnected`; the reconnect banner
  appears and the controls become insensitive.
- The last values stay visible; the app does not panic or quit.
- Reconnect is automatic; the header returns to `ZG01`, the controls are
  read again and the reconnect banner disappears.
- A write started just before the unplug never lands on the next card and
  never overwrites a value read after reconnect.

## 9. No root and no hidden fallback

1. Run the GUI as the logged-in user with no `sudo`.
2. Check `ps -o user,pid,cmd -C zg01-control`.
3. Look for any helper that opens `/dev/snd` as root.

Expected:

- The process runs as the desktop user.
- No setuid binary, no polkit action and no root daemon is required.
- If the user cannot open the card, the connection view explains the USB and
  module checks instead of falling back to a privileged path.

## 10. Clean shutdown

1. Close the window while meters are running.
2. Reopen it immediately.

Expected:

- The process exits within a second; no thread keeps it alive.
- Reopening reads the card again and shows current values.
- `journalctl --user -b` has no panic or use-after-free warning.

## Report template

```
Date:
Distribution / desktop:
Kernel:
Driver commit:
GUI commit:
Card index / driver:
Steps passed: 1 2 3 4 5 6 7 8 9 10
Failures and evidence (screenshots, journal lines):
Notes:
```
