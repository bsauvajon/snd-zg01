//! The Mixage screen model: the three stream sends and the stream output.
//!
//! The model is pure data, like [`crate::model::AppState`]: it names the
//! ALSA controls the screen shows and never touches GTK. A control the
//! driver does not publish becomes `None`, which hides its widget; a strip
//! whose level is missing is still built, so its meter keeps working.
//!
//! The three send levels are `Streaming ... Level Steps`, 0–127 device
//! steps with no established dB mapping: the screen shows the step value,
//! it never invents a dB readout.

use crate::meters::METER_CONTROLS;
use crate::model::Control;

/// One of the six meters the driver registers.
///
/// The variants are the six decoded meter controls of
/// [`crate::meters::METER_CONTROLS`]; `index` is the slot of the readings
/// in a [`crate::meters::MeterSnapshot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MeterId {
    /// `Meter Mic In`.
    MicIn,
    /// `Meter Mic Out`.
    MicOut,
    /// `Meter PC In`.
    PcIn,
    /// `Meter HDMI In`.
    HdmiIn,
    /// `Meter Game Mix`.
    GameMix,
    /// `Meter Stream Mix`.
    StreamMix,
}

impl MeterId {
    /// Every meter, in snapshot-slot order.
    pub const ALL: [MeterId; 6] = [
        MeterId::MicIn,
        MeterId::MicOut,
        MeterId::PcIn,
        MeterId::HdmiIn,
        MeterId::GameMix,
        MeterId::StreamMix,
    ];

    /// The slot of this meter in a [`crate::meters::MeterSnapshot`].
    pub const fn index(self) -> usize {
        match self {
            MeterId::MicIn => 0,
            MeterId::MicOut => 1,
            MeterId::PcIn => 2,
            MeterId::HdmiIn => 3,
            MeterId::GameMix => 4,
            MeterId::StreamMix => 5,
        }
    }

    /// How many channels the group publishes.
    pub const fn channel_count(self) -> usize {
        match self {
            MeterId::MicIn | MeterId::MicOut => 1,
            MeterId::PcIn | MeterId::HdmiIn | MeterId::GameMix | MeterId::StreamMix => 8,
        }
    }

    /// The ALSA name of the meter control.
    pub fn control(self) -> &'static str {
        METER_CONTROLS[self.index()]
    }
}

/// The three stream sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripId {
    /// The microphone send.
    Mic,
    /// The voice (USB chat) send.
    Voice,
    /// The game send.
    Game,
}

/// One of the three Linux-side sources the GUI can meter.
///
/// The variants are the PipeWire sources documented in
/// `docs/superpowers/specs/2026-11-08-zg01-pipewire-meters-design.md`;
/// `index` is the slot of the readings in a `LinuxMetersSnapshot`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinuxMeterId {
    /// The `Voice In` source.
    MicIn,
    /// The `Voice Out` sink monitor.
    VoiceOut,
    /// The `Game Out` sink monitor.
    GameOut,
}

impl LinuxMeterId {
    /// Every Linux meter, in snapshot-slot order.
    pub const ALL: [LinuxMeterId; 3] = [
        LinuxMeterId::MicIn,
        LinuxMeterId::VoiceOut,
        LinuxMeterId::GameOut,
    ];

    /// The slot of this source in a `LinuxMetersSnapshot`.
    pub const fn index(self) -> usize {
        match self {
            LinuxMeterId::MicIn => 0,
            LinuxMeterId::VoiceOut => 1,
            LinuxMeterId::GameOut => 2,
        }
    }
}

/// One stream send strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripModel {
    /// Which send the strip is.
    pub id: StripId,
    /// The channel title shown above the strip.
    pub title: String,
    /// The ALSA name of the level fader, when the driver publishes it.
    pub level: Option<String>,
    /// The ALSA name of the mute switch, when the driver publishes it.
    pub mute: Option<String>,
    /// The ALSA name of the FX switch, when the driver publishes it.
    pub fx: Option<String>,
    /// The card meter of the strip.
    ///
    /// The VOICE strip uses `PcIn` on a legacy driver and `StreamMix` on
    /// a V2 driver; the other strips keep their group on both layouts.
    pub meter: Option<MeterId>,
    /// The Linux-side source of the strip, when it has one.
    pub linux_meter: Option<LinuxMeterId>,
}

/// The stream output block: no fader, two output switches, the monitor
/// switch and the stream mix meter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamOutputModel {
    /// The ALSA name of the USB output switch.
    pub usb: Option<String>,
    /// The ALSA name of the HDMI output switch.
    pub hdmi: Option<String>,
    /// The ALSA name of the monitor switch.
    pub monitor: Option<String>,
    /// The card meter of the stream mix.
    pub meter: Option<MeterId>,
    /// The Linux-side source of the output, when it has one.
    pub linux_meter: Option<LinuxMeterId>,
}

/// The whole Mixage screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MixageModel {
    /// The three send strips, in sidebar order.
    pub strips: Vec<StripModel>,
    /// The stream output block.
    pub output: StreamOutputModel,
}

impl MixageModel {
    /// Builds the screen from the controls the driver publishes.
    ///
    /// A missing control is `None`, never an invented name. The strips are
    /// always three: a strip without a level still has its meter.
    pub fn from_controls(controls: &[Control]) -> Self {
        let legacy = find(controls, &["Meter Game Mix", "Meter HDMI In"]).is_none();
        MixageModel {
            strips: vec![
                StripModel {
                    id: StripId::Mic,
                    title: "MIC".to_string(),
                    level: find(controls, &["Streaming Mic Level Steps"]),
                    mute: find(controls, &["Streaming Mic Mute Steps"]),
                    fx: find(controls, &["Streaming Mic FX Steps"]),
                    meter: Some(MeterId::MicIn),
                    linux_meter: Some(LinuxMeterId::MicIn),
                },
                StripModel {
                    id: StripId::Voice,
                    title: "VOICE".to_string(),
                    level: find(controls, &["Streaming Voice Level Steps"]),
                    mute: find(controls, &["Streaming Voice Mute Steps"]),
                    fx: find(controls, &["Streaming Voice FX Steps"]),
                    meter: Some(if legacy {
                        MeterId::PcIn
                    } else {
                        MeterId::StreamMix
                    }),
                    linux_meter: Some(LinuxMeterId::VoiceOut),
                },
                StripModel {
                    id: StripId::Game,
                    title: "GAME".to_string(),
                    level: find(controls, &["Streaming Game Level Steps"]),
                    mute: find(controls, &["Streaming Game Mute Steps"]),
                    fx: find(controls, &["Streaming Game FX Steps"]),
                    meter: Some(MeterId::GameMix),
                    linux_meter: Some(LinuxMeterId::GameOut),
                },
            ],
            output: StreamOutputModel {
                usb: find(controls, &["Streaming USB Out Steps", "Streaming USB Out"]),
                hdmi: find(
                    controls,
                    &["Streaming HDMI Out Steps", "Streaming HDMI Out"],
                ),
                monitor: find(controls, &["Streaming Monitor Steps"]),
                meter: Some(MeterId::StreamMix),
                linux_meter: None,
            },
        }
    }
}

/// The first of `names` the driver publishes, as the name it published.
///
/// Some documents name the output switches without the driver's ` Steps`
/// suffix; accepting both spellings keeps the model and the hardware in
/// agreement, and the screen writes the name that was actually found.
fn find(controls: &[Control], names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        controls
            .iter()
            .any(|control| control.name() == *name)
            .then(|| (*name).to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::MeterId;
    use crate::meters::METER_CONTROLS;

    #[test]
    fn the_meter_ids_match_the_snapshot_slots() {
        for meter in MeterId::ALL {
            assert_eq!(meter.control(), METER_CONTROLS[meter.index()]);
        }
    }
}
