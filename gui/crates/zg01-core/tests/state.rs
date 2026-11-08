//! The application state: what the shell holds and when saving is allowed.

mod common;

use common::{control, controls_with_mic, state_with};
use zg01_core::model::{ControlValue, DeviceState, StateRead, STATE_READ_KNOWN, STATE_READ_STATUS};

fn readback_control(name: &str, value: i64, writable: bool) -> zg01_core::model::Control {
    let mut control = control(name, 0, 100);
    control.info.writable = writable;
    control.value = ControlValue::Known(value);
    control
}

#[test]
fn a_successful_read_marks_direct_fields_and_leaves_transformed_unknown() {
    let mut state = state_with(&[]);
    state.controls = controls_with_mic();
    state.control_mut("Gate").expect("Gate is present").value = ControlValue::Known(60);
    state
        .control_mut("Gate Threshold")
        .expect("Gate Threshold is present")
        .value = ControlValue::Known(50);
    state
        .controls
        .push(readback_control(STATE_READ_STATUS, 1, false));
    state
        .controls
        .push(readback_control(STATE_READ_KNOWN, 0x3fffc7, false));

    state.apply_readback();

    assert_eq!(state.state_read, StateRead::Known { known: 0x3fffc7 });
    assert!(state.can_save());
    assert_eq!(
        state.control("Gate").map(|control| control.value),
        Some(ControlValue::Known(60))
    );
    assert_eq!(
        state.control("Gate Threshold").map(|control| control.value),
        Some(ControlValue::Unknown)
    );
    assert_eq!(
        state.control("Gate Attack").map(|control| control.value),
        Some(ControlValue::Unknown)
    );
    assert_eq!(
        state
            .control("Compressor Attack")
            .map(|control| control.value),
        Some(ControlValue::Unknown)
    );
    assert_eq!(
        state
            .control("Compressor Threshold")
            .map(|control| control.value),
        Some(ControlValue::Known(0))
    );
    assert_eq!(
        state.control("EQ Band 1 Q").map(|control| control.value),
        Some(ControlValue::Known(0))
    );
    assert_eq!(
        state.control("EQ Band 1 Gain").map(|control| control.value),
        Some(ControlValue::Unknown)
    );
}

#[test]
fn a_partial_known_mask_leaves_the_missing_direct_field_unknown() {
    let mut state = state_with(&[]);
    state.controls = controls_with_mic();
    state
        .control_mut("Compressor Threshold")
        .expect("Compressor Threshold is present")
        .value = ControlValue::Known(61);
    state
        .controls
        .push(readback_control(STATE_READ_STATUS, 1, false));
    state.controls.push(readback_control(
        STATE_READ_KNOWN,
        0x3fffc7 & !(1 << 8),
        false,
    ));

    state.apply_readback();

    assert_eq!(
        state
            .control("Compressor Threshold")
            .map(|control| control.value),
        Some(ControlValue::Unknown)
    );
    assert_eq!(
        state.control("Gate").map(|control| control.value),
        Some(ControlValue::Known(0))
    );
    assert_eq!(
        state.control("EQ Band 1 Q").map(|control| control.value),
        Some(ControlValue::Known(0))
    );
    assert!(state.can_save());
}

#[test]
fn a_failed_read_marks_the_mic_chain_unknown_and_blocks_save() {
    let mut state = state_with(&[]);
    state.controls = controls_with_mic();
    state
        .controls
        .push(readback_control(STATE_READ_STATUS, -110, false));
    state
        .controls
        .push(readback_control(STATE_READ_KNOWN, 0, false));

    state.apply_readback();

    assert_eq!(state.state_read, StateRead::Failed);
    assert!(!state.can_save());
    for name in ["Gate", "Gate Threshold", "EQ Band 1 Q", "EQ Band 1 Gain"] {
        assert_eq!(
            state.control(name).map(|control| control.value),
            Some(ControlValue::Unknown),
            "{name}"
        );
    }
}

#[test]
fn a_pending_read_keeps_save_disabled() {
    let mut state = state_with(&[]);
    state.controls = controls_with_mic();
    state
        .controls
        .push(readback_control(STATE_READ_STATUS, 0, false));
    state
        .controls
        .push(readback_control(STATE_READ_KNOWN, 0, false));

    state.apply_readback();

    assert_eq!(state.state_read, StateRead::Pending);
    assert!(!state.can_save());
}

#[test]
fn a_read_is_internal_and_does_not_appear_in_advanced() {
    let action = readback_control(zg01_core::model::STATE_READ_ACTION, 0, true);
    let status = readback_control(STATE_READ_STATUS, 1, false);
    let known = readback_control(STATE_READ_KNOWN, 0x3fffff, false);
    assert!(action.is_internal());
    assert!(status.is_internal());
    assert!(known.is_internal());
}

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
