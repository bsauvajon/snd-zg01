# ZG01 Control — Linux desktop GUI design

Date: 2026-10-31
Status: draft for approval
Project: `snd-zg01` (Yamaha ZG01 Linux driver)

## Intent

The driver now exposes the ZG01 mic DSP as ALSA controls, and the
full-parameter-surface plan extends that to every addressable parameter
plus `.zgpreset` loading. `zgctl` and `amixer` are adequate for scripting
and debugging, but they are a poor fit for two real moments: preparing a
recording or stream, and watching levels while using the device.

Build a native Linux desktop application, **ZG01 Control**, that:

- opens on a working mixer, not on a Home dashboard;
- shows the card's internal meters live, and optionally the Linux-side
  stream levels next to them;
- exposes the mic chain as a console-like detail view with vertical
  faders;
- loads `.zgpreset` files, applies them, and persists settings through
  the existing `Save to ZG01` action;
- is honest about the driver's cache: it never pretends to read the
  device state it cannot read.

The application is a public GitHub project, distributed with the driver
packages, and targets the same audience: Linux users of the ZG01,
especially Arch/Omarchy and Debian/Ubuntu.

## Goals

1. One application with two modes: **setup** (slow, detailed) and
   **live** (glanceable, few clicks). There is no separate Home screen.
2. **Mixage** is the landing screen. It shows the three stream sends,
   the stream output block, and live meters. Clicking a channel opens its
   detail.
3. **Micro** is the detail screen: GATE, COMP, EQ and LIMITER as vertical
   faders, with sub-parameters behind an expander.
4. Internal meters come from the card. Linux levels (Game Out, Voice
   Out, Voice In) are shown beside them when PipeWire integration is
   available. The two sources are visually distinct.
5. Loading a `.zgpreset` reports every applied, skipped and out-of-range
   parameter before and after applying. Raw controls require an explicit
   opt-in, mirroring `zgctl preset load --allow-raw`.
6. Every exposed ALSA control remains reachable: curated screens for the
   common parameters, an **Avancé** table for the rest.
7. Native Rust + GTK4 + libadwaita application, packaged for Arch and
   Debian, translatable (English and French at minimum), GPL-2.0.
8. The UI states clearly that values come from the driver cache, and
   that settings changed outside the application are not reflected until
   they are written again from Linux.

## Non-goals

- A Home screen that mirrors the physical knobs and buttons.
- Decoding the `F0 43 20` state dump or reading settings back from the
  device in v1.
- `.zgpreset` export in v1. Loading and applying only.
- Firmware update, MIDI, ZG02 support, Windows or macOS.
- Userspace audio processing. All DSP stays on the ZG01.
- A light theme in v1. The application is dark-first; a light token set
  is a follow-up.
- A full preset library manager (rename, delete, collections). v1 has a
  file chooser and a short recent list.
- Replacing `zgctl`. The CLI remains the scripting interface.

## Anti-goals

- Showing a slider as "the device value" when it is only the driver
  cache.
- Duplicating the hardware front panel on a landing screen.
- Silently clamping preset values into a control's range.
- Blocking the GTK main loop on USB writes or ALSA ioctls.
- Bundling Yamaha binaries, factory presets or Yamaha artwork.
- Requiring root to run the GUI.

## Constraints

- ALSA control names are the public interface between the driver and the
  GUI. The curated layout table refers to them by name; the address is
  the stable key for presets.
- The driver's kcontrol getters return the last value written by the
  driver (a cache). The GUI must present this honestly.
- The full parameter surface and the `.zgpreset` parser are being built
  in `feat/zgpreset-controls` (see
  `docs/superpowers/plans/2026-10-31-zgpreset-full-controls.md`). That
  work lands before the GUI's preset and Avancé phases; the GUI reuses
  its C parser and control map.
- The device pushes 192-byte meter frames on bulk IN `0x83` roughly every
  50 ms. Their byte mapping and scale are not decoded yet.
- The driver claims USB interfaces 1, 3 and 4. Userspace cannot own
  interface 4 while the driver is bound.
- ALSA control names are limited to 44 bytes and must be unique.
- Wayland has no standard "always on top". Layer-shell works on
  wlroots-based compositors (Hyprland); GNOME does not support it.
- GTK4 and libadwaita versions vary across distributions.
- Debian's Rust packaging policy makes a cargo-based binary package
  harder than an Arch one; Flatpak is the fallback for cross-distro
  delivery.
- The application runs as the user; the user must be in the `audio`
  group, as with any ALSA application.

## Users and moments

Two moments drive the design:

| Moment | What the user does | Screen |
|---|---|---|
| Setup, before a recording or stream | Loads a preset, adjusts the mic chain, balances the stream mix, saves to the device | Mixage, then Micro |
| Live, during a recording or stream | Glances at levels, nudges a fader, mutes a source, checks the stream mix | Mixage, compact meters |

The user is the author first, then other ZG01 Linux users. The public
project should be installable and understandable without reading the
source, but it does not need to be a general-purpose mixer.

## Information architecture

Sidebar, top to bottom:

1. **Mixage** — landing. Three stream sends, stream output, meters,
   preset bar.
2. **Micro** — GATE, COMP, EQ, LIMITER, sub-parameters.
3. **Jeu** — GAME EFFECT slots, 3D CHAT SPACE, ZG SURROUND, FOCUS/EQ
   (v1.1).
4. **Casque** — headphone monitor EQ and levels (v1.1).
5. **Réglages** — HDMI, LED, firmware/version, application preferences
   (v1.1).
6. **Avancé** — every remaining ALSA control, searchable (v1).

The application opens on **Mixage**. There is no Home screen.

### Mixage wireframe

```
┌────────────────────────────────────────────────────────────────────┐
│ ● ZG01   Preset: Streaming            [Meters]  [Sauvegarder]      │
├────────┬───────────────────────────────────────────────────────────┤
│ MIC    │ Mixage                                                    │
│ JEU    │ ┌──────┬──────┬──────┬────────────────┐                  │
│ CASQUE │ │ MIC  │VOICE │ GAME │ STREAM OUT     │                  │
│ MIX ●  │ │ ▮ │  │ ▮ │  │ ▮ │  │ ▮ stream meter │                  │
│ AVANCÉ │ │ ▯ │  │ ▯ │  │ ▯ │  │                │                  │
│        │ │ █ │  │ █ │  │ █ │  │ [USB] [HDMI]   │                  │
│        │ │ 103  │ 110  │  96  │  [MON]         │                  │
│        │ └──────┴──────┴──────┴────────────────┘                  │
│        │ Preset  Streaming            [Charger]  [Récent ▾]        │
└────────┴───────────────────────────────────────────────────────────┘
```

The three fader strips are the **stream sends**, not a copy of the front
panel:

- **MIC**, **VOICE**, **GAME** map to `Streaming Mic Level Steps`,
  `Streaming Voice Level Steps` and `Streaming Game Level Steps`
  (0–127 device steps). They control what viewers/listeners receive, not
  what the local headphones hear. The physical knobs remain the local
  monitor control.
- **STREAM OUT** is not a level. The device exposes USB out, HDMI out
  and monitor as switches (`Streaming USB Out`, `Streaming HDMI Out`,
  `Streaming Monitor Steps`). The block shows those toggles and the
  `Meter Stream Mix` meter. `Streaming Out Mute Steps` is a raw,
  wide-range control; it stays in Avancé until its encoding is decoded,
  rather than being guessed as a boolean.
- The **monitor** path is deliberately not on this screen. The device
  exposes four monitor levels (`Monitor Headphone Level Steps`,
  `Monitor Speaker Level Steps`, `Monitor Voice Level Steps`,
  `Monitor Game Level Steps`) and they duplicate the front-panel knobs.
  They live on the **Casque** screen (v1.1) for users who need them.

This distinction is the answer to the "Home screen duplicates the
physical controls" objection: the mixer is a different signal path.

**Units.** Stream and monitor levels are 0–127 device steps with no
established dB mapping. The UI shows the step value and a 0–127 scale; it
does not invent a dB readout. Gain controls (`... Gain`) are the
exception: they use the driver's established 0.1 dB conversion.

### Micro wireframe

```
┌────────────────────────────────────────────────────────────────────┐
│ ● ZG01   Preset: Streaming            [Meters]  [Sauvegarder]      │
├────────┬───────────────────────────────────────────────────────────┤
│ MIC ●  │ Micro   chaîne de traitement                              │
│ JEU    │ ┌──────┬──────┬──────┐      ┌────┐                       │
│ CASQUE │ │ GATE │ COMP │ LIMIT│      │MIC │                       │
│ MIX    │ │  ▮   │  ▮   │  ▮   │      │ ▮  │                       │
│ AVANCÉ │ │  █   │  █   │  █   │      └────┘                       │
│        │ └──────┴──────┴──────┘                                   │
│        │ EQ   [80] [250] [1k] [3k]   4 bandes, gains verticaux     │
│        │ ▸ Avancé : out gain, seuil, attaque, release, ratio…      │
└────────┴───────────────────────────────────────────────────────────┘
```

The mic EQ has **four bands** on the ZG01, not six. The mockup shown
during brainstorming used six for illustration; the implementation uses
four gain faders plus frequency, Q and shape controls per band.

There is no writable global mic output level: `Mic Level Status` is
read-only and excluded from ALSA. The per-block output gains
(`Compressor Out Gain`, `Limiter Out Gain`) live in the advanced
expander, not as a fourth fader.

## Visual language

The chosen direction is **C — dark and spacious**, with vertical faders.

| Token | Value |
|---|---|
| Background | `#14161b` |
| Panel | `#1b1e26` |
| Panel border | `#272b35` |
| Sidebar | `#101217` |
| Text primary | `#eef1f6` |
| Text secondary | `#8f96a3` |
| Accent | `#c9b8ff` |
| Accent strong | `#8b6ff0` |
| Success | `#34c759` |
| Warning | `#f0b429` |
| Error | `#f0523b` |

- **Faders**: 3 px track, 15 × 7 px handle, tick marks, tabular value in
  accent above, label below. Drag, scroll, arrow keys; double-click
  resets to the control's default; Shift slows the drag for fine values.
- **Meters**: 7 px vertical bars, green → yellow → red gradient, peak-hold
  line for one second, clip marker. Card meters and PipeWire meters are
  drawn side by side: card meter solid, Linux meter thinner with a `PW`
  badge, so the source is never ambiguous.
- **Toggles**: pill switches, accent when on, muted when off. A toggle is
  never the only indicator of a destructive action; Save and Reset are
  buttons.
- **Panels**: 10 px radius, 1 px border, no heavy shadows. The window
  itself has one soft shadow.
- **Typography**: system UI font; tabular numerals for values; no text
  below 11 px in the real UI (the mockups used smaller sizes only to fit
  the comparison cards).
- **Density**: Mixage is dense; Micro is spacious; Avancé is a table.
- **Dark-first**: v1 forces the dark palette. The light theme is a
  follow-up and must reuse the same tokens.

### States

| State | Presentation |
|---|---|
| Card present, cache loaded | Green dot, "ZG01" in the header |
| Card absent | Connection view with `lsusb`, `modprobe`, `journalctl` hints |
| Cache values (normal) | Footer badge "Valeurs du driver (cache)" with a tooltip |
| Cache invalidated by Reset | Greyed values, banner "État inconnu — chargez un preset", Save disabled |
| Preset applying | Progress row with applied/skipped counters, cancel |
| Write failure | Toast with the control name and errno; the slider reverts |
| Disconnected while running | Controls disabled, reconnect banner |

## Product behavior

### Startup

1. Discover the ALSA card whose driver is `zg01` (same logic as
   `zgctl`).
2. Read every control into the model. This seeds the UI with the driver
   cache: built-in defaults after a module load, or the last values
   written since.
3. Show **Mixage**. The footer badge explains the cache.
4. If no card is found, show the connection view. Poll or watch for the
   card every two seconds, and reconnect automatically.

### Writing values

- Slider and toggle changes are written live, like `amixer cset`.
- Writes go through a dedicated worker thread, never the GTK main loop.
  A USB control frame can block; the UI must stay responsive.
- Changes are coalesced per control at ~30 Hz during a drag, and the
  final value is always written on release.
- The model updates optimistically. If the write fails, the previous
  value is restored and a toast reports the error.
- A write that returns success updates the driver cache; the UI's value
  and the cache then agree.

### Presets

- **Charger** opens a file chooser filtered to `.zgpreset`. Backups
  (`.zg01`) are not load targets: drag-and-drop or a selected backup
  reports "sauvegarde non prise en charge" without invoking the parser.
- The parser produces a preview: block type, display name, number of
  parameters, target control per address, skipped addresses, raw
  addresses, and out-of-range values. Nothing is written until the user
  confirms.
- Raw controls (`provenance: "raw"` in the control map, 53 addresses)
  are excluded unless the user ticks **Inclure les paramètres bruts**.
  This matches `zgctl preset load --allow-raw`. Vendor-model controls
  whose names end in `Steps` are not raw: they are step-valued controls
  with a known model, and they load normally.
- Applying writes each supported value through the same control writer.
  It reports applied, skipped and failed entries. It does not persist.
- `Sauvegarder sur le ZG01` writes the `Save to ZG01` action control.
  `Réinitialiser depuis le ZG01` writes `Reset to ZG01`.
- **Reset invalidates the displayed values.** `zg01_param_reset()` does
  not update `dev->param_cache`, so after the action the cache no longer
  describes the device. The GUI marks every writable control unknown,
  greys the values and disables Save until a preset is loaded or a
  control is written again. The driver is unchanged; a future
  `Cache Valid` status control could replace this local invalidation.
- Exporting a `.zgpreset` is out of scope for v1. The preset bar shows
  the loaded file name only.

The GUI depends on the C preset parser built by the full-parameter
surface plan. The parser is extracted into a small C library and called
through a thin FFI wrapper; the Python reference remains a test oracle.
If the full-surface work slips, the GUI ships without presets and
without the Avancé table, and those become the next milestone. It does
not grow a second parser.

### Advanced table

- Enumerates every ALSA control not claimed by a curated screen.
- Columns: name, value, type, range, unit, address (when known).
- Search and filter by screen/group; `provenance: "raw"` controls carry
  a warning. `Steps` in a vendor-model name is a unit hint, not a raw
  marker.
- Editing uses the same widgets and writer as the curated screens.
- This is the escape hatch for the 375-parameter surface; it is not a
  replacement for the curated screens.

### Disconnect and reconnect

- On disconnect: disable inputs, keep the last values visible, show a
  banner.
- On reconnect: re-enumerate controls and read the cache, which resets to
  defaults after a module reload. Offer **Réappliquer le dernier preset**
  if one was applied in this session; do not apply automatically.
- Never crash or hang on `ENODEV`/`EIO`.

### Compact meters mode

- A header action, **Meters**, hides the settings and shrinks the window
  to a narrow meter strip (the three sends, the stream output meter and
  the Micro meter pair).
- On compositors that support wlr-layer-shell (Hyprland), the strip can
  be pinned above other windows via `gtk4-layer-shell`.
- On GNOME and other compositors without layer-shell, it remains a normal
  window; the UI explains that always-on-top is a compositor feature.
- v1.1.

### Keyboard and accessibility

- `Ctrl+O` load preset, `Ctrl+S` save to device, `Ctrl+M` compact meters,
  `Ctrl+Q` quit, arrows adjust the focused fader, `Home`/`End` go to the
  control's bounds.
- Every custom fader and meter exposes an accessible name and value.
- Focus order follows the visual order; the sidebar is keyboard
  navigable.
- Colour is never the only indicator: clip has a label, mute has a
  slash, errors have text.

## Architecture

### Component view

```
┌─────────────────────── zg01-control (Rust) ───────────────────────┐
│  relm4 / GTK4 UI                                                  │
│    │ view models (controls, meters, device state)                  │
│    ├── zg01-core                                                   │
│    │     ├── alsa: card discovery, control enumerate/read/write    │
│    │     ├── model: controls, units, layout table                  │
│    │     ├── preset: FFI → libzgpreset (C)                         │
│    │     ├── meters: poll the driver's meter controls              │
│    │     └── pipewire (feature): peak levels of Linux streams      │
│    └── workers: control writer, meter poller, device watcher       │
└────────────────────────────────────────────────────────────────────┘
        │ ALSA ctl                    │ optional PipeWire
        ▼                             ▼
   snd-zg01 kernel module        PipeWire graph
        │ bulk 0x83 meters
        ▼
      ZG01 hardware
```

### Rust workspace

```
gui/
  Cargo.toml                 workspace
  crates/
    zg01-core/               library: ALSA, model, presets, meters
    zg01-gui/                binary: relm4 app, widgets, screens
  data/
    io.github.bsauvajon.Zg01Control.desktop
    io.github.bsauvajon.Zg01Control.metainfo.xml
    icons/…
    layout.toml              curated control → screen/widget mapping
  po/                        gettext translations
  Makefile                   build, test, install
```

- **Toolkit**: `gtk4-rs` + `libadwaita` + `relm4`. The custom fader and
  meter are `gtk::DrawingArea` subclasses with Cairo drawing, gestures
  and accessible roles.
- **ALSA**: the `alsa` crate for control enumeration and ioctls; raw
  `libc` ioctls only where the crate lacks a binding.
- **Threads**: `std::thread` + `glib` channels. No async runtime is
  required; the workload is small and periodic.
- **Logging**: `tracing` to stderr, visible in `journalctl`.
- **Config**: `~/.config/zg01-control/config.toml` for window geometry,
  last preset directory, recent presets, compact-mode preference and
  PipeWire meters on/off. Values live in the driver cache, not in this
  file.
- **Single instance**: `GtkApplication` application ID
  `io.github.bsauvajon.Zg01Control`.

### Control model

Each ALSA control becomes a `Control`:

```rust
struct Control {
    name: String,        // stable ALSA name
    kind: ControlKind,   // Bool, Integer, Bytes
    min: i64,
    max: i64,
    step: i64,
    unit: Unit,          // from layout.toml: Db, Hz, Q, Steps, Percent, Bool
    value: ControlValue, // Known(i64) or Unknown; same convention as zgctl
    writable: bool,
}
```

- Units and conversions match `zgctl` exactly: gain in 0.1 dB, frequency
  in Hz, Q in 0.01, raw steps and percent as integers. The GUI must not
  invent a second conversion table.
- The layout table maps control names to screens, groups, labels and
  widget types. Unknown controls fall through to Avancé.
- The table is committed and reviewed; it is not generated from the
  vendor model at runtime.

### Layout table

`gui/data/layout.toml`, compiled into the binary:

```toml
[[screen]]
id = "mic"
title = "Micro"

[[screen.group]]
id = "gate"
title = "GATE"
controls = [
  { name = "Gate Capture Switch", widget = "toggle" },
  { name = "Gate", widget = "fader", label = "Seuil", unit = "steps" },
]
```

Rules:

- A missing control is hidden, not an error. This lets the GUI land
  before the full 375-control surface and degrade gracefully.
- A control claimed by a curated screen never appears in Avancé.
- The table is validated by a test: every referenced name must exist in
  the generated control map (when present) and be unique.

### Meter path

The driver gains a background reader on bulk IN `0x83`:

- Submit a chain of bulk IN URBs (512-byte buffers) at probe, after the
  existing initialization.
- Parse every completed frame. Meter frames are identified by their
  header (`04 f0 43 10 | 04 3e 14 02`) and are 192 bytes; other frames
  are ignored for now.
- Update an array of atomic meter values. The frame rate is the device's
  ~20 Hz; no interpolation in the driver.
- Register read-only, volatile integer kcontrols for the decoded meters:
  `Meter Mic In`, `Meter Mic Out`, `Meter PC In`, `Meter Stream Mix`.
  `Meter Game Mix` and `Meter HDMI In` are reserved names, registered
  only when their fields are decoded; the GUI hides them until then.
- The exact range/unit is fixed by the decode phase. The control names
  are stable even if the unit changes.
- The reader is started at probe and cancelled at disconnect. It must not
  race with control writes: writes use `0x03`, reads use `0x83`.
  `zg01_param_read` currently has no callers; if it gains one, the reader
  must demux replies instead of discarding them.
- Verify that `alsactl store`/`restore` ignores these controls (read-only,
  volatile). If it does not, add the appropriate access flags or a
  documented exclusion.

**Mixage meter wiring (to validate in Phase 0).** The vendor map names six
meters; the strips use the following mapping, and any mismatch found
during decoding is corrected in the layout table rather than hidden:

| Strip | Card meter | Linux meter (optional, PipeWire) |
|---|---|---|
| MIC | `Meter Mic In` | `Voice In` source |
| VOICE | `Meter PC In` | `Voice Out` sink |
| GAME | — (deferred; `Meter Game Mix` not separable) | `Game Out` sink (v1.1) |
| STREAM OUT | `Meter Stream Mix` | none |
| Micro detail | `Meter Mic In`, `Meter Mic Out` | `Voice In` |

`Meter HDMI In` is reserved and not registered in v1. `Meter Game Mix`
has no separable game-only tap in the decoded frame: the stream mix
covers both game and voice, so the GAME card meter is deferred to v1.1
(PipeWire `Game Out`) or a later frame decode. The GUI shows the GAME
strip without a card meter until then.

Userspace polls the registered meter controls (four in v1) at 20 Hz in a
worker thread. This is negligible. The UI receives snapshots and redraws.
Peak hold and clip are computed in the GUI, not the driver.

### PipeWire levels (optional feature)

Linux-side levels are a "should" for v1, behind a `pipewire` cargo
feature:

- Capture the monitor ports of the `Game Out` and `Voice Out` sinks and
  the `Voice In` source with `pipewire-rs` streams (capture-sink for
  sinks, target object by node name).
- Compute peak per buffer, aggregate to ~20 Hz, send to the UI.
- If PipeWire is absent or the feature is disabled, hide the Linux
  meters; the card meters still work.
- The node names come from the UCM profile (`Game Out`, `Voice Out`,
  `Voice In`); match by `node.name`/`node.description` with a
  configurable fallback.
- This path never touches the driver and adds no kernel ABI.

### Preset path

- The C parser from the full-surface plan is built as `libzgpreset`
  (static) and linked into `zg01-core` with a `build.rs` + `cc`.
- The FFI surface is deliberately small:
  `zgpreset_open`, `zgpreset_block`, `zgpreset_name`,
  `zgpreset_count`, `zgpreset_param`, `zgpreset_error`,
  `zgpreset_free`.
- Address → control mapping comes from the generated control map in the
  same plan. The GUI does not parse `device-map-zg01.json` at runtime.
- The GUI applies values through the normal control writer, so live
  writes, throttling and error reporting are shared with the sliders.
- The Python parser remains the test oracle; the Rust side never
  reimplements MBDF.

## Kernel meter decoding

This is a prerequisite, tracked as Phase 0, and it is reverse
engineering, not UI work.

### Questions to answer

- Which bytes in the 192-byte frame carry which meter?
- What is the scale: 0–100, 0–127, dB × 10, or a device curve?
- Are there separate frames for different meter groups, or one frame
  with a fixed layout?
- Is the frame rate stable at ~20 Hz, and does it continue when no
  Windows application is connected?
- Do the meter frames interfere with control replies on `0x83`?

### Method

1. Use the existing captures in `captures/` (mic baseline, EQ, state
   read) and a fresh usbmon capture while the Linux driver is loaded.
2. Drive one meter at a time with a known signal: a sine at −60, −40,
   −20, −6 and 0 dBFS on the mic input; a known game/voice signal on the
   USB inputs; HDMI if available.
3. Correlate changing byte offsets with the known levels. Test both
   endiannesses and 1- or 2-byte fields.
4. Use the ZG Controller meters (MIC LEVEL, OUT LEVEL, mixer meters) as
   the visual reference while capturing.
5. Build `tools/decode-meters.py` as the reference decoder, with
   fixtures in `tests/fixtures/meters/`.
6. Document the result in `docs/METER_PROTOCOL.md` before writing the
   C parser.

**Done when**: the decoder prints values that track the hardware meters
within a stated tolerance for at least MIC IN, OUT, PC IN and STREAM, and
the mapping is documented and covered by a fixture. GAME and HDMI are
documented as undecoded, not fabricated.

If the decode stalls (for example, the frames turn out to be obfuscated
or to require a handshake), the GUI still ships with the settings half;
the meter panel shows "meters indisponibles" rather than invented data.
That is a product decision, not a silent fallback.

## Screen details

### Mixage

- **Three send strips** (MIC, VOICE, GAME). Each strip contains:
  - channel name and source subtitle (`MIC`, `VOICE · USB`, `GAME · HDMI/USB`);
  - card meter where decoded (MIC, VOICE) and optional Linux meter;
  - vertical fader over the 0–127 step range, with the step value;
  - channel state tags (`ON`, `MUTE`, `FX`);
  - for MIC, a chain badge `GATE › COMP › EQ › LIMIT`; clicking the
    strip opens Micro.
  GAME has no card meter in v1; its Linux `Game Out` meter arrives in
  v1.1.
- **Stream output block** (STREAM OUT): `Streaming USB Out`,
  `Streaming HDMI Out` and `Streaming Monitor Steps` as toggles, plus
  the `Meter Stream Mix` meter. No fader. `Streaming Out Mute Steps` is
  raw and stays in Avancé.
- The preset bar shows the loaded preset name, **Charger**, **Récent**.
- Header actions: **Meters** (compact mode), **Sauvegarder** (Save to
  ZG01).
- A small "format" indicator can show the incoming USB channel count as
  a successor to the Sound Check idea. It is a follow-up; it needs a
  reliable source for the format and is not in v1.

### Micro

- Three vertical faders: GATE, COMP, LIMIT. The read-only `Mic Level
  Status` is not a fader; `Compressor Out Gain` and `Limiter Out Gain`
  are in the advanced expander.
- Each block has an enable toggle, the main value, and a reset-to-default
  affordance.
- EQ: four vertical gain faders (bands 1–4), with frequency, Q and
  low/high shelf shape in a compact editor row. The EQ switch is a
  toggle. Preset names are shown when the raw preset control is decoded;
  otherwise the editor is "Custom".
- **Avancé** expander per block: out gain, threshold, attack, release,
  ratio, knee, range, hold, decay, as exposed by the full surface. Until
  those controls exist, the expander is hidden.
- Meters: MIC IN (card) and OUT (card) flank the chain, as in C1.

### Jeu, Casque, Réglages (v1.1)

- **Jeu**: three slot selectors, preset names, on/off and level for 3D
  CHAT SPACE, ZG SURROUND and FOCUS MODE/EQ. Deep EQ editing is deferred.
- **Casque**: monitor EQ bands, preset selection, `Monitor Headphone
  Level Steps` and `Monitor Speaker Level Steps`. `Mic Monitor Level
  Knob Status` is read-only and excluded from ALSA; it is shown as
  information only.
- **Réglages**: HDMI select/4K/HDCP, LED brightness, firmware/version
  read-only fields, application preferences (PipeWire meters, compact
  mode, language).
- These screens use the same widgets and layout table; they are not a
  second UI codebase.

### Connection view

- Shown when no `zg01` card exists.
- Text: check `lsusb | grep 0499:1513`, `sudo modprobe snd-zg01`,
  `journalctl -b -k --grep zg01`.
- Auto-reconnect when the card appears.

### About

- Application version, driver version, firmware version (if readable),
  ALSA card index.
- Disclaimer: unofficial, not affiliated with Yamaha; ZG01 is a Yamaha
  trademark.
- License and issue tracker link.

## Phases

### Phase 0 — meter decode

- Capture matrix and decoder.
- `docs/METER_PROTOCOL.md`, fixtures, tests.
- **Done when**: MIC IN, OUT, PC IN and STREAM decode and track the
  hardware within tolerance; GAME and HDMI are documented as undecoded.

### Phase 1 — driver meter controls

- Background `0x83` reader, frame parser, atomic snapshot.
- Six read-only volatile kcontrols.
- `make W=1` clean; no regression in control writes or audio.
- **Done when**: `amixer -c zg01` shows the meters and their values track
  the device; `alsactl` does not try to restore them; suspend/resume and
  replug still work.

### Phase 2 — Rust core

- Workspace, ALSA discovery, control model, layout table, worker threads,
  conversions.
- Unit tests for conversions, layout validation, write coalescing.
- A headless `zg01ctl-rs` example or a `cargo test` integration that
  lists controls on hardware.
- **Done when**: the core can read/write the mic controls and the meter
  controls on a real device, with tests passing in CI without hardware
  (using fixtures/fakes where possible).

### Phase 3 — GUI shell, Mixage, Micro

- relm4 app, window, sidebar, theme, custom fader and meter widgets.
- Mixage with three send strips, the stream output block and the preset
  bar placeholder.
- Micro with GATE/COMP/LIMIT/EQ.
- Connection, disconnect and error states.
- **Done when**: on hardware, the mixer shows live card meters, faders
  change the device, and Micro matches `zgctl` values; the UI stays
  responsive during a blocking write.

### Phase 4 — presets and Avancé

- FFI to `libzgpreset`, preview/apply report, raw opt-in.
- Avancé table with search/filter and type-aware editors.
- **Done when**: a real `.zgpreset` applies with a correct report, does
  not persist until Save, and out-of-range entries are skipped and
  listed; Avancé can edit every writable control.

### Release scope

**v1** ships Phases 0–4 plus Phase 6: meter decoding, driver meter
controls, Rust core, Mixage + Micro, presets + Avancé, packaging. This is
the smallest release that covers both moments (setup and live) and the
preset workflow.

**v1.1** ships Phase 5: Jeu, Casque, Réglages, compact meters and the
PipeWire level feature.

The full-parameter-surface work (`feat/zgpreset-controls`) lands before
the GUI's preset and Avancé phases. The GUI does not grow a second
parser, and those phases are gated on the C parser and control map being
available.

### Phase 5 — remaining screens, compact mode, PipeWire

- Jeu, Casque, Réglages.
- Compact meters mode with layer-shell on Hyprland.
- Optional PipeWire levels.
- **Done when**: the screens are navigable and consistent; the compact
  strip pins on Hyprland and behaves as a normal window elsewhere;
  PipeWire meters can be enabled/disabled without affecting card meters.

### Phase 6 — packaging, docs, release

- Arch and Debian packages, desktop file, metainfo, icon, translations.
- README section, screenshots, keybindings, troubleshooting.
- CI build/test/fmt/clippy; packaging tests.
- **Done when**: a clean install from the package launches the app, finds
  the card and passes the acceptance checklist.

## Testing

- **Rust unit tests**: conversions, layout parsing/validation, preset
  result mapping, raw gating by `provenance`, meter smoothing/peak hold,
  write coalescing, and the Reset → unknown-state transition.
- **Fakes**: an `AlsaBackend` trait with a fake implementation for CI;
  hardware tests are ignored unless `ZG01_TEST_DEVICE=1`.
- **Kernel**: `make W=1`, existing lifecycle tests, a parser fixture
  test, and a manual hardware checklist for meters.
- **GUI**: `cargo test`, `clippy -D warnings`, `fmt --check`; a smoke
  test that constructs the app with `GDK_BACKEND=headless` if the CI
  image supports it. Pixel tests are out of scope.
- **Packaging**: `makepkg` in a clean chroot (manual/CI) and `dpkg-buildpackage`
  in a container; at minimum, the package must install and launch.
- **Hardware acceptance checklist**: mixer meters, mic faders, preset
  apply, save/reset, disconnect/reconnect, suspend/resume, no root.

## Packaging and distribution

### Repository and release model

The GUI lives in the `snd-zg01` monorepo under `gui/`, but ships as its
own package and has its own release train:

- **Source of truth**: one repository for the driver, `zgctl`, the
  `.zgpreset` parser/control map and the GUI. Control names, the parser
  and the generated map cannot drift between repositories.
- **Packages**: `snd-zg01-dkms` (driver), `snd-zg01-tools` (`zgctl`,
  optional) and `snd-zg01-gui` (GUI, optional). Installing the GUI never
  pulls the driver's build dependencies, and installing the tools never
  pulls GTK.
- **Tags**: driver releases use `vX.Y.Z`; GUI releases use
  `gui-vX.Y.Z`. A GUI tag builds and publishes only the GUI artifacts; a
  driver tag builds the driver, tools and packages. The two can ship on
  different dates.
- **CI**: path-filtered workflows. `gui/**` changes run the Rust/GTK
  build and tests; `src/**` changes run the kernel build and lifecycle
  tests; changes to `tools/vendor/**`, the parser or the control map run
  both, because they are the shared contract.
- **Compatibility**: the GUI treats missing controls as hidden (the
  layout table degrades gracefully). The About dialog shows the driver
  version and the GUI version. A future control-map version check can
  warn when the installed driver is older than the GUI expects; v1
  relies on graceful degradation and the preset report.
- **Split later if needed**: if the GUI grows beyond the ZG01 (another
  device, a standalone product with its own community), it can be
  extracted into its own repository with the parser and control map as a
  released library. Splitting later is cheaper than merging two repos
  now.

### Source layout

- `gui/` in the `snd-zg01` repository, built by `gui/Makefile` (`build`,
  `test`, `install`, `uninstall`).
- **Arch**: `zg01-control-git` package, depends `gtk4 libadwaita`,
  makedepends `rust cargo`, optdepends `pipewire`; installs the binary,
  desktop file, metainfo, icon and translations.
- **Debian**: `snd-zg01-gui` binary package. Build-deps
  `cargo rustc libgtk-4-dev libadwaita-1-dev`; the source package vendors
  Rust crates (or the package uses a Flatpak for cross-distro users).
  Document the choice and the fallback.
- **Flatpak**: optional but recommended for non-Debian/Arch users. Needs
  `--device=alsa` (and no USB access beyond ALSA). The app ID matches the
  desktop file.
- **Translations**: gettext; English source strings, French translation
  shipped in v1. The UI must not concatenate translated fragments.
- **License**: GPL-2.0, matching the driver. The icon and screenshots are
  original.
- **Trademark**: "ZG01 Control" is an unofficial companion; the About
  dialog says so.

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Meter bytes/scale resist decoding | No internal meters in v1 | Time-box Phase 0; ship settings-only with an explicit "meters indisponibles" state |
| Cache staleness confuses users | Trust in the UI | Persistent badge, tooltip, docs; never claim a device read |
| Bulk IN reader conflicts with control replies | Control read failures | Keep `zg01_param_read` unused; demux if it gains callers |
| `alsactl` saves meter controls | Restore errors | Read-only + volatile flags; test store/restore |
| Debian Rust packaging | No Debian package | Vendor crates or ship Flatpak; document |
| Full-surface plan slips | No presets/Avancé | Preset and Avancé phases are gated on it; the GUI does not grow a second parser |
| PipeWire level capture is complex | Missing Linux meters | Feature-gated; card meters are the must-have |
| Wayland always-on-top | Compact mode not pinned | layer-shell on Hyprland; normal window elsewhere; explain |
| 375 controls overwhelm Avancé | Poor usability | Search/filter, curated screens, raw warning |
| GTK4 version skew | Build breaks on older distros | Pin minimum versions; test on Arch and Debian stable |
| Trademark/artwork | Legal complaint | Original name/icon, disclaimer, no Yamaha assets |

## Acceptance criteria

1. With a ZG01 connected, `zg01-control` opens on **Mixage**, shows live
   card meters for MIC IN, OUT, PC IN and STREAM, and the faders change
   the stream mix audibly and through `amixer`/`zgctl`. GAME has no card
   meter in v1; its strip uses PipeWire `Game Out` in v1.1.
2. Clicking the MIC strip opens **Micro**; GATE, COMP, LIMITER and the
   four EQ bands write through and read back the same values as `zgctl`.
3. Loading a real `.zgpreset` previews the mapping, applies supported
   values, reports skipped/out-of-range entries, and does not persist
   until **Sauvegarder sur le ZG01**.
4. **Sauvegarder** and **Réinitialiser** perform the same actions as
   `zgctl save`/`zgctl reset`.
5. Every writable control is reachable either on a curated screen or in
   **Avancé**.
6. Unplugging and replugging the device does not crash or hang the app;
   the UI reconnects and re-reads the cache.
7. The UI states that values come from the driver cache; no screen
   claims to read the device state.
8. The app builds and installs from the Arch and Debian packages, runs
   without root, and passes `cargo test`, `clippy -D warnings`,
   `fmt --check`, and the kernel `make W=1` build.
9. English and French UI strings are available; no Yamaha asset is
   shipped.
10. Compact meters mode works as a normal window everywhere and pins via
    layer-shell on Hyprland (v1.1).

## Decisions made in this session

- Visual direction **C** (dark, spacious, violet accent), with vertical
  faders.
- **Mixage** (C2) is the landing screen; **Micro** (C1) is the detail
  panel opened from the MIC strip.
- No Home screen; no duplication of the physical front panel.
- Rust + GTK4 + libadwaita + relm4.
- Meters are delivered as read-only volatile ALSA controls updated by a
  driver background reader (option A), not a hwdep stream.
- Settings use the driver cache and presets as the source of truth; the
  `F0 43 20` state dump is out of scope.
- Linux-side levels come from PipeWire, not the driver.
- Public GitHub project, packaged with the driver.
- **Repository model B**: source in the `snd-zg01` monorepo under `gui/`,
  separate `snd-zg01-gui` package, independent `gui-vX.Y.Z` tags and
  path-filtered CI.

### Resolutions from the builder check

These were found by comparing the design with
`tools/vendor/control-map-zg01.json` on `feat/zgpreset-controls`:

- **Mixage has three send strips, not five.** MIC, VOICE and GAME map to
  the `Streaming * Level Steps` controls. There is no streaming-output
  level in the device model, so STREAM OUT is an output block with
  switches and a meter, not a fader. The monitor path has four
  overlapping level controls and no meter; it moves to Casque (v1.1).
- **Stream and monitor levels are 0–127 steps, not dB.** The UI shows the
  step value and a 0–127 scale. Only `... Gain` controls use the
  established 0.1 dB conversion.
- **Micro has three faders.** `Mic Level Status` is read-only and absent
  from ALSA; the per-block out gains live in the advanced expander.
- **"Raw" means `provenance: "raw"` (53 addresses),** matching
  `zgctl --allow-raw`. A `Steps` suffix in a vendor-model name is not a
  raw marker (94 controls are in that case).
- **Reset invalidates the displayed values.** The driver does not update
  the cache on `zg01_param_reset()`; the GUI marks every writable control
  unknown and disables Save until a preset is loaded or a control is
  written.
- **Only `.zgpreset` is a load target.** `.zg01` backups are filtered out
  and rejected with a clear message; the parser is not extended.
- **The GUI ships as its own package** (`snd-zg01-gui` / Arch
  `zg01-control-git`), so `snd-zg01-tools` users do not pull GTK.

## Confirmed decisions

The human partner confirmed these after the builder check:

1. **Mixage set and units.** Three send faders (MIC, VOICE, GAME) plus
   the STREAM OUT switch/meter block; no monitor strip; values shown as
   0–127 steps, not an invented dB or percentage scale.
2. **First release scope.** v1 = Phases 0–4 + 6. Jeu, Casque, Réglages,
   compact meters and PipeWire levels ship in v1.1.
3. **Full-surface sequencing.** `feat/zgpreset-controls` lands before the
   GUI's preset and Avancé phases; the GUI does not grow a second parser.
4. **Dark-first.** v1 is dark-only; the light token set is a follow-up.
5. **Repository model B.** The GUI source stays in the `snd-zg01`
   monorepo under `gui/`, ships as a separate `snd-zg01-gui` package,
   and releases on its own `gui-vX.Y.Z` tags with path-filtered CI.
6. **Meter scope v1.** Register MIC IN, MIC OUT, PC IN and STREAM. GAME
   has no separable card tap; it uses PipeWire `Game Out` in v1.1. HDMI
   is deferred until a source is available.
