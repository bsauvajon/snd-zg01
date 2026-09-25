# Mic DSP control plan (ALSA kcontrols + zgctl)

Expose the ZG01 mic processing — GATE, COMP, EQ, LIMITER — as Linux
controls, at parity with the ZG Controller **MIC SETTINGS** screen.
This is the first subset of a broader effort to surface every ZG
Controller setting through the driver.

Reference screenshot (Yamaha, not vendored in this repository):
<https://fr.yamaha.com/fr/files/zg-controller-mic-settings_tcm122-1695971.jpg>

The screen has four processing blocks, each with an OFF/ON switch:

| Block | Control | Notes |
|---|---|---|
| GATE | OFF/ON + slider | value 0-100 (example: 44) |
| COMP | OFF/ON + slider | value 0-100 (example: 26) |
| EQ | OFF/ON + preset `Custom` + `EDIT` | preset selection plus a fine editor |
| LIMITER | OFF/ON + slider | value 0-100 (example: 42) |

`MIC LEVEL` / `OUT LEVEL` meters and the top-left `Save to ZG01` toggle
sit on the same screen. `RESET` restores defaults.

## ZG Controller screen map

Sidebar order, top to bottom: Home, MIC SETTINGS, MIC EFFECT, GAME
EFFECT, HEADPHONE MONITOR SETTINGS, STREAMING OUTPUT MIXER, SETTINGS.
Screenshots: <https://fr.yamaha.com/fr/audio/streaming-gaming/explore/guides/zg-controller.html>

| Screen | Controls |
|---|---|
| MIC SETTINGS | GATE (on/off + value), COMP (on/off + value), EQ (on/off + preset + `EDIT`), LIMITER (on/off + value); MIC/OUT level meters; `RESET` |
| MIC EFFECT | VOICE CHANGER 1 (preset + `EDIT`), VOICE CHANGER 2 / SE (radio + level), Echo (preset); `RESET` |
| GAME EFFECT | 3 slots, PRESETS (Engage-M/C/R, Search, Dramatic), USER PRESETS, `EDIT LIST` |
| HEADPHONE MONITOR SETTINGS | EQ (on/off + preset + `EDIT`), PRESETS (Gaming Headphones, Headphones, Earphones, YH-G01), USER PRESETS, MIC MONITOR LEVEL; `RESET` |
| STREAMING OUTPUT MIXER | INPUT MIC/VOICE/GAME (FX, mute, LEVEL), OUTPUT USB (on/off + MONITOR), HDMI (on/off), meters |
| SETTINGS | HARDWARE/APPLICATION: HDCP version, 4K MODE, output destination, firmware update, language |

Note the save behavior differs by screen: the DSP screens (MIC
SETTINGS, MIC EFFECT, GAME EFFECT, HEADPHONE MONITOR) expose a
`Save to ZG01` action button and a `RESET` button, while STREAMING OUTPUT
MIXER looks like a live mixer (no save control in the screenshot). On the
DSP screens, edits apply live and `Save to ZG01` only persists them to
non-volatile memory; unsaved edits are lost on next power-up.

## Goal

Read and write the mic DSP settings from Linux:

- ALSA kcontrols on the existing `zg01` card, visible in `amixer`,
  `alsactl`, and PipeWire.
- A `zgctl` CLI that reads and writes the same controls.

The mic is the first subset. The transport layer built here must be
reusable for the other screens later.

## Scope

In scope:

- Read current mic DSP state on driver load and on demand.
- Write enable/disable and the main value of GATE, COMP, LIMITER.
- Read and write the EQ preset; read and write EQ bands (from `EDIT`).

Out of scope for this plan:

- Other ZG Controller screens (ZG SURROUND, FOCUS MODE/EQ, 3D CHAT
  SPACE, STREAMING OUTPUT MIXER, HEADPHONE, SETTINGS (HARDWARE)).
- Firmware update traffic.
- MIDI port.

## Decisions

- **Interface**: ALSA kcontrols (`SNDRV_CTL_ELEM_IFACE_MIXER`) plus the
  `zgctl` userspace tool. `/proc` is not used; ALSA is the idiomatic
  ABI and integrates with existing tooling.
- **Capture**: Windows guest under libvirt/QEMU with `usb-host`
  passthrough; capture on the **Linux host with `usbmon`**. This yields
  the same usbmon format as `capture/zg01_init.pcap`, keeps every
  endpoint and response, and needs no USBPcap inside the guest.
- **Scope of first milestone**: read + write of the mic DSP subset.

## What is already known

Two transports are involved:

- **EP0 vendor requests** carry enumeration and status only: `0xc0
  bRequest 7` -> `80bb00`, `0xc0 bRequest 4` -> 1-byte status poll,
  `0xc0 bRequest 6` -> UTF-16 channel names, plus standard
  GET_DESCRIPTOR traffic.
- **Mixer parameters ride on interface 4 bulk**, not EP0. Host writes go
  to EP `0x03`, device traffic on EP `0x83`, in 512-byte frames. Each
  frame is 128 four-byte words (`<type 0x04> <3 data bytes>`), zero
  padded, terminated by a `05` word.

Observed mic parameter write on EP `0x03` (captured on the MIC SETTINGS
screen, see `captures/`):

```
04 f0 43 10  04 3e 14 01  04 01 00 00  04 <id> 00 00  04 00 00 00  04 00 <value>  05 f7 ..
```

- `id = 0x02`: GATE enable (`value` 0/1)
- `id = 0x03`: GATE level (a drag to high produced 0x41..0x69)

Other message types seen: `04 f0 43 00` carries preset-name payloads
(ASCII split across words, e.g. `_Preset Voice`, `Soprano voice`);
`04 f0 43 20` and `04 f0 43 30` look like query/ack/commit. The `0x83`
IN stream (192 bytes, continuously varying) is telemetry, probably meters
plus state.

The level scale is not pinned yet: a drag "high" produced 65..105, while
the UI shows small integers (GATE 44, COMP 26, LIMITER 42 in the
screenshot). The byte may be an internal/scaled value or a 0-127 range.

Earlier code only sends the `bRequest 7` handshake; it does not touch the
interface 4 transport.

The PDF manuals (User Guide, Data Sheet) describe the product at user
level only: no MIDI implementation chart, no register map. The
authoritative parameter reference is the ZG Controller built-in
operating guide (the `?` icon), read in the app.

## Capture method

### Setup

On the Linux host:

```bash
sudo modprobe usbmon
lsusb | grep 0499:1513          # confirm the ZG01 is visible
cat /sys/kernel/debug/usb/usbmon/*/  # or use tshark -D to list usbmon buses
```

Pass the ZG01 through to the Windows guest with QEMU `usb-host`
(vendorid `0x0499`, productid `0x1513`). Capture the matching
`usbmonN` interface for the whole session:

```bash
sudo tshark -i usbmon2 -w 00-mic-baseline.pcapng
```

### `Save to ZG01` is a persist action

On the DSP screens, `Save to ZG01` appears once a setting is modified
and persists it to the device's non-volatile memory; `RESET` cancels
un-saved edits. Settings are applied **live** as soon as they change, so
an action capture shows the write even without saving. Capture live
edits normally; take one extra trace pressing `Save to ZG01` to find the
persist command.

### Capture matrix

Start each trace from the same saved state; wait ~1 s, do exactly one
action, wait ~1 s, stop.

| File | Action |
|---|---|
| `00-mic-baseline.pcapng` | Open and close MIC SETTINGS; change nothing |
| `01-mic-reset.pcapng` | Press `RESET` |
| `10-gate-off-on.pcapng` | GATE off → on |
| `11-gate-on-off.pcapng` | GATE on → off |
| `12-gate-low.pcapng` | GATE slider to a low value |
| `13-gate-mid.pcapng` | GATE slider to a mid value |
| `14-gate-high.pcapng` | GATE slider to a high value |
| `20..24-comp-*.pcapng` | Same pattern for COMP |
| `30..34-limit-*.pcapng` | Same pattern for LIMITER |
| `40-eq-off-on.pcapng` | EQ off → on |
| `41-eq-preset-A.pcapng` | Select a named EQ preset |
| `42-eq-preset-B.pcapng` | Select another preset |
| `43-eq-edit-band.pcapng` | Open `EDIT`, change one band |

Add one trace per preset and per EQ band as the `EDIT` window is mapped.

### Export to diff-friendly JSONL

Prefer capturing directly to pcap/`.pcapng` on the host, then normalize:

```bash
python3 tools/normalize-usbmon.py 13-gate-mid.pcapng > 13-gate-mid.jsonl
```

`tools/normalize-usbmon.py` does not exist yet (Phase 0). Unlike
`tools/normalize-usbpcap.py`, it must key on bus/device rather than
`idVendor`/`idProduct` (usbmon does not expose those on every record),
and it must **keep response payloads and non-EP0 endpoints**.

## Decoding workflow

1. Subtract the baseline/open-close traffic from every action trace.
2. Compare pairs (off/on, low/high) for request, value, index, length,
   and bytes that change.
3. Repeat at three known UI values to separate absolute values from
   relative increments and to bound the range.
4. Identify the read command from the app-open trace: it likely reads
   the whole parameter set on connect. Correlate those reads with the
   known state produced by the writes.
5. Never replay an unknown message on Linux until direction, bounds,
   and expected response are understood.
6. Add a sanitized fixture and a unit test for each decoded message
   before exposing it.

## Implementation phases

### Phase 0 — usbmon tooling

- Add `tools/normalize-usbmon.py` (pcap → JSONL) preserving all
  endpoints and payloads.
- Document the libvirt + usbmon procedure in `docs/PROTOCOL_CAPTURE.md`.

**Done when**: an app-open capture converts to JSONL in which vendor
requests and any bulk traffic on `0x02/0x03/0x82/0x83` are
unambiguously identifiable.

### Phase 1 — read map

- Capture app-open in at least three known mic states and identify the
  state-read command, addressing, and value encoding.

**Done when**: one mic parameter (e.g. LIMITER value) is read and
correlated with the UI value at three distinct settings.

### Phase 2 — control transport

- Extend `src/zg01_control.c` with a reusable vendor-request send/recv
  helper (timeout, short-reply handling, bounds) on top of the existing
  card lifecycle.

**Done when**: a kernel-side helper can issue a decoded read request
and log a correct value.

### Phase 3 — kcontrols + zgctl

- Register GATE/COMP/LIMITER switches and value controls, and the EQ
  switch/preset, as ALSA kcontrols. Read state at init, write on `put`,
  read back and report errors.
- Add `zgctl` (`list`, `get`, `set`) using the same control names.

**Done when**: `amixer -c zg01` shows the mic controls and a `set`
audibly changes the device, verified by a paired capture.

### Phase 4 — EQ detail and tests

- Map the `EDIT` EQ window (bands, frequency/gain/Q) and expose it.
- Add fixtures, unit tests, and update `README.md` (move mic controls
  out of "not implemented").

**Done when**: EQ bands are readable/writable and the test suite covers
the decoded messages.

## Risks

- `Save to ZG01` buffering hides writes (see caveat above).
- The meters are periodic reads and may dominate traces; filter them
  out during diffing rather than treating them as control.
- The EQ `EDIT` window may use a different message shape or a bulk
  endpoint; confirm early.
- Firmware-update and per-stream audio traffic can be mistaken for
  control; isolate with a baseline.
- Keep raw captures out of Git until reviewed; no personal audio in
  traces intended for publication.
