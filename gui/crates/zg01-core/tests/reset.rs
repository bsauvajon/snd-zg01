//! The Reset transition and the action that drives it.
//!
//! `Reset to ZG01` makes the driver reload its persisted settings without
//! refreshing the parameter cache, so every value the GUI last read stops
//! describing the device. The action is written through the control
//! writer after the pending edits are flushed: the state then marks the
//! writable values unknown and blocks Save until they are known again. A
//! successful control write is the way back for one value.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{control, state_with};
use zg01_core::alsa::{AlsaBackend, AlsaError, ControlInfo, FakeAlsa, FakeError};
use zg01_core::model::{apply_reset, spawn_reset, ControlValue, RESET_ACTION};
use zg01_core::writer::ControlWriter;

/// A device that takes its time to answer, like a busy USB card.
struct SlowBackend {
    delay: Duration,
}

impl AlsaBackend for SlowBackend {
    fn list_controls(&self) -> Result<Vec<ControlInfo>, AlsaError> {
        Ok(vec![])
    }

    fn read(&self, _name: &str) -> Result<i64, AlsaError> {
        Ok(0)
    }

    fn write(&self, _name: &str, _value: i64) -> Result<(), AlsaError> {
        std::thread::sleep(self.delay);
        Ok(())
    }
}

/// A writer on a fake device that publishes `names`.
///
/// The outcome receiver is dropped: these tests only need the writes the
/// device received.
fn writer(names: &[&str]) -> (ControlWriter, Arc<FakeAlsa>) {
    let backend = Arc::new(FakeAlsa::new(
        names
            .iter()
            .map(|name| control(name, 0, 100).info)
            .collect(),
    ));
    let (writer, _outcomes) = ControlWriter::spawn(backend.clone());
    (writer, backend)
}

#[test]
fn reset_disables_save_until_a_value_is_known() {
    let (writer, _backend) = writer(&[RESET_ACTION]);
    let mut state = state_with(&["Gate"]);
    assert!(state.can_save());

    apply_reset(&mut state, &writer).expect("the reset reaches the device");

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
fn a_successful_reset_leaves_every_writable_value_unknown() {
    let (writer, _backend) = writer(&[RESET_ACTION]);
    let mut state = state_with(&["Gate", "Gate Threshold"]);

    apply_reset(&mut state, &writer).expect("the reset reaches the device");

    assert!(
        state
            .controls
            .iter()
            .all(|control| control.value == ControlValue::Unknown),
        "every value the GUI held is stale after the reload"
    );
    assert!(!state.can_save());
}

#[test]
fn a_failed_reset_reports_the_error_and_leaves_the_values_unknown() {
    let (writer, backend) = writer(&[RESET_ACTION]);
    backend.set_error(RESET_ACTION, FakeError::ReadOnly);
    let mut state = state_with(&["Gate"]);

    let error = apply_reset(&mut state, &writer).expect_err("the action fails");

    assert!(
        error.contains(RESET_ACTION),
        "the error names the action: {error}"
    );
    assert!(
        state
            .controls
            .iter()
            .all(|control| control.value == ControlValue::Unknown),
        "a failed action must not leave the GUI claiming known values"
    );
    assert!(!state.can_save());
}

#[test]
fn a_pending_write_is_flushed_before_the_reset() {
    let (writer, backend) = writer(&["Gate", RESET_ACTION]);
    let mut state = state_with(&["Gate"]);
    writer.set("Gate", 42);

    apply_reset(&mut state, &writer).expect("the reset reaches the device");

    assert_eq!(
        backend.writes(),
        vec![("Gate".to_string(), 42), (RESET_ACTION.to_string(), 1),],
        "a fader queued before the reset must land first, otherwise the \
         reset would silently discard it"
    );
}

/// The GUI must never wait for the card in its update loop: starting a
/// reset against a slow device returns at once, and the completion arrives
/// on the receiver.
#[test]
fn starting_a_reset_does_not_wait_for_a_slow_device() {
    let backend = Arc::new(SlowBackend {
        delay: Duration::from_millis(400),
    });
    let (writer, _outcomes) = ControlWriter::spawn(backend);

    let started = Instant::now();
    let completion = spawn_reset(writer);
    assert!(
        started.elapsed() < Duration::from_millis(100),
        "spawn_reset blocked the caller for {:?}",
        started.elapsed()
    );

    assert!(completion
        .recv_timeout(Duration::from_secs(2))
        .expect("the reset answers")
        .is_ok());
}

#[test]
fn a_successful_write_makes_only_that_control_known() {
    let (writer, _backend) = writer(&[RESET_ACTION]);
    let mut state = state_with(&["Gate", "Gate Threshold"]);
    apply_reset(&mut state, &writer).expect("the reset reaches the device");

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
    let (writer, _backend) = writer(&[RESET_ACTION]);
    let mut state = state_with(&["Gate"]);
    let mut meter = control("Meter Mic In", 0, 127);
    meter.info.writable = false;
    meter.value = ControlValue::Known(96);
    state.controls.push(meter);

    apply_reset(&mut state, &writer).expect("the reset reaches the device");

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
    let (writer, _backend) = writer(&[RESET_ACTION]);
    let mut state = state_with(&["Gate", "Save to ZG01", "Reset to ZG01"]);

    apply_reset(&mut state, &writer).expect("the reset reaches the device");

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
