//! The meter poller.
//!
//! The driver publishes the levels of the four decoded meters as read-only
//! ALSA controls. Userspace reads them at 20 Hz and hands the readings to
//! the GUI, which computes the peak hold and the clip indicators.
//!
//! A meter that cannot be read is *not* a meter that reads zero: a control
//! that does not exist, and a control that has no value because the device
//! does not stream, are both reported as [`None`].

use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::alsa::{AlsaBackend, AlsaError};

/// The four meter controls the driver registers.
///
/// `Meter Game Mix` and `Meter HDMI In` are reserved names and are not
/// registered by the driver.
pub const METER_CONTROLS: [&str; 4] = [
    "Meter Mic In",
    "Meter Mic Out",
    "Meter PC In",
    "Meter Stream Mix",
];

/// How often the meters are read by default.
pub const METER_INTERVAL: Duration = Duration::from_millis(50);

/// The readings of the four meters, taken at one moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeterSnapshot {
    /// The value of each meter of [`METER_CONTROLS`].
    ///
    /// `None` means the meter could not be read, not that it reads zero.
    pub values: [Option<i64>; 4],
    /// When the readings were taken.
    pub at: Instant,
    /// Whether a read failed because the card went away.
    ///
    /// The GUI drops the backend and re-reads the card when this is set;
    /// an `-ENODATA` meter is not a disconnect.
    pub lost: bool,
    /// The generation of the backend that produced the snapshot.
    ///
    /// A snapshot from an already replaced backend must not tear down the
    /// new card when its delayed liveness error arrives.
    pub generation: u64,
}

impl MeterSnapshot {
    /// The value of the meter at `index`, when it could be read.
    pub fn value(&self, index: usize) -> Option<i64> {
        self.values.get(index).copied().flatten()
    }
}

/// Reads the four meters once.
///
/// A control that does not exist and a control that has no value are
/// reported as [`None`]; any other failure is logged and reported the
/// same way, so one broken meter never hides the others.
pub fn poll_meters(backend: &dyn AlsaBackend) -> MeterSnapshot {
    poll_meters_with_generation(backend, 0)
}

/// Reads the four meters once for one backend generation.
pub fn poll_meters_with_generation(backend: &dyn AlsaBackend, generation: u64) -> MeterSnapshot {
    let mut values = [None; 4];
    let mut lost = false;
    for (slot, name) in values.iter_mut().zip(METER_CONTROLS) {
        *slot = match backend.read(name) {
            Ok(value) => Some(value),
            Err(AlsaError::Unknown(_)) | Err(AlsaError::NoData(_)) => None,
            Err(err) => {
                if err.is_liveness() {
                    lost = true;
                }
                tracing::warn!("could not read the meter {name}: {err}");
                None
            }
        };
    }
    MeterSnapshot {
        values,
        at: Instant::now(),
        lost,
        generation,
    }
}

/// Reads the meters in a worker thread until the receiver is dropped.
///
/// The readings are sent at `interval`; the sender stops when the
/// receiver is dropped.
pub fn spawn_poller(backend: Arc<dyn AlsaBackend>, interval: Duration) -> Receiver<MeterSnapshot> {
    spawn_poller_with_generation(backend, interval, 0)
}

/// Reads the meters in a worker for one backend generation.
pub fn spawn_poller_with_generation(
    backend: Arc<dyn AlsaBackend>,
    interval: Duration,
    generation: u64,
) -> Receiver<MeterSnapshot> {
    let (snapshots, receiver) = mpsc::channel::<MeterSnapshot>();
    std::thread::Builder::new()
        .name("zg01-meters".to_string())
        .spawn(move || {
            while snapshots
                .send(poll_meters_with_generation(&*backend, generation))
                .is_ok()
            {
                std::thread::sleep(interval);
            }
        })
        .expect("could not start the meter poller");
    receiver
}

#[cfg(test)]
mod tests {
    use super::{MeterSnapshot, METER_CONTROLS};

    #[test]
    fn the_meter_order_follows_the_controls() {
        assert_eq!(METER_CONTROLS.len(), 4);
        assert_eq!(METER_CONTROLS[0], "Meter Mic In");
        assert_eq!(METER_CONTROLS[3], "Meter Stream Mix");
    }

    #[test]
    fn a_snapshot_knows_its_values() {
        let snapshot = MeterSnapshot {
            values: [Some(1), None, Some(3), None],
            at: std::time::Instant::now(),
            lost: false,
            generation: 0,
        };
        assert_eq!(snapshot.value(0), Some(1));
        assert_eq!(snapshot.value(1), None);
        assert_eq!(snapshot.value(9), None);
    }
}
