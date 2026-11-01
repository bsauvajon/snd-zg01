//! Core library for the Yamaha ZG01 control GUI.
//!
//! This crate will host the ALSA control surface, the ZG01 model description
//! and the `.zgpreset` parser. Task 0 only wires up the crate skeleton so the
//! workspace builds and the GUI can start.

/// The ALSA control surface: card discovery and the control backend.
pub mod alsa;

/// The curated layout table: which control goes on which screen.
pub mod layout;

/// The control model: what a control is and how its value reads.
pub mod model;

/// The conversions between the raw device values and the user units.
pub mod units;

/// Placeholder identifier for the ALSA PCM device handled by the GUI.
///
/// The real device is resolved at runtime in a later task.
pub const DEFAULT_DEVICE: &str = "hw:CARD=zg01,DEV=0";

#[cfg(test)]
mod tests {
    use super::DEFAULT_DEVICE;

    #[test]
    fn default_device_is_not_empty() {
        assert!(!DEFAULT_DEVICE.is_empty());
    }
}
