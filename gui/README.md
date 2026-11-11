# ZG01 Control GUI

GTK4 / libadwaita control GUI for the Yamaha ZG01, built on
[relm4](https://relm4.org). This directory is a self-contained Cargo workspace;
it is independent from the kernel driver and the C tools that live in the
repository root.

## Layout

```
gui/
├── Cargo.toml            # workspace root
├── Cargo.lock            # committed, so builds are reproducible
├── crates/
│   ├── zg01-core/        # device, model and preset logic (library)
│   └── zg01-gui/         # relm4 application (binary: zg01-control)
├── data/                 # desktop entry, AppStream metadata, icon, layout
├── po/                   # gettext catalogue (English source, French ship)
└── Makefile
```

## Requirements

- Rust 1.93 or newer (the workspace is developed against 1.93.1)
- GTK 4.22 development files (`libgtk-4-dev`)
- libadwaita 1.9 development files (`libadwaita-1-dev`)
- libasound development files (`libasound2-dev`) for `zg01-core`
- `pkg-config`
- GNU `gettext` (`msgfmt`, `xgettext`, `msgmerge`) for the shipped French
  catalogue

On Debian/Ubuntu:

```sh
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev \
    libasound2-dev gettext
```

`cargo clippy` and `cargo fmt` need the matching toolchain components
(`rust-clippy` and `rustfmt` packages on Ubuntu, `rustup component add clippy
rustfmt` otherwise).

## Building and testing

```sh
make -C gui build   # cargo build --workspace --release
make -C gui test    # msgfmt --check + cargo test + clippy + fmt --check
make -C gui locale  # compile po/fr.po into target/locale/fr/LC_MESSAGES
make -C gui install DESTDIR=/tmp/stage PREFIX=/usr
make -C gui uninstall DESTDIR=/tmp/stage PREFIX=/usr
```

`install` places the binary, desktop entry, AppStream metadata, hicolor icon
and `zg01-control.mo` under `$(DESTDIR)$(PREFIX)`. `DESTDIR` and `PREFIX` are
independent: the default is `PREFIX=/usr/local` with no staging.

The binary is produced at `gui/target/release/zg01-control`.

```sh
cd gui
cargo run -p zg01-gui
```

The application ID is `io.github.bsauvajon.Zg01Control` and the main window is
titled `ZG01 Control`.

## Crate versions

Versions are pinned in `Cargo.lock`; the manifest uses caret requirements.

| Crate           | Version | Why                                                                       |
| --------------- | ------- | ------------------------------------------------------------------------- |
| `gtk4`          | 0.11.5  | Current gtk4-rs; MSRV 1.92, works with GTK 4.22.                           |
| `libadwaita`    | 0.9.2   | Matches the system libadwaita 1.9.1; MSRV compatible with Rust 1.93.      |
| `relm4`         | 0.11.0  | Current relm4; MSRV is exactly 1.93, which this toolchain satisfies.     |
| `alsa`          | 0.12.1  | Current ALSA bindings used by `zg01-core`.                                 |
| `alsa-sys`      | 0.6.1   | FFI bindings the `alsa` crate is built on, used for the element metadata.  |
| `serde`         | 1.0.229 | With the `derive` feature, for preset serialisation.                      |
| `toml`          | 1.1.8   | Current `toml` release, used by the `.zgpreset` parser.                    |
| `thiserror`     | 2.0.21  | Current major version for the error types.                                 |
| `tracing`       | 0.1.44  | Structured logging across the workspace.                                   |
| `tracing-subscriber` | 0.3.23 | Subscriber used by the GUI binary.                                   |
| `cc`            | 1.10.0  | Build dependency of `zg01-core`, reserved for the native helpers.          |

`relm4` is used with the `libadwaita` and `gnome_50` features: `gnome_50`
selects the GTK 4.22 / libadwaita 1.9 API surface, matching the system
libraries. The `libadwaita` feature makes relm4 create an `adw::Application`
and initialise libadwaita, so the window is an `adw::ApplicationWindow`.

## Packaging and translations

The GUI is the separate `snd-zg01-gui` (Debian) / `zg01-control-git` (Arch)
package. The install rules are the contract the CI packaging job and both
package recipes use.

User-facing strings are English in the source and translated through gettext.
`po/fr.po` is the shipped French catalogue; `po/POTFILES` lists the files
that carry messages. `make -C gui update-po` runs `xgettext` over those files
into `po/zg01-control.pot`, merges it into `fr.po` with `msgmerge`, and runs
`msgfmt --check`; fill new `msgstr` entries, then run `make -C gui check-po`
and `make -C gui locale` before running from the source tree with
`LANG=fr_FR.UTF-8`. `ZG01_LOCALEDIR` points the binary at another catalogue
directory (`<dir>/fr/LC_MESSAGES/zg01-control.mo`).

## Status

The v1 settings surface is complete: Mixage, Micro, Avancé, the preset
loader, Save/Reset, disconnect/reconnect, the custom fader and meter
widgets, the French translation and the packaging. The hardware
acceptance checklist lives in `../docs/GUI_TEST.md`.

**MIC state readback.** The driver reads the MIC state dump at probe; the
GUI shows the last successful read and offers **Read from ZG01** for a fresh
one. It displays the persisted gate, compressor, limiter and direct EQ
fields. The Mixage section still comes from the driver cache; the
DSP-transformed MIC fields (gate threshold/attack/hold/decay,
compressor/limiter attack/release/out gain, EQ gain/frequency) are shown
unknown. See `../docs/STATE_DUMP_PROTOCOL.md` and
`../docs/superpowers/specs/2026-11-08-zg01-mic-readback.md`.

**Live-meter acceptance is blocked for the card meters.** The decode and
the meter controls are implemented, and earlier captures showed live
values, but the latest hardware session returns only zero-length meter
frames and `-ENODATA` after a suspend, even after a power cycle. On that
device the GUI draws the four card meters dashed/unknown instead of a fake
zero. To keep the interface usable, the GUI also shows thin `PW` bars from
PipeWire: `Voice In` for MIC, `Voice Out` for VOICE and `Game Out` for
GAME; STREAM OUT has no Linux source. The card bars stay dashed until EP
`0x83` returns. See `../docs/METER_TEST.md` and
`../docs/METER_PROTOCOL.md`.

**Known limitations.** The Mixage faders and STREAM OUT switches configure
the device's **STREAMING OUTPUT MIXER**. The driver exposes only `Voice In`,
not the separate USB **Streaming** capture device, so those controls drive
neither the local monitor nor the OBS sources: mix `Voice In` + `Game Out` /
`Voice Out` in OBS, or capture the HDMI OUT. The local monitor is on the
physical knobs and the `Monitor *` controls (Avancé). The **Jeu**,
**Casque** and **Réglages** screens, compact/pinned meters, a PipeWire
enable/disable setting and persisted GUI preferences are v1.1 work; their
controls stay in Avancé where the driver exposes them. MIC readback covers
only the direct state-dump fields; transformed MIC fields and non-MIC
sections remain cached. Presets apply only to the controls the driver
publishes, and `.zg01` backups are rejected. The window is a snapshot: it
does not live-refresh after external ALSA writes.
