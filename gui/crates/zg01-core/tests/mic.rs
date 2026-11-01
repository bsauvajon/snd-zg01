//! The Micro screen model: three dynamics blocks and the four-band EQ.

mod common;

use common::controls_with_mic;
use zg01_core::mic::MicModel;

#[test]
fn builds_three_faders_and_four_bands() {
    let model = MicModel::from_controls(&controls_with_mic());
    assert_eq!(model.gate.value.as_deref(), Some("Gate"));
    assert_eq!(model.comp.value.as_deref(), Some("Compressor"));
    assert_eq!(model.limiter.value.as_deref(), Some("Limiter"));
    assert_eq!(model.eq.bands.len(), 4);
    assert_eq!(model.eq.bands[0].gain, "EQ Band 1 Gain");
    assert_eq!(model.eq.bands[3].gain, "EQ Band 4 Gain");
}

#[test]
fn out_gains_are_advanced_not_faders() {
    let model = MicModel::from_controls(&controls_with_mic());
    assert!(model
        .comp
        .advanced
        .iter()
        .any(|n| n == "Compressor Out Gain"));
    assert!(model
        .limiter
        .advanced
        .iter()
        .any(|n| n == "Limiter Out Gain"));
}

#[test]
fn the_enable_switches_are_wired() {
    let model = MicModel::from_controls(&controls_with_mic());
    assert_eq!(model.gate.toggle.as_deref(), Some("Gate Capture Switch"));
    assert_eq!(
        model.comp.toggle.as_deref(),
        Some("Compressor Capture Switch")
    );
    assert_eq!(
        model.limiter.toggle.as_deref(),
        Some("Limiter Capture Switch")
    );
    assert_eq!(model.eq.toggle.as_deref(), Some("EQ Capture Switch"));
}

#[test]
fn the_low_and_high_shapes_belong_to_the_outer_bands() {
    let model = MicModel::from_controls(&controls_with_mic());
    assert_eq!(model.eq.bands[0].shape.as_deref(), Some("EQ Low Shape"));
    assert_eq!(model.eq.bands[1].shape, None);
    assert_eq!(model.eq.bands[2].shape, None);
    assert_eq!(model.eq.bands[3].shape.as_deref(), Some("EQ High Shape"));
}

#[test]
fn the_advanced_list_is_in_control_order() {
    let model = MicModel::from_controls(&controls_with_mic());
    assert_eq!(
        model.gate.advanced,
        [
            "Gate Threshold",
            "Gate Range",
            "Gate Attack",
            "Gate Hold",
            "Gate Decay",
        ]
    );
    assert_eq!(
        model.comp.advanced,
        [
            "Compressor Threshold",
            "Compressor Ratio",
            "Compressor Attack",
            "Compressor Release",
            "Compressor Knee",
            "Compressor Out Gain",
        ]
    );
    assert_eq!(
        model.limiter.advanced,
        [
            "Limiter Threshold",
            "Limiter Attack",
            "Limiter Release",
            "Limiter Knee",
            "Limiter Out Gain",
        ]
    );
}

#[test]
fn missing_controls_are_hidden() {
    let mut controls = controls_with_mic();
    controls.retain(|c| {
        c.info.name != "Gate Attack"
            && c.info.name != "Compressor Out Gain"
            && c.info.name != "EQ High Shape"
            && c.info.name != "Limiter Capture Switch"
    });
    let model = MicModel::from_controls(&controls);

    assert!(
        !model.gate.advanced.iter().any(|n| n == "Gate Attack"),
        "a missing sub-parameter is not listed"
    );
    assert!(model.gate.advanced.iter().any(|n| n == "Gate Threshold"));
    assert!(
        !model
            .comp
            .advanced
            .iter()
            .any(|n| n == "Compressor Out Gain"),
        "a missing out gain is not listed"
    );
    assert_eq!(model.eq.bands[3].shape, None);
    assert_eq!(model.limiter.toggle, None);
    assert_eq!(model.limiter.value.as_deref(), Some("Limiter"));
    assert_eq!(model.eq.bands.len(), 4);
}

#[test]
fn a_control_name_the_driver_does_not_publish_is_never_invented() {
    let model = MicModel::from_controls(&common::controls(&[]));
    assert_eq!(model.gate.toggle, None);
    assert_eq!(model.gate.value, None);
    assert!(model.gate.advanced.is_empty());
    assert_eq!(model.eq.toggle, None);
    assert!(
        model.eq.bands.is_empty(),
        "a band is only built from controls that exist"
    );
}
