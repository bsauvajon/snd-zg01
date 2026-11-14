//! The control model: what a control is, what it holds and how it reads.
//!
//! The GUI talks to the ZG01 through its ALSA *control* elements. Every
//! element the driver publishes becomes a [`Control`]: the metadata the
//! driver publishes ([`ControlInfo`]), the value the GUI last read
//! ([`ControlValue`]) and the unit the value is expressed in
//! ([`Unit`]).
//!
//! The units are the ones `zgctl` documents and the driver uses:
//!
//! | Unit | Meaning |
//! |---|---|
//! | [`Unit::Db`] | decibels, in 0.1 dB steps (`60` is `+6.0 dB`) |
//! | [`Unit::Hz`] | hertz |
//! | [`Unit::Q`] | Q, in 0.01 steps (`120` is `1.20`) |
//! | [`Unit::Steps`] | a raw device step, shown as an integer |
//! | [`Unit::Percent`] | a percentage |
//! | [`Unit::Bool`] | a switch, shown as `on`/`off` |
//! | [`Unit::Raw`] | a value without a unit |

use std::collections::BTreeSet;
use std::sync::mpsc::{self, Receiver};

use serde::Deserialize;

use crate::alsa::{CardInfo, ControlInfo};
use crate::layout::Layout;
use crate::writer::ControlWriter;

/// The action control that persists the current settings on the device.
pub const SAVE_ACTION: &str = "Save to ZG01";

/// The action control that reloads the persisted settings from the device.
pub const RESET_ACTION: &str = "Reset to ZG01";

/// The action control that re-reads the MIC state dump from the device.
pub const STATE_READ_ACTION: &str = "State Read";

/// The read-only result of the last state read.
pub const STATE_READ_STATUS: &str = "State Read Status";

/// The read-only mask of the fields the last state read applied.
pub const STATE_READ_KNOWN: &str = "State Read Known";

/// The write-only action controls.
///
/// An action asks the device to do something; it holds no setting the GUI
/// displays. A reset therefore does not invalidate it, and it never keeps
/// Save disabled.
const ACTION_CONTROLS: [&str; 3] = [SAVE_ACTION, RESET_ACTION, STATE_READ_ACTION];

/// The direct MIC fields in the driver's state-dump mask.
///
/// The bit positions match `enum zg01_state_field` in `src/zg01_state.h`.
/// The values exist because the driver parsed the dump; the transformed
/// fields below do not.
const DIRECT_MIC: [(&str, u32); 19] = [
    ("Gate Capture Switch", 1 << 0),
    ("Gate", 1 << 1),
    ("Gate Range", 1 << 2),
    ("Compressor Capture Switch", 1 << 6),
    ("Compressor", 1 << 7),
    ("Compressor Threshold", 1 << 8),
    ("Compressor Ratio", 1 << 9),
    ("Compressor Knee", 1 << 10),
    ("Limiter Capture Switch", 1 << 11),
    ("Limiter", 1 << 12),
    ("Limiter Threshold", 1 << 13),
    ("Limiter Knee", 1 << 14),
    ("EQ Capture Switch", 1 << 15),
    ("EQ Low Shape", 1 << 16),
    ("EQ High Shape", 1 << 17),
    ("EQ Band 1 Q", 1 << 18),
    ("EQ Band 2 Q", 1 << 19),
    ("EQ Band 3 Q", 1 << 20),
    ("EQ Band 4 Q", 1 << 21),
];

/// The result of the last MIC state read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateRead {
    /// No read was attempted in this session.
    Unknown,
    /// A read is in flight.
    Pending,
    /// The read succeeded; `known` is the driver's field mask.
    Known {
        /// The driver's `State Read Known` mask.
        known: u32,
    },
    /// The read failed or timed out.
    Failed,
}

impl StateRead {
    /// Whether the device state was read successfully.
    pub fn is_known(self) -> bool {
        matches!(self, StateRead::Known { .. })
    }
}

/// The unit a control value is expressed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    /// Decibels, in 0.1 dB steps.
    Db,
    /// Hertz.
    Hz,
    /// Q, in 0.01 steps.
    Q,
    /// A raw device step, shown as an integer.
    Steps,
    /// A percentage.
    Percent,
    /// A switch.
    Bool,
    /// A value without a unit.
    Raw,
}

/// The value of a control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlValue {
    /// The device reported this value.
    Known(i64),
    /// The value is not known.
    ///
    /// A control that was never read, or whose value was invalidated (a
    /// reset reloads the device without telling the GUI what the device
    /// now holds), has no value to show.
    Unknown,
}

impl ControlValue {
    /// The value, when it is known.
    pub fn known(self) -> Option<i64> {
        match self {
            ControlValue::Known(value) => Some(value),
            ControlValue::Unknown => None,
        }
    }
}

/// One control of the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Control {
    /// What the driver publishes about the control.
    pub info: ControlInfo,
    /// The value the GUI last read.
    pub value: ControlValue,
    /// The unit the value is expressed in.
    pub unit: Unit,
}

impl Control {
    /// A control with a known value.
    pub fn new(info: ControlInfo, value: i64, unit: Unit) -> Self {
        Control {
            info,
            value: ControlValue::Known(value),
            unit,
        }
    }

    /// A control whose value is not known.
    pub fn unknown(info: ControlInfo, unit: Unit) -> Self {
        Control {
            info,
            value: ControlValue::Unknown,
            unit,
        }
    }

    /// The name of the control, e.g. `Gate Capture Switch`.
    pub fn name(&self) -> &str {
        &self.info.name
    }

    /// The value, when it is known.
    pub fn value(&self) -> Option<i64> {
        self.value.known()
    }

    /// Whether the driver accepts writes for this control.
    pub fn is_writable(&self) -> bool {
        self.info.writable
    }

    /// Whether this control is a write-only device action.
    ///
    /// [`SAVE_ACTION`], [`RESET_ACTION`] and [`STATE_READ_ACTION`] are
    /// actions, not settings: they are writable but have no value the GUI
    /// can rely on.
    pub fn is_action(&self) -> bool {
        ACTION_CONTROLS.contains(&self.name())
    }

    /// Whether this control is an implementation detail of the readback.
    ///
    /// The State Read status and known-mask controls are read-only and
    /// driver-internal; they do not belong in the Avancé table.
    pub fn is_internal(&self) -> bool {
        self.is_action() || matches!(self.name(), STATE_READ_STATUS | STATE_READ_KNOWN)
    }

    /// The value as it is shown to the user.
    pub fn display(&self) -> Option<String> {
        self.value().map(|value| format_value(self.unit, value))
    }
}

/// The device the application is talking to.
///
/// The shell shows the connection view when the device is
/// [`DeviceState::Disconnected`]; every other screen assumes the controls
/// of a connected card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceState {
    /// The card was found and its controls were read.
    Connected(CardInfo),
    /// No card of the ZG01 driver exists.
    Disconnected,
}

impl DeviceState {
    /// Whether the device is there.
    pub fn is_connected(&self) -> bool {
        matches!(self, DeviceState::Connected(_))
    }

    /// The card, when the device is connected.
    pub fn card(&self) -> Option<&CardInfo> {
        match self {
            DeviceState::Connected(card) => Some(card),
            DeviceState::Disconnected => None,
        }
    }
}

/// What the shell knows at one moment.
///
/// The state is pure data: it holds the connection state, the controls the
/// GUI read from the driver cache and the curated layout table. It has no
/// GTK dependency, so it can be built and tested without a display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppState {
    /// Whether the card is connected.
    pub device: DeviceState,
    /// Every control of the card, in the order the driver publishes them.
    pub controls: Vec<Control>,
    /// The curated layout table.
    pub layout: Layout,
    /// The last MIC state read result.
    pub state_read: StateRead,
    /// The values a Reset invalidated that are still unknown.
    ///
    /// The driver cache keeps the pre-reset values until each control is
    /// written again, so a reload must not turn these back into known
    /// values. A successful state read refreshes the direct MIC fields and
    /// a successful write refreshes one control; both remove the name.
    reset_invalidated: BTreeSet<String>,
}

impl AppState {
    /// A state that holds `controls` of a connected card.
    pub fn new(device: DeviceState, controls: Vec<Control>, layout: Layout) -> Self {
        AppState {
            device,
            controls,
            layout,
            state_read: StateRead::Unknown,
            reset_invalidated: BTreeSet::new(),
        }
    }

    /// A state installed after a card reload.
    ///
    /// A reload reads the driver cache, which a Reset does not refresh:
    /// the values `previous` invalidated stay unknown after
    /// [`Self::apply_readback`] until a successful write makes each one
    /// known again. Ordinary reloads carry no invalidation and keep the
    /// cached values, including the transformed MIC fields the state dump
    /// cannot refresh.
    pub fn reloaded(
        device: DeviceState,
        controls: Vec<Control>,
        layout: Layout,
        previous: &mut AppState,
    ) -> Self {
        let mut state = AppState::new(device, controls, layout);
        state.reset_invalidated = std::mem::take(&mut previous.reset_invalidated);
        state
    }

    /// Marks the values a Reset invalidated unknown again.
    fn reapply_reset_invalidation(&mut self) {
        for control in self.controls.iter_mut() {
            if self.reset_invalidated.contains(control.name()) {
                control.value = ControlValue::Unknown;
            }
        }
    }

    /// The control named `name`.
    pub fn control(&self, name: &str) -> Option<&Control> {
        self.controls.iter().find(|control| control.name() == name)
    }

    /// The control named `name`, mutably.
    pub fn control_mut(&mut self, name: &str) -> Option<&mut Control> {
        self.controls
            .iter_mut()
            .find(|control| control.name() == name)
    }

    /// Whether the values can be saved to the device.
    ///
    /// Save is blocked while the device is disconnected. It is allowed
    /// after a successful MIC state read, even though the transformed MIC
    /// fields and the non-MIC sections come from the driver cache: Save
    /// persists the device state and does not write those cached values.
    /// Before a read, the old all-known rule still applies, so a Reset
    /// cannot save the cache defaults and a fully applied preset can still
    /// be saved.
    pub fn can_save(&self) -> bool {
        self.device.is_connected()
            && (self.state_read.is_known()
                || self
                    .controls
                    .iter()
                    .filter(|control| is_writable_value(control))
                    .all(|control| control.value != ControlValue::Unknown))
    }

    /// Marks the device as gone without forgetting the values it last held.
    ///
    /// The shell keeps showing the last known values, disabled, until the
    /// card comes back and its controls are read again.
    pub fn disconnect(&mut self) {
        self.device = DeviceState::Disconnected;
    }

    /// Records that the device now holds `value` for the control `name`.
    ///
    /// This is what a successful write means: the driver cache and the
    /// device agree on that one value again. The other values keep their
    /// state, so Save stays blocked until every writable value is known.
    /// A Reset invalidation ends for this control, so a later reload keeps
    /// the value instead of hiding it again. Returns `false` when the
    /// device publishes no such control.
    pub fn mark_known(&mut self, name: &str, value: i64) -> bool {
        if self.control(name).is_none() {
            return false;
        }
        if let Some(control) = self.control_mut(name) {
            control.value = ControlValue::Known(value);
        }
        self.reset_invalidated.remove(name);
        true
    }

    /// Forgets the value of every writable value.
    ///
    /// This is what a reset does: `zg01_param_reset()` reloads the device
    /// without updating the driver cache, so the values the GUI shows no
    /// longer describe the device. Read-only controls and write-only
    /// actions are left alone. The names are remembered so a reload, which
    /// reads that stale cache, cannot claim these values are known again.
    pub fn mark_all_unknown(&mut self) {
        for control in self
            .controls
            .iter_mut()
            .filter(|control| is_writable_value(control))
        {
            control.value = ControlValue::Unknown;
            self.reset_invalidated.insert(control.name().to_string());
        }
        self.state_read = StateRead::Unknown;
    }

    /// Forgets one control value, when the device publishes it.
    fn mark_unknown(&mut self, name: &str) {
        if let Some(control) = self.control_mut(name) {
            control.value = ControlValue::Unknown;
        }
    }

    /// Forgets the direct MIC fields.
    ///
    /// Used when a Read failed or timed out: the values that came from the
    /// dump no longer describe the device. The transformed fields keep the
    /// driver cache, which a failed read does not invalidate.
    pub fn mark_mic_unknown(&mut self) {
        for (name, _) in DIRECT_MIC {
            self.mark_unknown(name);
        }
    }

    /// Applies the result of a MIC state read to the model.
    ///
    /// A successful read (`State Read Status == 1`) makes the direct MIC
    /// fields known from the values the driver just read; the fields the
    /// dump stores as DSP values keep the driver cache, never shown as if
    /// they came from the device. A value a Reset invalidated stays unknown
    /// unless this read refreshed it. A failed or pending read marks only
    /// the direct MIC chain unknown. The Mixage and Avancé sections are
    /// outside this scope and keep their cached values; the Mixage screen
    /// carries a notice.
    pub fn apply_readback(&mut self) {
        let status = self
            .control(STATE_READ_STATUS)
            .and_then(|control| control.value());
        let known = self
            .control(STATE_READ_KNOWN)
            .and_then(|control| control.value())
            .unwrap_or(0) as u32;

        if status == Some(1) {
            for (name, bit) in DIRECT_MIC {
                let value = if known & bit != 0 {
                    self.control(name).and_then(|control| control.value())
                } else {
                    None
                };
                match value {
                    Some(value) => {
                        self.mark_known(name, value);
                    }
                    None => self.mark_unknown(name),
                }
            }
            self.state_read = StateRead::Known { known };
        } else {
            self.mark_mic_unknown();
            self.state_read = match status {
                Some(0) => StateRead::Pending,
                Some(value) if value < 0 => StateRead::Failed,
                _ => StateRead::Unknown,
            };
        }
        // The direct fields the read just refreshed left the invalidation
        // set through `mark_known`; the ones the dump cannot refresh must
        // not come back from the driver cache on this reload.
        self.reapply_reset_invalidation();
    }
}

/// Applies a `Reset to ZG01` action and forgets every displayed value.
///
/// The action is written through `writer`: the pending coalesced values
/// are flushed first, then [`RESET_ACTION`] `= 1` is written and awaited.
/// The order matters — the driver reloads its persisted settings on Reset,
/// so a fader write flushed afterwards would silently undo the reset, and
/// a post-reset fader flushed before it would be overwritten.
///
/// Whatever the outcome, every writable value is then marked unknown: the
/// driver cache no longer describes the device, and a failed action must
/// not leave the GUI claiming to know what the device holds. The error is
/// returned for the caller to report.
pub fn apply_reset(state: &mut AppState, writer: &ControlWriter) -> Result<(), String> {
    let result = reset_device(writer);
    state.mark_all_unknown();
    result
}

/// Performs the `Reset to ZG01` action without touching the model.
///
/// The GUI runs this on a worker thread: the flush and the awaited action
/// can block on the USB device, and the GTK loop must not wait for them.
/// The caller marks the values unknown when the completion arrives.
pub fn reset_device(writer: &ControlWriter) -> Result<(), String> {
    let flushed = writer.flush_and_wait();
    let reset = writer.set_and_wait(RESET_ACTION, 1);
    match (flushed, reset) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(err), _) => Err(format!("could not flush the pending writes: {err}")),
        (Ok(()), Err(err)) => Err(err),
    }
}

/// Runs [`reset_device`] on a worker and returns its completion channel.
///
/// The function returns as soon as the worker is started; a delayed device
/// never blocks the caller. The receiver yields exactly one result.
pub fn spawn_reset(writer: ControlWriter) -> Receiver<Result<(), String>> {
    let (results, receiver) = mpsc::channel();
    let thread_results = results.clone();
    let spawned = std::thread::Builder::new()
        .name("zg01-reset".to_string())
        .spawn(move || {
            let _ = thread_results.send(reset_device(&writer));
        });
    if let Err(err) = spawned {
        let _ = results.send(Err(format!("could not start the reset thread: {err}")));
    }
    receiver
}

/// Whether a control holds a value the GUI must know to save.
fn is_writable_value(control: &Control) -> bool {
    control.is_writable() && !control.is_action()
}

/// Formats a value the way `zgctl` prints it.
///
/// The formats are the ones of `tools/zgctl.c`: `%+.1f dB` for the gains,
/// `%.2f kHz` for the frequencies above 1 kHz, `%.2f` for Q, `on`/`off`
/// for the switches and plain integers for the raw steps.
pub fn format_value(unit: Unit, value: i64) -> String {
    match unit {
        Unit::Bool => {
            if value != 0 {
                "on".to_string()
            } else {
                "off".to_string()
            }
        }
        Unit::Db => format!("{:+.1} dB", value as f64 / 10.0),
        Unit::Hz => {
            if value >= 1000 {
                format!("{:.2} kHz", value as f64 / 1000.0)
            } else {
                format!("{value} Hz")
            }
        }
        Unit::Q => format!("{:.2}", value as f64 / 100.0),
        Unit::Steps | Unit::Raw => format!("{value}"),
        Unit::Percent => format!("{value}%"),
    }
}

#[cfg(test)]
mod tests {
    use super::{format_value, Control, ControlValue, Unit};
    use crate::alsa::{ControlInfo, ControlKind};

    fn info(name: &str) -> ControlInfo {
        ControlInfo {
            name: name.to_string(),
            kind: ControlKind::Integer,
            min: 0,
            max: 100,
            step: 0,
            count: 1,
            writable: true,
        }
    }

    #[test]
    fn formats_the_gains_with_the_sign() {
        assert_eq!(format_value(Unit::Db, 0), "+0.0 dB");
        assert_eq!(format_value(Unit::Db, 60), "+6.0 dB");
        assert_eq!(format_value(Unit::Db, -25), "-2.5 dB");
    }

    #[test]
    fn formats_the_frequencies_in_kilohertz_above_one_kilohertz() {
        assert_eq!(format_value(Unit::Hz, 20), "20 Hz");
        assert_eq!(format_value(Unit::Hz, 999), "999 Hz");
        assert_eq!(format_value(Unit::Hz, 1000), "1.00 kHz");
        assert_eq!(format_value(Unit::Hz, 7500), "7.50 kHz");
        assert_eq!(format_value(Unit::Hz, 20000), "20.00 kHz");
    }

    #[test]
    fn formats_the_switches() {
        assert_eq!(format_value(Unit::Bool, 0), "off");
        assert_eq!(format_value(Unit::Bool, 1), "on");
        assert_eq!(format_value(Unit::Bool, 42), "on");
    }

    #[test]
    fn a_control_knows_its_value() {
        let control = Control::new(info("Gate"), 50, Unit::Steps);
        assert_eq!(control.value(), Some(50));
        assert_eq!(control.display().as_deref(), Some("50"));
        assert_eq!(control.name(), "Gate");
        assert!(control.is_writable());
    }

    #[test]
    fn an_unknown_control_has_no_value() {
        let control = Control::unknown(info("Gate"), Unit::Steps);
        assert_eq!(control.value, ControlValue::Unknown);
        assert_eq!(control.value(), None);
        assert_eq!(control.display(), None);
    }
}
