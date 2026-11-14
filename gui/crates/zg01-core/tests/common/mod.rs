//! Helpers shared by the integration tests of `zg01-core`.
//!
//! Every test file starts with `mod common;`.

// The helpers are not used by every test file, and an unused one is not a
// reason to fail the build.
#![allow(dead_code)]

use zg01_core::alsa::{CardInfo, ControlInfo, ControlKind};
use zg01_core::layout::Layout;
use zg01_core::model::{AppState, Control, ControlValue, DeviceState, StateRead, Unit};

/// A writable integer control with the given name and range.
pub fn control(name: &str, min: i64, max: i64) -> Control {
    Control {
        info: ControlInfo {
            name: name.to_string(),
            kind: ControlKind::Integer,
            min,
            max,
            step: 0,
            count: 1,
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

/// A read-only meter control with its declared number of values.
pub fn meter_control(name: &str, count: u32) -> Control {
    let mut control = control(name, 0, 127);
    control.info.count = count;
    control.info.writable = false;
    control
}

/// The four one-value meters of the legacy driver.
pub fn legacy_meter_controls() -> Vec<Control> {
    [
        "Meter Mic In",
        "Meter Mic Out",
        "Meter PC In",
        "Meter Stream Mix",
    ]
    .into_iter()
    .map(|name| meter_control(name, 1))
    .collect()
}

/// The six meters of the V2 driver.
pub fn v2_meter_controls() -> Vec<Control> {
    vec![
        meter_control("Meter Mic In", 1),
        meter_control("Meter Mic Out", 1),
        meter_control("Meter PC In", 8),
        meter_control("Meter HDMI In", 8),
        meter_control("Meter Game Mix", 8),
        meter_control("Meter Stream Mix", 8),
    ]
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

/// The controls the Micro screen claims: the three block switches and
/// values, the EQ switch and shapes, the four gain/frequency/Q triples and
/// every dynamics sub-parameter.
pub fn controls_with_mic() -> Vec<Control> {
    controls(&[
        "Gate Capture Switch",
        "Gate",
        "Gate Threshold",
        "Gate Range",
        "Gate Attack",
        "Gate Hold",
        "Gate Decay",
        "Compressor Capture Switch",
        "Compressor",
        "Compressor Threshold",
        "Compressor Ratio",
        "Compressor Attack",
        "Compressor Release",
        "Compressor Knee",
        "Compressor Out Gain",
        "Limiter Capture Switch",
        "Limiter",
        "Limiter Threshold",
        "Limiter Attack",
        "Limiter Release",
        "Limiter Knee",
        "Limiter Out Gain",
        "EQ Capture Switch",
        "EQ Low Shape",
        "EQ High Shape",
        "EQ Band 1 Gain",
        "EQ Band 1 Frequency",
        "EQ Band 1 Q",
        "EQ Band 2 Gain",
        "EQ Band 2 Frequency",
        "EQ Band 2 Q",
        "EQ Band 3 Gain",
        "EQ Band 3 Frequency",
        "EQ Band 3 Q",
        "EQ Band 4 Gain",
        "EQ Band 4 Frequency",
        "EQ Band 4 Q",
    ])
}

/// Every control of the generated map, one per name, in map order.
///
/// The range and the writability come from the map, so the Avancé tests
/// see the same surface the application reads from the driver.
pub fn all_controls() -> Vec<Control> {
    zg01_core::preset::control_map()
        .iter()
        .map(|entry| Control {
            info: ControlInfo {
                name: entry.control_name.clone(),
                kind: ControlKind::Integer,
                min: entry.min,
                max: entry.max,
                step: 0,
                count: 1,
                writable: entry.writable,
            },
            value: ControlValue::Known(0),
            unit: Unit::Steps,
        })
        .collect()
}

/// The card the tests pretend to be talking to.
pub fn card() -> CardInfo {
    CardInfo {
        index: 2,
        id: "zg01".to_string(),
        driver: "zg01_usb".to_string(),
        generation: None,
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
        state_read: StateRead::Unknown,
    }
}
