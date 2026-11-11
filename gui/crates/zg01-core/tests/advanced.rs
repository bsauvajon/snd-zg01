//! The Avancé screen model: every control the curated screens leave over.

mod common;

use common::{all_controls, controls};
use zg01_core::advanced::AdvancedModel;
use zg01_core::layout::{load_layout_str, Layout, LAYOUT_TOML};
use zg01_core::mic::MicModel;
use zg01_core::mixage::MixageModel;
use zg01_core::model::ControlValue;

fn layout() -> Layout {
    load_layout_str(LAYOUT_TOML).expect("the compiled layout table parses")
}

/// The curated models of the real control surface.
fn curated() -> (MixageModel, MicModel) {
    let controls = all_controls();
    (
        MixageModel::from_controls(&controls),
        MicModel::from_controls(&controls),
    )
}

/// The Avancé table of the real control surface.
fn model() -> AdvancedModel {
    let (mixage, mic) = curated();
    AdvancedModel::from_controls(&all_controls(), &layout(), &mixage, &mic)
}

#[test]
fn excludes_curated_controls() {
    let model = model();
    assert!(!model.rows.iter().any(|row| row.name == "Gate"));
    assert!(model
        .rows
        .iter()
        .any(|row| row.name == "P48V Capture Switch"));
}

#[test]
fn excludes_every_control_a_curated_screen_displays() {
    let model = model();

    // Mixage: the three sends and the stream outputs.
    for name in [
        "Streaming Mic Level Steps",
        "Streaming Voice Mute Steps",
        "Streaming Game FX Steps",
        "Streaming Monitor Steps",
    ] {
        assert!(
            !model.rows.iter().any(|row| row.name == name),
            "{name:?} is displayed by Mixage and must not appear twice"
        );
    }

    // Micro: the block switches and values, the sub-parameters and the EQ.
    for name in [
        "Gate Capture Switch",
        "Gate Threshold",
        "Compressor Out Gain",
        "EQ Band 1 Gain",
        "EQ Low Shape",
    ] {
        assert!(
            !model.rows.iter().any(|row| row.name == name),
            "{name:?} is displayed by Micro and must not appear twice"
        );
    }

    // The layout table claims the same controls as Micro; that must not
    // leave a gap where a control is claimed by neither.
    assert!(model
        .rows
        .iter()
        .any(|row| row.name == "Mic EQ Band 1 Switch Steps"));
}

#[test]
fn raw_flag_comes_from_provenance() {
    // The exclusion of the streaming send is pinned by the test above.
    // Build the table without the curated claims so the provenance rule is
    // visible on the exact controls the brief names.
    let model = AdvancedModel::from_controls(
        &all_controls(),
        &Layout {
            screens: Vec::new(),
        },
        &MixageModel::from_controls(&[]),
        &MicModel::from_controls(&[]),
    );

    let raw = model
        .rows
        .iter()
        .find(|row| row.name == "Game EQ Total High Gain Steps")
        .expect("a raw address is a row");
    assert!(raw.raw, "provenance == \"raw\" is the raw marker");

    let steps = model
        .rows
        .iter()
        .find(|row| row.name == "Streaming Mic Level Steps")
        .expect("a vendor-model control is a row");
    assert!(!steps.raw, "a vendor-model name ending in Steps is not raw");
}

#[test]
fn raw_is_read_from_the_control_map_not_from_the_name() {
    let model = model();

    // A raw address whose name does not end in `Steps`.
    let mode = model
        .rows
        .iter()
        .find(|row| row.name == "Streaming Mode Steps")
        .expect("Streaming Mode Steps is in Avancé");
    assert!(mode.raw);

    // A non-raw address whose name does end in `Steps`.
    let switch = model
        .rows
        .iter()
        .find(|row| row.name == "Mic EQ Band 1 Switch Steps")
        .expect("the per-band switches are not curated in v1");
    assert!(!switch.raw);
}

#[test]
fn search_filters_by_name() {
    let model = model();
    assert_eq!(model.filter("p48").len(), 1);
}

#[test]
fn search_is_case_insensitive_and_partial() {
    let model = model();
    assert_eq!(model.filter("P48").len(), 1);
    let capture: Vec<&str> = model
        .filter("CAPTURE SWITCH")
        .iter()
        .map(|row| row.name.as_str())
        .collect();
    assert_eq!(capture, ["P48V Capture Switch", "Mute Capture Switch"]);
    assert_eq!(model.filter("band 1").len(), model.filter("BAND 1").len());
}

#[test]
fn an_empty_search_returns_every_row() {
    let model = model();
    assert_eq!(model.filter("").len(), model.rows.len());
    assert_eq!(model.filter("   ").len(), model.rows.len());
}

#[test]
fn a_search_without_a_match_returns_nothing() {
    let model = model();
    assert!(model.filter("no such control").is_empty());
}

#[test]
fn the_rows_keep_the_driver_order() {
    let controls = all_controls();
    let (mixage, mic) = curated();
    let model = AdvancedModel::from_controls(&controls, &layout(), &mixage, &mic);

    let shown: Vec<&str> = model.rows.iter().map(|row| row.name.as_str()).collect();
    let expected: Vec<&str> = controls
        .iter()
        .map(|control| control.name())
        .filter(|name| shown.contains(name))
        .collect();
    assert_eq!(shown, expected);
}

#[test]
fn an_unknown_value_is_not_shown_as_zero() {
    let mut controls = all_controls();
    let control = controls
        .iter_mut()
        .find(|control| control.name() == "Game EQ Band 1 Gain")
        .expect("the game EQ bands are in Avancé");
    control.value = ControlValue::Unknown;

    let (mixage, mic) = curated();
    let model = AdvancedModel::from_controls(&controls, &layout(), &mixage, &mic);
    let row = model
        .rows
        .iter()
        .find(|row| row.name == "Game EQ Band 1 Gain")
        .expect("the game EQ band is a row");
    assert_eq!(row.value, ControlValue::Unknown);
}

#[test]
fn read_only_controls_are_listed_but_not_writable() {
    let model = model();
    let status = model
        .rows
        .iter()
        .find(|row| row.name == "Mic Level Status")
        .expect("a read-only status is still an escape-hatch row");
    assert!(!status.writable);
}

#[test]
fn the_device_actions_are_not_rows() {
    let controls = controls(&["P48V Capture Switch", "Save to ZG01", "Reset to ZG01"]);
    let model = AdvancedModel::from_controls(
        &controls,
        &layout(),
        &MixageModel::from_controls(&[]),
        &MicModel::from_controls(&[]),
    );

    let names: Vec<&str> = model.rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, ["P48V Capture Switch"]);
}

#[test]
fn the_state_readback_controls_are_not_rows() {
    let controls = controls(&[
        "P48V Capture Switch",
        zg01_core::model::STATE_READ_ACTION,
        zg01_core::model::STATE_READ_STATUS,
        zg01_core::model::STATE_READ_KNOWN,
    ]);
    let model = AdvancedModel::from_controls(
        &controls,
        &layout(),
        &MixageModel::from_controls(&[]),
        &MicModel::from_controls(&[]),
    );

    let names: Vec<&str> = model.rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, ["P48V Capture Switch"]);
}

#[test]
fn the_meters_are_listed_read_only_and_unknown() {
    let mut surface = all_controls();
    surface.extend(controls(&zg01_core::meters::METER_CONTROLS));
    for control in surface
        .iter_mut()
        .filter(|control| zg01_core::meters::METER_CONTROLS.contains(&control.name()))
    {
        control.value = ControlValue::Unknown;
        control.info.writable = false;
    }

    let (mixage, mic) = curated();
    let model = AdvancedModel::from_controls(&surface, &layout(), &mixage, &mic);
    for name in zg01_core::meters::METER_CONTROLS {
        let row = model
            .rows
            .iter()
            .find(|row| row.name == name)
            .expect("a meter is an escape-hatch row");
        assert!(!row.writable);
        assert_eq!(
            row.value,
            ControlValue::Unknown,
            "a meter that returns no data is unknown, not zero"
        );
    }
}

#[test]
fn every_row_carries_its_group() {
    let model = model();
    let row = |name: &str| {
        model
            .rows
            .iter()
            .find(|row| row.name == name)
            .map(|row| row.group.as_str())
            .expect("the row exists")
    };
    assert_eq!(row("P48V Capture Switch"), "MIC");
    assert_eq!(row("Streaming Mode Steps"), "STREAMING");
    assert_eq!(row("Game EQ Band 1 Gain"), "GAME");
    assert_eq!(row("Monitor Select Steps"), "MONITOR");
}
