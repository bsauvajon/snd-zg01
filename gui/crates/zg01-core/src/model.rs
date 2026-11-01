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

use serde::Deserialize;

use crate::alsa::ControlInfo;

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

    /// The value as it is shown to the user.
    pub fn display(&self) -> Option<String> {
        self.value().map(|value| format_value(self.unit, value))
    }
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
