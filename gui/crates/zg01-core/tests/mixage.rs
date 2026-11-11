//! The Mixage screen model: three stream sends and the stream output.

mod common;

use common::{controls, controls_with_mixage};
use zg01_core::mixage::{LinuxMeterId, MeterId, MixageModel};

#[test]
fn builds_three_stream_strips() {
    let model = MixageModel::from_controls(&controls_with_mixage());
    assert_eq!(model.strips.len(), 3);
    assert_eq!(
        model.strips[0].level.as_deref(),
        Some("Streaming Mic Level Steps")
    );
    assert_eq!(
        model.strips[1].level.as_deref(),
        Some("Streaming Voice Level Steps")
    );
    assert_eq!(
        model.strips[2].level.as_deref(),
        Some("Streaming Game Level Steps")
    );
}

#[test]
fn missing_level_hides_the_fader_not_the_strip() {
    let mut controls = controls_with_mixage();
    controls.retain(|c| c.info.name != "Streaming Voice Level Steps");
    let model = MixageModel::from_controls(&controls);
    assert_eq!(model.strips[1].level, None);
    assert_eq!(model.strips.len(), 3);
}

#[test]
fn stream_output_uses_the_three_switches() {
    let model = MixageModel::from_controls(&controls_with_mixage());
    assert_eq!(model.output.usb.as_deref(), Some("Streaming USB Out"));
    assert_eq!(model.output.hdmi.as_deref(), Some("Streaming HDMI Out"));
    assert_eq!(
        model.output.monitor.as_deref(),
        Some("Streaming Monitor Steps")
    );
}

#[test]
fn game_strip_has_its_card_meter() {
    let model = MixageModel::from_controls(&controls_with_mixage());
    assert_eq!(model.strips[2].meter, Some(MeterId::GameMix));
    assert_eq!(model.output.meter, Some(MeterId::StreamMix));
}

#[test]
fn strips_map_to_their_linux_meters() {
    let model = MixageModel::from_controls(&controls_with_mixage());
    assert_eq!(model.strips[0].linux_meter, Some(LinuxMeterId::MicIn));
    assert_eq!(model.strips[1].linux_meter, Some(LinuxMeterId::VoiceOut));
    assert_eq!(model.strips[2].linux_meter, Some(LinuxMeterId::GameOut));
    assert_eq!(model.output.linux_meter, None);
}

#[test]
fn linux_meter_ids_have_stable_slots() {
    assert_eq!(LinuxMeterId::MicIn.index(), 0);
    assert_eq!(LinuxMeterId::VoiceOut.index(), 1);
    assert_eq!(LinuxMeterId::GameOut.index(), 2);
    assert_eq!(LinuxMeterId::ALL.len(), 3);
}

#[test]
fn the_stream_output_also_finds_the_driver_suffixes() {
    // The spec names the switches without the driver's ` Steps` suffix;
    // the card only publishes `Streaming USB Out Steps` and
    // `Streaming HDMI Out Steps`, so both spellings are accepted and the
    // name that was actually found is kept for the writes.
    let controls = controls(&[
        "Streaming USB Out Steps",
        "Streaming HDMI Out Steps",
        "Streaming Monitor Steps",
    ]);
    let model = MixageModel::from_controls(&controls);
    assert_eq!(model.output.usb.as_deref(), Some("Streaming USB Out Steps"));
    assert_eq!(
        model.output.hdmi.as_deref(),
        Some("Streaming HDMI Out Steps")
    );
    assert_eq!(
        model.output.monitor.as_deref(),
        Some("Streaming Monitor Steps")
    );
}
