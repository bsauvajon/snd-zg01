//! The meter poller.
//!
//! The driver publishes the levels of the six card meter groups as
//! read-only ALSA controls. The mic groups carry one value; the PC, HDMI,
//! game and stream groups carry eight. Userspace reads them at 20 Hz and
//! hands the readings to the GUI, which computes the peak hold and the clip
//! indicators.
//!
//! A meter that cannot be read is *not* a meter that reads zero: a control
//! that does not exist, a control that has no value because the device does
//! not stream, and a channel the old driver does not publish are all
//! reported as [`None`].

use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::alsa::{AlsaBackend, AlsaError};
use crate::mixage::MeterId;

/// The six meter controls the driver registers.
pub const METER_CONTROLS: [&str; 6] = [
    "Meter Mic In",
    "Meter Mic Out",
    "Meter PC In",
    "Meter HDMI In",
    "Meter Game Mix",
    "Meter Stream Mix",
];

/// How often the meters are read by default.
pub const METER_INTERVAL: Duration = Duration::from_millis(50);

/// Which contract produced a snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeterLayout {
    /// Six controls; the surround groups carry eight values.
    V2,
    /// The four one-value controls of the old driver.
    Legacy,
}

/// The layout the declared meter controls describe.
///
/// `count` returns the declared number of values of one control, or `None`
/// when the driver does not publish it. Only the declaration matters: a
/// control that currently reads `-ENODATA` cannot flip the layout. The
/// legacy driver publishes one-value `Meter PC In` and `Meter Stream Mix`
/// and neither new group; anything else is V2, so a partial V2 driver with
/// eight-value arrays but no new control stays V2 in every consumer.
pub fn layout_from_counts(count: impl Fn(&str) -> Option<u32>) -> MeterLayout {
    let legacy = count("Meter Game Mix").is_none()
        && count("Meter HDMI In").is_none()
        && count("Meter PC In") == Some(1)
        && count("Meter Stream Mix") == Some(1);
    if legacy {
        MeterLayout::Legacy
    } else {
        MeterLayout::V2
    }
}

/// The readings of the six meter groups, taken at one moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeterSnapshot {
    /// The channels of each group, in [`MeterId`] slot order.
    ///
    /// Only channel 0 is used for the mic groups. `None` means the channel
    /// could not be read, not that it reads zero.
    pub values: [[Option<i64>; 8]; 6],
    /// Which control layout produced the snapshot.
    pub layout: MeterLayout,
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
    /// The value of one channel of `id`, when it could be read.
    pub fn value(&self, id: MeterId, channel: usize) -> Option<i64> {
        self.values[id.index()].get(channel).copied().flatten()
    }

    /// The loudest readable channel of `id`.
    pub fn max(&self, id: MeterId) -> Option<i64> {
        self.values[id.index()].iter().flatten().copied().max()
    }
}

/// Reads the meters once.
pub fn poll_meters(backend: &dyn AlsaBackend) -> MeterSnapshot {
    poll_meters_with_generation(backend, 0)
}

/// Reads the meters once for one backend generation.
///
/// A control that does not exist and a control that has no value are
/// reported as [`None`]; any other failure is logged and reported the same
/// way, so one broken meter never hides the others.
pub fn poll_meters_with_generation(backend: &dyn AlsaBackend, generation: u64) -> MeterSnapshot {
    let mut values = [[None; 8]; 6];
    let mut lost = false;

    for (id, name) in MeterId::ALL.iter().zip(METER_CONTROLS) {
        match backend.read_values(name) {
            Ok(readings) => {
                for (slot, value) in values[id.index()].iter_mut().zip(readings) {
                    *slot = Some(value);
                }
            }
            Err(AlsaError::Unknown(_)) | Err(AlsaError::NoData(_)) => {}
            Err(err) => {
                if err.is_liveness() {
                    lost = true;
                }
                tracing::warn!("could not read the meter {name}: {err}");
            }
        }
    }

    // The layout comes from the declared control counts, never from a
    // reading: a legacy driver with no meter data must stay legacy, and a
    // driver that publishes either new control must be treated as v2 even
    // when its arrays are short.
    let layout = layout_from_counts(|name| backend.control_count(name));

    MeterSnapshot {
        values,
        layout,
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
    use std::time::Instant;

    use super::{
        layout_from_counts, poll_meters_with_generation, MeterLayout, MeterSnapshot, METER_CONTROLS,
    };
    use crate::alsa::{ControlInfo, ControlKind, FakeAlsa, FakeError};
    use crate::mixage::MeterId;

    fn control(name: &str, count: u32) -> ControlInfo {
        ControlInfo {
            name: name.to_string(),
            kind: ControlKind::Integer,
            min: 0,
            max: 127,
            step: 0,
            count,
            writable: false,
        }
    }

    fn fake_v2() -> FakeAlsa {
        let fake = FakeAlsa::new(vec![
            control("Meter Mic In", 1),
            control("Meter Mic Out", 1),
            control("Meter PC In", 8),
            control("Meter HDMI In", 8),
            control("Meter Game Mix", 8),
            control("Meter Stream Mix", 8),
        ]);
        fake.set_value("Meter Mic In", 10);
        fake.set_value("Meter Mic Out", 11);
        fake.set_values("Meter PC In", vec![1, 2, 3, 4, 5, 6, 7, 8]);
        fake.set_values("Meter HDMI In", vec![9; 8]);
        fake.set_values("Meter Game Mix", vec![20; 8]);
        fake.set_values("Meter Stream Mix", vec![30; 8]);
        fake
    }

    #[test]
    fn the_six_meter_ids_have_stable_slots_and_counts() {
        assert_eq!(MeterId::MicIn.index(), 0);
        assert_eq!(MeterId::MicOut.index(), 1);
        assert_eq!(MeterId::PcIn.index(), 2);
        assert_eq!(MeterId::HdmiIn.index(), 3);
        assert_eq!(MeterId::GameMix.index(), 4);
        assert_eq!(MeterId::StreamMix.index(), 5);
        assert_eq!(MeterId::MicIn.channel_count(), 1);
        assert_eq!(MeterId::PcIn.channel_count(), 8);
        assert_eq!(MeterId::PcIn.control(), "Meter PC In");
        assert_eq!(METER_CONTROLS.len(), 6);
    }

    #[test]
    fn a_v2_snapshot_reads_by_group_and_channel() {
        let snapshot = poll_meters_with_generation(&fake_v2(), 3);
        assert_eq!(snapshot.layout, MeterLayout::V2);
        assert_eq!(snapshot.value(MeterId::MicIn, 0), Some(10));
        assert_eq!(snapshot.value(MeterId::MicIn, 1), None);
        assert_eq!(snapshot.value(MeterId::PcIn, 0), Some(1));
        assert_eq!(snapshot.value(MeterId::PcIn, 7), Some(8));
        assert_eq!(snapshot.value(MeterId::PcIn, 8), None);
        assert_eq!(snapshot.max(MeterId::PcIn), Some(8));
        assert_eq!(snapshot.value(MeterId::GameMix, 0), Some(20));
        assert_eq!(snapshot.generation, 3);
    }

    #[test]
    fn a_legacy_driver_keeps_its_own_layout() {
        let fake = FakeAlsa::new(vec![
            control("Meter Mic In", 1),
            control("Meter Mic Out", 1),
            control("Meter PC In", 1),
            control("Meter Stream Mix", 1),
        ]);
        fake.set_value("Meter Mic In", 52);
        fake.set_value("Meter Mic Out", 0);
        fake.set_value("Meter PC In", 38);
        fake.set_value("Meter Stream Mix", 126);

        let snapshot = poll_meters_with_generation(&fake, 0);
        assert_eq!(snapshot.layout, MeterLayout::Legacy);
        assert_eq!(snapshot.value(MeterId::PcIn, 0), Some(38));
        assert_eq!(snapshot.value(MeterId::StreamMix, 0), Some(126));
        assert_eq!(snapshot.value(MeterId::GameMix, 0), None);
        assert_eq!(snapshot.value(MeterId::HdmiIn, 0), None);
    }

    #[test]
    fn a_legacy_driver_without_readings_remains_legacy() {
        let fake = FakeAlsa::new(vec![
            control("Meter Mic In", 1),
            control("Meter Mic Out", 1),
            control("Meter PC In", 1),
            control("Meter Stream Mix", 1),
        ]);
        for name in [
            "Meter Mic In",
            "Meter Mic Out",
            "Meter PC In",
            "Meter Stream Mix",
        ] {
            fake.set_error(name, FakeError::NoData);
        }

        let snapshot = poll_meters_with_generation(&fake, 0);
        assert_eq!(snapshot.layout, MeterLayout::Legacy);
        assert!(snapshot
            .values
            .iter()
            .all(|group| group.iter().all(Option::is_none)));
    }

    #[test]
    fn one_new_control_prevents_legacy_detection() {
        let fake = FakeAlsa::new(vec![
            control("Meter PC In", 1),
            control("Meter Stream Mix", 1),
            control("Meter Game Mix", 8),
        ]);
        fake.set_value("Meter PC In", 38);

        let snapshot = poll_meters_with_generation(&fake, 0);
        assert_eq!(snapshot.layout, MeterLayout::V2);
        assert_eq!(snapshot.value(MeterId::PcIn, 0), Some(38));
    }

    #[test]
    fn eight_value_pc_and_stream_controls_without_the_new_groups_are_v2() {
        let fake = FakeAlsa::new(vec![
            control("Meter PC In", 8),
            control("Meter Stream Mix", 8),
        ]);

        let snapshot = poll_meters_with_generation(&fake, 0);
        assert_eq!(snapshot.layout, MeterLayout::V2);
    }

    #[test]
    fn the_layout_predicate_only_uses_declared_counts() {
        assert_eq!(
            layout_from_counts(|name| match name {
                "Meter PC In" | "Meter Stream Mix" => Some(1),
                _ => None,
            }),
            MeterLayout::Legacy
        );
        assert_eq!(
            layout_from_counts(|name| match name {
                "Meter PC In" | "Meter Stream Mix" => Some(8),
                _ => None,
            }),
            MeterLayout::V2,
            "eight-value arrays without the new groups are a partial v2 driver"
        );
        assert_eq!(
            layout_from_counts(|name| (name == "Meter Game Mix").then_some(8)),
            MeterLayout::V2
        );
    }

    #[test]
    fn a_short_v2_array_leaves_the_rest_unknown() {
        let fake = FakeAlsa::new(vec![
            control("Meter Mic In", 1),
            control("Meter PC In", 1),
            control("Meter HDMI In", 8),
            control("Meter Game Mix", 8),
            control("Meter Stream Mix", 8),
        ]);
        fake.set_value("Meter PC In", 38);

        let snapshot = poll_meters_with_generation(&fake, 0);
        assert_eq!(snapshot.layout, MeterLayout::V2);
        assert_eq!(snapshot.value(MeterId::PcIn, 0), Some(38));
        assert_eq!(snapshot.value(MeterId::PcIn, 1), None);
    }

    #[test]
    fn a_lost_card_is_marked_and_reads_nothing() {
        let fake = fake_v2();
        for name in METER_CONTROLS {
            fake.set_error(name, FakeError::DeviceLost);
        }
        let snapshot = poll_meters_with_generation(&fake, 0);
        assert!(snapshot.lost);
        assert!(snapshot
            .values
            .iter()
            .all(|group| group.iter().all(Option::is_none)));
    }

    #[test]
    fn a_snapshot_with_no_readings_is_all_none() {
        let snapshot = MeterSnapshot {
            values: [[None; 8]; 6],
            layout: MeterLayout::V2,
            at: Instant::now(),
            lost: false,
            generation: 0,
        };
        assert_eq!(snapshot.value(MeterId::PcIn, 0), None);
        assert_eq!(snapshot.max(MeterId::PcIn), None);
        assert!(!snapshot.lost);
    }

    #[test]
    fn an_unknown_meter_is_not_zero() {
        let fake = FakeAlsa::new(vec![control("Meter PC In", 8)]);
        fake.set_error("Meter PC In", FakeError::Unknown);
        let snapshot = poll_meters_with_generation(&fake, 0);
        assert_eq!(snapshot.value(MeterId::PcIn, 0), None);
        assert!(!snapshot.lost);
    }
}
