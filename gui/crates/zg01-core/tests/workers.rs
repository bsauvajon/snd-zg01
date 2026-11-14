//! Tests for the background workers: the control writer, the meter poller
//! and the device watcher.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zg01_core::alsa::{AlsaBackend, AlsaError, CardInfo, ControlInfo};
use zg01_core::device::{spawn_watcher, DeviceEvent};
use zg01_core::meters::{poll_meters, MeterLayout, METER_CONTROLS};
use zg01_core::mixage::MeterId;
use zg01_core::model::{SAVE_ACTION, STATE_READ_ACTION};
use zg01_core::state_read::spawn_state_read_with;
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
    fail_write: AtomicBool,
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
        if self.fail_write.load(Ordering::Relaxed) {
            return Err(AlsaError::Unknown(name.to_string()));
        }
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

/// A backend whose `Gate` write fails but whose action writes succeed.
#[derive(Default)]
struct GateFailingBackend {
    writes: Mutex<Vec<(String, i64)>>,
}

impl AlsaBackend for GateFailingBackend {
    fn list_controls(&self) -> Result<Vec<ControlInfo>, AlsaError> {
        Ok(vec![])
    }

    fn read(&self, _name: &str) -> Result<i64, AlsaError> {
        Ok(0)
    }

    fn write(&self, name: &str, value: i64) -> Result<(), AlsaError> {
        self.writes.lock().unwrap().push((name.to_string(), value));
        if name == "Gate" {
            return Err(AlsaError::ReadOnly(name.to_string()));
        }
        Ok(())
    }
}

/// A backend whose card has gone away.
struct LostBackend;

impl AlsaBackend for LostBackend {
    fn list_controls(&self) -> Result<Vec<ControlInfo>, AlsaError> {
        Ok(vec![])
    }

    fn read(&self, _name: &str) -> Result<i64, AlsaError> {
        Err(AlsaError::Alsa(alsa::Error::new("read", 19)))
    }

    fn write(&self, _name: &str, _value: i64) -> Result<(), AlsaError> {
        Err(AlsaError::Alsa(alsa::Error::new("write", 5)))
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

/// A direct write reports its outcome on the channel too: one write,
/// one outcome.
#[test]
fn set_and_wait_reports_one_outcome_on_the_channel() {
    let backend = Arc::new(RecordingBackend::default());
    let (writer, rx) = ControlWriter::spawn(backend);
    assert!(writer.set_and_wait("Gate", 42).is_ok());
    let outcome = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(
        outcome,
        WriteOutcome::Ok {
            name: "Gate".to_string(),
            value: 42
        }
    );
    assert!(
        rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "one write, one outcome"
    );
}

/// The outcome on the channel and the returned result tell the same
/// story when the direct write fails.
#[test]
fn a_failed_set_and_wait_reports_one_outcome_on_the_channel() {
    let backend = Arc::new(FailingBackend);
    let (writer, rx) = ControlWriter::spawn(backend);
    let error = writer.set_and_wait("Gate", 42).unwrap_err();
    let outcome = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    match outcome {
        WriteOutcome::Err {
            name,
            error: reported,
        } => {
            assert_eq!(name, "Gate");
            assert_eq!(reported, error, "the channel and the result agree");
        }
        other => panic!("expected a failure, got {other:?}"),
    }
    assert!(
        rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "one write, one outcome"
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

/// `flush_and_wait` is a global barrier: every pending control is written
/// before it returns, without waiting for the coalescing deadline.
#[test]
fn flush_and_wait_writes_every_pending_control() {
    let backend = Arc::new(RecordingBackend::default());
    let (writer, _rx) = ControlWriter::spawn(backend.clone());
    writer.set("Gate", 11);
    writer.set("Gain", 22);

    writer.flush_and_wait().expect("the flush succeeds");

    let mut writes = backend.writes();
    writes.sort();
    assert_eq!(
        writes,
        vec![("Gain".to_string(), 22), ("Gate".to_string(), 11)]
    );
}

/// The barrier reports the failing writes, so a caller that needs the
/// device state to be committed knows it is not.
#[test]
fn flush_and_wait_reports_a_failed_write() {
    let (writer, _rx) = ControlWriter::spawn(Arc::new(FailingBackend));
    writer.set("Gate", 7);

    let error = writer.flush_and_wait().expect_err("the write fails");

    assert!(
        error.contains("Gate"),
        "the error names the control: {error}"
    );
}

/// Save is a barrier followed by the action: a fader queued just before it
/// must land first, not after the persisted settings were captured.
#[test]
fn flush_set_and_wait_writes_a_pending_fader_before_the_save_action() {
    let backend = Arc::new(RecordingBackend::default());
    let (writer, _rx) = ControlWriter::spawn(backend.clone());
    writer.set("Gate", 42);

    writer
        .flush_set_and_wait(SAVE_ACTION, 1)
        .expect("the save succeeds");

    assert_eq!(
        backend.writes(),
        vec![("Gate".into(), 42), (SAVE_ACTION.into(), 1)],
        "the pending Gate write must reach the device before Save"
    );
}

/// A failed flush keeps Save off the device: saving would persist a value
/// the user did not ask for.
#[test]
fn flush_set_and_wait_does_not_save_after_a_failed_flush() {
    let backend = Arc::new(GateFailingBackend::default());
    let (writer, _rx) = ControlWriter::spawn(backend.clone());
    writer.set("Gate", 7);

    let error = writer
        .flush_set_and_wait(SAVE_ACTION, 1)
        .expect_err("the pending write fails");
    assert!(
        error.contains("Gate"),
        "the error names the control: {error}"
    );
    assert!(
        backend
            .writes
            .lock()
            .unwrap()
            .iter()
            .all(|(name, _)| name != SAVE_ACTION),
        "Save must not be written after a failed flush"
    );
}

/// A write that reports `ENODEV` is a disconnect, not a control failure.
#[test]
fn a_liveness_failure_is_reported_as_a_lost_device() {
    let (writer, rx) = ControlWriter::spawn_with_generation(Arc::new(LostBackend), 7);
    let error = writer.set_and_wait("Gate", 1).unwrap_err();
    assert!(!error.is_empty());

    match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
        WriteOutcome::DeviceLost {
            generation, name, ..
        } => {
            assert_eq!(name, "Gate");
            assert_eq!(generation, 7, "the outcome names its backend");
        }
        other => panic!("expected a lost device, got {other:?}"),
    }
}

#[test]
fn a_meter_read_that_loses_the_card_marks_the_snapshot() {
    let snapshot = poll_meters(&LostBackend);
    assert!(
        snapshot.lost,
        "a liveness failure is not just a missing meter"
    );
    assert!(snapshot
        .values
        .iter()
        .all(|group| group.iter().all(Option::is_none)));
}

#[test]
fn the_meter_controls_are_the_six_decoded_ones() {
    assert_eq!(
        METER_CONTROLS,
        [
            "Meter Mic In",
            "Meter Mic Out",
            "Meter PC In",
            "Meter HDMI In",
            "Meter Game Mix",
            "Meter Stream Mix"
        ]
    );
}

#[test]
fn the_meters_report_the_values() {
    let backend = RecordingBackend::default();
    let snapshot = poll_meters(&backend);
    assert_eq!(snapshot.layout, MeterLayout::V2);
    assert_eq!(snapshot.value(MeterId::PcIn, 0), Some(0));
    assert_eq!(snapshot.value(MeterId::PcIn, 1), None);
    assert_eq!(snapshot.value(MeterId::GameMix, 0), Some(0));
}

#[test]
fn a_missing_meter_is_none() {
    let backend = RecordingBackend::default();
    *backend.answer.lock().unwrap() = ReadAnswer::Unknown;
    let snapshot = poll_meters(&backend);
    assert!(snapshot
        .values
        .iter()
        .all(|group| group.iter().all(Option::is_none)));
}

#[test]
fn a_meter_without_a_value_is_none() {
    let backend = RecordingBackend::default();
    *backend.answer.lock().unwrap() = ReadAnswer::NoData;
    let snapshot = poll_meters(&backend);
    assert!(snapshot
        .values
        .iter()
        .all(|group| group.iter().all(Option::is_none)));
}

/// A card the watcher can report.
fn card() -> CardInfo {
    CardInfo {
        index: 2,
        id: "zg01".to_string(),
        driver: "zg01_usb".to_string(),
        generation: Some("1:5".to_string()),
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

/// A quick unplug and replug between two polls keeps the ALSA index but
/// changes the USB device number: the watcher must not miss it.
#[test]
fn the_watcher_reports_a_quick_replug_with_the_same_index() {
    let replugged = Arc::new(AtomicBool::new(false));
    let polls = Arc::new(AtomicUsize::new(0));
    let watched_replug = replugged.clone();
    let watched_polls = polls.clone();
    let rx = spawn_watcher(
        move || {
            let poll = watched_polls.fetch_add(1, Ordering::Relaxed);
            // Stay on the first device long enough for the first event,
            // then present the new one behind the same ALSA index.
            if poll >= 2 {
                watched_replug.store(true, Ordering::Relaxed);
            }
            let mut card = card();
            if watched_replug.load(Ordering::Relaxed) {
                card.generation = Some("1:7".to_string());
            }
            Some(card)
        },
        Duration::from_millis(20),
    );

    let first = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(first, DeviceEvent::Connected(card()));

    let second = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(second, DeviceEvent::Disconnected);
    let third = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let mut new_card = card();
    new_card.generation = Some("1:7".to_string());
    assert_eq!(third, DeviceEvent::Connected(new_card));
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

#[test]
fn a_state_read_writes_the_action_and_reports_success() {
    let backend = Arc::new(RecordingBackend::default());
    *backend.answer.lock().unwrap() = ReadAnswer::Value(1);
    let (writer, _outcomes) = ControlWriter::spawn_with_generation(backend.clone(), 0);

    let results = spawn_state_read_with(
        backend.clone(),
        writer,
        Duration::from_millis(100),
        Duration::from_millis(5),
    );
    let outcome = results
        .recv_timeout(Duration::from_secs(2))
        .expect("the state read worker answers");

    assert!(outcome.succeeded());
    assert_eq!(backend.writes(), vec![(STATE_READ_ACTION.to_string(), 1)]);
}

#[test]
fn a_pending_state_read_waits_without_a_second_write() {
    let backend = Arc::new(RecordingBackend::default());
    let (writer, _outcomes) = ControlWriter::spawn_with_generation(backend.clone(), 0);

    let results = spawn_state_read_with(
        backend.clone(),
        writer,
        Duration::from_millis(40),
        Duration::from_millis(5),
    );
    let outcome = results
        .recv_timeout(Duration::from_secs(2))
        .expect("the state read worker answers");

    assert_eq!(outcome.status, 0);
    assert!(outcome.pending_before);
    assert!(!outcome.succeeded());
    assert!(
        backend.writes().is_empty(),
        "a pending read must not be restarted"
    );
}

#[test]
fn a_failed_state_action_is_not_a_success() {
    let backend = Arc::new(RecordingBackend::default());
    *backend.answer.lock().unwrap() = ReadAnswer::Value(1);
    backend.fail_write.store(true, Ordering::Relaxed);
    let (writer, _outcomes) = ControlWriter::spawn_with_generation(backend.clone(), 0);

    let results = spawn_state_read_with(
        backend.clone(),
        writer,
        Duration::from_millis(100),
        Duration::from_millis(5),
    );
    let outcome = results
        .recv_timeout(Duration::from_secs(2))
        .expect("the state read worker answers");

    assert_eq!(outcome.status, 1);
    assert!(outcome.wrote.is_err());
    assert!(
        !outcome.succeeded(),
        "a stale success status must not hide a failed action"
    );
}
