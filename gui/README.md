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
└── Makefile
```

## Requirements

- Rust 1.93 or newer (the workspace is developed against 1.93.1)
- GTK 4.22 development files (`libgtk-4-dev`)
- libadwaita 1.9 development files (`libadwaita-1-dev`)
- libasound development files (`libasound2-dev`) for `zg01-core`
- `pkg-config`

On Debian/Ubuntu:

```sh
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev \
    libasound2-dev
```

`cargo clippy` and `cargo fmt` need the matching toolchain components
(`rust-clippy` and `rustfmt` packages on Ubuntu, `rustup component add clippy
rustfmt` otherwise).

## Building and testing

```sh
make -C gui build   # cargo build --workspace --release
make -C gui test    # cargo test + clippy -D warnings + fmt --check
```

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

## Status

Task 0 delivers the workspace and an application skeleton only: a single
`ZG01 Control` window (1100x680) with a placeholder label. The ALSA control
surface, the ZG01 model screens and the `.zgpreset` handling are added by the
follow-up tasks.
