//! Tests for the background workers: the control writer, the meter poller
//! and the device watcher.

use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zg01_core::alsa::{AlsaBackend, AlsaError, CardInfo, ControlInfo};
use zg01_core::device::{spawn_watcher, DeviceEvent};
use zg01_core::meters::{poll_meters, METER_CONTROLS};
use zg01_core::writer::{ControlWriter, WriteOutcome};

/// What a backend answers to a read.
#[derive(Clone, Copy)]
enum ReadAnswer {
    /// The control holds this value.
    Value(i64),
    /// The control does not exist.
    Unknown,
    /// The control exists but has no value.
    NoData,
}

impl Default for ReadAnswer {
    fn default() -> Self {
        ReadAnswer::Value(0)
    }
}

/// A backend that records the writes it receives.
#[derive(Default)]
struct RecordingBackend {
    writes: Mutex<Vec<(String, i64)>>,
    answer: Mutex<ReadAnswer>,
}

impl RecordingBackend {
    /// The writes the backend received, in order.
    fn writes(&self) -> Vec<(String, i64)> {
        self.writes.lock().unwrap().clone()
    }
}

impl AlsaBackend for RecordingBackend {
    fn list_controls(&self) -> Result<Vec<ControlInfo>, AlsaError> {
        Ok(vec![])
    }

    fn read(&self, _name: &str) -> Result<i64, AlsaError> {
        match *self.answer.lock().unwrap() {
            ReadAnswer::Value(value) => Ok(value),
            ReadAnswer::Unknown => Err(AlsaError::Unknown("Meter".to_string())),
            ReadAnswer::NoData => Err(AlsaError::NoData("Meter".to_string())),
        }
    }

    fn write(&self, name: &str, value: i64) -> Result<(), AlsaError> {
        self.writes.lock().unwrap().push((name.into(), value));
        Ok(())
    }
}

/// A backend whose writes always fail.
#[derive(Default)]
struct FailingBackend;

impl AlsaBackend for FailingBackend {
    fn list_controls(&self) -> Result<Vec<ControlInfo>, AlsaError> {
        Ok(vec![])
    }

    fn read(&self, _name: &str) -> Result<i64, AlsaError> {
        Ok(0)
    }

    fn write(&self, name: &str, _value: i64) -> Result<(), AlsaError> {
        Err(AlsaError::ReadOnly(name.to_string()))
    }
}

#[test]
fn coalesces_to_the_last_value() {
    let backend = Arc::new(RecordingBackend::default());
    let (writer, _rx) = ControlWriter::spawn(backend.clone());
    for value in 1..=10 {
        writer.set("Gate", value);
    }
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(backend.writes(), vec![("Gate".into(), 10)]);
}

#[test]
fn writes_every_control_only_once() {
    let backend = Arc::new(RecordingBackend::default());
    let (writer, _rx) = ControlWriter::spawn(backend.clone());
    writer.set("Gate", 1);
    writer.set("Gain", 2);
    std::thread::sleep(Duration::from_millis(100));
    let mut writes = backend.writes();
    writes.sort();
    assert_eq!(
        writes,
        vec![("Gain".to_string(), 2), ("Gate".to_string(), 1)]
    );
}

#[test]
fn reports_a_successful_write() {
    let backend = Arc::new(RecordingBackend::default());
    let (writer, rx) = ControlWriter::spawn(backend);
    writer.set("Gate", 7);
    let outcome = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(
        outcome,
        WriteOutcome::Ok {
            name: "Gate".to_string(),
            value: 7
        }
    );
}

#[test]
fn reports_a_failed_write() {
    let backend = Arc::new(FailingBackend);
    let (writer, rx) = ControlWriter::spawn(backend);
    writer.set("Gate", 7);
    let outcome = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    match outcome {
        WriteOutcome::Err { name, error } => {
            assert_eq!(name, "Gate");
            assert!(
                error.contains("Gate"),
                "the error names the control: {error}"
            );
        }
        other => panic!("expected a failure, got {other:?}"),
    }
    assert!(
        rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "one failure, one outcome"
    );
}

#[test]
fn set_and_wait_writes_the_value_at_once() {
    let backend = Arc::new(RecordingBackend::default());
    let (writer, _rx) = ControlWriter::spawn(backend.clone());
    writer.set_and_wait("Gate", 42).unwrap();
    assert_eq!(backend.writes(), vec![("Gate".into(), 42)]);
}

#[test]
fn set_and_wait_reports_the_failure() {
    let backend = Arc::new(FailingBackend);
    let (writer, _rx) = ControlWriter::spawn(backend);
    let error = writer.set_and_wait("Gate", 42).unwrap_err();
    assert!(
        error.contains("Gate"),
        "the error names the control: {error}"
    );
}

#[test]
fn set_and_wait_is_not_overwritten_by_a_pending_value() {
    let backend = Arc::new(RecordingBackend::default());
    let (writer, _rx) = ControlWriter::spawn(backend.clone());
    writer.set("Gate", 1);
    writer.set_and_wait("Gate", 2).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(backend.writes(), vec![("Gate".into(), 2)]);
}

#[test]
fn the_meter_controls_are_the_four_decoded_ones() {
    assert_eq!(
        METER_CONTROLS,
        [
            "Meter Mic In",
            "Meter Mic Out",
            "Meter PC In",
            "Meter Stream Mix"
        ]
    );
}

#[test]
fn the_meters_report_the_values() {
    let backend = RecordingBackend::default();
    let snapshot = poll_meters(&backend);
    assert_eq!(snapshot.values, [Some(0); 4]);
}

#[test]
fn a_missing_meter_is_none() {
    let backend = RecordingBackend::default();
    *backend.answer.lock().unwrap() = ReadAnswer::Unknown;
    let snapshot = poll_meters(&backend);
    assert_eq!(snapshot.values, [None; 4]);
}

#[test]
fn a_meter_without_a_value_is_none() {
    let backend = RecordingBackend::default();
    *backend.answer.lock().unwrap() = ReadAnswer::NoData;
    let snapshot = poll_meters(&backend);
    assert_eq!(snapshot.values, [None; 4]);
}

/// A card the watcher can report.
fn card() -> CardInfo {
    CardInfo {
        index: 2,
        id: "zg01".to_string(),
        driver: "zg01_usb".to_string(),
    }
}

#[test]
fn the_watcher_emits_only_transitions() {
    let state = Arc::new(Mutex::new(None::<CardInfo>));
    let watched = state.clone();
    let rx = spawn_watcher(
        move || watched.lock().unwrap().clone(),
        Duration::from_millis(20),
    );

    // Nothing is connected yet: no event.
    assert!(matches!(
        rx.recv_timeout(Duration::from_millis(120)),
        Err(RecvTimeoutError::Timeout)
    ));

    *state.lock().unwrap() = Some(card());
    let event = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(event, DeviceEvent::Connected(card()));

    // Still connected: no duplicate.
    assert!(matches!(
        rx.recv_timeout(Duration::from_millis(120)),
        Err(RecvTimeoutError::Timeout)
    ));

    *state.lock().unwrap() = None;
    let event = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(event, DeviceEvent::Disconnected);

    // Still gone: no duplicate.
    assert!(matches!(
        rx.recv_timeout(Duration::from_millis(120)),
        Err(RecvTimeoutError::Timeout)
    ));
}

#[test]
fn the_watcher_reports_the_card_that_came_back() {
    let state = Arc::new(Mutex::new(Some(card())));
    let watched = state.clone();
    let rx = spawn_watcher(
        move || watched.lock().unwrap().clone(),
        Duration::from_millis(20),
    );
    let event = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(event, DeviceEvent::Connected(card()));
}
