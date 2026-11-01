//! The Reset state transition.
//!
//! `Reset to ZG01` makes the driver reload its persisted settings without
//! refreshing the parameter cache, so every value the GUI last read stops
//! describing the device. The state marks the writable values unknown and
//! blocks Save until they are known again; a successful control write is
//! the way back for one value.

mod common;

use common::{control, state_with};
use zg01_core::model::{apply_reset, ControlValue};

#[test]
fn reset_disables_save_until_a_value_is_known() {
    let mut state = state_with(&["Gate"]);
    assert!(state.can_save());

    apply_reset(&mut state);

    assert!(!state.can_save());
    assert_eq!(
        state.control("Gate").map(|control| control.value),
        Some(ControlValue::Unknown)
    );
    assert!(
        state.device.is_connected(),
        "a reset reloads the device, it does not unplug it"
    );

    state.controls[0].value = ControlValue::Known(42);

    assert!(state.can_save());
}

#[test]
fn a_successful_write_makes_only_that_control_known() {
    let mut state = state_with(&["Gate", "Gate Threshold"]);
    apply_reset(&mut state);

    assert!(state.mark_known("Gate", 42));

    assert_eq!(
        state.control("Gate").map(|control| control.value),
        Some(ControlValue::Known(42))
    );
    assert_eq!(
        state.control("Gate Threshold").map(|control| control.value),
        Some(ControlValue::Unknown),
        "writing one control says nothing about the others"
    );
    assert!(
        !state.can_save(),
        "Save stays blocked while another writable value is unknown"
    );

    assert!(state.mark_known("Gate Threshold", 7));
    assert!(state.can_save());
}

#[test]
fn an_unknown_control_is_not_marked_known() {
    let mut state = state_with(&["Gate"]);

    assert!(!state.mark_known("Nope", 1));

    assert_eq!(
        state.control("Gate").map(|control| control.value),
        Some(ControlValue::Known(0))
    );
}

#[test]
fn reset_keeps_read_only_values() {
    let mut state = state_with(&["Gate"]);
    let mut meter = control("Meter Mic In", 0, 127);
    meter.info.writable = false;
    meter.value = ControlValue::Known(96);
    state.controls.push(meter);

    apply_reset(&mut state);

    assert_eq!(
        state.control("Meter Mic In").map(|control| control.value),
        Some(ControlValue::Known(96)),
        "the meters are not part of the reset"
    );
}

#[test]
fn a_disconnect_keeps_the_last_values() {
    let mut state = state_with(&["Gate"]);

    state.disconnect();

    assert!(!state.device.is_connected());
    assert!(!state.can_save());
    assert_eq!(
        state.control("Gate").map(|control| control.value),
        Some(ControlValue::Known(0)),
        "the last known values stay visible, they are just not writable"
    );
}

#[test]
fn the_action_controls_are_not_values() {
    let mut state = state_with(&["Gate", "Save to ZG01", "Reset to ZG01"]);

    apply_reset(&mut state);

    assert_eq!(
        state.control("Save to ZG01").map(|control| control.value),
        Some(ControlValue::Known(0)),
        "a write-only action has no value to invalidate"
    );
    assert_eq!(
        state.control("Reset to ZG01").map(|control| control.value),
        Some(ControlValue::Known(0))
    );

    assert!(state.mark_known("Gate", 42));
    assert!(
        state.can_save(),
        "the action controls must never keep Save disabled forever"
    );
}
