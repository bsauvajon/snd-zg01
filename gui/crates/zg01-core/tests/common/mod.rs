//! Helpers shared by the integration tests of `zg01-core`.
//!
//! Every test file starts with `mod common;`.

// The helpers are not used by every test file, and an unused one is not a
// reason to fail the build.
#![allow(dead_code)]

use zg01_core::alsa::{CardInfo, ControlInfo, ControlKind};
use zg01_core::layout::Layout;
use zg01_core::model::{AppState, Control, ControlValue, DeviceState, Unit};

/// A writable integer control with the given name and range.
pub fn control(name: &str, min: i64, max: i64) -> Control {
    Control {
        info: ControlInfo {
            name: name.to_string(),
            kind: ControlKind::Integer,
            min,
            max,
            step: 0,
            writable: true,
        },
        value: ControlValue::Known(0),
        unit: Unit::Steps,
    }
}

/// One control per name, with a `0..100` range.
pub fn controls(names: &[&str]) -> Vec<Control> {
    names.iter().map(|name| control(name, 0, 100)).collect()
}

/// The controls the Mixage screen claims: the three send levels, the
/// three mutes, the two per-send FX switches and the stream outputs.
pub fn controls_with_mixage() -> Vec<Control> {
    controls(&[
        "Streaming Mic Level Steps",
        "Streaming Voice Level Steps",
        "Streaming Game Level Steps",
        "Streaming Mic Mute Steps",
        "Streaming Voice Mute Steps",
        "Streaming Game Mute Steps",
        "Streaming Voice FX Steps",
        "Streaming Game FX Steps",
        "Streaming USB Out",
        "Streaming HDMI Out",
        "Streaming Monitor Steps",
    ])
}

/// The card the tests pretend to be talking to.
pub fn card() -> CardInfo {
    CardInfo {
        index: 2,
        id: "zg01".to_string(),
        driver: "zg01_usb".to_string(),
    }
}

/// A connected state that holds one known control per name.
pub fn state_with(names: &[&str]) -> AppState {
    AppState {
        device: DeviceState::Connected(card()),
        controls: controls(names),
        layout: Layout {
            screens: Vec::new(),
        },
    }
}
