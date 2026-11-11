mod common;

use zg01_core::alsa::ControlInfo;
use zg01_core::layout::{load_layout_str, validate_layout, LayoutWarning, Severity, LAYOUT_TOML};
use zg01_core::model::{format_value, Control, Unit};
use zg01_core::units::{raw_to_user, user_to_raw, Conv};

/// The metadata of the controls the layout table claims.
fn infos(controls: Vec<Control>) -> Vec<ControlInfo> {
    controls.into_iter().map(|control| control.info).collect()
}

/// The controls the Micro screen of the layout table claims.
const MIC_CONTROLS: &[&str] = &[
    "Gate Capture Switch",
    "Gate",
    "Compressor Capture Switch",
    "Compressor",
    "Limiter Capture Switch",
    "Limiter",
    "EQ Capture Switch",
    "EQ Low Shape",
    "EQ High Shape",
    "EQ Band 1 Gain",
    "EQ Band 1 Frequency",
    "EQ Band 1 Q",
    "EQ Band 2 Gain",
    "EQ Band 2 Frequency",
    "EQ Band 2 Q",
    "EQ Band 3 Gain",
    "EQ Band 3 Frequency",
    "EQ Band 3 Q",
    "EQ Band 4 Gain",
    "EQ Band 4 Frequency",
    "EQ Band 4 Q",
];

#[test]
fn formats_units_like_zgctl() {
    assert_eq!(format_value(Unit::Db, 60), "+6.0 dB");
    assert_eq!(format_value(Unit::Db, -25), "-2.5 dB");
    assert_eq!(format_value(Unit::Hz, 7500), "7.50 kHz");
    assert_eq!(format_value(Unit::Q, 120), "1.20");
    assert_eq!(format_value(Unit::Steps, 103), "103");
    assert_eq!(format_value(Unit::Bool, 1), "on");
}

#[test]
fn matches_driver_conversions() {
    assert_eq!(raw_to_user(Conv::Gain, 180), 0);
    assert_eq!(user_to_raw(Conv::Gain, 0), 180);
    assert_eq!(raw_to_user(Conv::Gain, 240), 60);
    assert_eq!(raw_to_user(Conv::Freq, 1000), 1000);
    assert_eq!(raw_to_user(Conv::Freq, 20000), 20000);
    assert_eq!(raw_to_user(Conv::Q, 12), 100);
    assert_eq!(user_to_raw(Conv::Q, 100), 12);
    assert_eq!(user_to_raw(Conv::Q, 1600), 60);
    assert_eq!(raw_to_user(Conv::Id, 42), 42);
}

#[test]
fn formats_the_small_units_like_zgctl() {
    assert_eq!(format_value(Unit::Bool, 0), "off");
    assert_eq!(format_value(Unit::Hz, 500), "500 Hz");
    assert_eq!(format_value(Unit::Hz, 1000), "1.00 kHz");
    assert_eq!(format_value(Unit::Raw, 42), "42");
    assert_eq!(format_value(Unit::Percent, 50), "50%");
}

#[test]
fn the_identity_conversion_is_the_identity() {
    assert_eq!(user_to_raw(Conv::Id, 42), 42);
    assert_eq!(user_to_raw(Conv::Id, -7), -7);
}

#[test]
fn the_compiled_layout_parses() {
    let layout = load_layout_str(LAYOUT_TOML).expect("the layout table parses");
    let screen = layout
        .screens
        .iter()
        .find(|screen| screen.id == "mic")
        .expect("the Micro screen is in the table");
    assert_eq!(screen.title, "Micro");

    let ids: Vec<&str> = screen
        .groups
        .iter()
        .map(|group| group.id.as_str())
        .collect();
    assert_eq!(ids, ["gate", "comp", "limit", "eq"]);
}

#[test]
fn the_layout_table_names_the_drivers_controls() {
    let layout = load_layout_str(LAYOUT_TOML).expect("the layout table parses");
    let controls = infos(common::controls(MIC_CONTROLS));
    let warnings = validate_layout(&layout, &controls);
    assert!(
        warnings.is_empty(),
        "the layout table and the driver disagree: {warnings:?}"
    );
}

#[test]
fn a_missing_control_is_a_warning() {
    let layout = load_layout_str(
        r#"
[[screen]]
id = "mic"
title = "Micro"

[[screen.group]]
id = "gate"
title = "GATE"
controls = [
  { name = "Gate", widget = "fader", unit = "steps" },
]
"#,
    )
    .expect("the layout table parses");
    let warnings = validate_layout(&layout, &infos(common::controls(&["Gate Threshold"])));
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].severity(), Severity::Warning);
    assert!(matches!(&warnings[0], LayoutWarning::MissingControl { name, .. } if name == "Gate"));
}

#[test]
fn a_duplicate_assignment_is_an_error() {
    let layout = load_layout_str(
        r#"
[[screen]]
id = "mic"
title = "Micro"

[[screen.group]]
id = "gate"
title = "GATE"
controls = [
  { name = "Gate", widget = "fader", unit = "steps" },
]

[[screen.group]]
id = "comp"
title = "COMP"
controls = [
  { name = "Gate", widget = "fader", unit = "steps" },
]
"#,
    )
    .expect("the layout table parses");
    let warnings = validate_layout(&layout, &infos(common::controls(&["Gate"])));
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].severity(), Severity::Error);
    assert!(matches!(&warnings[0], LayoutWarning::DuplicateControl { name, .. } if name == "Gate"));
}

#[test]
fn a_broken_layout_table_is_an_error() {
    assert!(load_layout_str("this is not TOML").is_err());
    assert!(load_layout_str(
        r#"
[[screen]]
id = "mic"
title = "Micro"

[[screen.group]]
id = "gate"
title = "GATE"
controls = [
  { name = "Gate", widget = "dial" },
]
"#
    )
    .is_err());
}
