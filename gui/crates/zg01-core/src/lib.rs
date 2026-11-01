//! Core library for the Yamaha ZG01 control GUI.
//!
//! The crate holds the ALSA control surface, the ZG01 model description,
//! the background workers that keep the GTK thread free of USB traffic,
//! and the `.zgpreset` parser.

/// The ALSA control surface: card discovery and the control backend.
pub mod alsa;

/// The device watcher: the connection transitions of the ZG01.
pub mod device;

/// The value/fraction geometry of the custom widgets.
pub mod geometry;

/// The curated layout table: which control goes on which screen.
pub mod layout;

/// The meter poller: the readings of the four decoded meters.
pub mod meters;

/// The Mixage screen model: the three stream sends and the stream output.
pub mod mixage;

/// The Micro screen model: the dynamics chain and the four-band EQ.
pub mod mic;

/// The control model: what a control is and how its value reads.
pub mod model;

/// The conversions between the raw device values and the user units.
pub mod units;

/// The coalescing control writer.
pub mod writer;

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
