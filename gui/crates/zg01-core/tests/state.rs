//! The application state: what the shell holds and when saving is allowed.

mod common;

use common::state_with;
use zg01_core::model::{ControlValue, DeviceState};

#[test]
fn reset_marks_values_unknown_and_blocks_save() {
    let mut state = state_with(&["Gate"]);
    assert!(state.can_save());
    state.mark_all_unknown();
    assert!(!state.can_save());
    assert!(state
        .controls
        .iter()
        .all(|c| c.value == ControlValue::Unknown));
}

#[test]
fn disconnect_blocks_save() {
    let mut state = state_with(&["Gate"]);
    state.device = DeviceState::Disconnected;
    assert!(!state.can_save());
}

#[test]
fn an_unknown_read_only_value_does_not_block_save() {
    let mut state = state_with(&["Gate"]);
    let mut meter = common::control("Meter Mic In", 0, 127);
    meter.info.writable = false;
    meter.value = ControlValue::Unknown;
    state.controls.push(meter);

    assert!(state.can_save());
}

#[test]
fn reset_keeps_read_only_values() {
    let mut state = state_with(&["Gate"]);
    let mut meter = common::control("Meter Mic In", 0, 127);
    meter.info.writable = false;
    meter.value = ControlValue::Known(96);
    state.controls.push(meter);

    state.mark_all_unknown();

    let meter = state
        .controls
        .iter()
        .find(|control| control.name() == "Meter Mic In")
        .expect("the meter is still there");
    assert_eq!(meter.value, ControlValue::Known(96));
}

#[test]
fn a_connected_card_is_the_device() {
    let state = state_with(&["Gate"]);
    assert!(state.device.is_connected());
    assert_eq!(
        state.device.card().map(|card| card.id.as_str()),
        Some("zg01")
    );
}
