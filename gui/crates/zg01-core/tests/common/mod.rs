//! Helpers shared by the integration tests of `zg01-core`.
//!
//! Every test file starts with `mod common;`.

// The helpers are not used by every test file, and an unused one is not a
// reason to fail the build.
#![allow(dead_code)]

use zg01_core::alsa::{ControlInfo, ControlKind};
use zg01_core::model::{Control, ControlValue, Unit};

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
